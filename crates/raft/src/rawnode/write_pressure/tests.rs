use super::*;
use crate::driver::{ApplyWaitOutcome, NodeDriver};
use crate::transport::{InProcHub, RaftTransport};
use crate::{Command, MemStateMachine};
use kv9_engine::{ColumnFamily, Engine, WalEngine};
use std::sync::Barrier;
use std::time::Duration;

fn put(key: impl Into<Vec<u8>>) -> Command {
    Command::Put {
        cf: 0,
        key: key.into(),
        value: vec![7; 1024],
    }
}

fn retained<S: PersistentRaftStorage>(peer: &RaftPeer<S>) -> u64 {
    let status = peer.status_snapshot();
    status
        .log_last_index
        .saturating_sub(status.log_first_index - 1)
}

#[test]
fn concurrent_preflight_success_cannot_overbook_uncommitted_entries() {
    let nodes = [NodeId(1), NodeId(2), NodeId(3)];
    let cluster = InProcessCluster::new(RegionId(9), &nodes).unwrap();
    cluster.peers()[0].campaign().unwrap();
    cluster
        .run_until(100, "leader", |c| c.leader().is_some())
        .unwrap();
    let peer = cluster.peer(cluster.leader().unwrap()).unwrap();
    let committed = peer.raft_committed();
    let before = peer.status_snapshot().log_last_index;
    let limit = retained(peer) + 7;
    peer.set_max_write_log_entries(limit);
    let barrier = Barrier::new(48);
    let accepted = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..48)
            .map(|i| {
                let barrier = &barrier;
                scope.spawn(move || {
                    peer.check_write_pressure().unwrap();
                    barrier.wait();
                    peer.propose_command_in_term(&put(format!("key-{i}")), None)
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|handle| match handle.join().unwrap() {
                Ok(at) => Some(at),
                Err(Error::WriteBackpressure {
                    region: RegionId(9),
                }) => None,
                other => panic!("unexpected admission: {other:?}"),
            })
            .collect::<Vec<_>>()
    });
    assert_eq!(accepted.len(), 7);
    assert_eq!(retained(peer), limit);
    assert_eq!(peer.status_snapshot().log_last_index, before + 7);
    assert_eq!(
        peer.raft_committed(),
        committed,
        "no quorum traffic was pumped"
    );
    // A stricter local threshold on followers must not interrupt replication.
    for follower in cluster.peers().iter().filter(|p| p.node != peer.node) {
        follower.set_max_write_log_entries(1);
    }
    cluster
        .run_until(100, "accepted suffix commits on every replica", |c| {
            c.peers().iter().all(|p| p.raft_committed().0 >= before + 7)
        })
        .unwrap();
    for p in cluster.peers() {
        p.check_write_pressure().unwrap_err();
        assert!(p.lock().fatal.is_none());
    }
}

#[test]
fn inclusive_boundary_empty_compacted_tail_and_control_progress() {
    let storage = MemStorage::new_with_conf_state((vec![1], vec![]));
    let peer = RaftPeer::with_storage(NodeId(1), RegionId(9), storage.clone()).unwrap();
    peer.campaign().unwrap();
    peer.pump().unwrap();
    assert_eq!(retained(&peer), 1, "leader barrier counts too");
    peer.set_max_write_log_entries(1);
    assert!(matches!(
        peer.propose_command_in_term(&put(b"blocked"), None),
        Err(Error::WriteBackpressure { .. })
    ));
    let control = peer.propose_command_in_term(&Command::Noop, None).unwrap();
    peer.pump().unwrap();
    assert_eq!(peer.raft_committed(), control.index);
    // Preserve the compacted base term/index when making the tail empty.
    // Production uses the exact authorized floor and checkpoint-history guards.
    let snapshot = raft::Storage::snapshot(&storage, control.index.0, 0).unwrap();
    storage.wl().apply_snapshot(snapshot).unwrap();
    assert_eq!(retained(&peer), 0);
    peer.propose_command_in_term(&put(b"one-slot"), None)
        .unwrap();
    assert_eq!(retained(&peer), 1);
    assert!(matches!(
        peer.propose_command_in_term(&put(b"no-slot"), None),
        Err(Error::WriteBackpressure { .. })
    ));
}

#[test]
fn disk_restart_keeps_pressure_and_real_compaction_releases_it() {
    use crate::storage::DiskRaftStorage;
    let dir = std::env::temp_dir().join(format!(
        "kv9-write-pressure-{}",
        kv9_common::StoreIncarnation::mint().unwrap()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let open = || {
        let engine = Arc::new(WalEngine::open(dir.join("data.wal")).unwrap().0);
        let storage = DiskRaftStorage::open(&dir.join("raft"), &[1]).unwrap().0;
        let peer = Arc::new(RaftPeer::with_storage(NodeId(1), RegionId(9), storage).unwrap());
        peer.set_max_write_log_entries(3);
        let hub = InProcHub::new();
        let driver = NodeDriver::new(
            peer,
            Arc::new(hub.endpoint(NodeId(1))) as Arc<dyn RaftTransport>,
            MemStateMachine::with_engine(engine.clone()).unwrap(),
        )
        .unwrap();
        driver.peer().campaign().unwrap();
        driver.step().unwrap();
        (driver, engine)
    };
    let (driver, engine) = open();
    let first = driver.propose(&put(b"first")).unwrap();
    let last = driver.propose(&put(b"last")).unwrap();
    assert!(matches!(
        driver.propose(&put(b"never")),
        Err(Error::WriteBackpressure { .. })
    ));
    driver.step().unwrap();
    for at in [first, last] {
        assert!(matches!(
            driver.wait_applied(at, Duration::ZERO).unwrap(),
            ApplyWaitOutcome::Applied(_)
        ));
    }
    assert_eq!(
        engine.get(ColumnFamily::Default, b"last").unwrap(),
        Some(vec![7; 1024])
    );
    assert_eq!(engine.get(ColumnFamily::Default, b"never").unwrap(), None);
    drop(driver);
    drop(engine);
    let (driver, engine) = open();
    assert!(matches!(
        driver.propose(&put(b"after-restart")),
        Err(Error::WriteBackpressure { .. })
    ));
    assert_eq!(
        engine.get(ColumnFamily::Default, b"first").unwrap(),
        Some(vec![7; 1024])
    );
    let bytes_before = driver.peer().log_file_bytes();
    let floor = kv9_common::AppliedPosition {
        term: last.term,
        index: last.index.0,
    };
    driver
        .peer()
        .truncate_retained_log(floor, b"test committed decision")
        .unwrap();
    assert!(driver.peer().reclaim_log().unwrap());
    assert!(driver.peer().log_file_bytes() < bytes_before);
    let resumed = driver.propose(&put(b"after-compaction")).unwrap();
    driver.step().unwrap();
    assert!(matches!(
        driver.wait_applied(resumed, Duration::ZERO).unwrap(),
        ApplyWaitOutcome::Applied(_)
    ));
    assert_eq!(
        engine
            .get(ColumnFamily::Default, b"after-compaction")
            .unwrap(),
        Some(vec![7; 1024])
    );
    drop(driver);
    drop(engine);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn pressure_is_local_to_each_group_and_zero_is_explicit_opt_out() {
    let peers: Vec<_> = [RegionId(1), RegionId(2)]
        .into_iter()
        .map(|region| {
            let p = RaftPeer::new(NodeId(1), region, &[NodeId(1)]).unwrap();
            p.campaign().unwrap();
            p.pump().unwrap();
            p.set_max_write_log_entries(1);
            p
        })
        .collect();
    peers[1].set_max_write_log_entries(0);
    for _ in 0..4 {
        assert!(matches!(
            peers[0].propose_command_in_term(&put(b"full"), None),
            Err(Error::WriteBackpressure {
                region: RegionId(1)
            })
        ));
        peers[1]
            .propose_command_in_term(&put(b"available"), None)
            .unwrap();
    }
    assert_eq!(retained(&peers[0]), 1);
    assert_eq!(retained(&peers[1]), 5);
}
