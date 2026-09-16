use super::*;
use kv9_common::{BootstrapGeneration, ClusterId, NodeId, RootDescriptor, RootVoter};
use kv9_engine::{ColumnFamily, Engine};
use kv9_meta::{
    codec::memcmp_uint,
    schema::{ColumnId, NODES_DESC},
    ColumnValue, MetaStore, RowValue,
};

struct Fixture {
    path: PathBuf,
    store: MetaStore<WalEngine>,
    identity: StoreIdentity,
    creation: CommittedCreation,
}
impl Fixture {
    fn new() -> Self {
        let base = std::env::var_os("KV9_TEST_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let path = base.join(format!(
            "kv9-group-prepare-{}",
            StoreIncarnation::mint().unwrap()
        ));
        fs::create_dir_all(&path).unwrap();
        let store = MetaStore::new(Arc::new(
            WalEngine::open(path.join("metadata.wal")).unwrap().0,
        ));
        let voters: Vec<_> = (1..=3)
            .map(|id| RootVoter {
                node_id: NodeId(id),
                addr: format!("127.0.0.1:{}", 19000 + id).parse().unwrap(),
                store_incarnation: StoreIncarnation::from_bytes([id as u8; 16]),
            })
            .collect();
        let root = RootDescriptor::new(
            ClusterId::from_bytes([8; 16]),
            BootstrapGeneration::from_bytes([9; 16]),
            voters.clone(),
            b"group-store-test",
        )
        .unwrap();
        let mut txn = store.begin().unwrap();
        kv9_meta::root::initialize_root(&mut txn, &root).unwrap();
        for voter in voters {
            let mut row = RowValue::new();
            row.set(ColumnId(2), ColumnValue::Text(voter.addr.to_string()));
            row.set(ColumnId(3), ColumnValue::Uint(2));
            row.set(ColumnId(4), ColumnValue::Uint(0));
            row.set(
                ColumnId(5),
                ColumnValue::Bytes(voter.store_incarnation.as_bytes().to_vec()),
            );
            txn.insert(&NODES_DESC, &[memcmp_uint(voter.node_id.0)], row)
                .unwrap();
        }
        let intent = kv9_meta::data_groups::plan_empty_group(
            &mut txn,
            [7; 16],
            &[NodeId(1), NodeId(2), NodeId(3)],
        )
        .unwrap();
        txn.commit().unwrap();
        let creation = kv9_meta::data_groups::committed_creation(&store, intent.task())
            .unwrap()
            .unwrap();
        Self {
            path,
            store,
            identity: StoreIdentity::for_voter(&root, NodeId(1)).unwrap(),
            creation,
        }
    }
    fn manager(&self) -> RegionManager {
        RegionManager::new(&self.path, self.identity)
    }
    fn group_dir(&self) -> PathBuf {
        self.path
            .join("data-groups")
            .join(self.creation.intent().region().0.to_string())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("retained failed group fixture: {}", self.path.display());
        } else {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn test_transport(runtime: &tokio::runtime::Runtime, f: &Fixture) -> Arc<GrpcTransport> {
    GrpcTransport::new(
        f.identity.node_id,
        None,
        runtime.handle().clone(),
        f.identity.root_digest,
    )
}

#[test]
fn group_control_bounds_activation_and_quarantines_failures() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let f = Fixture::new();
    let mut txn = f.store.begin().unwrap();
    let mut regions = vec![f.creation.intent().region()];
    kv9_meta::data_groups::activation::plan_activation(&mut txn, f.creation.intent()).unwrap();
    for operation in [11, 12] {
        let intent = kv9_meta::data_groups::plan_empty_group(
            &mut txn,
            [operation; 16],
            &[NodeId(1), NodeId(2), NodeId(3)],
        )
        .unwrap();
        kv9_meta::data_groups::activation::plan_activation(&mut txn, &intent).unwrap();
        regions.push(intent.region());
    }
    let requests = kv9_meta::data_groups::activation::committed_activations(&f.store).unwrap();
    let mut manager = f.manager();
    let transport = test_transport(&runtime, &f);
    manager.reconcile_activation(&requests, &transport, Duration::from_secs(60));
    assert!(
        manager.groups.is_empty(),
        "uncommitted desire must not start any group"
    );
    txn.commit().unwrap();
    let requests = kv9_meta::data_groups::activation::committed_activations(&f.store).unwrap();
    // An orphan store must be quarantined, without starving later requests.
    fs::create_dir_all(f.group_dir()).unwrap();
    fs::write(f.group_dir().join("raft.log"), b"orphan").unwrap();
    manager.reconcile_activation(&requests, &transport, Duration::from_secs(60));
    assert_eq!(
        manager.groups.len(),
        1,
        "one attempt per turn, including failure"
    );
    assert!(matches!(
        manager.groups.get(&regions[0]),
        Some(LocalGroup::Failed(_))
    ));
    assert!(manager.status(regions[1]).is_err());
    manager.reconcile_activation(&requests, &transport, Duration::from_secs(60));
    assert_eq!(manager.groups.len(), 2);
    assert!(manager.status(regions[1]).is_ok());
    assert!(manager.status(regions[2]).is_err());
    manager.reconcile_activation(&requests, &transport, Duration::from_secs(60));
    assert_eq!(manager.groups.len(), 3);
    assert!(manager.status(regions[2]).is_ok());
    for _ in 0..3 {
        manager.reconcile_activation(&requests, &transport, Duration::from_secs(60));
    }
    assert_eq!(fs::read(f.group_dir().join("raft.log")).unwrap(), b"orphan");
    let status: serde_json::Value = serde_json::from_str(&manager.status_json()).unwrap();
    assert_eq!(status[0]["state"], "failed");
    assert_eq!(status[1]["state"], "active");
    // Even the right node number on a replacement disk has no authority.
    let foreign_path = f.path.join("replacement");
    let mut identity = f.identity;
    identity.store_incarnation = StoreIncarnation::from_bytes([99; 16]);
    let mut foreign = RegionManager::new(&foreign_path, identity);
    foreign.reconcile_activation(&requests, &transport, Duration::from_secs(60));
    assert!(foreign.groups.is_empty());
    assert!(!foreign_path.exists());
}

#[test]
fn group_activation_publication_cuts_never_start_an_unfenced_voter() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for step in [
        PrepareStep::ActiveFileSync,
        PrepareStep::ActiveRename,
        PrepareStep::ActiveDirectorySync,
    ] {
        for after in [false, true] {
            let f = Fixture::new();
            let mut manager = f.manager();
            manager.prepare(&f.creation).unwrap();
            let transport = test_transport(&runtime, &f);
            let region = f.creation.intent().region();
            let mut fired = false;
            let result = manager.start_group(
                region,
                &transport,
                Duration::from_secs(60),
                &mut |at, done| {
                    if at == step && done == after {
                        fired = true;
                        return Err(invalid("injected activation interruption"));
                    }
                    Ok(())
                },
            );
            assert!(fired && result.is_err());
            assert!(
                manager.status(region).is_err(),
                "failed publication exposed a voter"
            );
            assert!(
                transport.register_group(region).is_ok(),
                "inbox was bound before Active publication finished"
            );
            assert!(
                manager
                    .activate(&f.creation, &transport, Duration::from_secs(60))
                    .is_err(),
                "ambiguous activation must require recovery"
            );
            drop(manager);
            let mut recovered = f.manager();
            recovered.recover(&f.store).unwrap();
            let transport = test_transport(&runtime, &f);
            recovered
                .activate(&f.creation, &transport, Duration::from_secs(60))
                .unwrap();
            assert!(recovered.status(region).is_ok());
            assert_eq!(
                read_record(&f.group_dir().join(RECORD))
                    .unwrap()
                    .unwrap()
                    .phase,
                Phase::Active
            );
        }
    }
}

#[test]
fn group_activation_recovery_refuses_missing_or_empty_stores_and_foreign_history() {
    use kv9_raft::rawnode::PersistentRaftStorage;
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for mode in 0..10 {
        let f = Fixture::new();
        let mut manager = f.manager();
        manager
            .activate(
                &f.creation,
                &test_transport(&runtime, &f),
                Duration::from_secs(60),
            )
            .unwrap();
        drop(manager);
        let log = f.group_dir().join("raft/raft.log");
        let engine = f.group_dir().join("data.wal");
        match mode {
            0 => fs::remove_file(&log).unwrap(),
            1 => fs::write(&log, []).unwrap(),
            2 => fs::remove_file(&engine).unwrap(),
            3 => {
                fs::remove_file(&log).unwrap();
                DiskRaftStorage::open(&f.group_dir().join("raft"), &[1, 2, 4]).unwrap();
            }
            4 => {
                let e = WalEngine::open(&engine).unwrap().0;
                let mut batch = kv9_engine::WriteBatch::new();
                batch.put(ColumnFamily::Default, b"foreign".to_vec(), b"data".to_vec());
                e.write(batch).unwrap();
            }
            5 => {
                // A rollback to Ready must not turn a previously voting store
                // into an empty replica, even if the local record is valid.
                let storage = DiskRaftStorage::recover(&f.group_dir().join("raft")).unwrap();
                storage
                    .set_hardstate(&raft::eraftpb::HardState {
                        term: 3,
                        vote: 1,
                        ..Default::default()
                    })
                    .unwrap();
                let mut record = read_record(&f.group_dir().join(RECORD)).unwrap().unwrap();
                record.phase = Phase::StorageReady;
                publish(&f.group_dir(), &record, &mut |_, _| Ok(())).unwrap();
            }
            6 | 7 => {
                if mode == 7 {
                    let storage = DiskRaftStorage::recover(&f.group_dir().join("raft")).unwrap();
                    storage
                        .append(&[raft::eraftpb::Entry {
                            index: 1,
                            term: 2,
                            data: kv9_raft::Command::Noop.encode().into(),
                            ..Default::default()
                        }])
                        .unwrap();
                    storage
                        .set_hardstate(&raft::eraftpb::HardState {
                            term: 2,
                            vote: 1,
                            commit: 1,
                            ..Default::default()
                        })
                        .unwrap();
                }
                let e = WalEngine::open(&engine).unwrap().0;
                e.write_applied(
                    kv9_engine::WriteBatch::new(),
                    kv9_common::AppliedPosition { term: 3, index: 1 },
                )
                .unwrap();
            }
            8 => fs::remove_dir_all(f.group_dir().join("data.segments")).unwrap(),
            9 => {
                let storage = DiskRaftStorage::recover(&f.group_dir().join("raft")).unwrap();
                storage
                    .append(&[1, 2].map(|index| raft::eraftpb::Entry {
                        index,
                        term: 2,
                        data: kv9_raft::Command::Noop.encode().into(),
                        ..Default::default()
                    }))
                    .unwrap();
                storage
                    .set_hardstate(&raft::eraftpb::HardState {
                        term: 2,
                        vote: 1,
                        commit: 2,
                        ..Default::default()
                    })
                    .unwrap();
                let e = WalEngine::open(&engine).unwrap().0;
                e.write_applied(
                    kv9_engine::WriteBatch::new(),
                    kv9_common::AppliedPosition { term: 1, index: 1 },
                )
                .unwrap();
                e.write_applied(
                    kv9_engine::WriteBatch::new(),
                    kv9_common::AppliedPosition { term: 2, index: 2 },
                )
                .unwrap();
                // The final position agrees; an earlier frame is from a wrong
                // term. Validation must inspect the replay, not just its tail.
            }
            _ => unreachable!(),
        }
        let mut recovered = f.manager();
        recovered.recover(&f.store).unwrap();
        assert!(
            recovered.prepare(&f.creation).is_err(),
            "unsafe active recovery accepted mode={mode}"
        );
        if mode == 0 {
            assert!(!log.exists(), "missing Active log was recreated");
        }
        if mode == 1 {
            assert_eq!(
                fs::metadata(log).unwrap().len(),
                0,
                "empty Active log was initialized"
            );
        }
        if mode == 2 {
            assert!(!engine.exists(), "missing Active engine was recreated");
        }
    }
}

#[test]
fn group_preparation_recovers_every_publication_boundary() {
    use PrepareStep::*;
    for step in [
        IntentFileSync,
        IntentRename,
        IntentDirectorySync,
        RaftOpen,
        EngineOpen,
        ReadyFileSync,
        ReadyRename,
        ReadyDirectorySync,
    ] {
        for after in [false, true] {
            let f = Fixture::new();
            let mut manager = f.manager();
            let mut fired = false;
            let result = manager.prepare_observed(&f.creation, &mut |at, completed| {
                if at == step && completed == after {
                    fired = true;
                    return Err(invalid("injected preparation interruption"));
                }
                Ok(())
            });
            assert!(
                fired && result.is_err(),
                "fault must hit {step:?} after={after}"
            );
            assert!(
                manager.prepare(&f.creation).is_err(),
                "failed publication must poison this group until recovery"
            );
            drop(manager);
            let mut recovered = f.manager();
            recovered.recover(&f.store).unwrap();
            let observation = recovered.prepare(&f.creation).unwrap_or_else(|e| {
                panic!("preparation must recover at {step:?} after={after}: {e}")
            });
            assert_eq!(observation.intent_digest, f.creation.intent().digest());
            assert_eq!(
                read_record(&f.group_dir().join(RECORD))
                    .unwrap()
                    .unwrap()
                    .phase,
                Phase::StorageReady
            );
            assert_eq!(recovered.observations().len(), 1);
        }
    }
}

#[test]
fn group_preparation_refuses_identity_conflicts_and_duplicate_disk_owner() {
    let f = Fixture::new();
    let identity = StoreIdentity {
        store_incarnation: StoreIncarnation::from_bytes([99; 16]),
        ..f.identity
    };
    assert!(
        RegionManager::new(&f.path, identity)
            .prepare(&f.creation)
            .is_err(),
        "replacement disk must refuse before preparing any local files"
    );
    assert!(!f.group_dir().exists());
    let mut manager = f.manager();
    let first = manager.prepare(&f.creation).unwrap();
    assert_eq!(
        manager.prepare(&f.creation).unwrap(),
        first,
        "duplicate prepare must be idempotent"
    );
    assert!(
        f.manager().prepare(&f.creation).is_err(),
        "second owner must not open the same group logs"
    );
    assert!(
        RegionManager::new(&f.path, identity)
            .prepare(&f.creation)
            .is_err(),
        "replacement disk must not reuse the old voter identity"
    );
    drop(manager);
    let mut record = read_record(&f.group_dir().join(RECORD)).unwrap().unwrap();
    record.incarnation = StoreIncarnation::from_bytes([99; 16]);
    publish(&f.group_dir(), &record, &mut |_, _| Ok(())).unwrap();
    assert!(
        f.manager().prepare(&f.creation).is_err(),
        "committed intent must match the local durable binding"
    );
}

#[test]
fn group_preparation_never_reinitializes_ready_or_orphaned_logs() {
    for missing in ["raft/raft.log", "data.wal", RECORD] {
        let f = Fixture::new();
        f.manager().prepare(&f.creation).unwrap();
        let target = f.group_dir().join(missing);
        fs::remove_file(&target).unwrap();
        let mut recovered = f.manager();
        recovered.recover(&f.store).unwrap();
        assert!(
            recovered.prepare(&f.creation).is_err(),
            "missing {missing} must refuse instead of reinitializing"
        );
        assert!(!target.exists(), "refusal must not recreate {missing}");
    }
    let f = Fixture::new();
    f.manager().prepare(&f.creation).unwrap();
    let path = f.group_dir().join("raft/raft.log");
    fs::write(&path, []).unwrap();
    assert!(
        f.manager().prepare(&f.creation).is_err(),
        "empty ready Raft log must never regain a fresh voter set"
    );
    assert_eq!(fs::metadata(&path).unwrap().len(), 0);
}

#[test]
fn group_preparation_rejects_voting_history_foreign_configuration_and_data() {
    use kv9_raft::rawnode::PersistentRaftStorage;
    for mode in 0..3 {
        let f = Fixture::new();
        f.manager().prepare(&f.creation).unwrap();
        if mode == 0 {
            let storage = DiskRaftStorage::recover(&f.group_dir().join("raft")).unwrap();
            storage
                .set_hardstate(&raft::eraftpb::HardState {
                    term: 1,
                    vote: 1,
                    ..Default::default()
                })
                .unwrap();
        } else if mode == 1 {
            fs::remove_file(f.group_dir().join("raft/raft.log")).unwrap();
            DiskRaftStorage::open(&f.group_dir().join("raft"), &[1, 2, 4]).unwrap();
        } else {
            let engine = WalEngine::open(f.group_dir().join("data.wal")).unwrap().0;
            let mut batch = kv9_engine::WriteBatch::new();
            batch.put(
                ColumnFamily::Default,
                b"foreign".to_vec(),
                b"value".to_vec(),
            );
            engine.write(batch).unwrap();
        }
        assert!(
            f.manager().prepare(&f.creation).is_err(),
            "unstarted group must reject nonempty or foreign storage, mode={mode}"
        );
    }
}

#[test]
fn group_preparation_corrupt_group_does_not_prevent_other_group_recovery() {
    let f = Fixture::new();
    let mut txn = f.store.begin().unwrap();
    let second = kv9_meta::data_groups::plan_empty_group(
        &mut txn,
        [8; 16],
        &[NodeId(1), NodeId(2), NodeId(3)],
    )
    .unwrap();
    txn.commit().unwrap();
    let second = kv9_meta::data_groups::committed_creation(&f.store, second.task())
        .unwrap()
        .unwrap();
    let mut manager = f.manager();
    manager.prepare(&f.creation).unwrap();
    manager.prepare(&second).unwrap();
    drop(manager);
    fs::write(f.group_dir().join(RECORD), b"corrupt").unwrap();
    let mut recovered = f.manager();
    recovered
        .recover(&f.store)
        .expect("one corrupt group must not abort peer group recovery");
    assert!(recovered.prepare(&f.creation).is_err());
    assert_eq!(
        recovered.prepare(&second).unwrap().region,
        second.intent().region(),
        "one failed group must not prevent an independent group recovering"
    );
}
