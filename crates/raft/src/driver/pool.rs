//! Fixed workers for dynamically registered groups. One coalesced signal per
//! group plus a wake hint bounds scheduler memory independently of traffic.
//! Metadata retains its dedicated owner outside this pool.

use super::*;
use crate::work::{TickDeadline, WorkSignal};
use crate::RaftGroup;

const MAX_GROUPS: usize = 255;
const MAX_WORKERS: usize = 32;

struct Slot<S: PersistentRaftStorage, E: crate::ApplyStore + 'static> {
    driver: Arc<NodeDriver<S, E>>,
    ticks: TickDeadline,
    running: bool,
    finished: bool,
}

struct Registry<S: PersistentRaftStorage, E: crate::ApplyStore + 'static> {
    slots: Vec<Slot<S, E>>,
    cursor: usize,
    stopping: bool,
}

struct Shared<S: PersistentRaftStorage, E: crate::ApplyStore + 'static> {
    registry: Mutex<Registry<S, E>>,
    wake: Arc<WorkSignal>,
    tick_every: Duration,
}

/// A node-local pool, not a cluster authority. Registration is append-only;
/// failed slots retain their identity until shutdown/recovery. No worker or
/// registry lock is held across another group's persistence/application work.
pub struct DriverPool<S: PersistentRaftStorage, E: crate::ApplyStore + 'static> {
    shared: Arc<Shared<S, E>>,
    workers: Vec<std::thread::JoinHandle<()>>,
}

impl<S: PersistentRaftStorage, E: crate::ApplyStore + 'static> DriverPool<S, E> {
    pub fn new(workers: usize, tick_every: Duration) -> Result<Self> {
        if !(1..=MAX_WORKERS).contains(&workers)
            || tick_every.is_zero()
            || tick_every > Duration::from_secs(60)
        {
            return Err(Error::Config(
                "invalid shared Raft worker count or tick interval".into(),
            ));
        }
        let mut pool = Self {
            shared: Arc::new(Shared {
                registry: Mutex::new(Registry {
                    slots: Vec::new(),
                    cursor: 0,
                    stopping: false,
                }),
                wake: Arc::new(WorkSignal::default()),
                tick_every,
            }),
            workers: Vec::new(),
        };
        for index in 0..workers {
            let shared = pool.shared.clone();
            let worker = std::thread::Builder::new()
                .name(format!("raft-data-{index}"))
                .spawn(move || run(shared))
                .map_err(|e| Error::Raft(format!("start shared Raft worker: {e}")))?;
            pool.workers.push(worker);
        }
        Ok(pool)
    }

    pub fn register(&self, driver: Arc<NodeDriver<S, E>>) -> Result<()> {
        let mut registry = self.shared.registry.lock().expect("Raft pool poisoned");
        if registry.stopping || registry.slots.len() >= MAX_GROUPS {
            return Err(Error::Raft("shared Raft pool is stopped or full".into()));
        }
        if driver.peer.region_id().0 <= kv9_common::META_REGION_0.0
            || registry
                .slots
                .iter()
                .any(|s| s.driver.peer.region_id() == driver.peer.region_id())
        {
            return Err(Error::Raft(
                "shared Raft group is reserved or already registered".into(),
            ));
        }
        driver
            .pump_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Error::Raft("background pump already owns this driver".into()))?;
        if let Err(error) = driver.peer.work_signal.subscribe(&self.shared.wake) {
            driver.stop();
            return Err(error);
        }
        registry.slots.push(Slot {
            driver,
            ticks: TickDeadline::new(Instant::now(), self.shared.tick_every),
            running: false,
            finished: false,
        });
        drop(registry);
        self.shared.wake.notify();
        Ok(())
    }

    pub fn worker_count(&self) -> usize {
        self.workers.len()
    }
}

fn run<S: PersistentRaftStorage, E: crate::ApplyStore + 'static>(shared: Arc<Shared<S, E>>) {
    while shared.wake.begin_turn() {
        let (selected, deadline) = {
            let mut registry = shared.registry.lock().expect("Raft pool poisoned");
            if registry.stopping {
                break;
            }
            let now = Instant::now();
            let mut deadline = now + Duration::from_secs(60);
            let mut selected = None;
            for offset in 0..registry.slots.len() {
                let index = (registry.cursor + offset) % registry.slots.len();
                let slot = &mut registry.slots[index];
                if slot.running || slot.finished {
                    continue;
                }
                deadline = deadline.min(slot.ticks.next());
                if slot.driver.peer.work_signal.has_work() || now >= slot.ticks.next() {
                    slot.running = true;
                    let tick = slot.ticks.due(now);
                    selected = Some((index, slot.driver.clone(), tick));
                    registry.cursor = (index + 1) % registry.slots.len();
                    break;
                }
            }
            (selected, deadline)
        };
        if let Some((index, driver, tick)) = selected {
            // A second worker can take another group while this one blocks on
            // I/O. Neither this hint nor a producer hint advances Raft time.
            shared.wake.notify();
            let alive =
                !driver.stop.load(Ordering::Relaxed) && driver.peer.work_signal.begin_turn();
            let result = if alive {
                let _owner = driver.pump_gate.lock().expect("pump gate poisoned");
                if tick {
                    driver.peer.tick_once();
                }
                driver.step_observed()
            } else {
                Err(Error::Raft("shared Raft group stopped".into()))
            };
            if result.is_err() {
                driver.stop();
            } else if driver.peer.has_pending_ready() {
                driver.peer.work_signal.notify();
            }
            let mut registry = shared.registry.lock().expect("Raft pool poisoned");
            registry.slots[index].running = false;
            registry.slots[index].finished = result.is_err();
            drop(registry);
            // A producer may have notified while this group was in flight.
            // Its child pending bit survives; wake after releasing ownership.
            shared.wake.notify();
        } else {
            shared.wake.wait_until(deadline);
        }
    }
}

impl<S: PersistentRaftStorage, E: crate::ApplyStore + 'static> Drop for DriverPool<S, E> {
    fn drop(&mut self) {
        let drivers: Vec<_> = {
            let mut registry = self.shared.registry.lock().expect("Raft pool poisoned");
            registry.stopping = true;
            registry.slots.iter().map(|s| s.driver.clone()).collect()
        };
        self.shared.wake.stop();
        for driver in drivers {
            driver.stop();
        }
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kv9_common::RegionId;

    fn driver(region: u64) -> Arc<NodeDriver> {
        let hub = crate::transport::InProcHub::new();
        NodeDriver::new(
            Arc::new(RaftPeer::new(NodeId(1), RegionId(region), &[NodeId(1)]).unwrap()),
            Arc::new(hub.endpoint(NodeId(1))),
            MemStateMachine::new(),
        )
        .unwrap()
    }

    fn put(driver: &NodeDriver, value: u8) -> ProposedAt {
        driver
            .propose(&Command::Put {
                cf: 0,
                key: b"key".to_vec(),
                value: vec![value],
            })
            .unwrap()
    }

    fn applied(driver: &NodeDriver, at: ProposedAt) {
        assert!(
            matches!(
                driver.wait_applied(at, Duration::from_secs(3)),
                Ok(ApplyWaitOutcome::Applied(_))
            ),
            "shared owner must apply work without waiting for the 60-second tick"
        );
    }

    #[test]
    fn group_pool_preexisting_and_parked_notifications_do_not_wait_for_ticks() {
        let pool = DriverPool::new(1, Duration::from_secs(60)).unwrap();
        let d = driver(10);
        d.peer.campaign().unwrap();
        let first = put(&d, 1); // Work precedes pool subscription.
        pool.register(d.clone()).unwrap();
        applied(&d, first);
        let parked = pool.shared.wake.observe_next_park();
        pool.shared.wake.notify();
        parked.recv_timeout(Duration::from_secs(3)).unwrap();
        let next = put(&d, 2);
        applied(&d, next);
        assert_eq!(
            d.get(kv9_engine::ColumnFamily::Default, b"key").unwrap(),
            Some(vec![2])
        );
        drop(pool);
        assert!(
            d.stop.load(Ordering::Relaxed),
            "pool shutdown must stop its drivers"
        );
        assert!(
            d.spawn(Duration::from_millis(1)).is_err(),
            "a stopped pooled driver cannot acquire a new owner"
        );
    }

    #[test]
    fn group_pool_blocked_group_does_not_hold_registry_or_other_worker() {
        let pool = DriverPool::new(2, Duration::from_secs(60)).unwrap();
        let slow = driver(10);
        let healthy = driver(20);
        let held = slow.pump_gate.lock().unwrap();
        pool.register(slow.clone()).unwrap();
        slow.peer.campaign().unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if pool.shared.registry.lock().unwrap().slots[0].running {
                break;
            }
            assert!(Instant::now() < deadline, "blocked group was not scheduled");
            std::thread::yield_now();
        }
        pool.register(healthy.clone()).unwrap();
        healthy.peer.campaign().unwrap();
        let healthy_at = put(&healthy, 2);
        // Release the blocked worker even if the negative control fails, so
        // Drop can join it instead of making a failed test hang forever.
        let result = healthy.wait_applied(healthy_at, Duration::from_secs(3));
        let slow_at = put(&slow, 1); // Notification while this slot is in flight.
        drop(held);
        assert!(
            matches!(result, Ok(ApplyWaitOutcome::Applied(_))),
            "one blocked group starved another worker"
        );
        applied(&slow, slow_at);
        assert_eq!(pool.worker_count(), 2);
    }

    #[test]
    fn group_pool_caps_groups_and_refuses_duplicate_background_owners() {
        let pool = DriverPool::new(2, Duration::from_secs(60)).unwrap();
        assert!(pool.register(driver(0)).is_err());
        assert!(pool.register(driver(1)).is_err());
        let first = driver(2);
        pool.register(first.clone()).unwrap();
        assert!(pool.register(first.clone()).is_err());
        assert!(pool.register(driver(2)).is_err());
        assert!(first.spawn(Duration::from_millis(1)).is_err());
        let another = DriverPool::new(1, Duration::from_secs(60)).unwrap();
        assert!(another.register(first).is_err());
        for id in 3..=MAX_GROUPS as u64 + 1 {
            pool.register(driver(id)).unwrap();
        }
        assert!(pool.register(driver(MAX_GROUPS as u64 + 2)).is_err());
        assert_eq!(pool.shared.registry.lock().unwrap().slots.len(), MAX_GROUPS);
        assert_eq!(
            pool.worker_count(),
            2,
            "groups cannot allocate extra worker threads"
        );
    }
}
