//! Task #24: dynamic membership through the REAL apply pipeline.
//!
//! The flow under test is the one the runtime will drive: a cluster of 3
//! declared voters is live; a 4th node starts with an EMPTY configuration
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
use kv9_raft::driver::NodeDriver;
use kv9_raft::transport::{InProcHub, RaftTransport};
use kv9_raft::{cf_code, Command, MemStateMachine, RaftGroup, RaftPeer, Role};
use kv9_engine::ColumnFamily;

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
    let early = nodes[leader].driver.propose(&put(b"early", b"pre-join")).unwrap();
    drive_until(&mut nodes, "early write applied", |ns| {
        ns[leader]
            .driver
            .wait_applied(early, Duration::from_millis(1))
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
        ns[leader]
            .driver
            .wait_applied(at, Duration::from_millis(1))
            .unwrap_or(false)
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
        ns[leader]
            .driver
            .wait_applied(at, Duration::from_millis(1))
            .unwrap_or(false)
    });
    let receipt = nodes[leader]
        .driver
        .wait_conf_applied(at, Duration::from_secs(5))
        .unwrap();
    assert_eq!(receipt.voters, vec![1, 2, 3, 4]);
    assert!(receipt.learners.is_empty());

    // All four report the new membership; node 4 is now an ordinary follower.
    drive_until(&mut nodes, "all nodes see 4 voters", |ns| {
        ns.iter().all(|n| n.driver.status().voters == vec![1, 2, 3, 4])
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
        ns.iter()
            .filter(|n| n.alive)
            .all(|n| {
                n.driver
                    .wait_applied(after, Duration::from_millis(1))
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
