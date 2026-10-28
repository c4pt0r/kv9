use super::*;
use crate::driver::{ApplyWaitOutcome, NodeDriver};
use crate::transport::{InProcHub, RaftTransport};
use crate::{Command, MemStateMachine};
use kv9_common::store_lifecycle::StoreGuard;
use kv9_common::{
    BootstrapGeneration, ClusterId, KeyspaceId, RootDescriptor, RootDigest, RootVoter,
    StoreIncarnation, TenantId,
};
use kv9_engine::{ColumnFamily, MemEngine};
use raft::prelude::{ConfChangeSingle, ConfChangeTransition, ConfChangeType};
use std::path::PathBuf;

struct Stores {
    paths: Vec<PathBuf>,
    _guards: Vec<StoreGuard>,
    root: RootDescriptor,
    confirmations: Vec<ConfirmedRawTtlFormat>,
}

impl Stores {
    fn new(count: u64) -> Self {
        let mut paths = Vec::new();
        let mut guards = Vec::new();
        let mut voters = Vec::new();
        for id in 1..=count {
            let path = std::env::temp_dir().join(format!(
                "kv9-ttl-gate-{}",
                StoreIncarnation::mint().unwrap()
            ));
            let mut guard = StoreGuard::lock(&path).unwrap();
            let record = guard.prepare(NodeId(id)).unwrap();
            voters.push(RootVoter {
                node_id: NodeId(id),
                addr: format!("127.0.0.1:{}", 24000 + id).parse().unwrap(),
                store_incarnation: record.incarnation,
            });
            paths.push(path);
            guards.push(guard);
        }
        let root = RootDescriptor::new(
            ClusterId::mint().unwrap(),
            BootstrapGeneration::mint().unwrap(),
            voters,
            b"ttl-gate-test",
        )
        .unwrap();
        let mut confirmations = Vec::new();
        for (i, guard) in guards.iter_mut().enumerate() {
            let identity = StoreIdentity::for_voter(&root, NodeId(i as u64 + 1)).unwrap();
            guard.bind(&root, &identity).unwrap();
            guard.activate(&identity).unwrap();
            confirmations.push(ConfirmedRawTtlFormat::from_local(
                guard.raw_ttl_format_fence(&root, &identity).unwrap(),
            ));
        }
        Self {
            paths,
            _guards: guards,
            root,
            confirmations,
        }
    }

    fn range(&self) -> DataRange {
        DataRange {
            root: self.root.digest(),
            creation: RootDigest::from_bytes([2; 32]),
            region: RegionId(100),
            keyspace: KeyspaceId(100),
            tenant: TenantId(1),
            conf_ver: 1,
            version: 1,
            start: vec![],
            end: vec![],
            sealed: false,
        }
    }

    fn collect(&self, probe: RawTtlActivationProbe) -> RawTtlActivationConfirmations {
        let identities: Vec<_> = probe
            .replicas()
            .map(|n| self.confirmations[n.0 as usize - 1].identity())
            .collect();
        probe.bind_stores(&identities).unwrap()
    }

    fn ready(&self, probe: RawTtlActivationProbe) -> PreparedRawTtlActivation {
        let replicas: Vec<_> = probe.replicas().collect();
        let mut collector = self.collect(probe);
        for id in replicas {
            collector
                .confirm(self.confirmations[id.0 as usize - 1])
                .unwrap();
        }
        collector.finish().unwrap()
    }
}
impl Drop for Stores {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

type Driver = NodeDriver<MemStorage, MemEngine>;
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
fn pump(drivers: &[Arc<Driver>]) {
    for _ in 0..30 {
        for driver in drivers {
            driver.step().unwrap();
        }
    }
}
fn cluster(stores: &Stores, conf: ConfState) -> Vec<Arc<Driver>> {
    let hub = InProcHub::new();
    let drivers: Vec<_> = stores
        .confirmations
        .iter()
        .map(|store| {
            let node = store.identity().node_id;
            let peer = Arc::new(
                RaftPeer::with_storage(
                    node,
                    RegionId(100),
                    MemStorage::new_with_conf_state(conf.clone()),
                )
                .unwrap(),
            );
            let mut sm = MemStateMachine::new();
            let range = stores.range();
            sm.set_data_group(range.root, range.region, range.creation)
                .unwrap();
            NodeDriver::new(
                peer,
                Arc::new(hub.endpoint(node)) as Arc<dyn RaftTransport>,
                sm,
            )
            .unwrap()
        })
        .collect();
    drivers[0].peer().campaign().unwrap();
    pump(&drivers);
    drivers[0]
        .propose(&Command::DataRange {
            expected: None,
            next: stores.range(),
        })
        .unwrap();
    pump(&drivers);
    drivers
}

#[tokio::test]
async fn full_joint_configuration_requires_every_store_and_returns_exact_apply() {
    let stores = Stores::new(5);
    let conf = ConfState {
        voters: vec![1, 2, 4],
        voters_outgoing: vec![1, 2, 3],
        learners: vec![5],
        learners_next: vec![3],
        ..Default::default()
    };
    let drivers = cluster(&stores, conf);
    let probe = drivers[0].begin_raw_ttl_activation(deadline()).unwrap();
    assert_eq!(
        probe.replicas().map(|n| n.0).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5]
    );
    let mut incomplete = stores.collect(probe);
    for i in [0, 1, 3, 4, 0] {
        incomplete.confirm(stores.confirmations[i]).unwrap();
    }
    // Incoming majority + ordinary learners + repeated self still omit outgoing/next node 3.
    assert!(incomplete.finish().is_err());
    let ready = stores.ready(drivers[0].begin_raw_ttl_activation(deadline()).unwrap());
    let wait = drivers[0].propose_raw_ttl_activation(ready).unwrap();
    let at = wait.proposed_at();
    pump(&drivers);
    assert_eq!(
        wait.wait().await.unwrap(),
        ApplyWaitOutcome::Applied(kv9_common::AppliedPosition {
            term: at.term,
            index: at.index.0
        })
    );
    for driver in &drivers {
        assert_eq!(
            driver
                .get(ColumnFamily::Default, kv9_common::raw_ttl::CLOCK_KEY)
                .unwrap(),
            Some(ClockWatermark::default().encode().to_vec())
        );
    }
}

#[test]
fn identity_binding_rejects_duplicates_missing_members_and_replaced_stores() {
    let stores = Stores::new(3);
    let drivers = cluster(&stores, ConfState::from((vec![1, 2, 3], vec![])));
    let probe = || drivers[0].begin_raw_ttl_activation(deadline()).unwrap();
    let ids: Vec<_> = stores.confirmations.iter().map(|c| c.identity()).collect();
    assert!(probe().bind_stores(&ids[..2]).is_err());
    assert!(probe().bind_stores(&[ids[0], ids[1], ids[1]]).is_err());
    let mut changed = ids.clone();
    changed[2].store_incarnation = StoreIncarnation::mint().unwrap();
    let mut collector = probe().bind_stores(&changed).unwrap();
    assert!(collector.confirm(stores.confirmations[2]).is_err());
    assert!(collector.finish().is_err());
    changed[2].root_digest = RootDigest::from_bytes([9; 32]);
    assert!(probe().bind_stores(&changed).is_err());
    let foreign = Stores::new(1);
    assert!(stores
        .collect(probe())
        .confirm(foreign.confirmations[0])
        .is_err());
}

#[tokio::test]
async fn activation_can_progress_at_capacity_but_direct_ttl_data_proposals_cannot() {
    use crate::command::RawTtlMutation;
    use kv9_common::raw_ttl::{ExpiryPlan, ExpiryRecord};
    let stores = Stores::new(1);
    let drivers = cluster(&stores, ConfState::from((vec![1], vec![])));
    let driver = &drivers[0];
    driver.peer().set_max_write_log_entries(1);
    let ready = stores.ready(driver.begin_raw_ttl_activation(deadline()).unwrap());
    let wait = driver.propose_raw_ttl_activation(ready).unwrap();
    let at = wait.proposed_at();
    pump(&drivers);
    assert!(matches!(
        wait.wait().await.unwrap(),
        ApplyWaitOutcome::Applied(_)
    ));
    for action in [
        RawTtlAction::Observe {
            observed: ClockWatermark::default(),
        },
        RawTtlAction::Write {
            plan: ExpiryPlan::permanent(ClockWatermark::default()),
            mutations: vec![RawTtlMutation::Delete { key: b"k".to_vec() }],
        },
        RawTtlAction::Cleanup {
            candidates: vec![(
                b"k".to_vec(),
                ExpiryRecord::new(
                    stores.range().creation,
                    kv9_common::AppliedPosition {
                        term: at.term,
                        index: at.index.0,
                    },
                    0,
                )
                .unwrap(),
            )],
        },
    ] {
        let command = RawTtlCommand::new(&stores.range(), action).unwrap();
        assert!(matches!(
            driver.propose_raw_ttl_command(command, deadline()),
            Err(Error::WriteBackpressure {
                region: RegionId(100)
            })
        ));
        assert_eq!(driver.status().log_last_index, at.index.0);
        assert_eq!(driver.async_apply_snapshot().in_flight, 0);
    }
}

fn change(driver: &Driver, kind: ConfChangeType, node: u64) {
    driver
        .peer()
        .propose_conf_change_traced(ConfChangeV2 {
            changes: vec![ConfChangeSingle {
                node_id: node,
                change_type: kind,
                ..Default::default()
            }]
            .into(),
            transition: ConfChangeTransition::Auto,
            ..Default::default()
        })
        .unwrap();
}

#[test]
fn pending_and_applied_membership_changes_invalidate_previous_confirmations() {
    let stores = Stores::new(3);
    let drivers = cluster(&stores, ConfState::from((vec![1, 2, 3], vec![])));
    let ready = stores.ready(drivers[0].begin_raw_ttl_activation(deadline()).unwrap());
    change(&drivers[0], ConfChangeType::AddLearnerNode, 4);
    assert!(drivers[0].begin_raw_ttl_activation(deadline()).is_err());
    assert!(drivers[0].propose_raw_ttl_activation(ready).is_err());
    pump(&drivers);
    assert_eq!(
        drivers[0]
            .begin_raw_ttl_activation(deadline())
            .unwrap()
            .replicas()
            .count(),
        4
    );

    // The same member set after an add/remove cycle is still a new configuration.
    change(&drivers[0], ConfChangeType::RemoveNode, 4);
    pump(&drivers);
    let old = stores.ready(drivers[0].begin_raw_ttl_activation(deadline()).unwrap());
    change(&drivers[0], ConfChangeType::AddLearnerNode, 4);
    pump(&drivers);
    change(&drivers[0], ConfChangeType::RemoveNode, 4);
    pump(&drivers);
    assert!(drivers[0].propose_raw_ttl_activation(old).is_err());
}

#[test]
fn leader_term_peer_recreation_and_deadline_are_not_reusable_authority() {
    let stores = Stores::new(1);
    let conf = ConfState::from((vec![1], vec![]));
    let drivers = cluster(&stores, conf.clone());
    let ready = stores.ready(drivers[0].begin_raw_ttl_activation(deadline()).unwrap());
    let replacement = cluster(&stores, conf);
    assert!(replacement[0].propose_raw_ttl_activation(ready).is_err());
    let ready = stores.ready(drivers[0].begin_raw_ttl_activation(deadline()).unwrap());
    let peer = drivers[0].peer();
    let mut message = Message::default();
    message.set_msg_type(raft::prelude::MessageType::MsgHeartbeat);
    message.set_from(2);
    message.set_to(1);
    message.set_term(peer.term() + 1);
    peer.step_message(message);
    peer.campaign().unwrap();
    pump(&drivers);
    assert!(drivers[0].propose_raw_ttl_activation(ready).is_err());
    let mut ready = stores.ready(drivers[0].begin_raw_ttl_activation(deadline()).unwrap());
    ready.probe.deadline = Instant::now(); // Deterministic elapsed-deadline injection; no sleep.
    assert!(drivers[0].propose_raw_ttl_activation(ready).is_err());
    assert!(drivers[0].begin_raw_ttl_activation(Instant::now()).is_err());
    assert!(drivers[0]
        .begin_raw_ttl_activation(Instant::now() + Duration::from_secs(31))
        .is_err());
}

#[test]
fn every_ordinary_proposal_face_refuses_enable_without_appending() {
    let stores = Stores::new(1);
    let drivers = cluster(&stores, ConfState::from((vec![1], vec![])));
    let driver = &drivers[0];
    let command = Command::RawTtl(
        RawTtlCommand::new(
            &stores.range(),
            RawTtlAction::Enable {
                observed: ClockWatermark::default(),
            },
        )
        .unwrap(),
    );
    let index = driver.peer().lock().raw.raft.raft_log.last_index();
    assert!(driver.propose(&command).is_err());
    assert!(driver
        .propose_in_term(&command, driver.peer().term())
        .is_err());
    assert!(driver
        .propose_with_async_wait(&command, deadline())
        .is_err());
    let Command::RawTtl(ttl) = &command else {
        unreachable!()
    };
    assert!(driver
        .propose_raw_ttl_command(ttl.clone(), deadline())
        .is_err());
    assert!(RaftGroup::propose(driver.peer().as_ref(), &command).is_err());
    assert!(crate::SingleNodeRaft::new(NodeId(1), RegionId(100))
        .propose(&command)
        .is_err());
    assert_eq!(driver.peer().lock().raw.raft.raft_log.last_index(), index);
    assert_eq!(driver.async_apply_snapshot().in_flight, 0);
}

#[test]
fn full_apply_queue_and_stopped_owner_refuse_activation_before_append() {
    let stores = Stores::new(1);
    let drivers = cluster(&stores, ConfState::from((vec![1], vec![])));
    let driver = &drivers[0];
    let ready = stores.ready(driver.begin_raw_ttl_activation(deadline()).unwrap());
    let mut waits = Vec::new();
    for _ in 0..driver.async_apply_snapshot().limit {
        waits.push(
            driver
                .propose_with_async_wait(&Command::Noop, deadline())
                .unwrap(),
        );
    }
    let index = driver.peer().lock().raw.raft.raft_log.last_index();
    assert!(driver.propose_raw_ttl_activation(ready).is_err());
    assert_eq!(driver.peer().lock().raw.raft.raft_log.last_index(), index);
    drop(waits);
    pump(&drivers);
    let ready = stores.ready(driver.begin_raw_ttl_activation(deadline()).unwrap());
    driver.stop();
    assert!(driver.begin_raw_ttl_activation(deadline()).is_err());
    assert!(driver.propose_raw_ttl_activation(ready).is_err());
    assert_eq!(driver.peer().lock().raw.raft.raft_log.last_index(), index);
}

#[tokio::test]
async fn sealing_between_collection_and_ordered_apply_never_activates_stale_range() {
    let stores = Stores::new(1);
    let drivers = cluster(&stores, ConfState::from((vec![1], vec![])));
    let driver = &drivers[0];
    let ready = stores.ready(driver.begin_raw_ttl_activation(deadline()).unwrap());
    let mut sealed = stores.range();
    sealed.sealed = true;
    sealed.version += 1;
    driver
        .propose(&Command::DataRange {
            expected: Some(stores.range().digest()),
            next: sealed,
        })
        .unwrap();
    // The seal is appended but not yet applied, so proposal can be accepted.
    let wait = driver.propose_raw_ttl_activation(ready).unwrap();
    pump(&drivers);
    assert!(matches!(
        wait.wait().await.unwrap(),
        ApplyWaitOutcome::FenceRejected { .. }
    ));
    assert!(driver
        .get(ColumnFamily::Default, kv9_common::raw_ttl::CLOCK_KEY)
        .unwrap()
        .is_none());
    assert!(driver.begin_raw_ttl_activation(deadline()).is_err());
}

#[tokio::test]
async fn activation_before_seal_refuses_the_split_that_would_drop_expiry_metadata() {
    let stores = Stores::new(1);
    let drivers = cluster(&stores, ConfState::from((vec![1], vec![])));
    let driver = &drivers[0];
    let ready = stores.ready(driver.begin_raw_ttl_activation(deadline()).unwrap());
    let activation = driver.propose_raw_ttl_activation(ready).unwrap();
    let mut sealed = stores.range();
    sealed.sealed = true;
    sealed.version += 1;
    let (_, seal) = driver
        .propose_with_async_wait(
            &Command::DataRange {
                expected: Some(stores.range().digest()),
                next: sealed,
            },
            deadline(),
        )
        .unwrap();
    pump(&drivers);
    assert!(matches!(
        activation.wait().await.unwrap(),
        ApplyWaitOutcome::Applied(_)
    ));
    assert!(matches!(
        seal.wait().await.unwrap(),
        ApplyWaitOutcome::RawTtlRejected {
            reason: kv9_common::raw_ttl::TtlRejection::LegacyCommand,
            ..
        }
    ));
    let actual = driver
        .get(ColumnFamily::Default, kv9_common::data_range::RANGE_KEY)
        .unwrap()
        .unwrap();
    assert_eq!(DataRange::decode(&actual).unwrap(), stores.range());
}

#[tokio::test]
async fn transfer_format_gate_requires_full_joint_set_and_binds_the_peer_and_configuration() {
    use crate::command::{RawTtlSplit, RawTtlSplitAction};
    use crate::rawnode::ttl_split::{PreparedTtlSplit, TtlSplitProbe};
    use kv9_common::raw_ttl::split::{SplitPlan, SEAL_KEY};
    let stores = Stores::new(5);
    let conf = ConfState {
        voters: vec![1, 2, 4],
        voters_outgoing: vec![1, 2, 3],
        learners: vec![5],
        learners_next: vec![3],
        ..Default::default()
    };
    let drivers = cluster(&stores, conf.clone());
    let activation = stores.ready(drivers[0].begin_raw_ttl_activation(deadline()).unwrap());
    let wait = drivers[0].propose_raw_ttl_activation(activation).unwrap();
    pump(&drivers);
    wait.wait().await.unwrap();
    let parent = stores.range();
    let mut low = parent.clone();
    low.region = RegionId(101);
    low.creation = RootDigest::from_bytes([3; 32]);
    low.end = b"m".to_vec();
    let mut high = parent.clone();
    high.region = RegionId(102);
    high.creation = RootDigest::from_bytes([4; 32]);
    high.start = b"m".to_vec();
    let command = RawTtlSplit::new(RawTtlSplitAction::Seal(SplitPlan {
        operation: [9; 16],
        parent,
        low,
        high,
    }))
    .unwrap();
    let probe = || {
        drivers[0]
            .peer()
            .begin_ttl_split_confirmation(command.clone(), deadline())
            .unwrap()
    };
    let identities = stores
        .confirmations
        .iter()
        .map(|c| c.identity())
        .collect::<Vec<_>>();
    let mut incomplete = probe().bind_stores(&identities).unwrap();
    for i in [0, 1, 3, 4, 0] {
        incomplete.confirm(stores.confirmations[i]).unwrap();
    }
    assert!(incomplete.finish().is_err());
    let ready = |p: TtlSplitProbe| -> PreparedTtlSplit {
        let mut c = p.bind_stores(&identities).unwrap();
        for reply in &stores.confirmations {
            c.confirm(*reply).unwrap();
        }
        c.finish().unwrap()
    };
    let replacement = cluster(&stores, conf);
    assert!(replacement[0]
        .peer()
        .propose_ttl_split_confirmed(ready(probe()))
        .is_err());
    let at = drivers[0]
        .peer()
        .propose_ttl_split_confirmed(ready(probe()))
        .unwrap();
    pump(&drivers);
    assert!(matches!(
        drivers[0].wait_applied(at, Duration::from_secs(1)).unwrap(),
        ApplyWaitOutcome::Applied(_)
    ));
    for driver in &drivers {
        assert!(driver
            .get(ColumnFamily::Default, SEAL_KEY)
            .unwrap()
            .is_some());
    }
    // The same command's retry still needs the original full configuration.
    let prepared = ready(probe());
    let after_apply = ready(probe());
    // Leaving joint is the valid next transition. raft-rs turns an invalid
    // nested membership change into a no-op, which changes no authority.
    drivers[0]
        .peer()
        .propose_conf_change_traced(ConfChangeV2::default())
        .unwrap();
    assert!(drivers[0]
        .peer()
        .propose_ttl_split_confirmed(prepared)
        .is_err());
    pump(&drivers);
    assert!(drivers[0]
        .peer()
        .propose_ttl_split_confirmed(after_apply)
        .is_err());
}

#[test]
fn space_probe_cannot_extend_a_protected_format_confirmation_deadline() {
    use kv9_common::log_budget::{headroom::DiskSpaceProbe, LogBudget};
    use std::sync::atomic::{AtomicBool, Ordering};
    #[derive(Debug)]
    struct SlowSpace {
        deadline: Mutex<Instant>,
        called: AtomicBool,
    }
    impl DiskSpaceProbe for SlowSpace {
        fn available_bytes(&self) -> std::io::Result<u64> {
            self.called.store(true, Ordering::SeqCst);
            let remaining = self
                .deadline
                .lock()
                .unwrap()
                .saturating_duration_since(Instant::now());
            std::thread::sleep(remaining + Duration::from_millis(2));
            Ok(u64::MAX)
        }
        fn check_path(&self, _: &std::path::Path) -> std::io::Result<()> {
            Ok(())
        }
    }
    let stores = Stores::new(1);
    let drivers = cluster(
        &stores,
        ConfState {
            voters: vec![1],
            ..Default::default()
        },
    );
    let driver = &drivers[0];
    let space = Arc::new(SlowSpace {
        deadline: Mutex::new(deadline()),
        called: AtomicBool::new(false),
    });
    let budget =
        Arc::new(LogBudget::with_disk_headroom(64 * 1024 * 1024, 1, space.clone()).unwrap());
    let debt = budget
        .track(&stores.paths[0].join("fixture-raft"), 0)
        .unwrap();
    driver.attach_log_debt(debt).unwrap();
    let mut prepared = stores.ready(
        driver
            .peer()
            .begin_raw_ttl_activation(stores.range(), deadline())
            .unwrap(),
    );
    prepared.probe.deadline = Instant::now() + Duration::from_millis(100);
    *space.deadline.lock().unwrap() = prepared.probe.deadline;
    let before = driver.status().log_last_index;
    // Test the shared protected append seam, without fabricating a production
    // split/source authorization or changing the activation command policy.
    let error = driver
        .peer()
        .propose_format_confirmed(prepared, Command::Noop.encode(), true)
        .unwrap_err();
    assert!(space.called.load(Ordering::SeqCst));
    assert!(error.to_string().contains("deadline"));
    assert_eq!(driver.status().log_last_index, before);
    assert_eq!(budget.snapshot().reserved_bytes, 0);
    assert_eq!(budget.snapshot().pending_positions, 0);
    assert_eq!(budget.snapshot().disk_headroom.unwrap().refusals, 0);
}
