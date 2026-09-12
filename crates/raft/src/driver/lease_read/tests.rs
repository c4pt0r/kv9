use super::*;
use crate::lease::ClockReading;
use crate::lease_policy::LeasePolicy;
use crate::rawnode::LeaseClock;
use crate::storage::DiskRaftStorage;
use crate::transport::{InProcEndpoint, InProcHub};
use crate::RaftGroup;
use kv9_common::{NodeId, RegionId};
use kv9_engine::ColumnFamily;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::AtomicU64;
use std::task::{Context, Waker};

// An explicitly controlled logical clock for boundary tests, not a claim
// about the host clock's drift or behavior under suspension.
#[derive(Default)]
struct TestClock {
    now: AtomicU64,
    samples: AtomicU64,
    expire_at_sample: AtomicU64,
}

impl LeaseClock for TestClock {
    fn sample(&self) -> crate::lease::Result<ClockReading> {
        let sample = self.samples.fetch_add(1, Ordering::SeqCst) + 1;
        if sample == self.expire_at_sample.load(Ordering::SeqCst) {
            self.now.store(200, Ordering::SeqCst);
        }
        Ok(ClockReading {
            domain: 1,
            nanos: self.now.load(Ordering::SeqCst),
        })
    }
}

struct CountTransport {
    endpoint: InProcEndpoint,
    sends: Arc<AtomicU64>,
}

impl RaftTransport for CountTransport {
    fn set_work_signal(&self, signal: Arc<crate::work::WorkSignal>) {
        self.endpoint.set_work_signal(signal);
    }
    fn send(&self, to: NodeId, msg: raft::eraftpb::Message) {
        self.sends.fetch_add(1, Ordering::SeqCst);
        self.endpoint.send(to, msg);
    }
    fn drain(&self) -> Vec<raft::eraftpb::Message> {
        self.endpoint.drain()
    }
}

struct Fixture {
    drivers: Vec<Arc<NodeDriver<DiskRaftStorage, WalEngine>>>,
    clocks: Vec<Arc<TestClock>>,
    sends: Arc<AtomicU64>,
    root: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.drivers.clear();
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "kv9-lease-read-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir(&root).unwrap();
        let mut fixture = Self {
            drivers: Vec::new(),
            clocks: Vec::new(),
            sends: Arc::default(),
            root,
        };
        let hub = InProcHub::new();
        for node in 1..=3 {
            let (storage, _) =
                DiskRaftStorage::open(&fixture.root.join(format!("raft-{node}")), &[1, 2, 3])
                    .unwrap();
            let clock = Arc::new(TestClock::default());
            let peer = Arc::new(
                crate::rawnode::RaftPeer::with_lease_storage(
                    NodeId(node),
                    RegionId(0),
                    storage,
                    LeasePolicy {
                        node,
                        group: 0,
                        configuration: 7,
                        voters: vec![1, 2, 3],
                        promise_ns: 100,
                        drift_ppb: 0,
                        margin_ns: 0,
                    },
                    clock.clone(),
                )
                .unwrap(),
            );
            let (engine, _) =
                WalEngine::open(fixture.root.join(format!("engine-{node}.wal"))).unwrap();
            fixture.drivers.push(
                NodeDriver::new(
                    peer,
                    Arc::new(CountTransport {
                        endpoint: hub.endpoint(NodeId(node)),
                        sends: fixture.sends.clone(),
                    }),
                    MemStateMachine::with_engine(Arc::new(engine)).unwrap(),
                )
                .unwrap(),
            );
            clock.now.store(100, Ordering::SeqCst);
            fixture.clocks.push(clock);
        }
        fixture.drivers[0].peer().campaign().unwrap();
        fixture.pump();
        assert_eq!(fixture.drivers[0].status().role, Role::Leader);
        assert!(
            fixture.local_view().is_some(),
            "actual quorum renewal never established authority"
        );
        fixture
    }

    fn pump(&self) {
        for _ in 0..16 {
            for driver in &self.drivers {
                driver.step().unwrap();
            }
        }
    }

    fn local_view(&self) -> Option<LeaseReadView> {
        let started = Instant::now();
        self.drivers[0]
            .try_lease_read_view(started, started + Duration::from_secs(5))
            .unwrap()
    }

    fn put(&self, value: &[u8]) -> u64 {
        let at = self.drivers[0]
            .propose(&Command::Put {
                cf: 0,
                key: b"key".to_vec(),
                value: value.to_vec(),
            })
            .unwrap();
        self.pump();
        at.index.0
    }
}

fn pending<F: Future>(mut future: Pin<&mut F>) {
    assert!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending(),
        "read succeeded before its required authority/application"
    );
}

#[tokio::test]
async fn valid_lease_returns_the_exact_owned_view_without_a_read_quorum_round() {
    let fixture = Fixture::new();
    let at = fixture.put(b"first");
    let driver = &fixture.drivers[0];
    let sends = fixture.sends.load(Ordering::SeqCst);
    let groups = driver.async_read_snapshot().group_attempts;
    let ReadPreparation::Lease(view) = driver
        .read_preparation_async(Duration::from_secs(5))
        .await
        .unwrap()
    else {
        panic!("valid lease missed its local path");
    };
    assert!(view.applied_through() >= at);
    assert_eq!(fixture.sends.load(Ordering::SeqCst), sends);
    assert_eq!(driver.async_read_snapshot().group_attempts, groups);
    assert_eq!(driver.lease_read_hits(), 1);
    assert_eq!(driver.async_read_snapshot().in_flight, 0);
    fixture.put(b"second");
    fixture.clocks[0].now.store(200, Ordering::SeqCst);
    assert_eq!(
        view.into_view().get(ColumnFamily::Default, b"key").unwrap(),
        Some(b"first".to_vec()),
        "authorized snapshot was replaced after a later write or expiry"
    );
    assert_eq!(
        driver.get(ColumnFamily::Default, b"key").unwrap(),
        Some(b"second".to_vec())
    );
}

#[tokio::test]
async fn expired_isolated_leader_times_out_without_returning_local_data() {
    let fixture = Fixture::new();
    fixture.put(b"old");
    let driver = &fixture.drivers[0];
    fixture.clocks[0].now.store(200, Ordering::SeqCst);
    let mut read = Box::pin(driver.read_preparation_async(Duration::from_millis(25)));
    pending(read.as_mut());
    // Followers do not run: renewal requests and ReadIndex cannot get ACKs.
    for _ in 0..4 {
        driver.step().unwrap();
    }
    assert!(matches!(
        read.await,
        Err(ReadIndexError::Unconfirmed {
            phase: BarrierPhase::QuorumConfirmation,
            ..
        })
    ));
    assert_eq!(driver.lease_read_hits(), 0);
    driver.step().unwrap();
    assert_eq!(driver.async_read_snapshot().in_flight, 0);
}

#[tokio::test]
async fn expiry_after_snapshot_capture_discards_local_authority_and_queues_fallback() {
    let fixture = Fixture::new();
    fixture.put(b"old");
    let driver = &fixture.drivers[0];
    let clock = &fixture.clocks[0];
    clock
        .expire_at_sample
        .store(clock.samples.load(Ordering::SeqCst) + 2, Ordering::SeqCst);
    let mut read = Box::pin(driver.read_preparation_async(Duration::from_secs(5)));
    pending(read.as_mut());
    assert_eq!(clock.now.load(Ordering::SeqCst), 200);
    let snapshot = driver.async_read_snapshot();
    assert_eq!(
        (snapshot.in_flight, snapshot.queued, snapshot.local_reserved),
        (1, 1, 0)
    );
    assert_eq!(driver.lease_read_hits(), 0);
    drop(read);
    driver.step().unwrap();
    assert_eq!(driver.async_read_snapshot().in_flight, 0);
}

#[tokio::test]
async fn fresh_commit_frontier_cannot_authorize_an_unapplied_view() {
    let fixture = Fixture::new();
    fixture.put(b"old");
    let driver = &fixture.drivers[0];
    driver.pause_apply(true);
    let at = fixture.put(b"new");
    assert!(driver.status().raft_committed >= at);
    assert!(driver.status().applied_index < at);
    assert!(
        fixture.local_view().is_none(),
        "cached renewal commit ignored a newer committed write"
    );
    let mut read = Box::pin(driver.read_preparation_async(Duration::from_secs(5)));
    pending(read.as_mut());
    fixture.pump();
    pending(read.as_mut());
    assert_eq!(driver.lease_read_hits(), 0);
    driver.pause_apply(false);
    fixture.pump();
    let ReadPreparation::Quorum(barrier) = read.await.unwrap() else {
        panic!("queued fallback fabricated lease authority");
    };
    assert!(barrier.index() >= at);
    assert_eq!(
        driver.get(ColumnFamily::Default, b"key").unwrap(),
        Some(b"new".to_vec())
    );
    assert_eq!(driver.async_read_snapshot().in_flight, 0);
}

#[tokio::test]
async fn owner_and_apply_contention_transfer_one_slot_without_blocking_the_reader() {
    let fixture = Fixture::new();
    let driver = &fixture.drivers[0];
    for kind in 0..3 {
        let owner = (kind == 0).then(|| driver.pump_gate.lock().unwrap());
        let sm = (kind == 1).then(|| driver.sm.lock().unwrap());
        let applied = (kind == 2).then(|| driver.driver_applied.lock().unwrap());
        let mut read = Box::pin(driver.read_preparation_async(Duration::from_secs(5)));
        pending(read.as_mut());
        let snapshot = driver.async_read_snapshot();
        assert_eq!(
            (snapshot.in_flight, snapshot.local_reserved, snapshot.queued),
            (1, 0, 1)
        );
        drop(read);
        drop((owner, sm, applied));
        driver.step().unwrap();
        assert_eq!(driver.async_read_snapshot().in_flight, 0);
    }
    assert_eq!(driver.lease_read_hits(), 0);
}

#[tokio::test]
async fn a_valid_lease_cannot_bypass_the_existing_read_admission_limit() {
    let fixture = Fixture::new();
    let driver = &fixture.drivers[0];
    let mut waiting = Vec::new();
    for _ in 0..driver.async_read_snapshot().limit {
        let mut read = Box::pin(driver.read_barrier_async(Duration::from_secs(5)));
        pending(read.as_mut());
        waiting.push(read);
    }
    assert!(
        matches!(
            driver.read_preparation_async(Duration::from_secs(5)).await,
            Err(ReadIndexError::Failed(_))
        ),
        "valid lease bypassed the shared read admission limit"
    );
    assert_eq!(driver.lease_read_hits(), 0);
    drop(waiting);
    fixture.pump();
    assert_eq!(driver.async_read_snapshot().in_flight, 0);
}

#[tokio::test]
async fn stopped_and_failed_owners_never_authorize_a_new_lease_view() {
    for failed in [false, true] {
        let fixture = Fixture::new();
        let driver = &fixture.drivers[0];
        if failed {
            driver
                .peer()
                .propose_traced(vec![0xff, 0xee, 0xdd])
                .unwrap();
            let mut observed = false;
            for _ in 0..16 {
                for next in &fixture.drivers {
                    if next.step().is_err() {
                        observed = true;
                    }
                }
                if driver.status().fatal.is_some() {
                    break;
                }
            }
            assert!(observed && driver.status().fatal.is_some());
        } else {
            driver.stop();
        }
        assert!(matches!(
            driver.read_preparation_async(Duration::from_secs(5)).await,
            Err(ReadIndexError::Failed(_))
        ));
        assert_eq!(driver.lease_read_hits(), 0);
        assert_eq!(driver.async_read_snapshot().in_flight, 0);
    }
}

#[test]
fn a_read_ticket_from_another_peer_installation_cannot_authorize_a_view() {
    let first = Fixture::new();
    let second = Fixture::new();
    let ticket = first.drivers[0]
        .peer()
        .try_begin_lease_read(5_000_000_000)
        .unwrap()
        .unwrap();
    let through = second.drivers[0].driver_applied().unwrap().index;
    assert!(
        !second.drivers[0]
            .peer()
            .try_finish_lease_read(ticket, through)
            .unwrap(),
        "foreign peer installation authorized a lease view"
    );
}
