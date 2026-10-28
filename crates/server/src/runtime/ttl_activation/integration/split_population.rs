//! The ordinary Raw split path shares the TTL fixture's real Raft/WAL drivers.
//! TTL remains disabled here: its format-transfer protocol is still separate.
use super::*;
use kv9_common::codec::{encode_key, KeyMode};
use kv9_common::data_range::RANGE_KEY;
use kv9_raft::{FencedInner, KvOp, RegionFence, StateMachine};

async fn applied(drivers: &[Driver], at: kv9_raft::ProposedAt) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while drivers
            .iter()
            .any(|d| d.status().applied_index < at.index.0)
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}
fn fenced(range: &DataRange, ops: Vec<KvOp>) -> Command {
    Command::Fenced {
        fence: RegionFence {
            region_id: range.region.0,
            conf_ver: range.conf_ver,
            version: range.version,
        },
        inner: FencedInner::Write { ops },
    }
}
fn put(key: Vec<u8>, value: &[u8]) -> KvOp {
    KvOp::Put {
        cf: 0,
        key,
        value: value.to_vec(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn split_population_retries_preserve_published_child_writes_across_three_replica_reopen() {
    let mut f = Fixture::new().await;
    let root = f.range.root;
    let mut txn = f.backends[1].node.meta_raft.store.begin().unwrap();
    let mut creations = Vec::new();
    for operation in [61, 62] {
        let creation = kv9_meta::data_groups::plan_empty_group(
            &mut txn,
            [operation; 16],
            &[NodeId(1), NodeId(2), NodeId(3)],
        )
        .unwrap();
        kv9_meta::data_groups::activation::plan_activation(&mut txn, &creation).unwrap();
        creations.push(creation);
    }
    let at = f.meta[1]
        .propose(&Command::from_batch(&txn.into_batch()))
        .unwrap();
    applied(&f.meta, at).await;
    let intent = f.backends[1]
        .record_split_intent(
            "test",
            root,
            [63; 16],
            f.range.region,
            b"m",
            creations[0].task(),
            creations[1].task(),
        )
        .unwrap();
    applied(
        &f.meta,
        kv9_raft::ProposedAt {
            term: intent.applied.term,
            index: kv9_raft::LogIndex(intent.applied.index),
        },
    )
    .await;

    for i in 0..3 {
        let engine = f.backends[i]
            .raw_directory
            .by_region(f.range.region)
            .unwrap()
            .capture_parts()
            .1
            .clone();
        f.backends[i]
            .group_handles
            .lock()
            .unwrap()
            .insert(f.range.region, (engine, f.data[i].clone()));
    }
    let mut children = Vec::new();
    for creation in &creations {
        let hub = InProcHub::new();
        let mut drivers = Vec::new();
        for i in 0..3 {
            let path = f
                .path
                .join(format!("n{}/split-{}", i + 1, creation.region().0));
            let engine = Arc::new(WalEngine::open(path.with_extension("wal")).unwrap().0);
            let storage = DiskRaftStorage::open(&path, &[1, 2, 3]).unwrap().0;
            let peer = Arc::new(
                RaftPeer::with_storage(NodeId(i as u64 + 1), creation.region(), storage).unwrap(),
            );
            let mut sm = MemStateMachine::with_engine(engine.clone()).unwrap();
            sm.set_data_group(root, creation.region(), creation.digest())
                .unwrap();
            let driver = NodeDriver::new(
                peer,
                Arc::new(hub.endpoint(NodeId(i as u64 + 1))) as Arc<dyn RaftTransport>,
                sm,
            )
            .unwrap();
            f.backends[i]
                .group_handles
                .lock()
                .unwrap()
                .insert(creation.region(), (engine, driver.clone()));
            drivers.push(driver);
        }
        drivers[0].peer().campaign().unwrap();
        pump(&drivers);
        assert_eq!(drivers[0].status().role, Role::Leader);
        for driver in &drivers {
            f.owners
                .push(driver.spawn(Duration::from_millis(5)).unwrap());
            f.data.push(driver.clone());
        }
        children.push(drivers);
    }

    let key = |s: &[u8]| encode_key(KeyMode::Raw, f.range.keyspace, s).unwrap();
    let mut original = (0..300)
        .map(|i| put(key(format!("a{i:04}").as_bytes()), b"parent"))
        .collect::<Vec<_>>();
    original.push(put(key(b"z"), b"high"));
    let at = f.data[0]
        .propose(&fenced(&f.range, original.clone()))
        .unwrap();
    applied(&f.data[..3], at).await;
    let seal = f.backends[0]
        .seal_split_parent("test", root, [63; 16])
        .unwrap();
    applied(
        &f.data[..3],
        kv9_raft::ProposedAt {
            term: seal.cut.term,
            index: kv9_raft::LogIndex(seal.cut.index),
        },
    )
    .await;
    // A delayed client write cannot change the already-sealed copy source.
    let late_parent = f.data[0]
        .propose(&fenced(&f.range, vec![put(key(b"a0000"), b"late parent")]))
        .unwrap();
    assert!(matches!(
        f.data[0]
            .wait_applied(late_parent, Duration::from_secs(5))
            .unwrap(),
        ApplyWaitOutcome::FenceRejected { .. }
    ));
    for high in [false, false, true] {
        let result = f.backends[0]
            .populate_split_child("test", root, [63; 16], high)
            .unwrap();
        assert_eq!(result.rows_copied, if high { 1 } else { 300 });
        assert_eq!(result.parent_half_digest, result.child_digest);
    }
    for drivers in &children {
        let barrier = drivers[0].propose(&Command::Noop).unwrap();
        applied(drivers, barrier).await;
    }
    // Metadata and child leaders differ. Publication validates follower copies.
    f.backends[1].publish_split("test", root, [63; 16]).unwrap();
    let barrier = f.meta[1].propose(&Command::Noop).unwrap();
    applied(&f.meta, barrier).await;
    let ranges =
        kv9_meta::data_groups::ranges::committed_ranges(&f.backends[1].node.meta_raft.store)
            .unwrap();
    let low = ranges
        .iter()
        .find(|r| r.range().region == creations[0].region())
        .unwrap()
        .range()
        .clone();
    for (creation, drivers) in creations.iter().zip(&children) {
        let range = ranges
            .iter()
            .find(|r| r.range().region == creation.region())
            .unwrap()
            .range()
            .clone();
        let at = drivers[0]
            .propose(&Command::DataRange {
                expected: None,
                next: range,
            })
            .unwrap();
        applied(drivers, at).await;
    }
    let replacement_key = key(b"a0000");
    let deleted_key = key(b"a0001");
    let at = children[0][0]
        .propose(&fenced(
            &low,
            vec![
                put(replacement_key.clone(), b"new child value"),
                KvOp::Delete {
                    cf: 0,
                    key: deleted_key.clone(),
                },
            ],
        ))
        .unwrap();
    applied(&children[0], at).await;
    // An old caller that preflighted before binding can still have a batch in
    // flight. Submit the exact old copy bytes after the new writes are committed.
    original.pop();
    let mut sealed_parent = f.range.clone();
    sealed_parent.sealed = true;
    sealed_parent.version = seal.sealed_version;
    // One delayed frame from the first page; production frames are bounded.
    let rows = original
        .into_iter()
        .take(256)
        .map(|op| match op {
            KvOp::Put { key, value, .. } => (key, value),
            _ => unreachable!(),
        })
        .collect();
    let stale = Command::PopulateRaw(
        kv9_raft::RawPopulation::new(sealed_parent, low.clone(), [63; 16], rows).unwrap(),
    );
    let at = children[0][0].propose(&stale).unwrap();
    assert!(
        matches!(children[0][0].wait_applied(at, Duration::from_secs(5)).unwrap(), ApplyWaitOutcome::FenceRejected { region, .. } if region == low.region)
    );
    applied(&children[0], at).await;
    assert!(
        matches!(f.backends[0].populate_split_child("test", root, [63; 16], false), Err(Error::StaleEpoch { region }) if region == low.region)
    );
    assert!(
        !f.backends[1]
            .publish_split("test", root, [63; 16])
            .unwrap()
            .changed
    );
    for backend in &f.backends {
        let handles = backend.group_handles.lock().unwrap();
        let engine = &handles.get(&low.region).unwrap().0;
        assert_eq!(
            engine.get(ColumnFamily::Default, &replacement_key).unwrap(),
            Some(b"new child value".to_vec())
        );
        assert_eq!(
            engine.get(ColumnFamily::Default, &deleted_key).unwrap(),
            None
        );
    }

    for driver in f.meta.iter().chain(f.data.iter()) {
        driver.stop();
    }
    for thread in f.owners.drain(..) {
        thread.join().unwrap();
    }
    children.clear();
    f.data.clear();
    for backend in &f.backends {
        backend.group_handles.lock().unwrap().clear();
    }
    // Actual production WAL recovery observes both the publication and the
    // rejected batch. A new state machine must retain the same barrier.
    for id in 1..=3 {
        let path = f.path.join(format!("n{id}/split-{}", low.region.0));
        let mut storage = DiskRaftStorage::open(&path, &[1, 2, 3]).unwrap().0;
        let (engine, _, _) = kv9_raft::state_machine::checkpoint_recovery::data_group::open(
            &mut storage,
            path.with_extension("wal"),
            None,
            kv9_raft::state_machine::checkpoint_recovery::data_group::DataCheckpointIdentity {
                cluster: f.backends[0]
                    .node
                    .local_cluster_identity()
                    .unwrap()
                    .unwrap(),
                root,
                region: low.region,
                creation: low.creation,
            },
        )
        .unwrap();
        let engine = Arc::new(engine);
        assert_eq!(
            engine.get(ColumnFamily::Default, RANGE_KEY).unwrap(),
            Some(low.encode())
        );
        let mut sm = MemStateMachine::with_engine(engine.clone()).unwrap();
        sm.set_data_group(root, low.region, low.creation).unwrap();
        let result = sm
            .apply_at(
                AppliedPosition {
                    term: at.term,
                    index: sm.applied_index().0 + 1,
                },
                &stale,
            )
            .unwrap();
        assert_eq!(
            result.outcome,
            kv9_raft::ApplyOutcome::FenceRejected(low.region)
        );
        assert_eq!(
            engine.get(ColumnFamily::Default, &replacement_key).unwrap(),
            Some(b"new child value".to_vec())
        );
        assert_eq!(
            engine.get(ColumnFamily::Default, &deleted_key).unwrap(),
            None
        );
    }
    f.close().await;
}

#[test]
fn split_population_bounds_follow_nested_parent_range() {
    let range = DataRange {
        root: RootDigest::from_bytes([1; 32]),
        creation: RootDigest::from_bytes([2; 32]),
        region: RegionId(100),
        keyspace: kv9_common::KeyspaceId(100),
        tenant: TenantId::DEFAULT,
        conf_ver: 1,
        version: 2,
        start: b"b".to_vec(),
        end: b"t".to_vec(),
        sealed: true,
    };
    let encoded = |k: &[u8]| encode_key(KeyMode::Raw, range.keyspace, k).unwrap();
    assert_eq!(
        RuntimeBackend::split_bounds(&range, b"m").unwrap(),
        (encoded(b"b"), encoded(b"m"), encoded(b"t"))
    );
    for key in [b"a", b"b", b"t", b"z"] {
        assert!(RuntimeBackend::split_bounds(&range, key).is_err());
    }
    let mut unbounded = range.clone();
    unbounded.end.clear();
    let bounds = RuntimeBackend::split_bounds(&unbounded, b"m").unwrap();
    assert_eq!(bounds.0, encoded(b"b"));
    assert_eq!(
        bounds.2,
        encode_key(KeyMode::Raw, kv9_common::KeyspaceId(101), b"").unwrap()
    );
}
