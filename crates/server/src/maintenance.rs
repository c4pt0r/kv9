//! Shared admission for node background maintenance, never protocol authority.
//! Admission precedes dispatch and its non-clone permit lives in the actual
//! synchronous job. Foreground/consensus work never waits for these permits.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const CLASSES: usize = 10;
const JOBS: usize = 4;
const HEAVY: usize = 2;
const BURST: usize = 4;
const REFILL: Duration = Duration::from_millis(10);

#[derive(Clone, Copy, Debug)]
pub(crate) enum Class {
    Ttl,
    DataCheckpoint,
    MetadataCheckpoint,
    Compaction,
    CompactionPlan,
    RaftReclaim,
    RetiredReclaim,
    CheckpointRetention,
    Split,
    ReplicaPressure,
}
const ALL: [Class; CLASSES] = [
    Class::Ttl,
    Class::DataCheckpoint,
    Class::MetadataCheckpoint,
    Class::Compaction,
    Class::CompactionPlan,
    Class::RaftReclaim,
    Class::RetiredReclaim,
    Class::CheckpointRetention,
    Class::Split,
    Class::ReplicaPressure,
];
impl Class {
    fn index(self) -> usize {
        self as usize
    }
    fn heavy(self) -> bool {
        !matches!(
            self,
            Self::Ttl | Self::CheckpointRetention | Self::ReplicaPressure
        )
    }
    fn limit(self) -> usize {
        match self {
            Self::Ttl | Self::DataCheckpoint | Self::ReplicaPressure => 2,
            _ => 1,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Ttl => "ttl",
            Self::DataCheckpoint => "data_checkpoint",
            Self::MetadataCheckpoint => "metadata_checkpoint",
            Self::Compaction => "compaction",
            Self::CompactionPlan => "compaction_plan",
            Self::RaftReclaim => "raft_reclaim",
            Self::RetiredReclaim => "retired_reclaim",
            Self::CheckpointRetention => "checkpoint_retention",
            Self::Split => "split",
            Self::ReplicaPressure => "replica_pressure",
        }
    }
}

#[derive(Clone, Default, serde::Serialize)]
pub(crate) struct ClassStats {
    class: &'static str,
    active: usize,
    peak: usize,
    admitted: u64,
    released: u64,
    deferred_polls: u64,
    wait_millis: u64,
    max_wait_millis: u64,
    occupied_millis: u64,
}
#[derive(Clone, serde::Serialize)]
pub(crate) struct Snapshot {
    pub active: usize,
    pub peak: usize,
    pub heavy_active: usize,
    pub heavy_peak: usize,
    pub waiting_classes: usize,
    job_limit: usize,
    heavy_limit: usize,
    dispatch_burst: usize,
    dispatch_refill_millis: u64,
    classes: [ClassStats; CLASSES],
}
struct State {
    stats: Snapshot,
    waiting: [Option<Instant>; CLASSES],
    registered: [bool; CLASSES],
    tokens: usize,
    refilled: Instant,
}
pub(crate) struct Budget {
    state: Mutex<State>,
}
pub(crate) struct Controls {
    pub compaction: Lane,
    pub compaction_plan: Lane,
    pub raft_reclaim: Lane,
    pub retired_reclaim: Lane,
    pub compaction_after: Option<u64>,
    pub plan_after: Option<u64>,
    pub raft_reclaim_after: Option<u64>,
    pub retired_reclaim_after: Option<u64>,
}
impl Controls {
    pub(crate) fn new(budget: &Arc<Budget>) -> Self {
        Self {
            compaction: budget.lane(Class::Compaction),
            compaction_plan: budget.lane(Class::CompactionPlan),
            raft_reclaim: budget.lane(Class::RaftReclaim),
            retired_reclaim: budget.lane(Class::RetiredReclaim),
            compaction_after: None,
            plan_after: None,
            raft_reclaim_after: None,
            retired_reclaim_after: None,
        }
    }
    pub(crate) fn withdraw(&mut self) {
        self.compaction.withdraw();
        self.compaction_plan.withdraw();
        self.raft_reclaim.withdraw();
        self.retired_reclaim.withdraw();
    }
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                stats: Snapshot {
                    active: 0,
                    peak: 0,
                    heavy_active: 0,
                    heavy_peak: 0,
                    waiting_classes: 0,
                    job_limit: JOBS,
                    heavy_limit: HEAVY,
                    dispatch_burst: BURST,
                    dispatch_refill_millis: REFILL.as_millis() as u64,
                    classes: ALL.map(|class| ClassStats {
                        class: class.name(),
                        ..ClassStats::default()
                    }),
                },
                waiting: [None; CLASSES],
                registered: [false; CLASSES],
                tokens: BURST,
                refilled: Instant::now(),
            }),
        }
    }
}
impl Budget {
    /// Exactly one dispatcher per class. The dispatcher keeps this lane even
    /// across idle periods; there are no per-group queue allocations.
    pub(crate) fn lane(self: &Arc<Self>, class: Class) -> Lane {
        let mut state = self.state.lock().expect("maintenance budget");
        assert!(
            !state.registered[class.index()],
            "duplicate maintenance dispatcher"
        );
        state.registered[class.index()] = true;
        Lane {
            budget: self.clone(),
            class,
        }
    }
    pub(crate) fn snapshot(&self) -> Snapshot {
        let state = self.state.lock().expect("maintenance budget");
        let now = Instant::now();
        let mut stats = state.stats.clone();
        stats.waiting_classes = state.waiting.iter().flatten().count();
        for (i, waiting) in state.waiting.iter().enumerate() {
            stats.classes[i].wait_millis =
                waiting.map_or(0, |since| millis(now.saturating_duration_since(since)));
        }
        stats
    }
}

/// A single bounded demand slot per maintenance class. Call only with eligible
/// work, poll on the existing timer, and withdraw when no work remains. No lock
/// is held across filesystem, object, engine, RPC or consensus operations.
pub(crate) struct Lane {
    budget: Arc<Budget>,
    class: Class,
}
impl Lane {
    pub(crate) fn try_acquire(&mut self) -> Option<Permit> {
        self.try_at(Instant::now())
    }
    fn try_at(&mut self, now: Instant) -> Option<Permit> {
        let mut state = self.budget.state.lock().expect("maintenance budget");
        let elapsed = now.saturating_duration_since(state.refilled);
        let units = elapsed.as_nanos() / REFILL.as_nanos();
        if units > 0 {
            state.tokens = BURST.min(state.tokens + units.min(BURST as u128) as usize);
            // Preserve fractional credit without carrying an unbounded burst.
            state.refilled =
                now - Duration::from_nanos((elapsed.as_nanos() % REFILL.as_nanos()) as u64);
        }
        let i = self.class.index();
        state.waiting[i].get_or_insert(now);
        // Oldest eligible class wins. A heavy class blocked by its own cap
        // cannot prevent a TTL page using the remaining node capacity.
        let winner = ALL
            .into_iter()
            .filter(|c| {
                state.stats.classes[c.index()].active < c.limit()
                    && (!c.heavy() || state.stats.heavy_active < HEAVY)
            })
            .filter_map(|c| state.waiting[c.index()].map(|at| (at, c.index())))
            .min();
        if state.stats.active >= JOBS || state.tokens == 0 || winner.map(|(_, w)| w) != Some(i) {
            let stats = &mut state.stats.classes[i];
            stats.deferred_polls = stats.deferred_polls.saturating_add(1);
            return None;
        }
        let waiting = state.waiting[i].take().unwrap();
        state.tokens -= 1;
        state.stats.active += 1;
        state.stats.peak = state.stats.peak.max(state.stats.active);
        state.stats.heavy_active += usize::from(self.class.heavy());
        state.stats.heavy_peak = state.stats.heavy_peak.max(state.stats.heavy_active);
        let stats = &mut state.stats.classes[i];
        stats.active += 1;
        stats.peak = stats.peak.max(stats.active);
        stats.admitted = stats.admitted.saturating_add(1);
        stats.max_wait_millis = stats.max_wait_millis.max(millis(now - waiting));
        Some(Permit {
            budget: self.budget.clone(),
            class: self.class,
            started: Instant::now(),
        })
    }
    pub(crate) fn withdraw(&mut self) {
        self.budget
            .state
            .lock()
            .expect("maintenance budget")
            .waiting[self.class.index()] = None;
    }
}
impl Drop for Lane {
    fn drop(&mut self) {
        let mut state = self.budget.state.lock().expect("maintenance budget");
        state.waiting[self.class.index()] = None;
        state.registered[self.class.index()] = false;
    }
}

/// Owned by the actual job, including blocking-pool queue time and synchronous
/// I/O. Cancellation of its waiter must never release this permit early.
pub(crate) struct Permit {
    budget: Arc<Budget>,
    class: Class,
    started: Instant,
}
impl Drop for Permit {
    fn drop(&mut self) {
        let mut state = self.budget.state.lock().expect("maintenance budget");
        state.stats.active -= 1;
        state.stats.heavy_active -= usize::from(self.class.heavy());
        let stats = &mut state.stats.classes[self.class.index()];
        stats.active -= 1;
        stats.released = stats.released.saturating_add(1);
        stats.occupied_millis = stats
            .occupied_millis
            .saturating_add(millis(self.started.elapsed()));
    }
}
fn millis(duration: Duration) -> u64 {
    duration.as_millis().min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests;
