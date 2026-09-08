//! Failure schedules execute the real DiskRaftStorage and raft-rs Ready loop.

use super::*;
use crate::rawnode::RaftPeer;
use kv9_common::fs::testing::{Crash, Fault, ModelFs, Operation};
use kv9_common::{NodeId, RegionId};
use raft::prelude::{Message, MessageType};

const DIRECTORY: &str = "/new-parent/replica/raft";

fn open(fs: &ModelFs) -> DiskRaftStorage<ModelFs> {
    DiskRaftStorage::open_on(fs.clone(), Path::new(DIRECTORY), &[1, 2, 3])
        .unwrap()
        .0
}

fn vote_request(candidate: u64, term: u64) -> Message {
    Message {
        msg_type: MessageType::MsgRequestVote,
        from: candidate,
        to: 1,
        term,
        ..Default::default()
    }
}

fn granted(messages: &[Message], candidate: u64, term: u64) -> bool {
    messages.iter().any(|m| {
        m.msg_type == MessageType::MsgRequestVoteResponse
            && m.to == candidate
            && m.term == term
            && !m.reject
    })
}

#[test]
fn acknowledged_vote_survives_loss_of_unsynced_namespace() {
    let fs = ModelFs::default();
    let peer = RaftPeer::with_storage(NodeId(1), RegionId(1), open(&fs)).unwrap();
    peer.step_message(vote_request(2, 7));
    assert!(
        granted(&peer.pump(), 2, 7),
        "positive control: the first vote must leave the real Ready loop"
    );
    assert!(fs
        .events()
        .iter()
        .any(|e| e.operation == Operation::SyncData));
    drop(peer);
    fs.crash(Crash::LoseUnsynced);
    let peer = RaftPeer::with_storage(NodeId(1), RegionId(1), open(&fs)).unwrap();
    peer.step_message(vote_request(3, 7));
    assert!(
        !granted(&peer.pump(), 3, 7),
        "a durable voter must not grant two candidates in the same term after power loss"
    );
}

#[test]
fn partial_append_failure_prevents_later_acknowledgment() {
    let fs = ModelFs::default();
    let store = open(&fs);
    // Establish the namespace separately: this cell isolates failed-write
    // continuation from the independent directory-publication defect.
    fs::sync_ancestors(&fs, Path::new(DIRECTORY)).unwrap();
    let hs = HardState {
        term: 7,
        vote: 2,
        ..Default::default()
    };
    store.set_hardstate(&hs).unwrap();
    fs.clear_events();
    fs.fail_at(0, Fault::ShortWrite { bytes: 4, errno: 5 });
    let failed = HardState {
        term: 8,
        vote: 3,
        ..Default::default()
    };
    assert!(store.set_hardstate(&failed).is_err());
    assert!(
        fs.fault_arrived(),
        "the real append must reach the selected short write"
    );
    let path = Path::new(DIRECTORY).join("raft.log");
    let broken = fs.visible_bytes(&path).unwrap();
    assert!(
        store.set_hardstate(&failed).is_err(),
        "a failed append must refuse later acknowledgments until recovery"
    );
    assert_eq!(
        fs.visible_bytes(&path).unwrap(),
        broken,
        "a poisoned writer must not append after the partial frame"
    );
    assert_eq!(raft::Storage::initial_state(&store).unwrap().hard_state, hs);
    drop(store);
    fs.crash(Crash::KeepUnsynced);
    assert_eq!(
        raft::Storage::initial_state(&open(&fs)).unwrap().hard_state,
        hs
    );
}

#[test]
fn every_initialization_io_cut_refuses_then_recovers() {
    let baseline = ModelFs::default();
    drop(open(&baseline));
    let cuts: Vec<_> = baseline
        .events()
        .into_iter()
        .filter(|e| !(e.operation == Operation::CreateDir && e.path == Path::new("/")))
        .collect();
    assert!(cuts.iter().any(|e| e.operation == Operation::SyncDir));
    for cut in &cuts {
        for errno in [5, 28] {
            for fault in [Fault::Before(errno), Fault::After(errno)] {
                let fs = ModelFs::default();
                fs.fail_at(cut.number, fault);
                let result = DiskRaftStorage::open_on(fs.clone(), Path::new(DIRECTORY), &[1, 2, 3]);
                assert!(
                    fs.fault_arrived(),
                    "initialization fault did not arrive: {cut:?} {fault:?}"
                );
                assert!(
                    result.is_err(),
                    "initialization I/O failure must refuse opening: {cut:?} {fault:?}"
                );
                // Retry while unsynced names from the previous process remain
                // visible. Publishing only newly created paths would lose them.
                let store = open(&fs);
                let hs = HardState {
                    term: 7,
                    vote: 2,
                    ..Default::default()
                };
                store.set_hardstate(&hs).unwrap();
                drop(store);
                fs.crash(Crash::LoseUnsynced);
                assert_eq!(raft::Storage::initial_state(&open(&fs)).unwrap().hard_state, hs,
                    "retry over visible but unsynced names lost an acknowledged vote: {cut:?} {fault:?}");
            }
        }
    }
    println!(
        "initialization matrix: {} named cuts, {} error/recovery cells",
        cuts.len(),
        cuts.len() * 4
    );
}

#[test]
fn write_and_sync_errors_preserve_acknowledged_prefix_for_seeded_crashes() {
    let mut cells = 0;
    for errno in [5, 28] {
        let faults = [
            (0, Fault::Before(errno)),
            (0, Fault::After(errno)),
            (0, Fault::ShortWrite { bytes: 4, errno }),
            (1, Fault::Before(errno)),
            (1, Fault::After(errno)),
        ];
        for (at, fault) in faults {
            for seed in 0..32 {
                let fs = ModelFs::default();
                let store = open(&fs);
                let old = HardState {
                    term: 7,
                    vote: 2,
                    ..Default::default()
                };
                let proposed = HardState {
                    term: 8,
                    vote: 3,
                    ..Default::default()
                };
                store.set_hardstate(&old).unwrap();
                fs.clear_events();
                fs.fail_at(at, fault);
                assert!(store.set_hardstate(&proposed).is_err());
                assert!(
                    fs.fault_arrived(),
                    "write fault did not arrive: {at} {fault:?} seed={seed}"
                );
                assert_eq!(
                    raft::Storage::initial_state(&store).unwrap().hard_state,
                    old,
                    "failed persistence must not publish its memory update"
                );
                let before = fs.events().len();
                assert!(store.set_hardstate(&proposed).is_err());
                assert_eq!(
                    fs.events().len(),
                    before,
                    "poisoned writes must perform no more filesystem operations"
                );
                drop(store);
                fs.crash(Crash::Seeded(seed));
                let recovered = raft::Storage::initial_state(&open(&fs)).unwrap().hard_state;
                assert!(recovered == old || recovered == proposed,
                    "recovery lost the acknowledged prefix or invented a vote: {at} {fault:?} seed={seed}: {recovered:?}");
                if at == 1 && matches!(fault, Fault::After(_)) {
                    assert_eq!(
                        recovered, proposed,
                        "an error after completed sync must retain that sync's effect"
                    );
                }
                cells += 1;
            }
        }
    }
    assert_eq!(cells, 320);
    println!("append matrix: {cells} write/sync/error/seed cells");
}

#[test]
fn recovered_unsynced_tail_is_durable_before_a_repeated_crash() {
    let fs = ModelFs::default();
    let store = open(&fs);
    let hs = HardState {
        term: 7,
        vote: 2,
        ..Default::default()
    };
    store.set_hardstate(&hs).unwrap();
    drop(store);
    // A complete frame can survive process death without its file sync.
    let proposed = HardState {
        term: 8,
        vote: 3,
        ..Default::default()
    };
    let payload = proposed.write_to_bytes().unwrap();
    let path = Path::new(DIRECTORY).join("raft.log");
    let mut f = fs.open_append(&path).unwrap();
    let mut body = vec![REC_HARD_STATE];
    body.extend_from_slice(&payload);
    f.write_all(&(body.len() as u32).to_be_bytes()).unwrap();
    f.write_all(&fnv1a(&body).to_be_bytes()).unwrap();
    f.write_all(&body).unwrap();
    drop(f);
    let store = open(&fs);
    assert_eq!(
        raft::Storage::initial_state(&store).unwrap().hard_state,
        proposed
    );
    drop(store);
    fs.crash(Crash::LoseUnsynced);
    assert_eq!(
        raft::Storage::initial_state(&open(&fs)).unwrap().hard_state,
        proposed,
        "a recovered vote exposed to Raft must survive the next power loss"
    );
}
