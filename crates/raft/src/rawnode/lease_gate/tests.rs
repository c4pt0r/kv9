use super::*;
use crate::lease::{Authority, Renewal};
use crate::storage::DiskRaftStorage;
use raft::eraftpb::MessageType::*;
use raft::Storage;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Default)]
struct TestClock {
    time: AtomicU64,
    failed: AtomicBool,
}

impl LeaseClock for TestClock {
    fn sample(&self) -> crate::lease::Result<ClockReading> {
        if self.failed.load(Ordering::SeqCst) {
            return Err(Refused::ClockChanged);
        }
        Ok(ClockReading {
            domain: 1,
            nanos: self.time.load(Ordering::SeqCst),
        })
    }
}

impl TestClock {
    fn set(&self, time: u64) {
        self.time.store(time, Ordering::SeqCst);
    }
}

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn policy(node: u64, voters: &[u64]) -> LeasePolicy {
    LeasePolicy {
        node,
        group: 0,
        configuration: 7,
        voters: voters.to_vec(),
        promise_ns: 100,
        drift_ppb: 0,
        margin_ns: 0,
    }
}

fn fresh(node: u64, voters: &[u64]) -> (Directory, Arc<TestClock>, RaftPeer<DiskRaftStorage>) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = Directory(std::env::temp_dir().join(format!(
        "kv9-lease-votes-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    )));
    std::fs::create_dir(&dir.0).unwrap();
    let (storage, _) = DiskRaftStorage::open(&dir.0, voters).unwrap();
    let clock = Arc::new(TestClock::default());
    let peer = RaftPeer::with_lease_storage(
        NodeId(node),
        RegionId(0),
        storage,
        policy(node, voters),
        clock.clone(),
    )
    .unwrap();
    (dir, clock, peer)
}

fn message(kind: raft::eraftpb::MessageType, from: u64, to: u64, term: u64) -> Message {
    Message {
        msg_type: kind,
        from,
        to,
        term,
        ..Default::default()
    }
}

fn establish_hold(peer: &RaftPeer<DiskRaftStorage>, clock: &TestClock) -> Renewal {
    clock.set(100); // Complete recovery quarantine before any grant.
    peer.step_message(message(MsgHeartbeat, 1, peer.node.0, 1));
    peer.pump().unwrap();
    let mut g = peer.lock();
    let term = g.raw.raft.term;
    let leader = g.raw.raft.leader_id;
    assert_eq!((term, leader), (1, 1));
    let renewal = Renewal {
        authority: Authority {
            group: 0,
            configuration: 7,
            leader,
            term,
            incarnation: 91,
        },
        generation: 0,
        sequence: 1,
        promise_ns: 100,
    };
    // Only this private test seam establishes a grant. There is no network
    // envelope or grant-publication adapter yet; no service read can use it.
    g.lease
        .as_mut()
        .unwrap()
        .voter
        .promise(clock.sample().unwrap(), term, leader, renewal)
        .unwrap();
    renewal
}

fn forced_vote(to: u64, term: u64) -> Message {
    let mut m = message(MsgRequestVote, 3, to, term);
    m.context = raft::CAMPAIGN_TRANSFER.to_vec().into();
    m
}

#[test]
fn quarantine_blocks_campaign_ticks_timeout_and_forced_vote_until_exact_boundary() {
    let (_dir, clock, peer) = fresh(2, &[1, 2, 3]);
    for time in [0, 99] {
        clock.set(time);
        assert!(peer.campaign().is_err());
        for _ in 0..100 {
            peer.tick_once();
        }
        peer.step_message(message(MsgTimeoutNow, 1, 2, 1));
        peer.step_message(forced_vote(2, 2));
        assert_eq!(peer.term(), 0);
        assert_eq!(peer.lock().raw.raft.vote, 0);
        assert!(peer.pump().unwrap().is_empty());
    }
    clock.set(100);
    peer.step_message(forced_vote(2, 2));
    let messages = peer.pump().unwrap();
    assert!(messages
        .iter()
        .any(|m| m.get_msg_type() == MsgRequestVoteResponse && !m.reject));
    let hs = peer.lock().raw.store().initial_state().unwrap().hard_state;
    assert_eq!(
        (hs.term, hs.vote),
        (2, 3),
        "vote was not durable before publication"
    );
}

#[test]
fn higher_term_replication_preserves_hold_and_forced_election_cannot_bypass_it() {
    let (_dir, clock, peer) = fresh(2, &[1, 2, 3]);
    establish_hold(&peer, &clock);
    clock.set(150);
    peer.step_message(message(MsgHeartbeat, 3, 2, 2));
    assert_eq!(
        peer.term(),
        2,
        "replication must still process higher terms"
    );
    peer.pump().unwrap();
    peer.step_message(forced_vote(2, 3));
    peer.step_message(message(MsgTimeoutNow, 3, 2, 2));
    assert!(peer.campaign().is_err());
    for _ in 0..100 {
        peer.tick_once();
    }
    assert_eq!(peer.term(), 2);
    assert_eq!(peer.lock().raw.raft.vote, 0);
    assert!(peer.pump().unwrap().is_empty());
    clock.set(200);
    peer.step_message(forced_vote(2, 3));
    assert!(peer
        .pump()
        .unwrap()
        .iter()
        .any(|m| m.get_msg_type() == MsgRequestVoteResponse && !m.reject));
}

#[test]
fn permitted_prevote_response_rechecks_clock_before_implicit_self_vote() {
    let (_dir, clock, peer) = fresh(2, &[1, 2, 3]);
    clock.set(100);
    peer.campaign().unwrap();
    assert_eq!(peer.lock().raw.raft.state, StateRole::PreCandidate);
    peer.pump().unwrap();
    // The response can internally advance PreCandidate -> Candidate and cast
    // a real self-vote. A clock fault after campaign must fence this route too.
    clock.failed.store(true, Ordering::SeqCst);
    peer.step_message(message(MsgRequestPreVoteResponse, 1, 2, 1));
    assert_eq!(peer.lock().raw.raft.state, StateRole::PreCandidate);
    assert_eq!(peer.lock().raw.raft.vote, 0);
    assert!(peer.pump().unwrap().is_empty());
    clock.failed.store(false, Ordering::SeqCst);
    clock.set(1_000);
    peer.step_message(message(MsgRequestPreVoteResponse, 1, 2, 1));
    assert_eq!(
        peer.lock().raw.raft.vote,
        0,
        "sampling fence must remain terminal"
    );
}

#[test]
fn eligible_prevote_quorum_still_enters_actual_election() {
    let (_dir, clock, peer) = fresh(2, &[1, 2, 3]);
    clock.set(100);
    peer.campaign().unwrap();
    peer.pump().unwrap();
    peer.step_message(message(MsgRequestPreVoteResponse, 1, 2, 1));
    assert_eq!(peer.lock().raw.raft.state, StateRole::Candidate);
    let messages = peer.pump().unwrap();
    assert!(messages.iter().any(|m| m.get_msg_type() == MsgRequestVote));
    let hs = peer.lock().raw.store().initial_state().unwrap().hard_state;
    assert_eq!((hs.term, hs.vote), (1, 2));
}

#[test]
fn restart_increments_epoch_restarts_quarantine_and_cannot_disable_or_shorten_policy() {
    let (dir, clock, peer) = fresh(2, &[1, 2, 3]);
    establish_hold(&peer, &clock);
    assert_eq!(peer.lease_incarnation(), Some(1));
    drop(peer);
    let storage = DiskRaftStorage::recover(&dir.0).unwrap();
    assert!(RaftPeer::with_storage(NodeId(2), RegionId(0), storage).is_err());
    let storage = DiskRaftStorage::recover(&dir.0).unwrap();
    let mut shorter = policy(2, &[1, 2, 3]);
    shorter.promise_ns = 1;
    assert!(
        RaftPeer::with_lease_storage(NodeId(2), RegionId(0), storage, shorter, clock.clone())
            .is_err()
    );
    let storage = DiskRaftStorage::recover(&dir.0).unwrap();
    let restarted_clock = Arc::new(TestClock::default());
    let peer = RaftPeer::with_lease_storage(
        NodeId(2),
        RegionId(0),
        storage,
        policy(2, &[1, 2, 3]),
        restarted_clock.clone(),
    )
    .unwrap();
    assert_eq!(peer.lease_incarnation(), Some(2));
    assert!(peer.campaign().is_err());
    restarted_clock.set(99);
    peer.step_message(forced_vote(2, 2));
    assert_eq!(peer.term(), 1);
    restarted_clock.set(100);
    peer.step_message(forced_vote(2, 2));
    assert!(peer
        .pump()
        .unwrap()
        .iter()
        .any(|m| m.get_msg_type() == MsgRequestVoteResponse && !m.reject));
}

#[test]
fn leader_quorum_ticks_continue_while_voting_is_held() {
    let (_dir, clock, peer) = fresh(1, &[1, 2, 3]);
    clock.set(100);
    peer.campaign().unwrap();
    peer.pump().unwrap();
    peer.step_message(message(MsgRequestPreVoteResponse, 2, 1, 1));
    peer.pump().unwrap();
    peer.step_message(message(MsgRequestVoteResponse, 2, 1, 1));
    peer.pump().unwrap();
    assert_eq!(peer.role(), Role::Leader);
    establish_hold(&peer, &clock);
    clock.set(150);
    for _ in 0..20 {
        peer.tick_once();
    }
    assert_eq!(
        peer.role(),
        Role::Follower,
        "lease gate suppressed check_quorum stepdown"
    );
    assert!(peer.campaign().is_err());
}

#[test]
fn snapshot_and_membership_cannot_change_fixed_lease_configuration() {
    let (_dir, clock, peer) = fresh(2, &[1, 2, 3]);
    establish_hold(&peer, &clock);
    let mut snapshot = message(MsgSnapshot, 3, 2, 2);
    snapshot.mut_snapshot().mut_metadata().index = 10;
    snapshot.mut_snapshot().mut_metadata().term = 2;
    snapshot
        .mut_snapshot()
        .mut_metadata()
        .set_conf_state(ConfState::from((vec![2, 3, 4], vec![])));
    peer.step_message(snapshot);
    assert_eq!(peer.term(), 1);
    assert_eq!(peer.membership().0, [1, 2, 3]);
    assert!(peer
        .propose_conf_change_traced(ConfChangeV2::default())
        .is_err());
    let before = peer.membership();
    assert!(peer
        .apply_conf_change_bytes(
            EntryKind::ConfChangeV2,
            &ConfChangeV2::default().write_to_bytes().unwrap(),
            1
        )
        .is_err());
    assert_eq!(peer.membership(), before);
    clock.set(1_000);
    assert!(
        peer.campaign().is_err(),
        "unproved membership apply must fence the voter"
    );
}

#[test]
fn installation_rejects_wrong_peer_membership_and_volatile_storage() {
    let (_dir, _clock, peer) = fresh(2, &[1, 2, 3]);
    let clock = Arc::new(TestClock::default());
    assert!(RaftPeer::with_lease_storage(
        NodeId(2),
        RegionId(0),
        MemStorage::new_with_conf_state(ConfState::from((vec![1, 2, 3], vec![]))),
        policy(2, &[1, 2, 3]),
        clock.clone()
    )
    .is_err());
    let mut wrong = policy(2, &[1, 2, 3]);
    wrong.group = 1;
    assert!(InstalledLease::install(
        NodeId(2),
        RegionId(0),
        &peer.lock().raw,
        wrong,
        clock.clone()
    )
    .is_err());
    assert!(InstalledLease::install(
        NodeId(2),
        RegionId(0),
        &peer.lock().raw,
        policy(2, &[2, 3, 4]),
        clock
    )
    .is_err());
    assert_eq!(peer.lease_incarnation(), Some(1));
}

#[test]
fn clock_regression_and_storage_failure_permanently_fence_voting() {
    let (_dir, clock, peer) = fresh(2, &[1, 2, 3]);
    establish_hold(&peer, &clock);
    clock.set(99);
    assert!(peer.campaign().is_err());
    clock.set(1_000);
    assert!(peer.campaign().is_err());
    let (_dir2, clock2, peer2) = fresh(2, &[1, 2, 3]);
    clock2.set(100);
    peer2
        .lock()
        .fail_storage("injected", Error::Raft("disk failure".into()));
    assert!(peer2.campaign().is_err());
    assert!(peer2.lock().lease_may_vote().is_err());
}

#[test]
fn isolated_leader_grant_quorum_blocks_replacement_until_expiry_then_raft_progresses() {
    let (_d1, c1, p1) = fresh(1, &[1, 2, 3]);
    let (_d2, c2, p2) = fresh(2, &[1, 2, 3]);
    let (_d3, c3, p3) = fresh(3, &[1, 2, 3]);
    let peers = [&p1, &p2, &p3];
    for c in [&c1, &c2, &c3] {
        c.set(100);
    }
    let deliver = |isolated: bool| {
        for peer in peers {
            for m in peer.pump().unwrap() {
                if isolated && (m.from == 1 || m.to == 1) {
                    continue;
                }
                peers[(m.to - 1) as usize].step_message(m);
            }
        }
    };
    p1.campaign().unwrap();
    for _ in 0..10 {
        deliver(false);
    }
    assert_eq!(p1.role(), Role::Leader);
    assert!(p1.lock().raw.raft.commit_to_current_term());
    establish_hold(&p1, &c1);
    establish_hold(&p2, &c2);
    for c in [&c1, &c2, &c3] {
        c.set(150);
    }
    // Isolate old leader 1. Unpromised voter 3 starts a forced election.
    // The only possible replacement quorum is {2,3}; voter 2 must refuse.
    p3.step_message(message(MsgTimeoutNow, 1, 3, 1));
    let requests = p3.pump().unwrap();
    let request = requests
        .iter()
        .find(|m| m.to == 2 && m.get_msg_type() == MsgRequestVote)
        .unwrap()
        .clone();
    p2.step_message(request.clone());
    for _ in 0..10 {
        deliver(true);
    }
    assert_ne!(p3.role(), Role::Leader);
    assert_eq!(p2.lock().raw.raft.vote, 1);
    assert!(p2.campaign().is_err());
    // At the exact hold boundary, the same deferred request can receive a
    // durable vote. Ordinary Raft commitment and Safe ReadIndex still work.
    c2.set(200);
    c3.set(200);
    p2.step_message(request);
    for _ in 0..10 {
        deliver(true);
    }
    assert_eq!(p3.role(), Role::Leader);
    assert!(p3.term() > p1.term());
    let at = p3
        .propose(&crate::Command::Put {
            cf: 0,
            key: b"after-expiry".to_vec(),
            value: b"v".to_vec(),
        })
        .unwrap();
    for _ in 0..10 {
        deliver(true);
    }
    assert!(p3.committed_index().0 >= at.0);
    assert!(p3.read_index(b"fresh-safe-read-index".to_vec()).unwrap());
    assert!(
        p3.take_read_states().is_empty(),
        "lease vote binding must not synthesize a ReadState"
    );
    for _ in 0..10 {
        deliver(true);
    }
    let reads = p3.take_read_states();
    assert!(reads
        .iter()
        .any(|s| s.request_ctx == b"fresh-safe-read-index" && s.index >= at.0));
}
