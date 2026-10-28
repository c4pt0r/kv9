use super::*;
use kv9_common::data_range::{DataRange, RANGE_KEY};
use kv9_common::raw_ttl::{ClockSample, ClockWatermark, CLOCK_KEY};
use kv9_common::store_lifecycle::StoreGuard;
use kv9_common::{AppliedPosition, KeyspaceId, TenantId};
use kv9_engine::checkpoint::{CheckpointManifest, FlushScope, RemoteUploader};
use kv9_engine::{MemoryObjectStore, ObjectKey, ObjectStore, WriteBatch};
use kv9_raft::command::{RawTtlAction, RawTtlCommand};
use kv9_raft::rawnode::PersistentRaftStorage;
use kv9_raft::snapshot_install::{adopt_for_runtime, data_image, JointInstaller};
use kv9_raft::{Command, ManifestChangePayload};
use raft::prelude::{ConfState, Entry, HardState, Snapshot};

fn floor(ms: u64) -> ClockWatermark {
    ClockSample::with_uncertainty(ms, 0)
        .unwrap()
        .advance(ClockWatermark::default())
        .unwrap()
}

struct Installed {
    path: PathBuf,
    guard: StoreGuard,
    identity: StoreIdentity,
    store: MetaStore<WalEngine>,
    migration: kv9_meta::data_groups::migration::CommittedMigration,
    range: DataRange,
    uploader: RemoteUploader,
    objects: Arc<MemoryObjectStore>,
    original_manifest: CheckpointManifest,
}
impl Installed {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "kv9-adopt-recovery-{}",
            StoreIncarnation::mint().unwrap()
        ));
        let mut guard = StoreGuard::lock(&path).unwrap();
        let prepared = guard.prepare(NodeId(4)).unwrap();
        let root = RootDescriptor::new(
            ClusterId::mint().unwrap(),
            BootstrapGeneration::mint().unwrap(),
            vec![RootVoter {
                node_id: NodeId(4),
                addr: "127.0.0.1:19444".parse().unwrap(),
                store_incarnation: prepared.incarnation,
            }],
            b"adoption-recovery-fixture",
        )
        .unwrap();
        let identity = StoreIdentity::for_voter(&root, NodeId(4)).unwrap();
        guard.bind(&root, &identity).unwrap();
        let store = MetaStore::new(Arc::new(
            WalEngine::open(path.join("metadata.wal")).unwrap().0,
        ));
        let mut txn = store.begin().unwrap();
        kv9_meta::root::initialize_root(&mut txn, &root).unwrap();
        for id in 1..=4 {
            let mut row = RowValue::new();
            row.set(
                ColumnId(2),
                ColumnValue::Text(format!("127.0.0.1:{}", 19000 + id)),
            );
            row.set(ColumnId(3), ColumnValue::Uint(2));
            row.set(ColumnId(4), ColumnValue::Uint(0));
            let incarnation = if id == 4 {
                prepared.incarnation
            } else {
                StoreIncarnation::from_bytes([id as u8; 16])
            };
            row.set(
                ColumnId(5),
                ColumnValue::Bytes(incarnation.as_bytes().to_vec()),
            );
            txn.insert(&NODES_DESC, &[memcmp_uint(id)], row).unwrap();
        }
        let creation = kv9_meta::data_groups::plan_empty_group(
            &mut txn,
            [1; 16],
            &[NodeId(1), NodeId(2), NodeId(3)],
        )
        .unwrap();
        kv9_meta::data_groups::activation::plan_activation(&mut txn, &creation).unwrap();
        kv9_meta::data_groups::migration::plan_migration(
            &mut txn,
            [2; 16],
            creation.task(),
            NodeId(4),
        )
        .unwrap();
        txn.commit().unwrap();
        let migration = kv9_meta::data_groups::migration::committed_migrations(&store)
            .unwrap()
            .remove(0);
        let range = DataRange {
            root: root.digest(),
            creation: creation.digest(),
            region: creation.region(),
            keyspace: KeyspaceId(100),
            tenant: TenantId(1),
            conf_ver: 1,
            version: 1,
            start: vec![],
            end: vec![],
            sealed: false,
        };
        let objects = Arc::new(MemoryObjectStore::new());
        let uploader = RemoteUploader::for_harness(objects.clone());
        let source = WalEngine::open(path.join("image.wal")).unwrap().0;
        let mut batch = WriteBatch::new();
        batch.put(ColumnFamily::Default, RANGE_KEY.to_vec(), range.encode());
        batch.put(
            ColumnFamily::Default,
            CLOCK_KEY.to_vec(),
            floor(1000).encode().to_vec(),
        );
        source
            .write_applied(batch, AppliedPosition { term: 3, index: 10 })
            .unwrap();
        let manifest = uploader
            .upload(
                source
                    .freeze(FlushScope {
                        cluster: identity.cluster_id.to_string(),
                        region: range.region.0,
                        conf_ver: 1,
                        version: 1,
                    })
                    .unwrap(),
            )
            .unwrap()
            .into_manifest();
        let mut snapshot = Snapshot {
            data: data_image(&range, &manifest).unwrap().into(),
            ..Default::default()
        };
        snapshot.mut_metadata().index = 10;
        snapshot.mut_metadata().term = 3;
        snapshot
            .mut_metadata()
            .set_conf_state(ConfState::from((vec![1, 2, 3], vec![4])));
        JointInstaller::open(&guard, identity, range.clone())
            .unwrap()
            .install(
                &snapshot,
                &HardState {
                    term: 5,
                    vote: 2,
                    commit: 10,
                    ..Default::default()
                },
                &uploader,
            )
            .unwrap();
        Self {
            path,
            guard,
            identity,
            store,
            migration,
            range,
            uploader,
            objects,
            original_manifest: manifest,
        }
    }

    fn advance_and_compact(&self) -> AppliedPosition {
        let adopted = adopt_for_runtime(
            &self.guard,
            self.identity,
            self.range.region,
            &self.uploader,
        )
        .unwrap();
        let mut storage = DiskRaftStorage::recover(&adopted.raft_directory).unwrap();
        let engine = Arc::new(adopted.open_engine(&mut storage, &self.uploader).unwrap().0);
        let mut state = MemStateMachine::with_engine(engine.clone()).unwrap();
        state
            .set_data_group(self.range.root, self.range.region, self.range.creation)
            .unwrap();
        let cut = AppliedPosition { term: 5, index: 11 };
        let observe = Command::RawTtl(
            RawTtlCommand::new(
                &self.range,
                RawTtlAction::Observe {
                    observed: floor(2000),
                },
            )
            .unwrap(),
        );
        storage
            .persist_ready(
                &[Entry {
                    term: 5,
                    index: 11,
                    data: observe.encode().into(),
                    ..Default::default()
                }],
                Some(&HardState {
                    term: 5,
                    vote: 2,
                    commit: 11,
                    ..Default::default()
                }),
            )
            .unwrap();
        state.apply_at(cut, &observe).unwrap();
        let plan = RemoteUploader::plan(
            engine
                .freeze(FlushScope {
                    cluster: self.identity.cluster_id.to_string(),
                    region: self.range.region.0,
                    conf_ver: 1,
                    version: 1,
                })
                .unwrap(),
        )
        .unwrap();
        let manifest = plan.manifest().clone();
        let change = Command::ManifestChange(
            ManifestChangePayload::from_prepared(self.uploader.upload_planned(plan).unwrap(), 0)
                .unwrap(),
        );
        storage
            .persist_ready(
                &[Entry {
                    term: 5,
                    index: 12,
                    data: change.encode().into(),
                    ..Default::default()
                }],
                Some(&HardState {
                    term: 5,
                    vote: 2,
                    commit: 12,
                    ..Default::default()
                }),
            )
            .unwrap();
        state
            .apply_at(AppliedPosition { term: 5, index: 12 }, &change)
            .unwrap();
        engine.checkpoint_applied(&manifest).unwrap();
        // Deliberately create the advanced files before catalog authority to
        // test that RegionManager refuses them without a committed decision.
        storage
            .compact_retained_prefix(cut, &adopted.configuration, b"test-only compacted files")
            .unwrap();
        cut
    }
}
impl Drop for Installed {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn adopted_advanced_base_requires_committed_catalog_floor_then_recovers_ttl() {
    let f = Installed::new();
    let cut = f.advance_and_compact();
    let adopted = adopt_for_runtime(&f.guard, f.identity, f.range.region, &f.uploader).unwrap();
    let selected = WalEngine::checkpoint_reference(&adopted.engine_wal)
        .unwrap()
        .unwrap();
    let mut removed = 0;
    for file in &f.original_manifest.files {
        if !selected.files.iter().any(|current| current.key == file.key) {
            f.objects
                .delete(&ObjectKey::new(file.key.clone()).unwrap())
                .unwrap();
            removed += 1;
        }
    }
    assert!(removed > 0);
    assert!(f.uploader.restore(&f.original_manifest).is_err());
    drop(adopted);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let transport = GrpcTransport::new(
        f.identity.node_id,
        None,
        runtime.handle().clone(),
        f.identity.root_digest,
    );
    let mut manager = RegionManager::new(&f.path, f.identity, 2);
    let empty = RecoveryDecisions {
        truncations: &[],
        compactions: &[],
    };
    let error = manager
        .adopt_installed(
            &f.guard,
            &f.migration,
            &transport,
            Duration::from_millis(10),
            &f.uploader,
            empty,
        )
        .unwrap_err();
    assert!(error.to_string().contains("committed compaction decision"));
    assert!(manager.driver(f.range.region).is_err());
    assert!(manager.recovery_status(f.range.region).is_err());
    let mut txn = f.store.begin().unwrap();
    kv9_meta::data_groups::compaction::plan_compaction(&mut txn, f.range.region, cut).unwrap();
    txn.commit().unwrap();
    let decisions =
        kv9_meta::data_groups::compaction::committed_compaction_floors(&f.store).unwrap();
    manager
        .adopt_installed(
            &f.guard,
            &f.migration,
            &transport,
            Duration::from_millis(10),
            &f.uploader,
            RecoveryDecisions {
                truncations: &[],
                compactions: &decisions,
            },
        )
        .unwrap();
    let driver = manager.driver(f.range.region).unwrap();
    assert_eq!(
        driver.peer().status_snapshot().log_first_index,
        cut.index + 1
    );
    assert_eq!(driver.raw_ttl_state().unwrap().1, floor(2000));
    assert!(driver.driver_applied().unwrap().index >= cut.index);
    assert_eq!(driver.status().fatal, None);
    let status = manager.recovery_status(f.range.region).unwrap();
    assert_eq!(status.store(), f.identity);
    assert!(status.driver_started());
    assert_eq!(
        status.selection_relation(),
        RecoverySelectionRelation::SameImageAsOpen
    );
    let facts = status.at_open().unwrap();
    assert!(!facts.used_original_install_image());
    assert_eq!(facts.protocol_base(), Some(cut));
    assert_eq!(
        facts.selected().unwrap().manifest(),
        RootDigest::sha256(&selected.encode().unwrap())
    );
    assert_eq!(facts.selected().unwrap().watermark(), Some(floor(2000)));
    assert_eq!(
        facts.selected().unwrap().publication().unwrap().at(),
        AppliedPosition { term: 5, index: 12 }
    );
    let original = facts.installed().unwrap();
    assert_eq!(original.image().watermark(), Some(floor(1000)));
    assert_eq!(
        original.image().manifest(),
        RootDigest::sha256(&f.original_manifest.encode().unwrap())
    );
    assert!(!original.verified_original_during_adoption());
    let json: serde_json::Value = serde_json::from_str(&manager.status_json()).unwrap();
    assert_eq!(
        json[0]["recovery_at_open"]["installed"]["local_adoption_records_required"],
        true
    );
    assert_eq!(
        json[0]["recovery_at_open"]["installed"]["engine_open_used_original_image"],
        false
    );
    manager.retire_published_parent(f.range.region).unwrap();
    assert!(manager.recovery_status(f.range.region).is_err());
}

#[test]
fn adopted_original_base_still_refuses_uncommitted_engine_tail_before_starting_driver() {
    let f = Installed::new();
    let adopted = adopt_for_runtime(&f.guard, f.identity, f.range.region, &f.uploader).unwrap();
    let mut storage = DiskRaftStorage::recover(&adopted.raft_directory).unwrap();
    let engine = adopted.open_engine(&mut storage, &f.uploader).unwrap().0;
    let mut batch = WriteBatch::new();
    batch.put(
        ColumnFamily::Default,
        CLOCK_KEY.to_vec(),
        floor(2000).encode().to_vec(),
    );
    engine
        .write_applied(batch, AppliedPosition { term: 5, index: 11 })
        .unwrap();
    drop(engine);
    drop(storage);
    drop(adopted);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let transport = GrpcTransport::new(
        f.identity.node_id,
        None,
        runtime.handle().clone(),
        f.identity.root_digest,
    );
    let mut manager = RegionManager::new(&f.path, f.identity, 2);
    let error = manager
        .adopt_installed(
            &f.guard,
            &f.migration,
            &transport,
            Duration::from_millis(10),
            &f.uploader,
            RecoveryDecisions {
                truncations: &[],
                compactions: &[],
            },
        )
        .unwrap_err();
    assert!(error.to_string().contains("commit"));
    assert!(manager.driver(f.range.region).is_err());
    assert!(manager.recovery_status(f.range.region).is_err());
}
