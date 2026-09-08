//! Failure schedules execute the real DiskRaftStorage and raft-rs Ready loop.

use super::*;
use crate::rawnode::RaftPeer;
use kv9_common::fs::testing::{Crash, Fault, ModelFs, Operation};
use kv9_common::{NodeId, RegionId};
use raft::prelude::{Message, MessageType};
use std::sync::Arc;

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

#[derive(Clone, Copy, Debug)]
enum DriverCut {
    Vote,
    Configuration,
}

fn prepared_driver(
    fs: &ModelFs,
    scenario: DriverCut,
) -> (
    Arc<crate::driver::NodeDriver<DiskRaftStorage<ModelFs>>>,
    crate::transport::InProcEndpoint,
) {
    use crate::{transport::RaftTransport, RaftGroup};
    let voters: &[u64] = match scenario {
        DriverCut::Vote => &[1, 2, 3],
        DriverCut::Configuration => &[1],
    };
    let store = DiskRaftStorage::open_on(fs.clone(), Path::new(DIRECTORY), voters)
        .unwrap()
        .0;
    let peer = Arc::new(RaftPeer::with_storage(NodeId(1), RegionId(1), store).unwrap());
    let hub = crate::transport::InProcHub::new();
    let remote = hub.endpoint(NodeId(2));
    let driver = crate::driver::NodeDriver::new(
        peer.clone(),
        Arc::new(hub.endpoint(NodeId(1))),
        crate::MemStateMachine::new(),
    )
    .unwrap();
    match scenario {
        DriverCut::Vote => remote.send(NodeId(1), vote_request(2, 7)),
        DriverCut::Configuration => {
            peer.campaign().unwrap();
            driver.step().unwrap();
            driver.step().unwrap();
            peer.read_index(b"must-not-escape-failed-ready".to_vec())
                .unwrap();
            driver.add_learner(NodeId(2)).unwrap();
        }
    }
    fs.clear_events();
    (driver, remote)
}

#[test]
fn driver_persistence_failures_stop_without_poisoning_observation_locks() {
    use crate::{transport::RaftTransport, RaftGroup};
    let mut cells = 0;
    let mut causes = std::collections::BTreeSet::new();
    for scenario in [DriverCut::Vote, DriverCut::Configuration] {
        let fs = ModelFs::default();
        let (driver, remote) = prepared_driver(&fs, scenario);
        driver.step().unwrap();
        match scenario {
            DriverCut::Vote => assert!(granted(&remote.drain(), 2, 7)),
            DriverCut::Configuration => assert_eq!(driver.status().learners, [2]),
        }
        let cuts = fs.events();
        assert!(!cuts.is_empty());
        for cut in cuts {
            assert!(matches!(
                cut.operation,
                Operation::Write | Operation::SyncData
            ));
            for errno in [5, 28] {
                for fault in [Fault::Before(errno), Fault::After(errno)] {
                    let fs = ModelFs::default();
                    let (driver, remote) = prepared_driver(&fs, scenario);
                    let before = driver.status();
                    fs.fail_at(cut.number, fault);
                    let error = driver
                        .step()
                        .expect_err("failed persistence must stop the driver");
                    assert!(matches!(error, Error::Raft(_)));
                    assert!(
                        fs.fault_arrived(),
                        "fault did not arrive: {scenario:?} {cut:?}"
                    );
                    // This used to panic while holding peer.inner, leaving even
                    // status() unusable. Observation must survive the I/O error.
                    let status = driver.status();
                    let fatal = status.fatal.expect("runtime must observe a fatal cause");
                    for operation in ["append", "hardstate", "confstate"] {
                        if fatal.contains(&format!("during {operation}:")) {
                            causes.insert(operation);
                        }
                    }
                    assert_eq!(status.applied_index, before.applied_index);
                    assert_eq!(status.driver_applied, before.driver_applied);
                    assert_eq!(status.conf_index, before.conf_index);
                    assert!(
                        remote.drain().is_empty(),
                        "failed Ready must emit no messages"
                    );
                    assert!(driver.peer().take_read_states().is_empty());
                    let operations = fs.events().len();
                    let term = driver.peer().term();
                    for _ in 0..3 {
                        assert!(driver.tick_and_step().is_err());
                        driver.peer().step_message(vote_request(2, term + 10));
                        assert!(driver.peer().pump().is_err());
                        assert!(driver.peer().campaign().is_err());
                        assert!(driver.peer().read_index(b"after-failure".to_vec()).is_err());
                        assert!(driver.peer().propose_raw_for_harness(vec![1]).is_err());
                        assert!(driver.add_learner(NodeId(3)).is_err());
                    }
                    assert_eq!(
                        fs.events().len(),
                        operations,
                        "fatal peer performed more I/O"
                    );
                    assert_eq!(driver.peer().term(), term, "fatal peer processed a message");
                    assert_eq!(driver.status().fatal.as_deref(), Some(fatal.as_str()));
                    assert!(remote.drain().is_empty());
                    cells += 1;
                }
            }
        }
    }
    assert_eq!(
        causes.into_iter().collect::<Vec<_>>(),
        ["append", "confstate", "hardstate"]
    );
    println!("driver persistence matrix: {cells} named write/sync failure cells");
}

#[test]
fn acknowledged_vote_survives_loss_of_unsynced_namespace() {
    let fs = ModelFs::default();
    let peer = RaftPeer::with_storage(NodeId(1), RegionId(1), open(&fs)).unwrap();
    peer.step_message(vote_request(2, 7));
    assert!(
        granted(&peer.pump().unwrap(), 2, 7),
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
        !granted(&peer.pump().unwrap(), 3, 7),
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
