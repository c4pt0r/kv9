//! Task #24: dynamic membership through the REAL apply pipeline.
//!
//! The flow under test is the one the runtime will drive: a cluster of 3
//! declared voters is live; a 4th node starts seeded with the log-start
//! (bootstrap) voter set, itself in NEITHER set
//! (`RaftPeer::new_joining` — it is nobody until admitted); the leader
//! proposes `AddLearnerNode`; the conf entry travels the log, each driver's
//! apply loop routes it through `apply_conf_change` (NOT `Command::decode` —
//! the pre-#24 pipeline fataled here, which was this test's natural positive
//! control); the joiner catches up from index 1 by plain AppendEntries (no
//! log truncation in Phase 1); promotion turns it into a voter; killing the
//! leader then proves the new 4-voter quorum really includes it.
//!
//! Assertions follow the #23/#24 review contract:
//! - a learner NEVER campaigns (calling `campaign()` on it must not produce
//!   a leader);
//! - every node reports the IDENTICAL voters/learners lists (list divergence
//!   is an earlier split-brain signal than a wrong leader — Ren);
//! - the joiner's role is derived from MEMBERSHIP (learner while in the
//!   learner set, follower/voter after promotion), and an admitted-then-caught
//!   -up node applies commands proposed BEFORE it ever joined;
//! - post-failover the new leader is drawn from the CURRENT voter set.

use std::sync::Arc;
use std::time::Duration;

use kv9_common::{NodeId, RegionId};
use kv9_engine::ColumnFamily;
use kv9_raft::driver::NodeDriver;
use kv9_raft::transport::{InProcHub, RaftTransport};
use kv9_raft::{cf_code, Command, EntryKind, MemStateMachine, RaftGroup, RaftPeer, Role};
use protobuf::Message as PbMessage;
use raft::eraftpb::{ConfChangeSingle, ConfChangeType, ConfChangeV2};

struct Node {
    driver: Arc<NodeDriver>,
    alive: bool,
}

fn put(key: &[u8], value: &[u8]) -> Command {
    Command::Put {
        cf: cf_code(ColumnFamily::Default),
        key: key.to_vec(),
        value: value.to_vec(),
    }
}

/// Step every alive node once. Apply errors crash the test: a poisoned driver
/// here means the pipeline mis-routed an entry.
fn drive(nodes: &mut [Node]) {
    for n in nodes.iter() {
        if n.alive {
            n.driver.tick_and_step().expect("driver poisoned");
        }
    }
}

fn drive_until<F: Fn(&[Node]) -> bool>(nodes: &mut [Node], what: &str, cond: F) {
    for _ in 0..2000 {
        drive(nodes);
        if cond(nodes) {
            return;
        }
    }
    panic!("condition never reached: {what}");
}

fn leader_of(nodes: &[Node]) -> Option<usize> {
    let mut found = None;
    for (i, n) in nodes.iter().enumerate() {
        if n.alive && n.driver.status().role == Role::Leader {
            if found.is_some() {
                return None; // two leaders visible: not settled
            }
            found = Some(i);
        }
    }
    found
}

#[test]
fn learner_joins_catches_up_promotes_and_survives_failover() {
    let region = RegionId(1);
    let voters: Vec<NodeId> = (1..=3).map(NodeId).collect();
    let hub = InProcHub::new();

    let mut nodes: Vec<Node> = voters
        .iter()
        .map(|&id| {
            let peer = Arc::new(RaftPeer::new(id, region, &voters).unwrap());
            let endpoint = hub.endpoint(id);
            let driver = NodeDriver::new(
                peer,
                Arc::new(endpoint) as Arc<dyn RaftTransport>,
                MemStateMachine::new(),
            );
            Node {
                driver,
                alive: true,
            }
        })
        .collect();

    // Elect a leader among the 3 declared voters.
    nodes[0].driver.peer().campaign().unwrap();
    drive_until(&mut nodes, "initial leader", |ns| leader_of(ns).is_some());
    let leader = leader_of(&nodes).unwrap();

    // A command proposed BEFORE node 4 exists — the joiner must catch it up
    // from the log later (AppendEntries from index 1; no truncation).
    let early = nodes[leader]
        .driver
        .propose(&put(b"early", b"pre-join"))
        .unwrap();
    drive_until(&mut nodes, "early write applied", |ns| {
        ns[leader]
            .driver
            .wait_applied(early, Duration::from_millis(1))
            .map(|o| matches!(o, kv9_raft::driver::ApplyWaitOutcome::Applied(_)))
            .unwrap_or(false)
    });

    // ---- Join: node 4 starts as NOBODY (empty config). ----
    let joiner_id = NodeId(4);
    let peer4 = Arc::new(RaftPeer::new_joining(joiner_id, region, &voters).unwrap());
    let endpoint4 = hub.endpoint(joiner_id);
    let driver4 = NodeDriver::new(
        peer4,
        Arc::new(endpoint4) as Arc<dyn RaftTransport>,
        MemStateMachine::new(),
    );
    nodes.push(Node {
        driver: driver4,
        alive: true,
    });

    // Leader admits it as a LEARNER; confirm by exact (term, index) and by the
    // membership apply_conf_change actually produced.
    let at = nodes[leader].driver.add_learner(joiner_id).unwrap();
    drive_until(&mut nodes, "learner conf applied on leader", |ns| {
        ns[leader].driver.status().conf_index == at.index.0
    });
    let receipt = nodes[leader]
        .driver
        .wait_conf_applied(at, Duration::from_secs(5))
        .unwrap();
    assert_eq!(receipt.voters, vec![1, 2, 3]);
    assert_eq!(receipt.learners, vec![4]);

    // The joiner catches up (including the early pre-join write) and reports
    // itself a LEARNER — membership-derived, since raft has no learner
    // StateRole.
    drive_until(&mut nodes, "joiner caught up + learner role", |ns| {
        let s = ns[3].driver.status();
        s.role == Role::Learner
            && ns[3]
                .driver
                .wait_applied(early, Duration::from_millis(1))
                .map(|o| matches!(o, kv9_raft::driver::ApplyWaitOutcome::Applied(_)))
                .unwrap_or(false)
    });

    // Every node — including the learner — reports IDENTICAL lists.
    for n in nodes.iter() {
        let s = n.driver.status();
        assert_eq!(s.voters, vec![1, 2, 3], "voter list diverged");
        assert_eq!(s.learners, vec![4], "learner list diverged");
    }

    // A learner must never campaign its way to leadership.
    let _ = nodes[3].driver.peer().campaign();
    for _ in 0..50 {
        drive(&mut nodes);
        assert_ne!(
            nodes[3].driver.status().role,
            Role::Leader,
            "a learner became leader"
        );
    }

    // ---- Promote: learner -> voter, one change at a time. ----
    let at = nodes[leader].driver.promote_voter(joiner_id).unwrap();
    drive_until(&mut nodes, "promotion applied on leader", |ns| {
        ns[leader].driver.status().conf_index == at.index.0
    });
    let receipt = nodes[leader]
        .driver
        .wait_conf_applied(at, Duration::from_secs(5))
        .unwrap();
    assert_eq!(receipt.voters, vec![1, 2, 3, 4]);
    assert!(receipt.learners.is_empty());

    // All four report the new membership; node 4 is now an ordinary follower.
    drive_until(&mut nodes, "all nodes see 4 voters", |ns| {
        ns.iter()
            .all(|n| n.driver.status().voters == vec![1, 2, 3, 4])
    });
    assert_eq!(nodes[3].driver.status().role, Role::Follower);

    // ---- Failover: kill the leader; the new 4-voter quorum (3 of 4) must
    // elect a replacement FROM THE VOTER SET, and a post-failover write must
    // apply on every survivor. ----
    nodes[leader].alive = false;
    drive_until(&mut nodes, "post-failover leader", |ns| {
        leader_of(ns).is_some_and(|l| l != leader)
    });
    let new_leader = leader_of(&nodes).unwrap();
    let s = nodes[new_leader].driver.status();
    assert!(
        s.voters.contains(&s.node_id.0),
        "new leader is not in the voter set"
    );

    let after = nodes[new_leader]
        .driver
        .propose(&put(b"after", b"post-failover"))
        .unwrap();
    drive_until(&mut nodes, "post-failover write on all survivors", |ns| {
        ns.iter().filter(|n| n.alive).all(|n| {
            n.driver
                .wait_applied(after, Duration::from_millis(1))
                .map(|o| matches!(o, kv9_raft::driver::ApplyWaitOutcome::Applied(_)))
                .unwrap_or(false)
        })
    });
}

/// The joining constructor's identity guarantee: before any conf change admits
/// it, a joining node is in NEITHER set and must report `Unconfigured` — never
/// a healthy-looking follower (Ren's three-way rule; the pre-#24 code could
/// only say "follower").
#[test]
fn unadmitted_joiner_reports_unconfigured_not_follower() {
    let region = RegionId(9);
    let hub = InProcHub::new();
    let voters: Vec<NodeId> = (1..=3).map(NodeId).collect();
    let peer = Arc::new(RaftPeer::new_joining(NodeId(7), region, &voters).unwrap());
    let endpoint = hub.endpoint(NodeId(7));
    let driver = NodeDriver::new(
        peer,
        Arc::new(endpoint) as Arc<dyn RaftTransport>,
        MemStateMachine::new(),
    );
    driver.tick_and_step().unwrap();
    let s = driver.status();
    assert_eq!(s.role, Role::Unconfigured);
    // It knows the cluster's bootstrap voters, but is in neither set itself.
    assert_eq!(s.voters, vec![1, 2, 3]);
    assert!(s.learners.is_empty());
}

fn cc_bytes(node: u64, kind: ConfChangeType) -> Vec<u8> {
    let mut step = ConfChangeSingle::default();
    step.set_change_type(kind);
    step.node_id = node;
    let mut cc = ConfChangeV2::default();
    cc.set_changes(vec![step].into());
    cc.write_to_bytes().unwrap()
}

/// The restart replay guard, deterministically (Tess's P0): single-step conf
/// ops are RELATIVE — replaying an already-applied `AddLearner(4)` onto a
/// config where 4 was later promoted DEMOTES it. A conf entry at or below the
/// recovered boundary must be skipped, not re-applied.
///
/// Sensitivity: without the `index <= conf_applied` guard, the final assert
/// fails with node 4 demoted to learner (verified by running against the
/// guard-free code).
#[test]
fn stale_conf_entry_replay_is_skipped_not_reapplied() {
    let voters: Vec<NodeId> = (1..=3).map(NodeId).collect();
    let peer = RaftPeer::new(NodeId(1), RegionId(3), &voters).unwrap();

    let (v, l) = peer
        .apply_conf_change_bytes(
            EntryKind::ConfChangeV2,
            &cc_bytes(4, ConfChangeType::AddLearnerNode),
            5,
        )
        .unwrap();
    assert_eq!((v, l), (vec![1, 2, 3], vec![4]));

    let (v, l) = peer
        .apply_conf_change_bytes(
            EntryKind::ConfChangeV2,
            &cc_bytes(4, ConfChangeType::AddNode),
            6,
        )
        .unwrap();
    assert_eq!((v, l), (vec![1, 2, 3, 4], vec![]));

    // Replay of the OLD AddLearner at index 5 (as a restart would hand it
    // back with Config.applied = 0): membership must be UNCHANGED.
    let (v, l) = peer
        .apply_conf_change_bytes(
            EntryKind::ConfChangeV2,
            &cc_bytes(4, ConfChangeType::AddLearnerNode),
            5,
        )
        .unwrap();
    assert_eq!(
        (v, l),
        (vec![1, 2, 3, 4], vec![]),
        "stale conf replay demoted a promoted voter"
    );
}

/// Cindy's hold-release condition 2: after a conf change in a LATER term than
/// the last command, status must keep reporting the COMMAND's own
/// `(applied_index, applied_term)` pair — never the old command index glued to
/// the conf entry's newer term (a position that never existed). Sensitivity:
/// with conf entries pushed into the command ring (the pre-fix behavior) the
/// `applied_term` assert below reads the conf term and fails.
///
/// SOLE DEFENSE: until a dynamic-membership external E2E exists, no external
/// gate performs conf changes, so nothing outside this test can observe this
/// defect class — anyone touching the ring code answers to THIS test alone
/// (hold-release audit, 2026-08-28).
#[test]
fn status_pair_stays_command_sourced_across_conf_changes() {
    let region = RegionId(2);
    let voters: Vec<NodeId> = (1..=3).map(NodeId).collect();
    let hub = InProcHub::new();
    let mut nodes: Vec<Node> = voters
        .iter()
        .map(|&id| {
            let peer = Arc::new(RaftPeer::new(id, region, &voters).unwrap());
            let endpoint = hub.endpoint(id);
            Node {
                driver: NodeDriver::new(
                    peer,
                    Arc::new(endpoint) as Arc<dyn RaftTransport>,
                    MemStateMachine::new(),
                ),
                alive: true,
            }
        })
        .collect();

    nodes[0].driver.peer().campaign().unwrap();
    drive_until(&mut nodes, "leader", |ns| leader_of(ns).is_some());
    let first = leader_of(&nodes).unwrap();

    // Last command, in the FIRST leader's term.
    let cmd_at = nodes[first]
        .driver
        .propose(&put(b"pair", b"anchor"))
        .unwrap();
    drive_until(&mut nodes, "command applied everywhere", |ns| {
        ns.iter().all(|n| {
            n.driver
                .wait_applied(cmd_at, Duration::from_millis(1))
                .map(|o| matches!(o, kv9_raft::driver::ApplyWaitOutcome::Applied(_)))
                .unwrap_or(false)
        })
    });

    // Force a term change: kill the leader, elect another.
    nodes[first].alive = false;
    drive_until(&mut nodes, "new leader", |ns| {
        leader_of(ns).is_some_and(|l| l != first)
    });
    let second = leader_of(&nodes).unwrap();
    let new_term = nodes[second].driver.status().term;
    assert!(new_term > cmd_at.term, "term must have advanced");

    // Conf change in the NEW term, with NO new command.
    let conf_at = nodes[second].driver.add_learner(NodeId(9)).unwrap();
    assert_eq!(conf_at.term, new_term);
    let receipt = nodes[second]
        .driver
        .wait_conf_applied(conf_at, Duration::from_secs(5));
    // Drive until the conf is applied on the new leader.
    for _ in 0..2000 {
        drive(&mut nodes);
        if nodes[second].driver.status().conf_index == conf_at.index.0 {
            break;
        }
    }
    drop(receipt); // first call may have raced the pump; re-check below
    let receipt = nodes[second]
        .driver
        .wait_conf_applied(conf_at, Duration::from_secs(5))
        .unwrap();
    assert_eq!(receipt.learners, vec![9]);

    let s = nodes[second].driver.status();
    assert_eq!(s.conf_index, conf_at.index.0, "conf_index is its own field");
    assert_eq!(
        (s.applied_index, s.applied_term),
        (cmd_at.index.0, cmd_at.term),
        "status pair must be the last COMMAND's own (index, term) — a conf \
         entry in a newer term must not contaminate it"
    );
}

/// Tess's P0, recovery half: after learner->promote, a REOPENED node comes
/// back with the FINAL membership and the conf boundary, and replay does not
/// re-apply conf entries. The mechanical distinguisher: re-application would
/// append fresh `REC_CONF_STATE_AT` records, so the raft log FILE MUST NOT
/// GROW during replay (a converged end-state alone could hide a
/// demote-then-repromote round trip — Tess's correction).
#[test]
fn reopen_recovers_conf_state_without_reapplying() {
    use kv9_raft::rawnode::PersistentRaftStorage;
    use kv9_raft::storage::DiskRaftStorage;

    let region = RegionId(7);
    let voters: Vec<NodeId> = (1..=3).map(NodeId).collect();
    let base = std::env::temp_dir().join(format!(
        "kv9-membership-reopen-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&base);
    let dir = |n: u64| {
        let d = base.join(format!("n{n}"));
        std::fs::create_dir_all(&d).unwrap();
        d
    };

    let voter_ids: Vec<u64> = voters.iter().map(|n| n.0).collect();
    let hub = InProcHub::new();
    let mk = |id: NodeId| {
        let (storage, _) = DiskRaftStorage::open(&dir(id.0), &voter_ids).unwrap();
        let peer = Arc::new(RaftPeer::with_storage(id, region, storage).unwrap());
        let endpoint = hub.endpoint(id);
        NodeDriver::new(
            peer,
            Arc::new(endpoint) as Arc<dyn RaftTransport>,
            MemStateMachine::new(),
        )
    };
    let mut drivers: Vec<_> = voters.iter().map(|&id| mk(id)).collect();
    type DiskDriver = Arc<NodeDriver<DiskRaftStorage>>;
    let drive_all = |ds: &[DiskDriver]| {
        for d in ds {
            d.tick_and_step().expect("disk driver poisoned");
        }
    };
    let wait_for = |ds: &mut Vec<DiskDriver>, what: &str, cond: &dyn Fn(&[DiskDriver]) -> bool| {
        for _ in 0..2000 {
            drive_all(ds);
            if cond(ds) {
                return;
            }
        }
        panic!("condition never reached: {what}");
    };

    drivers[0].peer().campaign().unwrap();
    wait_for(&mut drivers, "disk leader", &|ds| {
        ds.iter().any(|d| d.status().role == Role::Leader)
    });
    let leader = drivers
        .iter()
        .position(|d| d.status().role == Role::Leader)
        .unwrap();

    // Join node 4 (disk-backed, seeded with the log-start voters), promote it.
    drivers.push(mk(NodeId(4)));
    let at = drivers[leader].add_learner(NodeId(4)).unwrap();
    wait_for(&mut drivers, "disk learner applied", &|ds| {
        ds[leader].status().conf_index == at.index.0
    });
    let at = drivers[leader].promote_voter(NodeId(4)).unwrap();
    let _ = at;
    wait_for(&mut drivers, "disk promote applied", &|ds| {
        ds.iter().all(|d| d.status().voters == vec![1, 2, 3, 4])
    });

    // Restart node 2: reopen its raft log alone and replay.
    let victim = 1usize; // index of node 2
    assert_eq!(drivers[victim].status().node_id, NodeId(2));
    drop(drivers.remove(victim));

    let log_path = dir(2).join("raft.log");
    let len_before = std::fs::metadata(&log_path).unwrap().len();

    let (storage, was_pristine) = DiskRaftStorage::open(&dir(2), &voter_ids).unwrap();
    assert!(!was_pristine);
    assert!(
        storage.recovered_conf_index() > 0,
        "conf boundary must be recovered from the paired record"
    );
    let recovered = storage.recovered_conf_index();
    let peer = Arc::new(RaftPeer::with_storage(NodeId(2), region, storage).unwrap());
    let driver = NodeDriver::new(
        peer,
        Arc::new(hub.endpoint(NodeId(2))) as Arc<dyn RaftTransport>,
        MemStateMachine::new(),
    );
    // Drive alone: with raft applied starting at 0, the committed prefix is
    // re-handed to the driver; commands re-apply into the fresh (volatile)
    // state machine, conf entries must be SKIPPED by the boundary guard.
    for _ in 0..200 {
        driver.tick_and_step().expect("reopen replay poisoned");
    }
    let s = driver.status();
    assert_eq!(s.fatal, None);
    assert_eq!(
        s.voters,
        vec![1, 2, 3, 4],
        "reopen lost the final membership"
    );
    assert_eq!(s.conf_index, recovered);
    let len_after = std::fs::metadata(&log_path).unwrap().len();
    assert_eq!(
        len_before, len_after,
        "replay re-applied conf entries (fresh ConfStateAt records were written)"
    );

    let _ = std::fs::remove_dir_all(&base);
}
