use super::*;
use crate::command::{RawTtlAction, RawTtlCommand, RawTtlMutation};
use crate::driver::NodeDriver;
use crate::transport::{InProcHub, RaftTransport};
use crate::{Command, ManifestChangePayload, MemStateMachine, RaftGroup, RaftPeer};
use kv9_common::raw_ttl::{ClockSample, ExpiryPlan};
use kv9_common::{KeyspaceId, NodeId, StoreIncarnation, TenantId};
use kv9_engine::{MemoryObjectStore, ReplicatedEngine, WriteBatch};
use std::path::PathBuf;
use std::sync::Arc;

fn range() -> DataRange {
    DataRange {
        root: RootDigest::from_bytes([1; 32]),
        creation: RootDigest::from_bytes([2; 32]),
        region: RegionId(100),
        keyspace: KeyspaceId(100),
        tenant: TenantId(1),
        conf_ver: 1,
        version: 1,
        start: b"a".to_vec(),
        end: b"z".to_vec(),
        sealed: false,
    }
}
fn identity() -> DataCheckpointIdentity {
    DataCheckpointIdentity {
        cluster: ClusterId::from_bytes([3; 16]),
        root: range().root,
        region: range().region,
        creation: range().creation,
    }
}
fn scope() -> kv9_engine::checkpoint::FlushScope {
    kv9_engine::checkpoint::FlushScope {
        cluster: identity().cluster.to_string(),
        region: 100,
        conf_ver: 1,
        version: 1,
    }
}
fn physical(key: &[u8]) -> Vec<u8> {
    kv9_common::codec::encode_key(KeyMode::Raw, range().keyspace, key).unwrap()
}
fn directory() -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "kv9-data-checkpoint-{}",
        StoreIncarnation::mint().unwrap()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}
fn pump(driver: &NodeDriver<DiskRaftStorage, WalEngine>) {
    for _ in 0..10 {
        driver.step().unwrap();
    }
}
struct Fixture {
    path: PathBuf,
    engine: Arc<WalEngine>,
    driver: Arc<NodeDriver<DiskRaftStorage, WalEngine>>,
    objects: Arc<MemoryObjectStore>,
    uploader: RemoteUploader,
}
impl Fixture {
    fn new() -> Self {
        Self::with_ttl(true)
    }
    fn with_ttl(enable: bool) -> Self {
        let path = directory();
        let engine = Arc::new(WalEngine::open(path.join("data.wal")).unwrap().0);
        engine.enable_segmentation().unwrap();
        let storage = DiskRaftStorage::open(&path.join("raft"), &[1]).unwrap().0;
        let mut sm = MemStateMachine::with_engine(engine.clone()).unwrap();
        sm.set_data_group(range().root, range().region, range().creation)
            .unwrap();
        let hub = InProcHub::new();
        let driver = NodeDriver::new(
            Arc::new(RaftPeer::with_storage(NodeId(1), range().region, storage).unwrap()),
            Arc::new(hub.endpoint(NodeId(1))) as Arc<dyn RaftTransport>,
            sm,
        )
        .unwrap();
        driver.peer().campaign().unwrap();
        pump(&driver);
        driver
            .propose(&Command::DataRange {
                expected: None,
                next: range(),
            })
            .unwrap();
        pump(&driver);
        // Activation injection isolates recovery; runtime activation has separate real-fence tests.
        if enable {
            driver
                .peer()
                .propose_raw_for_harness(
                    Command::RawTtl(
                        RawTtlCommand::new(
                            &range(),
                            RawTtlAction::Enable {
                                observed: ClockWatermark::default(),
                            },
                        )
                        .unwrap(),
                    )
                    .encode(),
                )
                .unwrap();
            pump(&driver);
        }
        let objects = Arc::new(MemoryObjectStore::new());
        let uploader = RemoteUploader::for_harness(objects.clone());
        Self {
            path,
            engine,
            driver,
            objects,
            uploader,
        }
    }
    fn commit(&self, command: Command) -> AppliedPosition {
        let at = self.driver.propose(&command).unwrap();
        pump(&self.driver);
        assert!(self.driver.driver_applied().unwrap().index >= at.index.0);
        AppliedPosition {
            term: at.term,
            index: at.index.0,
        }
    }
    fn put(&self, key: &[u8], value: Vec<u8>, ttl: Option<u64>) -> AppliedPosition {
        let committed = self.driver.raw_ttl_state().unwrap().1;
        let plan = ExpiryPlan::resolve(
            ttl,
            ClockSample::with_uncertainty(1000, 10).unwrap(),
            committed,
        )
        .unwrap();
        self.commit(Command::RawTtl(
            RawTtlCommand::new(
                &range(),
                RawTtlAction::Write {
                    plan,
                    mutations: vec![RawTtlMutation::Put {
                        key: key.to_vec(),
                        value,
                    }],
                },
            )
            .unwrap(),
        ))
    }
    fn publish(&self) -> (CheckpointManifest, AppliedPosition) {
        let generation = self.driver.manifest_pair(100).unwrap().generation;
        let planned = RemoteUploader::plan(self.engine.freeze(scope()).unwrap()).unwrap();
        let manifest = planned.manifest().clone();
        let prepared = self.uploader.upload_planned(planned).unwrap();
        let at = self.commit(Command::ManifestChange(
            ManifestChangePayload::from_prepared(prepared, generation).unwrap(),
        ));
        assert_eq!(
            self.driver.manifest_pair(100).unwrap().generation,
            generation + 1
        );
        assert_eq!(
            self.driver
                .manifest_at(100, generation + 1)
                .unwrap()
                .unwrap()
                .2,
            manifest.encode().unwrap()
        );
        (manifest, at)
    }
}
fn tree_bytes(path: &Path) -> u64 {
    std::fs::read_dir(path)
        .unwrap()
        .map(|e| {
            let p = e.unwrap().path();
            let m = std::fs::metadata(&p).unwrap();
            if m.is_dir() {
                tree_bytes(&p)
            } else {
                m.len()
            }
        })
        .sum()
}

#[test]
fn ttl_image_and_tail_survive_two_generations_and_real_segment_reclamation() {
    let f = Fixture::new();
    // Fixed live key count, enough overwrite debt to close a real 16 MiB segment.
    for byte in 0..18 {
        f.put(b"bulk", vec![byte; 1024 * 1024], None);
    }
    f.put(b"bulk", b"last".to_vec(), None);
    let live = f.put(b"live", b"KV9TTL01 arbitrary value".to_vec(), Some(60));
    f.put(b"expired", b"preserve image bytes".to_vec(), Some(0));
    f.put(b"removed", b"old".to_vec(), Some(1));
    let plan = ExpiryPlan::permanent(f.driver.raw_ttl_state().unwrap().1);
    f.commit(Command::RawTtl(
        RawTtlCommand::new(
            &range(),
            RawTtlAction::Write {
                plan,
                mutations: vec![RawTtlMutation::Delete {
                    key: b"removed".to_vec(),
                }],
            },
        )
        .unwrap(),
    ));
    let before = tree_bytes(&f.path.join("data.segments"));
    let (manifest1, _) = f.publish();
    assert!(f.engine.checkpoint_applied(&manifest1).unwrap());
    let after = tree_bytes(&f.path.join("data.segments"));
    assert!(
        before > 16 * 1024 * 1024 && after < before / 2,
        "{before} -> {after}"
    );
    f.put(b"tail", b"v1".to_vec(), None);
    let (manifest2, published) = f.publish();
    assert!(f.engine.checkpoint_applied(&manifest2).unwrap());
    let last = f.put(b"tail", b"v2".to_vec(), None);
    let path = f.path.clone();
    let objects = f.objects.clone();
    f.driver.stop();
    drop(f);
    let uploader = RemoteUploader::for_harness(objects);
    let mut storage = DiskRaftStorage::recover(&path.join("raft")).unwrap();
    let (engine, replay, anchor) = open(
        &mut storage,
        path.join("data.wal"),
        Some(&uploader),
        identity(),
    )
    .unwrap();
    let anchor = anchor.unwrap();
    assert_eq!(anchor.range(), Some(&range()));
    assert_eq!(anchor.publication().manifest(), &manifest2);
    assert_eq!(anchor.publication().generation(), 2);
    assert_eq!(anchor.publication().publication(), published);
    assert_eq!(
        anchor.publication().configuration().state().get_voters(),
        &[1]
    );
    assert_eq!(
        engine.applied_position().unwrap(),
        kv9_engine::DurableAppliedPosition::AppliedThrough(last)
    );
    assert!(replay.replayed_records >= 2);
    assert_eq!(
        engine
            .get(ColumnFamily::Default, &physical(b"tail"))
            .unwrap(),
        Some(b"v2".to_vec())
    );
    assert_eq!(
        engine
            .get(ColumnFamily::Default, &physical(b"live"))
            .unwrap(),
        Some(b"KV9TTL01 arbitrary value".to_vec())
    );
    let expiry = engine
        .get(ColumnFamily::Write, &physical(b"live"))
        .unwrap()
        .unwrap();
    assert_eq!(
        expiry,
        ExpiryRecord::new(range().creation, live, 61010)
            .unwrap()
            .encode()
    );
    assert!(engine
        .get(ColumnFamily::Default, &physical(b"expired"))
        .unwrap()
        .is_some());
    assert!(engine
        .get(ColumnFamily::Write, &physical(b"expired"))
        .unwrap()
        .is_some());
    assert!(engine
        .get(ColumnFamily::Default, &physical(b"removed"))
        .unwrap()
        .is_none());
    assert!(engine
        .get(ColumnFamily::Write, &physical(b"removed"))
        .unwrap()
        .is_none());
    assert_eq!(anchor.watermark().unwrap().unix_ms(), 990);
    drop(engine);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn recovery_refuses_missing_remote_objects_wrong_identity_and_unpublished_checkpoint() {
    let f = Fixture::new();
    f.put(b"key", b"v".to_vec(), Some(10));
    let planned = RemoteUploader::plan(f.engine.freeze(scope()).unwrap()).unwrap();
    let manifest = planned.manifest().clone();
    f.uploader.upload_planned(planned).unwrap();
    // Deliberately forge a premature local selection. Upload alone is not a
    // successful ManifestChange, even when every SST can be read back.
    f.engine.checkpoint_applied(&manifest).unwrap();
    let path = f.path.clone();
    let objects = f.objects.clone();
    f.driver.stop();
    drop(f);
    let uploader = RemoteUploader::for_harness(objects);
    let mut storage = DiskRaftStorage::recover(&path.join("raft")).unwrap();
    assert!(open(
        &mut storage,
        path.join("data.wal"),
        Some(&uploader),
        identity()
    )
    .is_err());
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();

    let f = Fixture::new();
    f.put(b"key", b"v".to_vec(), Some(10));
    let (manifest, _) = f.publish();
    f.engine.checkpoint_applied(&manifest).unwrap();
    let path = f.path.clone();
    let objects = f.objects.clone();
    f.driver.stop();
    drop(f);
    let uploader = RemoteUploader::for_harness(objects.clone());
    for identity in [
        DataCheckpointIdentity {
            root: RootDigest::from_bytes([9; 32]),
            ..identity()
        },
        DataCheckpointIdentity {
            creation: RootDigest::from_bytes([9; 32]),
            ..identity()
        },
        DataCheckpointIdentity {
            region: RegionId(101),
            ..identity()
        },
        DataCheckpointIdentity {
            cluster: ClusterId::from_bytes([9; 16]),
            ..identity()
        },
    ] {
        let mut storage = DiskRaftStorage::recover(&path.join("raft")).unwrap();
        assert!(open(
            &mut storage,
            path.join("data.wal"),
            Some(&uploader),
            identity
        )
        .is_err());
    }
    let mut storage = DiskRaftStorage::recover(&path.join("raft")).unwrap();
    assert!(open(&mut storage, path.join("data.wal"), None, identity()).is_err());
    use kv9_engine::ObjectStore;
    objects
        .delete(&kv9_engine::ObjectKey::new(manifest.files[0].key.clone()).unwrap())
        .unwrap();
    assert!(open(
        &mut storage,
        path.join("data.wal"),
        Some(&uploader),
        identity()
    )
    .is_err());
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn verified_sst_image_still_requires_valid_range_ttl_and_system_metadata() {
    for fault in 0..9 {
        let path = directory();
        let engine = WalEngine::open(path.join("image.wal")).unwrap().0;
        let mut batch = WriteBatch::new();
        batch.put(ColumnFamily::Default, RANGE_KEY.to_vec(), range().encode());
        batch.put(
            ColumnFamily::Default,
            CLOCK_KEY.to_vec(),
            ClockWatermark::default().encode().to_vec(),
        );
        batch.put(ColumnFamily::Default, physical(b"key"), b"opaque".to_vec());
        let record = ExpiryRecord::new(
            RootDigest::from_bytes([7; 32]),
            AppliedPosition {
                term: 88,
                index: 9999,
            },
            4,
        )
        .unwrap();
        batch.put(
            ColumnFamily::Write,
            physical(b"key"),
            record.encode().to_vec(),
        );
        match fault {
            0 => &mut batch, // Imported source identity is preserved, never compared to this group's index.
            1 => batch.delete(ColumnFamily::Default, CLOCK_KEY.to_vec()),
            2 => batch.put(
                ColumnFamily::Default,
                CLOCK_KEY.to_vec(),
                b"bad clock".to_vec(),
            ),
            3 => batch.put(
                ColumnFamily::Write,
                physical(b"key"),
                b"bad expiry".to_vec(),
            ),
            4 => batch.delete(ColumnFamily::Default, physical(b"key")),
            5 => batch.put(
                ColumnFamily::Default,
                physical(b"zz-outside"),
                b"foreign".to_vec(),
            ),
            6 => batch.put(ColumnFamily::Lock, physical(b"key"), b"foreign CF".to_vec()),
            7 => batch.put(
                ColumnFamily::Default,
                manifest_pair_key(100),
                ManifestPair {
                    generation: 1,
                    last_change_id: vec![4; 32],
                }
                .encode(),
            ),
            8 => batch.put(
                ColumnFamily::Default,
                COMPACTION_CONFIRMED_KEY.to_vec(),
                [1_u64.to_be_bytes(), 11_u64.to_be_bytes()].concat(),
            ),
            _ => unreachable!(),
        };
        engine
            .write_applied(batch, AppliedPosition { term: 1, index: 10 })
            .unwrap();
        let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()));
        let manifest = uploader
            .upload(engine.freeze(scope()).unwrap())
            .unwrap()
            .into_manifest();
        let restored = uploader.restore(&manifest).unwrap();
        let view = restored.snapshot().unwrap();
        let result = inspect_base(view.as_ref(), &manifest, identity());
        assert_eq!(result.is_ok(), fault == 0, "fault {fault}: {result:?}");
        if fault == 0 {
            assert_eq!(
                view.get(ColumnFamily::Write, &physical(b"key"))
                    .unwrap()
                    .unwrap(),
                record.encode()
            );
        }
        drop(view);
        drop(restored);
        drop(engine);
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[test]
fn a_checksummed_positioned_tail_cannot_regress_ttl_or_replace_range_ownership() {
    for fault in 0..12 {
        let f = Fixture::new();
        f.put(b"key", b"value".to_vec(), Some(10));
        let (manifest, _) = f.publish();
        f.engine.checkpoint_applied(&manifest).unwrap();
        f.driver.pause_apply(true);
        let plan = ExpiryPlan::permanent(f.driver.raw_ttl_state().unwrap().1);
        let at = f
            .driver
            .propose(&Command::RawTtl(
                RawTtlCommand::new(
                    &range(),
                    RawTtlAction::Write {
                        plan,
                        mutations: vec![RawTtlMutation::Put {
                            key: b"tail".to_vec(),
                            value: b"real command".to_vec(),
                        }],
                    },
                )
                .unwrap(),
            ))
            .unwrap();
        pump(&f.driver);
        assert!(f.driver.status().raft_committed >= at.index.0);
        let mut corrupt = WriteBatch::new();
        // Deliberately inject a false engine effect with valid framing and a
        // real committed position. Position matching alone cannot detect it.
        match fault {
            0 => {
                corrupt.put(
                    ColumnFamily::Default,
                    CLOCK_KEY.to_vec(),
                    ClockWatermark::default().encode().to_vec(),
                );
            }
            1 => {
                corrupt.delete(ColumnFamily::Default, CLOCK_KEY.to_vec());
            }
            2..=8 => {
                let mut next = range();
                match fault {
                    2 => next.creation = RootDigest::from_bytes([8; 32]),
                    3 => next.tenant = TenantId(2),
                    4 => next.keyspace = KeyspaceId(101),
                    5 => next.end = b"y".to_vec(),
                    6 => next.conf_ver += 1,
                    7 => next.version += 1,
                    8 => {
                        next.version += 1;
                        next.sealed = true;
                    }
                    _ => unreachable!(),
                }
                corrupt.put(ColumnFamily::Default, RANGE_KEY.to_vec(), next.encode());
            }
            9 => {
                for ms in [2000, 1500, 3000] {
                    corrupt.put(
                        ColumnFamily::Default,
                        CLOCK_KEY.to_vec(),
                        ClockSample::with_uncertainty(ms, 0)
                            .unwrap()
                            .advance(ClockWatermark::default())
                            .unwrap()
                            .encode()
                            .to_vec(),
                    );
                }
            }
            10 => {
                corrupt.delete(ColumnFamily::Default, CLOCK_KEY.to_vec());
                corrupt.put(
                    ColumnFamily::Default,
                    CLOCK_KEY.to_vec(),
                    plan.observed().encode().to_vec(),
                );
            }
            11 => {
                let mut next = range();
                next.tenant = TenantId(2);
                corrupt.put(ColumnFamily::Default, RANGE_KEY.to_vec(), next.encode());
                corrupt.put(ColumnFamily::Default, RANGE_KEY.to_vec(), range().encode());
            }
            _ => unreachable!(),
        }
        f.engine
            .write_applied(
                corrupt,
                AppliedPosition {
                    term: at.term,
                    index: at.index.0,
                },
            )
            .unwrap();
        let path = f.path.clone();
        let objects = f.objects.clone();
        f.driver.stop();
        drop(f);
        let mut storage = DiskRaftStorage::recover(&path.join("raft")).unwrap();
        let uploader = RemoteUploader::for_harness(objects);
        assert!(
            open(
                &mut storage,
                path.join("data.wal"),
                Some(&uploader),
                identity()
            )
            .is_err(),
            "fault {fault}"
        );
        drop(storage);
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[test]
fn legacy_range_can_seal_in_the_tail_after_a_non_ttl_checkpoint() {
    let f = Fixture::with_ttl(false);
    let (manifest, _) = f.publish();
    f.engine.checkpoint_applied(&manifest).unwrap();
    let mut sealed = range();
    sealed.sealed = true;
    sealed.version += 1;
    let last = f.commit(Command::DataRange {
        expected: Some(range().digest()),
        next: sealed.clone(),
    });
    assert_eq!(
        f.engine.get(ColumnFamily::Default, RANGE_KEY).unwrap(),
        Some(sealed.encode())
    );
    let path = f.path.clone();
    let objects = f.objects.clone();
    f.driver.stop();
    drop(f);
    let mut storage = DiskRaftStorage::recover(&path.join("raft")).unwrap();
    let uploader = RemoteUploader::for_harness(objects);
    let (engine, _, checkpoint) = open(
        &mut storage,
        path.join("data.wal"),
        Some(&uploader),
        identity(),
    )
    .unwrap();
    let checkpoint = checkpoint.unwrap();
    assert_eq!(checkpoint.range(), Some(&range()));
    assert!(checkpoint.watermark().is_none());
    assert_eq!(
        engine.get(ColumnFamily::Default, RANGE_KEY).unwrap(),
        Some(sealed.encode())
    );
    assert_eq!(
        engine.applied_position().unwrap(),
        kv9_engine::DurableAppliedPosition::AppliedThrough(last)
    );
    drop(engine);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn compacting_away_checkpoint_protocol_evidence_cannot_be_laundered_into_recovery_authority() {
    let f = Fixture::new();
    f.put(b"key", b"value".to_vec(), Some(10));
    let (manifest, publication) = f.publish();
    f.engine.checkpoint_applied(&manifest).unwrap();
    // Negative control: erase the required configuration/publication history.
    // Even with the live compaction restriction, already damaged files must
    // not be accepted as a valid checkpoint recovery source.
    let path = f.path.clone();
    let objects = f.objects.clone();
    f.driver.stop();
    drop(f);
    let mut storage = DiskRaftStorage::recover(&path.join("raft")).unwrap();
    // Deliberately bypass the now-protected live peer after stopping it. Old
    // files with already erased evidence must still fail full recovery.
    let crate::storage::ConfigurationLookup::Found(conf) =
        storage.configuration_at_committed(publication).unwrap()
    else {
        panic!("committed publication configuration");
    };
    storage
        .compact_retained_prefix(publication, conf.state(), b"test-only proof removal")
        .unwrap();
    let uploader = RemoteUploader::for_harness(objects);
    assert!(open(
        &mut storage,
        path.join("data.wal"),
        Some(&uploader),
        identity()
    )
    .is_err());
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn completed_open_dependencies_distinguish_local_wal_and_exact_published_image() {
    // The observation stores no SST list, range bounds or import cursor bytes.
    assert!(std::mem::size_of::<DataRecoveryDependencies>() <= 1024);
    for checkpoint in [false, true] {
        let f = Fixture::new();
        f.put(b"live", b"value".to_vec(), Some(60));
        let image_clock = ClockWatermark::decode(
            &f.engine
                .get(ColumnFamily::Default, CLOCK_KEY)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        let published = checkpoint.then(|| {
            let (manifest, at) = f.publish();
            f.engine.checkpoint_applied(&manifest).unwrap();
            (manifest, at)
        });
        let last = f.commit(Command::RawTtl(
            RawTtlCommand::new(
                &range(),
                RawTtlAction::Observe {
                    observed: ClockSample::with_uncertainty(2000, 0)
                        .unwrap()
                        .advance(image_clock)
                        .unwrap(),
                },
            )
            .unwrap(),
        ));
        let path = f.path.clone();
        let objects = f.objects.clone();
        f.driver.stop();
        drop(f);
        let uploader = RemoteUploader::for_harness(objects.clone());
        let mut storage = DiskRaftStorage::recover(&path.join("raft")).unwrap();
        let (engine, _, facts) = open_with_dependencies(
            &mut storage,
            path.join("data.wal"),
            Some(&uploader),
            identity(),
        )
        .unwrap();
        assert_eq!(facts.identity().creation, range().creation);
        assert_eq!(
            facts.recovered_through(),
            kv9_engine::DurableAppliedPosition::AppliedThrough(last)
        );
        assert!(facts.protocol_base().is_none());
        assert!(facts.installed().is_none());
        assert!(!facts.used_original_install_image());
        match published {
            None => assert!(facts.selected().is_none()),
            Some((manifest, at)) => {
                let image = facts.selected().unwrap();
                assert_eq!(
                    image.manifest(),
                    RootDigest::sha256(&manifest.encode().unwrap())
                );
                assert_eq!(image.cut(), manifest.position());
                assert_eq!(
                    image.state(),
                    RecoveryImageState::Bound(RootDigest::sha256(&range().encode()))
                );
                assert_eq!(image.watermark(), Some(image_clock));
                assert_eq!(image.configuration_at(), None);
                let publication = image.publication().unwrap();
                assert_eq!(publication.at(), at);
                assert!(at.index > image.cut().index);
                assert_eq!(publication.generation(), 1);
                assert_eq!(
                    publication.change_id().as_slice(),
                    manifest.change_id(0).unwrap()
                );
                assert_ne!(
                    engine
                        .get(ColumnFamily::Default, CLOCK_KEY)
                        .unwrap()
                        .unwrap(),
                    image_clock.encode()
                );
                drop(engine);
                use kv9_engine::ObjectStore;
                objects
                    .delete(&kv9_engine::ObjectKey::new(manifest.files[0].key.clone()).unwrap())
                    .unwrap();
                assert!(open_with_dependencies(
                    &mut storage,
                    path.join("data.wal"),
                    Some(&uploader),
                    identity()
                )
                .is_err());
            }
        }
        drop(storage);
        std::fs::remove_dir_all(path).unwrap();
    }
}

mod history;

mod candidate;
