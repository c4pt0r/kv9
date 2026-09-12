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
    // This voting-only fixture bypasses renewal transport deliberately. The
    // renewal tests below exercise the production envelope and driver boundary.
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

#[cfg(target_os = "linux")]
#[test]
fn clock_contract_is_rechecked_before_the_durable_installation() {
    use crate::lease_clock::{ClockBounds, LinuxBoottimeClock};
    let dir = Directory(
        std::env::temp_dir().join(format!("kv9-lease-clock-policy-{}", std::process::id())),
    );
    std::fs::create_dir(&dir.0).unwrap();
    let mut p = policy(1, &[1]);
    p.promise_ns = 1_000_000_000;
    p.drift_ppb = 1_000_000;
    p.margin_ns = 4_005;
    let clock = Arc::new(
        LinuxBoottimeClock::new(
            &p,
            ClockBounds {
                drift_ppb: 1_000_000,
                error_ns: 1_000,
            },
        )
        .unwrap(),
    );
    p.margin_ns -= 1;
    let (storage, _) = DiskRaftStorage::open(&dir.0, &[1]).unwrap();
    assert!(
        matches!(RaftPeer::with_lease_storage(NodeId(1), RegionId(0), storage, p, clock),
        Err(Error::Raft(ref cause)) if cause.contains("InvalidTiming")),
        "installation accepted a clock whose error exceeds its policy margin"
    );
    let storage = DiskRaftStorage::recover(&dir.0).unwrap();
    assert_eq!(
        storage.recovered_lease_epoch(),
        None,
        "invalid clock policy durably opted the peer into leases"
    );
    let peer = RaftPeer::with_storage(NodeId(1), RegionId(0), storage).unwrap();
    drop(peer);
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

// The following helpers inspect algorithm certificates only. They cannot mint
// a ReadBarrier or an immutable server read view.
fn certificate(peer: &RaftPeer<DiskRaftStorage>) -> bool {
    let mut g = peer.lock();
    let PeerInner { raw, lease, .. } = &mut *g;
    let lease = lease.as_mut().unwrap();
    let Ok(now) = lease.synchronize(raw) else {
        return false;
    };
    let progress = lease.progress(raw);
    lease
        .leader
        .as_mut()
        .is_some_and(|leader| leader.begin_read(now, progress, u64::MAX).is_ok())
}

struct BoundPeers {
    drains: Vec<DrainToken<DiskRaftStorage>>,
    peers: Vec<Arc<RaftPeer<DiskRaftStorage>>>,
    clocks: Vec<Arc<TestClock>>,
    _dirs: Vec<Directory>,
}

impl BoundPeers {
    fn new() -> Self {
        let mut peers = Vec::new();
        let mut clocks = Vec::new();
        let mut dirs = Vec::new();
        for node in 1..=3 {
            let (dir, clock, peer) = fresh(node, &[1, 2, 3]);
            clock.set(100);
            peers.push(Arc::new(peer));
            clocks.push(clock);
            dirs.push(dir);
        }
        peers[0].campaign().unwrap();
        for _ in 0..12 {
            for peer in &peers {
                for m in peer.pump().unwrap() {
                    peers[(m.to - 1) as usize].step_message(m);
                }
            }
        }
        assert_eq!(peers[0].role(), Role::Leader);
        assert!(peers[0].lock().raw.raft.commit_to_current_term());
        assert!(!certificate(&peers[0]));
        Self {
            drains: peers.iter().map(|p| DrainToken::mint(p).unwrap()).collect(),
            peers,
            clocks,
            _dirs: dirs,
        }
    }

    fn request(&self) -> Message {
        let batch = self.drains[0].pump_owned().unwrap();
        assert!(!batch.messages.iter().any(lease_wire::reserved));
        self.drains[0]
            .complete_pump(batch.publication)
            .unwrap()
            .into_iter()
            .find(|m| m.to == 2)
            .expect("request to voter 2")
    }

    fn grant(&self, request: Message) -> Message {
        self.peers[1].step_message(request);
        let batch = self.drains[1].pump_owned().unwrap();
        assert!(!batch.messages.iter().any(lease_wire::reserved));
        let persisted = self.peers[1]
            .lock()
            .raw
            .store()
            .initial_state()
            .unwrap()
            .hard_state;
        assert_eq!(persisted.term, self.peers[0].term());
        self.drains[1]
            .complete_pump(batch.publication)
            .unwrap()
            .into_iter()
            .find(lease_wire::reserved)
            .expect("durable grant after complete pump")
    }
}

#[test]
fn renewal_certificate_requires_exact_grant_and_successful_owning_pump() {
    let f = BoundPeers::new();
    let request = f.request();
    let mut echo = request.clone();
    echo.from = 2;
    echo.to = 1;
    echo.msg_type = MsgHeartbeatResponse;
    f.peers[0].step_message(echo.clone());
    echo.log_term = 0;
    f.peers[0].step_message(echo);
    let batch = f.drains[0].pump_owned().unwrap();
    f.drains[0].complete_pump(batch.publication).unwrap();
    assert!(
        !certificate(&f.peers[0]),
        "heartbeat echo granted authority"
    );

    f.peers[0].step_message(f.grant(request));
    assert!(!certificate(&f.peers[0]), "ACK receipt published authority");
    let batch = f.drains[0].pump_owned().unwrap();
    assert!(
        !certificate(&f.peers[0]),
        "Ready persistence published authority"
    );
    f.drains[0].complete_pump(batch.publication).unwrap();
    assert!(certificate(&f.peers[0]));
    assert!(
        f.peers[0].take_read_states().is_empty(),
        "lease fabricated a Safe ReadIndex receipt"
    );
}

#[test]
fn grant_after_capture_belongs_to_the_next_pump() {
    let f = BoundPeers::new();
    let grant = f.grant(f.request());
    let older = f.drains[0].pump_owned().unwrap();
    f.peers[0].step_message(grant);
    f.drains[0].complete_pump(older.publication).unwrap();
    assert!(
        !certificate(&f.peers[0]),
        "later input leaked into an older pump"
    );
    let current = f.drains[0].pump_owned().unwrap();
    f.drains[0].complete_pump(current.publication).unwrap();
    assert!(certificate(&f.peers[0]));
}

#[test]
fn pump_publication_rejects_foreign_peer_and_superseded_turn() {
    let f = BoundPeers::new();
    let request = f.request();
    f.peers[1].step_message(request.clone());
    let foreign = f.drains[1].pump_owned().unwrap();
    assert!(f.drains[2].complete_pump(foreign.publication).is_err());
    f.peers[1].step_message(request);
    let older = f.drains[1].pump_owned().unwrap();
    let newer = f.drains[1].pump_owned().unwrap();
    assert!(f.drains[1].complete_pump(older.publication).is_err());
    assert!(f.drains[1]
        .complete_pump(newer.publication)
        .unwrap()
        .is_empty());
    assert!(!certificate(&f.peers[0]));
}

#[test]
fn delayed_request_grant_and_quorum_publication_cannot_extend_send_deadline() {
    let f = BoundPeers::new();
    let delayed = f.drains[0].pump_owned().unwrap();
    f.clocks[0].set(200);
    assert!(f.drains[0]
        .complete_pump(delayed.publication)
        .unwrap()
        .is_empty());
    assert!(!certificate(&f.peers[0]));

    let f = BoundPeers::new();
    f.peers[1].step_message(f.request());
    let delayed = f.drains[1].pump_owned().unwrap();
    f.clocks[1].set(200);
    assert!(f.drains[1]
        .complete_pump(delayed.publication)
        .unwrap()
        .is_empty());
    assert!(
        f.peers[1].campaign().is_ok(),
        "duplicate publication extended voter hold"
    );

    let f = BoundPeers::new();
    f.peers[0].step_message(f.grant(f.request()));
    let delayed = f.drains[0].pump_owned().unwrap();
    f.clocks[0].set(200);
    assert!(f.drains[0]
        .complete_pump(delayed.publication)
        .unwrap()
        .is_empty());
    assert!(
        !certificate(&f.peers[0]),
        "expired quorum published authority"
    );
}

#[test]
fn old_round_and_old_term_grants_cannot_authorize_a_renewal() {
    let f = BoundPeers::new();
    let old = f.grant(f.request());
    f.clocks[0].set(150);
    f.clocks[1].set(150);
    let current = f.request();
    f.peers[0].step_message(old.clone());
    let batch = f.drains[0].pump_owned().unwrap();
    f.drains[0].complete_pump(batch.publication).unwrap();
    assert!(!certificate(&f.peers[0]));
    f.peers[0].step_message(f.grant(current));
    let batch = f.drains[0].pump_owned().unwrap();
    f.drains[0].complete_pump(batch.publication).unwrap();
    assert!(certificate(&f.peers[0]));
    let term = f.peers[0].term();
    f.peers[0].step_message(message(MsgHeartbeat, 3, 1, term + 1));
    f.peers[0].step_message(old);
    let batch = f.drains[0].pump_owned().unwrap();
    f.drains[0].complete_pump(batch.publication).unwrap();
    assert!(!certificate(&f.peers[0]));
    assert_eq!(f.peers[0].role(), Role::Follower);
}

#[test]
fn transfer_and_clock_failure_revoke_renewals_without_erasing_voter_hold() {
    let f = BoundPeers::new();
    f.peers[0].step_message(f.grant(f.request()));
    let batch = f.drains[0].pump_owned().unwrap();
    f.drains[0].complete_pump(batch.publication).unwrap();
    assert!(certificate(&f.peers[0]));
    f.peers[0].transfer_leader_for_tests(NodeId(2));
    assert!(!certificate(&f.peers[0]));
    assert!(f.peers[0].campaign().is_err());
    f.clocks[0].set(150);
    let batch = f.drains[0].pump_owned().unwrap();
    assert!(f.drains[0]
        .complete_pump(batch.publication)
        .unwrap()
        .is_empty());

    let f = BoundPeers::new();
    f.peers[0].step_message(f.grant(f.request()));
    let batch = f.drains[0].pump_owned().unwrap();
    f.clocks[0].failed.store(true, Ordering::SeqCst);
    assert!(f.drains[0]
        .complete_pump(batch.publication)
        .unwrap()
        .is_empty());
    f.clocks[0].failed.store(false, Ordering::SeqCst);
    f.clocks[0].set(1_000);
    assert!(!certificate(&f.peers[0]));
    assert!(f.peers[0].campaign().is_err());
}

#[test]
fn failed_driver_apply_never_publishes_staged_grant() {
    use crate::transport::{InProcHub, RaftTransport};
    let (_dir, clock, peer) = fresh(2, &[1, 2, 3]);
    clock.set(100);
    let peer = Arc::new(peer);
    let hub = InProcHub::new();
    let remote = hub.endpoint(NodeId(1));
    let driver = crate::driver::NodeDriver::new(
        peer.clone(),
        Arc::new(hub.endpoint(NodeId(2))),
        crate::MemStateMachine::new(),
    )
    .unwrap();
    let mut append = message(MsgAppend, 1, 2, 1);
    append.commit = 1;
    append.entries.push(raft::eraftpb::Entry {
        term: 1,
        index: 1,
        data: vec![0xff, 0xee, 0xdd].into(),
        ..Default::default()
    });
    let renewal = Renewal {
        authority: Authority {
            group: 0,
            configuration: 7,
            leader: 1,
            term: 1,
            incarnation: 1,
        },
        generation: 0,
        sequence: 1,
        promise_ns: 100,
    };
    remote.send(NodeId(2), append);
    remote.send(
        NodeId(2),
        lease_wire::encode(Kind::Request, renewal, &policy(1, &[1, 2, 3]), 2),
    );
    assert!(driver.step().is_err());
    assert!(driver.status().fatal.is_some());
    assert!(
        !remote.drain().iter().any(lease_wire::reserved),
        "failed application published a grant"
    );
    assert!(peer.lock().lease.as_ref().unwrap().fenced);
    clock.set(1_000);
    assert!(peer.campaign().is_err());

    // A failing pump must also revoke an already published certificate.
    let (_single_dir, single_clock, single) = fresh(1, &[1]);
    single_clock.set(100);
    let single = Arc::new(single);
    let single_driver = crate::driver::NodeDriver::new(
        single.clone(),
        Arc::new(InProcHub::new().endpoint(NodeId(1))),
        crate::MemStateMachine::new(),
    )
    .unwrap();
    single.campaign().unwrap();
    for _ in 0..3 {
        single_driver.step().unwrap();
    }
    assert!(certificate(&single));
    single.propose_traced(vec![0xff, 0xee, 0xdd]).unwrap();
    assert!(single_driver.step().is_err());
    assert!(
        !certificate(&single),
        "failed apply retained an active certificate"
    );
}

#[test]
fn actual_drivers_renew_then_refuse_expired_isolated_authority_and_stop_revokes() {
    use crate::transport::InProcHub;
    let mut dirs = Vec::new();
    let mut clocks = Vec::new();
    let mut drivers = Vec::new();
    let hub = InProcHub::new();
    for node in 1..=3 {
        let (dir, clock, peer) = fresh(node, &[1, 2, 3]);
        clock.set(100);
        drivers.push(
            crate::driver::NodeDriver::new(
                Arc::new(peer),
                Arc::new(hub.endpoint(NodeId(node))),
                crate::MemStateMachine::new(),
            )
            .unwrap(),
        );
        clocks.push(clock);
        dirs.push(dir);
    }
    drivers[0].peer().campaign().unwrap();
    for _ in 0..16 {
        for driver in &drivers {
            driver.step().unwrap();
        }
    }
    assert!(certificate(drivers[0].peer()));
    for clock in &clocks {
        clock.set(150);
    }
    for _ in 0..12 {
        for driver in &drivers {
            driver.step().unwrap();
        }
    }
    clocks[0].set(249);
    assert!(
        certificate(drivers[0].peer()),
        "renewal did not extend the first certificate"
    );
    // No follower driver runs after the cut: queued requests have no ACK.
    clocks[0].set(250);
    for _ in 0..4 {
        drivers[0].step().unwrap();
    }
    assert!(
        !certificate(drivers[0].peer()),
        "isolated expired leader retained authority"
    );
    clocks[1].set(250);
    clocks[2].set(250);
    // Drop traffic crossing the partition while preserving messages between
    // the surviving voters. This tests actual raft-rs elections after holds.
    use crate::transport::RaftTransport;
    for node in 2..=3 {
        hub.endpoint(NodeId(node)).drain();
    }
    drivers[1].peer().campaign().unwrap();
    for _ in 0..64 {
        for driver in &drivers[1..] {
            driver.tick_and_step().unwrap();
        }
        hub.endpoint(NodeId(1)).drain();
    }
    let replacement = drivers[1..]
        .iter()
        .find(|driver| driver.peer().role() == Role::Leader)
        .expect("the surviving quorum elects a replacement after promise expiry");
    assert!(certificate(replacement.peer()));
    assert!(!certificate(drivers[0].peer()));
    replacement.stop();
    assert!(
        !certificate(replacement.peer()),
        "driver stop retained lease authority"
    );
    drop(drivers);
    drop(dirs);
}
