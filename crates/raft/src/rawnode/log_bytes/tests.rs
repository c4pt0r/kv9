use super::*;
use crate::driver::{ApplyWaitOutcome, NodeDriver};
use crate::storage::DiskRaftStorage;
use crate::transport::{InProcHub, RaftTransport};
use crate::{Command, MemStateMachine};
use kv9_common::log_budget::LogBudget;
use kv9_engine::{ColumnFamily, Engine, WalEngine};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

mod replica_pressure;
mod rewrite_workspace;

type Driver = NodeDriver<DiskRaftStorage, WalEngine>;
fn directory() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "kv9-raft-log-debt-{}",
        kv9_common::StoreIncarnation::mint().unwrap()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
fn actual(path: &Path) -> u64 {
    std::fs::read_dir(path)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            if e.file_type().unwrap().is_dir() {
                actual(&e.path())
            } else {
                e.metadata().unwrap().len()
            }
        })
        .sum()
}
fn command() -> Command {
    Command::Put {
        cf: 0,
        key: b"key".to_vec(),
        value: vec![7; 4096],
    }
}
fn open(
    path: &Path,
    node: NodeId,
    region: RegionId,
    voters: &[u64],
    hub: &Arc<InProcHub>,
    budget: &Arc<LogBudget>,
) -> (Arc<Driver>, Arc<WalEngine>) {
    std::fs::create_dir_all(path).unwrap();
    let engine = Arc::new(WalEngine::open(path.join("data.wal")).unwrap().0);
    engine.attach_log_budget(budget).unwrap();
    let storage = DiskRaftStorage::open(&path.join("raft"), voters).unwrap().0;
    let debt = storage.attach_log_budget(budget).unwrap();
    let peer = Arc::new(RaftPeer::with_storage(node, region, storage).unwrap());
    let driver = NodeDriver::new(
        peer,
        Arc::new(hub.endpoint(node)) as Arc<dyn RaftTransport>,
        MemStateMachine::with_engine(engine.clone()).unwrap(),
    )
    .unwrap();
    driver.attach_log_debt(debt).unwrap();
    (driver, engine)
}
fn leader(driver: &Driver) {
    driver.peer().campaign().unwrap();
    driver.step().unwrap();
    assert_eq!(driver.status().role, Role::Leader);
}

#[test]
fn two_groups_share_byte_admission_and_cancelled_waits_keep_their_positions() {
    let dir = directory();
    let bytes = proposal_bytes(command().encode().len()).unwrap();
    let budget = Arc::new(LogBudget::new(2 * bytes + 64 * 1024).unwrap());
    let mut drivers = Vec::new();
    for region in 1..=2 {
        let (driver, _) = open(
            &dir.join(region.to_string()),
            NodeId(1),
            RegionId(region),
            &[1],
            &InProcHub::new(),
            &budget,
        );
        leader(&driver);
        drivers.push(driver);
    }
    let barrier = std::sync::Barrier::new(32);
    let accepted = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..32)
            .map(|i| {
                let driver = &drivers[i % 2];
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    match driver.propose_with_async_wait(
                        &command(),
                        Instant::now() + Duration::from_secs(10),
                    ) {
                        Ok((at, wait)) => {
                            drop(wait);
                            Some((i % 2, at))
                        }
                        Err(Error::WriteBackpressure { .. }) => None,
                        Err(e) => panic!("unexpected proposal: {e}"),
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(accepted.len(), 2);
    assert_eq!(budget.snapshot().pending_positions, 2);
    assert_eq!(budget.snapshot().reserved_bytes, 2 * bytes);
    for driver in &drivers {
        driver.pause_apply(true);
        driver.step().unwrap(); // service cancellation, but do not consume the log
    }
    assert!(drivers
        .iter()
        .all(|d| d.async_apply_snapshot().in_flight == 0));
    assert_eq!(budget.snapshot().reserved_bytes, 2 * bytes);
    for driver in &drivers {
        driver.pause_apply(false);
        driver.step().unwrap();
    }
    for (i, at) in accepted {
        assert!(matches!(
            drivers[i].wait_applied(at, Duration::ZERO).unwrap(),
            ApplyWaitOutcome::Applied(_)
        ));
    }
    assert_eq!(budget.snapshot().reserved_bytes, 0);
    assert_eq!(budget.snapshot().file_bytes, actual(&dir));
    assert!(
        drivers[0].propose(&command()).is_ok(),
        "real apply returns reserved capacity"
    );
    drop(drivers);
    assert_eq!(
        budget.snapshot().pending_positions,
        1,
        "owner drop is not a negative outcome"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn logical_compaction_keeps_file_debt_until_actual_raft_rewrite_restores_admission() {
    let dir = directory();
    let initial = Arc::new(LogBudget::new(64 * 1024 * 1024).unwrap());
    let (driver, engine) = open(
        &dir,
        NodeId(1),
        RegionId(9),
        &[1],
        &InProcHub::new(),
        &initial,
    );
    leader(&driver);
    let mut last = None;
    for _ in 0..20 {
        last = Some(driver.propose(&command()).unwrap());
        driver.step().unwrap();
    }
    let last = last.unwrap();
    drop(driver);
    drop(engine);
    let budget = Arc::new(
        LogBudget::new(actual(&dir) + proposal_bytes(command().encode().len()).unwrap() - 1)
            .unwrap(),
    );
    let (driver, engine) = open(
        &dir,
        NodeId(1),
        RegionId(9),
        &[1],
        &InProcHub::new(),
        &budget,
    );
    leader(&driver);
    assert!(matches!(
        driver.propose(&command()),
        Err(Error::WriteBackpressure { .. })
    ));
    let before = budget.snapshot().file_bytes;
    driver
        .peer()
        .truncate_retained_log(
            kv9_common::AppliedPosition {
                term: last.term,
                index: last.index.0,
            },
            b"fixture committed compaction decision",
        )
        .unwrap();
    assert!(budget.snapshot().file_bytes >= before);
    assert!(matches!(
        driver.propose(&command()),
        Err(Error::WriteBackpressure { .. })
    ));
    assert!(driver.peer().reclaim_log().unwrap());
    assert_eq!(budget.snapshot().file_bytes, actual(&dir));
    assert!(budget.snapshot().file_bytes < before);
    let resumed = driver.propose(&command()).unwrap();
    driver.step().unwrap();
    assert!(matches!(
        driver.wait_applied(resumed, Duration::ZERO).unwrap(),
        ApplyWaitOutcome::Applied(_)
    ));
    assert_eq!(budget.snapshot().reserved_bytes, 0);
    assert_eq!(budget.snapshot().file_bytes, actual(&dir));
    drop(driver);
    drop(engine);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn maximum_ttl_mutation_batch_with_rotation_fits_its_future_log_reservation() {
    use crate::command::{RawTtlAction, RawTtlCommand, RawTtlMutation};
    use kv9_common::data_range::DataRange;
    use kv9_common::raw_ttl::{ClockSample, ClockWatermark, ExpiryPlan};
    use kv9_common::{KeyspaceId, RootDigest, TenantId};
    let dir = directory();
    let budget = Arc::new(LogBudget::new(64 * 1024 * 1024).unwrap());
    let engine = Arc::new(WalEngine::open(dir.join("data.wal")).unwrap().0);
    engine.attach_log_budget(&budget).unwrap();
    engine.enable_segmentation().unwrap();
    let storage = DiskRaftStorage::open(&dir.join("raft"), &[1]).unwrap().0;
    let debt = storage.attach_log_budget(&budget).unwrap();
    let range = DataRange {
        root: RootDigest::from_bytes([1; 32]),
        creation: RootDigest::from_bytes([2; 32]),
        region: RegionId(100),
        keyspace: KeyspaceId(100),
        tenant: TenantId(1),
        conf_ver: 1,
        version: 1,
        start: Vec::new(),
        end: Vec::new(),
        sealed: false,
    };
    let mut sm = MemStateMachine::with_engine(engine.clone()).unwrap();
    sm.set_data_group(range.root, range.region, range.creation)
        .unwrap();
    let driver = NodeDriver::new(
        Arc::new(RaftPeer::with_storage(NodeId(1), range.region, storage).unwrap()),
        Arc::new(InProcHub::new().endpoint(NodeId(1))) as Arc<dyn RaftTransport>,
        sm,
    )
    .unwrap();
    driver.attach_log_debt(debt).unwrap();
    leader(&driver);
    driver
        .propose(&Command::DataRange {
            expected: None,
            next: range.clone(),
        })
        .unwrap();
    // Format authorization is covered by the existing activation integration
    // suite; this harness injection isolates actual WAL expansion/accounting.
    driver
        .peer()
        .propose_raw_for_harness(
            Command::RawTtl(
                RawTtlCommand::new(
                    &range,
                    RawTtlAction::Enable {
                        observed: ClockWatermark::default(),
                    },
                )
                .unwrap(),
            )
            .encode(),
        )
        .unwrap();
    driver.step().unwrap();
    let command = Command::RawTtl(
        RawTtlCommand::new(
            &range,
            RawTtlAction::Write {
                plan: ExpiryPlan::resolve(
                    Some(1),
                    ClockSample::with_uncertainty(100, 0).unwrap(),
                    ClockWatermark::default(),
                )
                .unwrap(),
                mutations: (0..crate::command::ttl::MAX_MUTATIONS)
                    .map(|i| RawTtlMutation::Put {
                        key: (i as u64).to_be_bytes().to_vec(),
                        value: Vec::new(),
                    })
                    .collect(),
            },
        )
        .unwrap(),
    );
    let allowance = proposal_bytes(command.encode().len()).unwrap();
    for _ in 0..48 {
        let before = actual(&dir);
        let at = driver.propose(&command).unwrap();
        assert_eq!(budget.snapshot().reserved_bytes, allowance);
        driver.step().unwrap();
        assert!(matches!(
            driver.wait_applied(at, Duration::ZERO).unwrap(),
            ApplyWaitOutcome::Applied(_)
        ));
        assert!(actual(&dir) - before < allowance);
        assert_eq!(budget.snapshot().file_bytes, actual(&dir));
        assert_eq!(budget.snapshot().reserved_bytes, 0);
    }
    let physical = kv9_common::codec::encode_key(
        kv9_common::codec::KeyMode::Raw,
        range.keyspace,
        &4095u64.to_be_bytes(),
    )
    .unwrap();
    assert!(engine
        .get(ColumnFamily::Write, &physical)
        .unwrap()
        .is_some());
    let stream = std::fs::read_dir(dir.join("data.segments"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert!(std::fs::read_dir(stream).unwrap().count() >= 2);
    drop(driver);
    drop(engine);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn durable_unapplied_suffix_is_reserved_again_after_restart_above_lowered_limit() {
    let dir = directory();
    let budget = Arc::new(LogBudget::new(64 * 1024 * 1024).unwrap());
    let (driver, engine) = open(
        &dir,
        NodeId(1),
        RegionId(9),
        &[1],
        &InProcHub::new(),
        &budget,
    );
    leader(&driver);
    let at = driver.propose(&command()).unwrap();
    driver.peer().pump().unwrap(); // persist Ready, deliberately do not apply
    assert_eq!(engine.get(ColumnFamily::Default, b"key").unwrap(), None);
    assert_eq!(budget.snapshot().pending_positions, 1);
    drop(driver);
    drop(engine);
    assert_eq!(budget.snapshot().pending_positions, 1);
    let lowered = Arc::new(LogBudget::new(1).unwrap());
    let (driver, engine) = open(
        &dir,
        NodeId(1),
        RegionId(9),
        &[1],
        &InProcHub::new(),
        &lowered,
    );
    assert_eq!(lowered.snapshot().pending_positions, 1);
    leader(&driver);
    assert!(matches!(
        driver.wait_applied(at, Duration::ZERO).unwrap(),
        ApplyWaitOutcome::Applied(_)
    ));
    assert_eq!(
        engine.get(ColumnFamily::Default, b"key").unwrap(),
        Some(vec![7; 4096])
    );
    assert_eq!(lowered.snapshot().reserved_bytes, 0);
    assert_eq!(lowered.snapshot().file_bytes, actual(&dir));
    let before = driver.status().log_last_index;
    assert!(matches!(
        driver.propose(&command()),
        Err(Error::WriteBackpressure { .. })
    ));
    assert_eq!(driver.status().log_last_index, before);
    let control = driver.propose(&Command::Noop).unwrap();
    driver.step().unwrap();
    assert!(matches!(
        driver.wait_applied(control, Duration::ZERO).unwrap(),
        ApplyWaitOutcome::Applied(_)
    ));
    assert_eq!(lowered.snapshot().reserved_bytes, 0);
    drop(driver);
    drop(engine);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn followers_above_their_threshold_still_persist_and_apply_the_leaders_accepted_entry() {
    let dir = directory();
    let hub = InProcHub::new();
    let mut drivers = Vec::new();
    let mut budgets = Vec::new();
    for n in 1..=3 {
        let budget = Arc::new(LogBudget::new(if n == 1 { 64 * 1024 * 1024 } else { 1 }).unwrap());
        let (driver, _) = open(
            &dir.join(n.to_string()),
            NodeId(n),
            RegionId(9),
            &[1, 2, 3],
            &hub,
            &budget,
        );
        drivers.push(driver);
        budgets.push(budget);
    }
    let pump = || {
        for _ in 0..30 {
            for d in &drivers {
                d.step().unwrap();
            }
        }
    };
    drivers[0].peer().campaign().unwrap();
    pump();
    assert_eq!(drivers[0].status().role, Role::Leader);
    let at = drivers[0].propose(&command()).unwrap();
    pump();
    for (i, driver) in drivers.iter().enumerate() {
        assert!(matches!(
            driver.wait_applied(at, Duration::ZERO).unwrap(),
            ApplyWaitOutcome::Applied(_)
        ));
        assert_eq!(
            driver.get(ColumnFamily::Default, b"key").unwrap(),
            Some(vec![7; 4096])
        );
        assert_eq!(budgets[i].snapshot().reserved_bytes, 0);
        assert_eq!(
            budgets[i].snapshot().file_bytes,
            actual(&dir.join((i + 1).to_string()))
        );
    }
    drop(drivers);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_persisted_replacement_releases_only_obsolete_future_apply_obligations() {
    let nodes = [NodeId(1), NodeId(2), NodeId(3)];
    let cluster = InProcessCluster::new(RegionId(9), &nodes).unwrap();
    let budgets: Vec<_> = nodes
        .iter()
        .map(|id| {
            let budget = Arc::new(LogBudget::new(128 * 1024 * 1024).unwrap());
            let debt = budget
                .track(Path::new(&format!("volatile-raft-{}", id.0)), 0)
                .unwrap();
            cluster.peer(*id).unwrap().attach_log_debt(debt, 0).unwrap();
            budget
        })
        .collect();
    cluster.peers()[0].campaign().unwrap();
    cluster
        .run_until(100, "initial leader", |c| c.leader().is_some())
        .unwrap();
    let old = cluster.leader().unwrap();
    for id in nodes.into_iter().filter(|id| *id != old) {
        cluster.set_alive(id, false);
    }
    for _ in 0..8 {
        cluster
            .peer(old)
            .unwrap()
            .propose_command_in_term(&command(), None)
            .unwrap();
    }
    cluster.peer(old).unwrap().pump().unwrap();
    let old_budget = &budgets[old.0 as usize - 1];
    assert_eq!(old_budget.snapshot().pending_positions, 8);
    cluster.set_alive(old, false);
    for id in nodes.into_iter().filter(|id| *id != old) {
        cluster.set_alive(id, true);
    }
    cluster
        .run_until(500, "replacement leader", |c| c.leader().is_some())
        .unwrap();
    let leader = cluster.leader().unwrap();
    assert_ne!(old, leader);
    let live = cluster
        .peer(leader)
        .unwrap()
        .propose_command_in_term(&command(), None)
        .unwrap();
    cluster
        .run_until(100, "replacement committed", |c| {
            c.peer(leader).unwrap().raft_committed() >= live.index
        })
        .unwrap();
    assert!(live.index.0 < 9);
    assert_eq!(
        old_budget.snapshot().pending_positions,
        8,
        "leadership loss is not a release"
    );
    cluster.set_alive(old, true);
    cluster
        .run_until(100, "old suffix replaced durably", |c| {
            let p = c.peer(old).unwrap();
            p.raft_committed() >= live.index && p.status_snapshot().log_last_index == live.index.0
        })
        .unwrap();
    assert_eq!(old_budget.snapshot().pending_positions, 1);
    assert_eq!(
        old_budget.snapshot().reserved_bytes,
        proposal_bytes(command().encode().len()).unwrap()
    );
    let mut sm = MemStateMachine::new();
    crate::drive_apply(&HarnessPump(cluster.peer(old).unwrap()), &mut sm).unwrap();
    cluster.peer(old).unwrap().applied_to(live.index.0);
    assert_eq!(old_budget.snapshot().reserved_bytes, 0);
}

#[test]
fn low_space_followers_apply_accepted_writes_then_refuse_new_data_after_election() {
    use kv9_common::log_budget::headroom::DiskSpaceProbe;
    use std::sync::atomic::{AtomicU64, Ordering};
    #[derive(Debug)]
    struct Space(AtomicU64);
    impl DiskSpaceProbe for Space {
        fn available_bytes(&self) -> std::io::Result<u64> {
            Ok(self.0.load(Ordering::SeqCst))
        }
        fn check_path(&self, _: &Path) -> std::io::Result<()> {
            Ok(())
        }
    }
    let dir = directory();
    let hub = InProcHub::new();
    let mut drivers = Vec::new();
    let mut budgets = Vec::new();
    let mut probes = Vec::new();
    for n in 1..=3 {
        let space = Arc::new(Space(AtomicU64::new(if n == 1 {
            64 * 1024 * 1024
        } else {
            0
        })));
        let budget = Arc::new(
            LogBudget::with_disk_headroom(128 * 1024 * 1024, 1024 * 1024, space.clone()).unwrap(),
        );
        let (driver, _) = open(
            &dir.join(n.to_string()),
            NodeId(n),
            RegionId(9),
            &[1, 2, 3],
            &hub,
            &budget,
        );
        drivers.push(driver);
        budgets.push(budget);
        probes.push(space);
    }
    let pump = || {
        for _ in 0..30 {
            for driver in &drivers {
                driver.step().unwrap();
            }
        }
    };
    drivers[0].peer().campaign().unwrap();
    pump();
    let at = drivers[0].propose(&command()).unwrap();
    pump();
    for driver in &drivers {
        assert!(matches!(
            driver.wait_applied(at, Duration::ZERO).unwrap(),
            ApplyWaitOutcome::Applied(_)
        ));
        assert_eq!(
            driver.get(ColumnFamily::Default, b"key").unwrap(),
            Some(vec![7; 4096])
        );
    }
    drivers[0].peer().lock().alive = false;
    let mut elected = None;
    for _ in 0..200 {
        for driver in &drivers[1..] {
            driver.tick_and_step().unwrap();
        }
        pump();
        elected = (1..3).find(|&i| drivers[i].status().role == Role::Leader);
        if elected.is_some() {
            break;
        }
    }
    let elected = elected.expect("remaining majority elected a leader");
    let before = drivers[elected].status().log_last_index;
    assert!(matches!(
        drivers[elected].propose(&command()),
        Err(Error::WriteBackpressure { .. })
    ));
    assert_eq!(drivers[elected].status().log_last_index, before);
    let control = drivers[elected].propose(&Command::Noop).unwrap();
    pump();
    assert!(matches!(
        drivers[elected]
            .wait_applied(control, Duration::ZERO)
            .unwrap(),
        ApplyWaitOutcome::Applied(_)
    ));
    probes[elected].0.store(64 * 1024 * 1024, Ordering::SeqCst);
    let resumed = drivers[elected].propose(&command()).unwrap();
    pump();
    for i in [1, 2] {
        assert!(matches!(
            drivers[i].wait_applied(resumed, Duration::ZERO).unwrap(),
            ApplyWaitOutcome::Applied(_)
        ));
        assert_eq!(budgets[i].snapshot().reserved_bytes, 0);
        assert_eq!(
            budgets[i].snapshot().file_bytes,
            actual(&dir.join((i + 1).to_string()))
        );
    }
    assert_eq!(
        budgets[elected].snapshot().disk_headroom.unwrap().refusals,
        1
    );
    assert_eq!(
        budgets[3 - elected]
            .snapshot()
            .disk_headroom
            .unwrap()
            .probes,
        0
    );
    drop(drivers);
    std::fs::remove_dir_all(dir).unwrap();
}
