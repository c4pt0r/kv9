use super::*;
use crate::ttl_clock::ClockResult;
use kv9_common::data_range::DataRange;
use kv9_common::raw_ttl::{ClockSample, ClockWatermark, ExpiryPlan, CLOCK_KEY};
use kv9_raft::command::{RawTtlAction, RawTtlCommand, RawTtlMutation};
use kv9_raft::transport::{InProcHub, RaftTransport};
use kv9_raw::raw::ttl::TtlRead;

struct Sample(ClockResult<ClockSample>);
impl TtlClock for Sample {
    fn sample(&self) -> ClockResult<ClockSample> {
        self.0
    }
    fn status(&self) -> ClockStatus {
        panic!("status is not sample authority")
    }
}

fn range() -> DataRange {
    DataRange {
        root: RootDigest::from_bytes([1; 32]),
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

fn open(path: &Path) -> (Arc<WalEngine>, Arc<NodeDriver<DiskRaftStorage, WalEngine>>) {
    let engine = Arc::new(WalEngine::open(path.join("engine.wal")).unwrap().0);
    let storage = DiskRaftStorage::open(&path.join("raft"), &[1]).unwrap().0;
    let mut sm = MemStateMachine::with_engine(engine.clone()).unwrap();
    let r = range();
    sm.set_data_group(r.root, r.region, r.creation).unwrap();
    let hub = InProcHub::new();
    let driver = NodeDriver::new(
        Arc::new(RaftPeer::with_storage(NodeId(1), r.region, storage).unwrap()),
        Arc::new(hub.endpoint(NodeId(1))) as Arc<dyn RaftTransport>,
        sm,
    )
    .unwrap();
    driver.peer().campaign().unwrap();
    pump(&driver);
    (engine, driver)
}
fn pump(driver: &NodeDriver<DiskRaftStorage, WalEngine>) {
    for _ in 0..20 {
        driver.step().unwrap();
    }
}
fn ttl(action: RawTtlAction) -> Command {
    Command::RawTtl(RawTtlCommand::new(&range(), action).unwrap())
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

#[tokio::test]
async fn clock_refusals_never_propose_and_qualified_observation_survives_recovery() {
    let path = std::env::temp_dir().join(format!(
        "kv9-ttl-clock-recovery-{}",
        StoreIncarnation::mint().unwrap()
    ));
    std::fs::create_dir_all(&path).unwrap();
    let (engine, driver) = open(&path);
    driver
        .propose(&Command::DataRange {
            expected: None,
            next: range(),
        })
        .unwrap();
    pump(&driver);
    assert!(matches!(
        submit_observation(
            &driver,
            &range(),
            &Sample(Err(ClockRefusal::SourceFailed)),
            deadline()
        ),
        Err(Error::RawTtlRejected {
            reason: kv9_common::raw_ttl::TtlRejection::NotEnabled,
            ..
        })
    ));
    // Explicit apply-fixture injection; production activation has separate
    // all-replica tests. No sampled time is created at format activation.
    driver
        .peer()
        .propose_raw_for_harness(
            ttl(RawTtlAction::Enable {
                observed: ClockWatermark::default(),
            })
            .encode(),
        )
        .unwrap();
    pump(&driver);
    let at = driver.driver_applied();
    for refusal in [
        ClockRefusal::WarmingUp,
        ClockRefusal::Unsynchronized,
        ClockRefusal::Fenced,
    ] {
        assert!(
            matches!(submit_observation(&driver, &range(), &Sample(Err(refusal)), deadline()), Err(Error::RawTtlClock { reason }) if reason == refusal)
        );
        pump(&driver);
        assert_eq!(driver.driver_applied(), at);
    }
    let write = ttl(RawTtlAction::Write {
        plan: ExpiryPlan::resolve(
            Some(1),
            ClockSample::with_uncertainty(1000, 0).unwrap(),
            ClockWatermark::default(),
        )
        .unwrap(),
        mutations: vec![RawTtlMutation::Put {
            key: b"k".to_vec(),
            value: b"v".to_vec(),
        }],
    });
    driver.propose(&write).unwrap();
    pump(&driver);
    let before = engine.snapshot().unwrap();
    let observation = submit_observation(
        &driver,
        &range(),
        &Sample(Ok(ClockSample::with_uncertainty(2100, 10).unwrap())),
        deadline(),
    )
    .unwrap();
    let at = observation.proposed_at();
    pump(&driver);
    assert_eq!(
        observation.wait().await.unwrap(),
        ApplyWaitOutcome::Applied(kv9_common::AppliedPosition {
            term: at.term,
            index: at.index.0
        })
    );
    let owner = driver.spawn(Duration::from_millis(10)).unwrap();
    let _barrier = driver.read_barrier(Duration::from_secs(1)).unwrap();
    let after = engine.snapshot().unwrap();
    assert_eq!(
        TtlRead::from_leader(&LeaderRead::new(before.as_ref(), true, None).unwrap())
            .unwrap()
            .get(range().keyspace, b"k")
            .unwrap(),
        Some(b"v".to_vec())
    );
    assert_eq!(
        TtlRead::from_leader(&LeaderRead::new(after.as_ref(), true, None).unwrap())
            .unwrap()
            .get(range().keyspace, b"k")
            .unwrap(),
        None
    );
    let clock = engine
        .get(kv9_engine::ColumnFamily::Default, CLOCK_KEY)
        .unwrap()
        .unwrap();
    assert_eq!(ClockWatermark::decode(&clock).unwrap().unix_ms(), 2090);
    driver.stop();
    owner.join().unwrap();
    drop(before);
    drop(after);
    drop(driver);
    drop(engine);

    let (engine, driver) = open(&path);
    assert_eq!(driver.raw_ttl_state().unwrap().1.unix_ms(), 2090);
    let before = driver.driver_applied();
    assert!(matches!(
        submit_observation(
            &driver,
            &range(),
            &Sample(Ok(ClockSample::with_uncertainty(1500, 0).unwrap())),
            deadline()
        ),
        Err(Error::RawTtlClock {
            reason: ClockRefusal::BehindWatermark
        })
    ));
    pump(&driver);
    assert_eq!(driver.driver_applied(), before);
    assert_eq!(
        engine
            .get(kv9_engine::ColumnFamily::Default, CLOCK_KEY)
            .unwrap(),
        Some(clock)
    );
    driver.stop();
    drop(driver);
    drop(engine);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn unconfigured_clock_has_no_wall_clock_fallback() {
    assert!(crate::ttl_clock::configured(None).unwrap().is_none());
}
