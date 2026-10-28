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
    // Indices remain stable while workers execute outside the registry lock.
    // Only a quiescent slot may become None and be reused.
    slots: Vec<Option<Slot<S, E>>>,
    cursor: usize,
    stopping: bool,
}

struct Shared<S: PersistentRaftStorage, E: crate::ApplyStore + 'static> {
    registry: Mutex<Registry<S, E>>,
    wake: Arc<WorkSignal>,
    tick_every: Duration,
}

/// A node-local pool, not a cluster authority. Explicit unregistration stops
/// and drains a slot before reuse; durable retirement belongs to the caller. No worker or
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
        let free = registry.slots.iter().position(Option::is_none);
        if registry.stopping || (free.is_none() && registry.slots.len() >= MAX_GROUPS) {
            return Err(Error::Raft("shared Raft pool is stopped or full".into()));
        }
        if driver.peer.region_id().0 <= kv9_common::META_REGION_0.0
            || registry
                .slots
                .iter()
                .flatten()
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
        let slot = Some(Slot {
            driver,
            ticks: TickDeadline::new(Instant::now(), self.shared.tick_every),
            running: false,
            finished: false,
        });
        if let Some(index) = free {
            registry.slots[index] = slot;
        } else {
            registry.slots.push(slot);
        }
        drop(registry);
        self.shared.wake.notify();
        Ok(())
    }

    pub fn worker_count(&self) -> usize {
        self.workers.len()
    }

    /// Stop this exact registered driver and release its slot once no worker
    /// owns it. False means an in-flight turn must finish; retry on a later
    /// maintenance turn, without waiting under the registry lock. Other users
    /// may still hold the driver/engine and must drain before deleting files.
    /// The stopped driver can never be registered or spawned again.
    pub fn try_unregister(&self, driver: &Arc<NodeDriver<S, E>>) -> Result<bool> {
        {
            let registry = self.shared.registry.lock().expect("Raft pool poisoned");
            let slot = registry
                .slots
                .iter()
                .flatten()
                .find(|slot| slot.driver.peer.region_id() == driver.peer.region_id());
            match slot {
                None => return Ok(true),
                Some(slot) if !Arc::ptr_eq(&slot.driver, driver) => {
                    return Err(Error::Raft("group is owned by a different driver".into()));
                }
                _ => {}
            }
        }
        // stop closes waiters and may acquire peer locks. Never nest it under
        // the registry lock, where it would block unrelated workers.
        driver.stop();
        let removed = {
            let mut registry = self.shared.registry.lock().expect("Raft pool poisoned");
            let slot = registry.slots.iter_mut().find(|slot| {
                slot.as_ref()
                    .is_some_and(|s| Arc::ptr_eq(&s.driver, driver))
            });
            match slot {
                None => return Ok(true), // another caller already removed it
                Some(slot) if slot.as_ref().unwrap().running => return Ok(false),
                Some(slot) => slot.take(),
            }
        };
        // Destructors (including transport unregistration) run outside the lock.
        drop(removed);
        self.shared.wake.notify();
        Ok(true)
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
                let Some(slot) = &mut registry.slots[index] else {
                    continue;
                };
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
            // Return the worker's reference BEFORE permitting slot reuse. A
            // successful unregister must mean this worker no longer owns it.
            drop(driver);
            let mut registry = shared.registry.lock().expect("Raft pool poisoned");
            let slot = registry.slots[index]
                .as_mut()
                .expect("running slot retained");
            slot.running = false;
            slot.finished = result.is_err();
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
            registry
                .slots
                .iter()
                .flatten()
                .map(|s| s.driver.clone())
                .collect()
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
            if pool.shared.registry.lock().unwrap().slots[0]
                .as_ref()
                .unwrap()
                .running
            {
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

    #[test]
    fn group_pool_unregister_waits_for_running_turn_and_reuses_stable_slots() {
        let pool = DriverPool::new(2, Duration::from_secs(60)).unwrap();
        let blocked = driver(10);
        let held = blocked.pump_gate.lock().unwrap();
        pool.register(blocked.clone()).unwrap();
        blocked.peer.campaign().unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if pool.shared.registry.lock().unwrap().slots[0]
                .as_ref()
                .unwrap()
                .running
            {
                break;
            }
            // Do not strand the worker on a failed scheduling assertion.
            if Instant::now() >= deadline {
                drop(held);
                panic!("blocked group was not scheduled");
            }
            std::thread::yield_now();
        }
        let waiting = pool.try_unregister(&blocked).unwrap();
        let healthy = driver(20);
        pool.register(healthy.clone()).unwrap();
        healthy.peer.campaign().unwrap();
        let at = put(&healthy, 1);
        let result = healthy.wait_applied(at, Duration::from_secs(3));
        drop(held);
        assert!(
            !waiting,
            "a running slot was reused before its owner returned"
        );
        assert!(matches!(result, Ok(ApplyWaitOutcome::Applied(_))));
        let deadline = Instant::now() + Duration::from_secs(3);
        while !pool.try_unregister(&blocked).unwrap() {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(
            pool.try_unregister(&blocked).unwrap(),
            "unregister is idempotent"
        );
        assert!(
            pool.register(blocked.clone()).is_err(),
            "a stopped driver cannot restart"
        );
        let old = Arc::downgrade(&blocked);
        drop(blocked);
        assert!(old.upgrade().is_none());

        for region in 30..30 + MAX_GROUPS as u64 + 10 {
            let next = driver(region);
            pool.register(next.clone()).unwrap();
            next.peer.campaign().unwrap();
            applied(&next, put(&next, 2));
            let deadline = Instant::now() + Duration::from_secs(3);
            while !pool.try_unregister(&next).unwrap() {
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
            let weak = Arc::downgrade(&next);
            drop(next);
            assert!(weak.upgrade().is_none());
            assert_eq!(pool.shared.registry.lock().unwrap().slots.len(), 2);
        }
        // Removing/reusing slot zero never shifts the still-live second slot.
        applied(&healthy, put(&healthy, 3));
    }

    #[test]
    fn group_pool_old_unregister_cannot_remove_or_stop_a_replacement() {
        let pool = DriverPool::new(1, Duration::from_secs(60)).unwrap();
        let old = driver(10);
        pool.register(old.clone()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !pool.try_unregister(&old).unwrap() {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        // Infrastructure allows a new owner; permanent ID fencing is the
        // RegionManager's responsibility, separately covered by its tests.
        let replacement = driver(10);
        pool.register(replacement.clone()).unwrap();
        assert!(pool.try_unregister(&old).is_err());
        assert!(!replacement.stop.load(Ordering::Relaxed));
        replacement.peer.campaign().unwrap();
        applied(&replacement, put(&replacement, 4));
    }
}
