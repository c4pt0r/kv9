use super::*;
use kv9_common::codec::encode_key;
use kv9_common::{
    BootstrapGeneration, ClusterId, KeyspaceId, NodeId, RegionId, RootDescriptor, RootVoter,
    TenantId,
};
use kv9_engine::checkpoint::FlushScope;
use kv9_engine::minio::{MinioConfig, MinioObjectStore};
use kv9_engine::{Engine, WriteBatch};
use raft::prelude::ConfState;
use std::sync::Arc;

struct Fixture {
    directory: PathBuf,
    guard: StoreGuard,
    identity: StoreIdentity,
    range: DataRange,
}
impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "kv9-joint-install-{}",
            StoreIncarnation::mint().unwrap()
        ));
        let mut guard = StoreGuard::lock(&directory).unwrap();
        let prepared = guard.prepare(NodeId(4)).unwrap();
        let root = RootDescriptor::new(
            ClusterId::mint().unwrap(),
            BootstrapGeneration::mint().unwrap(),
            vec![RootVoter {
                node_id: NodeId(4),
                addr: "127.0.0.1:19444".parse().unwrap(),
                store_incarnation: prepared.incarnation,
            }],
            b"test-only-fixture",
        )
        .unwrap();
        let identity = StoreIdentity::for_voter(&root, NodeId(4)).unwrap();
        guard.bind(&root, &identity).unwrap();
        let range = DataRange {
            root: root.digest(),
            creation: RootDigest::sha256(b"group-creation"),
            region: RegionId(100),
            keyspace: KeyspaceId(7),
            tenant: TenantId(1),
            conf_ver: 1,
            version: 1,
            start: b"a".to_vec(),
            end: b"z".to_vec(),
            sealed: false,
        };
        Self {
            directory,
            guard,
            identity,
            range,
        }
    }
    fn open(&self) -> JointInstaller<'_> {
        JointInstaller::open(&self.guard, self.identity, self.range.clone()).unwrap()
    }
    fn image(&self, uploader: &RemoteUploader, index: u64, foreign: u8) -> (Snapshot, HardState) {
        let source = self.directory.join(format!("source-{index}-{foreign}"));
        let (engine, _) = WalEngine::open(source).unwrap();
        let mut batch = WriteBatch::new();
        batch.put(
            ColumnFamily::Default,
            RANGE_KEY.to_vec(),
            self.range.encode(),
        );
        batch.put(
            if foreign == 2 {
                ColumnFamily::Lock
            } else {
                ColumnFamily::Default
            },
            encode_key(
                KeyMode::Raw,
                if foreign == 1 {
                    KeyspaceId(8)
                } else {
                    self.range.keyspace
                },
                if foreign == 3 { b"z" } else { b"key" },
            )
            .unwrap(),
            b"value".to_vec(),
        );
        if foreign == 4 {
            let mut wrong = self.range.clone();
            wrong.tenant = TenantId(9);
            batch.put(ColumnFamily::Default, RANGE_KEY.to_vec(), wrong.encode());
        }
        engine
            .write_applied(batch, AppliedPosition { term: 3, index })
            .unwrap();
        let manifest = uploader
            .upload(
                engine
                    .freeze(FlushScope {
                        cluster: self.identity.cluster_id.to_string(),
                        region: 100,
                        conf_ver: 1,
                        version: 1,
                    })
                    .unwrap(),
            )
            .unwrap()
            .into_manifest();
        let mut image = Snapshot {
            data: data_image(&self.range, &manifest).unwrap().into(),
            ..Default::default()
        };
        image.mut_metadata().index = index;
        image.mut_metadata().term = 3;
        image
            .mut_metadata()
            .set_conf_state(ConfState::from((vec![1, 2, 3], vec![4])));
        (
            image,
            HardState {
                term: 5,
                vote: 2,
                commit: index,
                ..Default::default()
            },
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.directory).unwrap();
    }
}
fn remote() -> RemoteUploader {
    RemoteUploader::new(Arc::new(
        MinioObjectStore::connect(MinioConfig::from_env().unwrap()).unwrap(),
    ))
}

#[test]
fn group_and_store_ownership_exclude_competing_installers_and_foreign_incarnations() {
    let fixture = Fixture::new();
    let installer = fixture.open();
    assert!(JointInstaller::open(&fixture.guard, fixture.identity, fixture.range.clone()).is_err());
    assert!(StoreGuard::lock(&fixture.directory).is_err());
    drop(installer);
    let mut foreign = fixture.identity;
    foreign.store_incarnation = StoreIncarnation::mint().unwrap();
    assert!(JointInstaller::open(&fixture.guard, foreign, fixture.range.clone()).is_err());
    let mut foreign_range = fixture.range.clone();
    foreign_range.end = b"y".to_vec();
    assert!(JointInstaller::open(&fixture.guard, fixture.identity, foreign_range).is_err());
    let installer = fixture.open();
    assert_eq!(
        &read(&installer.directory.join("group-record"), 256 * 1024).unwrap()[..8],
        b"KV9INS01"
    );
}

#[test]
fn existing_flat_history_is_never_adopted_or_overwritten() {
    for name in ["raft.log", "data.wal", "group-record"] {
        let fixture = Fixture::new();
        let directory = fixture.directory.join("data-groups/100");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join(name);
        fs::write(&path, b"old-durable-voting-history").unwrap();
        assert!(
            JointInstaller::open(&fixture.guard, fixture.identity, fixture.range.clone()).is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), b"old-durable-voting-history");
    }
}

#[test]
fn range_image_validation_checks_all_rows_and_column_families() {
    let fixture = Fixture::new();
    for mode in 0..6 {
        let (engine, _) =
            WalEngine::open(fixture.directory.join(format!("validation-{mode}"))).unwrap();
        let mut batch = WriteBatch::new();
        batch.put(
            ColumnFamily::Default,
            RANGE_KEY.to_vec(),
            fixture.range.encode(),
        );
        // Exercise pagination and prefix-related keys at its boundary.
        for n in 0..600 {
            batch.put(
                ColumnFamily::Default,
                encode_key(
                    KeyMode::Raw,
                    fixture.range.keyspace,
                    format!("k{n:04}").as_bytes(),
                )
                .unwrap(),
                vec![1],
            );
        }
        if mode != 0 {
            batch.put(
                if mode == 1 {
                    ColumnFamily::Lock
                } else {
                    ColumnFamily::Default
                },
                if mode == 5 {
                    b"invalid-key".to_vec()
                } else {
                    encode_key(
                        if mode == 4 {
                            KeyMode::Txn
                        } else {
                            KeyMode::Raw
                        },
                        if mode == 2 {
                            KeyspaceId(8)
                        } else {
                            fixture.range.keyspace
                        },
                        if mode == 3 { b"z" } else { b"key" },
                    )
                    .unwrap()
                },
                vec![2],
            );
        }
        engine
            .write_applied(batch, AppliedPosition { term: 3, index: 10 })
            .unwrap();
        let view = engine.snapshot().unwrap();
        let plan = RemoteUploader::plan(
            engine
                .freeze(FlushScope {
                    cluster: fixture.identity.cluster_id.to_string(),
                    region: 100,
                    conf_ver: 1,
                    version: 1,
                })
                .unwrap(),
        )
        .unwrap();
        let result = validate_view(view.as_ref(), &fixture.range, plan.manifest());
        if mode == 0 {
            assert_eq!(result.unwrap(), 600);
        } else {
            assert!(result.is_err());
        }
    }
}

#[test]
#[ignore = "requires an isolated real MinIO bucket and environment credentials"]
fn real_minio_every_install_cut_selects_only_a_complete_old_or_new_pair() {
    let uploader = remote();
    let cuts = [
        InstallStep::AfterEngine,
        InstallStep::AfterProtocol,
        InstallStep::BeforeSelector,
        InstallStep::AfterSelectorRename,
        InstallStep::AfterSelectorSync,
    ];
    for old in [false, true] {
        for cut in cuts {
            let fixture = Fixture::new();
            let (first, first_hs) = fixture.image(&uploader, 10, 0);
            let (next, next_hs) = fixture.image(&uploader, 20, 0);
            let mut installer = fixture.open();
            if old {
                installer.install(&first, &first_hs, &uploader).unwrap();
            }
            let old_selector = fs::read(installer.directory.join(SELECTOR)).ok();
            let mut arrived = false;
            let error = installer.install_observed(&next, &next_hs, &uploader, &mut |step, _| {
                if step == cut {
                    arrived = true;
                    return Err(invalid("injected post-operation EIO"));
                }
                Ok(())
            });
            assert!(error.is_err() && arrived);
            assert!(installer.install(&next, &next_hs, &uploader).is_err());
            assert!(
                installer.recover(&uploader).is_err(),
                "failed owner escaped without reopening"
            );
            let directory = installer.directory.clone();
            drop(installer);
            let mut recovered = fixture.open();
            let state = recovered.recover(&uploader).unwrap();
            let new = matches!(
                cut,
                InstallStep::AfterSelectorRename | InstallStep::AfterSelectorSync
            );
            assert_eq!(
                state.as_ref().map(|s| s.position.index),
                if new {
                    Some(20)
                } else if old {
                    Some(10)
                } else {
                    None
                }
            );
            drop(recovered);
            // Real process failure retains an unsynced rename. Separately
            // emulate the other permitted power-loss result: previous selector.
            // This is explicit namespace emulation, not a physical power cut.
            if cut == InstallStep::AfterSelectorRename {
                match &old_selector {
                    Some(bytes) => fs::write(directory.join(SELECTOR), bytes).unwrap(),
                    None => fs::remove_file(directory.join(SELECTOR)).unwrap(),
                }
                let mut recovered = fixture.open();
                assert_eq!(
                    recovered
                        .recover(&uploader)
                        .unwrap()
                        .map(|s| s.position.index),
                    if old { Some(10) } else { None }
                );
            }
            let mut recovered = fixture.open();
            let installed = recovered.install(&next, &next_hs, &uploader).unwrap();
            assert_eq!(installed.position.index, 20);
            assert_eq!(
                recovered.install(&next, &next_hs, &uploader).unwrap(),
                installed
            );
            assert_eq!(recovered.recover(&uploader).unwrap(), Some(installed));
        }
    }
}

#[test]
#[ignore = "requires an isolated real MinIO bucket and environment credentials"]
fn real_minio_corrupt_selected_files_never_fall_back_or_recreate() {
    let uploader = remote();
    for name in [
        "image",
        "ready",
        "data.wal",
        "data.checkpoint",
        "raft/raft.log",
    ] {
        for missing in [false, true] {
            let fixture = Fixture::new();
            let (first, first_hs) = fixture.image(&uploader, 10, 0);
            let (next, next_hs) = fixture.image(&uploader, 20, 0);
            let mut installer = fixture.open();
            let old = installer.install(&first, &first_hs, &uploader).unwrap();
            let new = installer.install(&next, &next_hs, &uploader).unwrap();
            let old_path = installer
                .directory
                .join(format!("install-{}/image", old.generation));
            let old_bytes = fs::read(&old_path).unwrap();
            let path = installer
                .directory
                .join(format!("install-{}/{name}", new.generation));
            if missing {
                fs::remove_file(&path).unwrap();
            } else {
                fs::write(&path, b"corrupt").unwrap();
            }
            drop(installer);
            let mut recovered = fixture.open();
            assert!(recovered.recover(&uploader).is_err());
            assert_eq!(fs::read(old_path).unwrap(), old_bytes);
            assert_eq!(path.exists(), !missing);
        }
    }
}

#[test]
#[ignore = "requires an isolated real MinIO bucket and environment credentials"]
fn real_minio_rejects_foreign_images_and_vote_or_cut_rollback() {
    let uploader = remote();
    for foreign in 1..=4 {
        let fixture = Fixture::new();
        let (image, hs) = fixture.image(&uploader, 10, foreign);
        let mut installer = fixture.open();
        assert!(installer.install(&image, &hs, &uploader).is_err());
        assert!(!installer.directory.join(SELECTOR).exists());
    }
    for defect in 0..7 {
        let fixture = Fixture::new();
        let (old, old_hs) = fixture.image(&uploader, 10, 0);
        let (mut new, mut hs) = fixture.image(&uploader, 20, 0);
        let mut installer = fixture.open();
        installer.install(&old, &old_hs, &uploader).unwrap();
        let original = fs::read(installer.directory.join(SELECTOR)).unwrap();
        match defect {
            0 => hs.vote = 3,
            1 => hs.vote = 0,
            2 => hs.term = 4,
            3 => {
                new = old.clone();
                hs.commit = 10;
                hs.term = 6;
            }
            4 => new.mut_metadata().index = 21,
            5 => new.mut_metadata().mut_conf_state().learners.clear(),
            6 => {
                let (r, mut m) = parse_image(&new).unwrap();
                for file in &mut m.files {
                    file.key = file.key.replacen(&m.scope.cluster, "foreign", 1);
                }
                m.scope.cluster = "foreign".into();
                new.data = data_image(&r, &m).unwrap().into();
            }
            _ => unreachable!(),
        }
        assert!(installer.install(&new, &hs, &uploader).is_err());
        assert_eq!(
            fs::read(installer.directory.join(SELECTOR)).unwrap(),
            original
        );
    }
}

#[test]
#[ignore = "requires an isolated real MinIO bucket and environment credentials"]
fn real_minio_missing_or_corrupt_sst_refuses_without_changing_selection() {
    use kv9_engine::{ObjectKey, ObjectStore};
    let store = Arc::new(MinioObjectStore::connect(MinioConfig::from_env().unwrap()).unwrap());
    let uploader = RemoteUploader::new(store.clone());
    let fixture = Fixture::new();
    let (image, hs) = fixture.image(&uploader, 10, 0);
    let (_, manifest) = parse_image(&image).unwrap();
    let key = ObjectKey::new(manifest.files[0].key.clone()).unwrap();
    let bytes = store.get(&key).unwrap().unwrap();
    let mut installer = fixture.open();
    let installed = installer.install(&image, &hs, &uploader).unwrap();
    let selector = fs::read(installer.directory.join(SELECTOR)).unwrap();
    drop(installer);
    for corrupt in [false, true] {
        // Only this fixture's freshly minted cluster/object is mutated.
        store.delete(&key).unwrap();
        if corrupt {
            store.put(&key, b"corrupt-test-object").unwrap();
        }
        let mut installer = fixture.open();
        assert!(installer.recover(&uploader).is_err());
        assert_eq!(
            fs::read(installer.directory.join(SELECTOR)).unwrap(),
            selector
        );
        drop(installer);
        store.delete(&key).unwrap();
        store.put(&key, &bytes).unwrap();
        let mut installer = fixture.open();
        assert_eq!(
            installer.recover(&uploader).unwrap(),
            Some(installed.clone())
        );
    }
}

#[test]
#[ignore = "requires an isolated real MinIO bucket and environment credentials"]
fn real_minio_failed_attempts_are_retained_and_bounded() {
    let uploader = remote();
    let fixture = Fixture::new();
    let (image, hs) = fixture.image(&uploader, 10, 0);
    for _ in 0..MAX_GENERATIONS {
        let mut installer = fixture.open();
        assert!(installer
            .install_observed(&image, &hs, &uploader, &mut |step, _| {
                if step == InstallStep::AfterEngine {
                    Err(invalid("injected stop"))
                } else {
                    Ok(())
                }
            })
            .is_err());
    }
    let mut installer = fixture.open();
    assert!(installer.recover(&uploader).unwrap().is_none());
    let before = fs::read_dir(&installer.directory).unwrap().count();
    let error = installer.install(&image, &hs, &uploader).unwrap_err();
    assert!(error.to_string().contains("budget exhausted"));
    assert_eq!(fs::read_dir(&installer.directory).unwrap().count(), before);
}
