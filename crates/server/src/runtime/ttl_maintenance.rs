//! Node-wide bounded TTL maintenance. Cursors are hints, never delete authority;
//! ordered Cleanup rechecks the exact expiry version and committed watermark.
//! Two blocking jobs share a dispatch rate and page budget across all groups.
//! Cancellation drains actual jobs; dropping a future never returns their slot.
use super::range_api::{RawDirectory, RawGroup};
use super::*;
use kv9_common::data_range::{DataRange, RANGE_KEY};
use kv9_common::raw_ttl::{clock::ClockRefusal, ClockWatermark, CLOCK_KEY};
use kv9_engine::ColumnFamily;
use kv9_raft::command::{RawTtlAction, RawTtlCommand};
use kv9_raw::raw::ttl::TtlRead;
use std::sync::Weak;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const WORKERS: usize = 2;
const TRACKED_GROUPS: usize = 256;
const VISITS_PER_TICK: usize = 32;
const INTERVAL: Duration = Duration::from_millis(50);
const IDLE: Duration = Duration::from_secs(1);
const DEADLINE: Duration = Duration::from_secs(2);
const CANDIDATES: usize = 256;
const EXAMINED: usize = 1024;
const CANDIDATE_BYTES: usize = 256 * 1024;
type Ready = Arc<dyn Fn(RegionId) -> Result<()> + Send + Sync>;
type Driver = NodeDriver<DiskRaftStorage, WalEngine>;

pub(super) fn enabled_from_env() -> Result<bool> {
    enabled(std::env::var("KV9_RAW_TTL_MAINTENANCE"))
}
fn enabled(value: std::result::Result<String, std::env::VarError>) -> Result<bool> {
    match value {
        Ok(v) if v == "1" => Ok(true),
        Ok(v) if v == "0" => Ok(false),
        Err(std::env::VarError::NotPresent) => Ok(true),
        _ => Err(Error::Config(
            "KV9_RAW_TTL_MAINTENANCE must be 0 or 1".into(),
        )),
    }
}

#[derive(Clone, Default, serde::Serialize)]
pub(super) struct Stats {
    pub(super) running: bool,
    pub(super) in_flight: usize,
    pub(super) peak_in_flight: usize,
    pub(super) tracked_groups: usize,
    pub(super) groups_with_cursor: usize,
    pub(super) groups_backing_off: usize,
    pub(super) groups_awaiting_apply: usize,
    pub(super) oldest_due_millis: u64,
    pub(super) pages: u64,
    pub(super) sweeps: u64,
    pub(super) examined: u64,
    /// Applied candidate checks, not a count of versions actually deleted.
    pub(super) candidates_applied: u64,
    pub(super) candidate_bytes: u64,
    pub(super) observations_applied: u64,
    pub(super) clock_refusals: u64,
    pub(super) failures: u64,
    pub(super) unconfirmed_submissions: u64,
    pub(super) skipped: u64,
    pub(super) backpressure: u64,
    pub(super) owner_changed: u64,
    pub(super) state_evictions: u64,
    pub(super) last_error: Option<String>,
    pub(super) last_clock_refusal: Option<ClockRefusal>,
}

pub(super) struct Maintenance {
    cancel: CancellationToken,
    task: Option<JoinHandle<()>>,
    stats: Arc<Mutex<Stats>>,
}
impl Maintenance {
    pub(super) fn start(backend: Arc<RuntimeBackend>, runtime: &tokio::runtime::Handle) -> Self {
        Self::spawn_with_budget(
            backend.raw_directory.clone(),
            backend.maintenance.clone(),
            Arc::new(move |_| backend.ensure_serving()),
            runtime,
        )
    }
    #[cfg(test)]
    pub(super) fn spawn(
        directory: Arc<RawDirectory>,
        ready: Ready,
        runtime: &tokio::runtime::Handle,
    ) -> Self {
        Self::spawn_with_budget(directory, Arc::default(), ready, runtime)
    }
    pub(super) fn spawn_with_budget(
        directory: Arc<RawDirectory>,
        budget: Arc<crate::maintenance::Budget>,
        ready: Ready,
        runtime: &tokio::runtime::Handle,
    ) -> Self {
        let cancel = CancellationToken::new();
        let stats = Arc::new(Mutex::new(Stats::default()));
        let task = runtime.spawn(run(directory, budget, ready, cancel.clone(), stats.clone()));
        Self {
            cancel,
            task: Some(task),
            stats,
        }
    }
    pub(super) fn snapshot(&self) -> Stats {
        self.stats.lock().expect("TTL maintenance stats").clone()
    }
    pub(super) fn stopped_unexpectedly(&self) -> bool {
        !self.cancel.is_cancelled() && self.task.as_ref().is_some_and(JoinHandle::is_finished)
    }
    pub(super) fn cancel(&self) {
        self.cancel.cancel();
    }
    pub(super) async fn drain(mut self) {
        self.cancel.cancel();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}
impl Drop for Maintenance {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

struct State {
    digest: RootDigest,
    owner: Weak<Driver>,
    next: Option<UserKey>,
    due: Instant,
    failures: u32,
    clock_failures: u32,
    pending: Option<ProposedAt>,
    visited: u64,
}
impl State {
    fn new(group: &RawGroup, now: Instant) -> Self {
        Self {
            digest: group.range_binding().digest(),
            owner: Arc::downgrade(group.capture_parts().0),
            next: None,
            due: now,
            failures: 0,
            clock_failures: 0,
            pending: None,
            visited: 0,
        }
    }
    fn matches(&self, group: &RawGroup) -> bool {
        self.digest == group.range_binding().digest()
            && self
                .owner
                .upgrade()
                .is_some_and(|owner| Arc::ptr_eq(&owner, group.capture_parts().0))
    }
}
struct Flight {
    group: Arc<RawGroup>,
    state: State,
    job: JoinHandle<PageResult>,
}
#[derive(Default)]
struct Work {
    examined: usize,
    candidate_bytes: usize,
    candidates_applied: usize,
    observed: bool,
    clock_refusal: Option<ClockRefusal>,
    pending: Option<ProposedAt>,
    unconfirmed: bool,
}
struct PageResult {
    work: Work,
    result: Result<Option<UserKey>>,
}

fn backoff(failures: u32) -> Duration {
    Duration::from_millis(
        250_u64
            .saturating_mul(1 << failures.saturating_sub(1).min(7))
            .min(30_000),
    )
}
fn remember(states: &mut BTreeMap<RegionId, State>, id: RegionId, state: State, stats: &mut Stats) {
    if states.len() == TRACKED_GROUPS && !states.contains_key(&id) {
        let victim = states
            .iter()
            .min_by_key(|(_, s)| s.visited)
            .map(|(id, _)| *id)
            .unwrap();
        states.remove(&victim);
        stats.state_evictions = stats.state_evictions.saturating_add(1);
    }
    states.insert(id, state);
}

async fn run(
    directory: Arc<RawDirectory>,
    budget: Arc<crate::maintenance::Budget>,
    ready: Ready,
    cancel: CancellationToken,
    shared: Arc<Mutex<Stats>>,
) {
    let mut states = BTreeMap::<RegionId, State>::new();
    let mut flights = Vec::<Flight>::new();
    let mut after = None;
    let mut sequence = 0_u64;
    let mut failed = false;
    let mut lane = budget.lane(crate::maintenance::Class::Ttl);
    let mut stats = Stats {
        running: true,
        ..Stats::default()
    };
    let mut interval = tokio::time::interval(INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    interval.tick().await;
    loop {
        // A bounded join set. Finished or cancelled is not enough: release the
        // slot only after the actual blocking closure returned (including panic).
        let mut i = 0;
        while i < flights.len() {
            if !flights[i].job.is_finished() {
                i += 1;
                continue;
            }
            let Flight {
                group,
                mut state,
                job,
            } = flights.swap_remove(i);
            let output = match job.await {
                Ok(output) => output,
                Err(error) => {
                    // A panic may lose a just-accepted proposal position. Stop
                    // dispatch, drain other owners, and let NodeRuntime fail.
                    failed = true;
                    stats.failures = stats.failures.saturating_add(1);
                    stats.last_error = Some(
                        format!("TTL maintenance worker: {error}")
                            .chars()
                            .take(160)
                            .collect(),
                    );
                    continue;
                }
            };
            state.pending = output.work.pending;
            stats.unconfirmed_submissions = stats
                .unconfirmed_submissions
                .saturating_add(u64::from(output.work.unconfirmed));
            stats.examined = stats.examined.saturating_add(output.work.examined as u64);
            stats.candidate_bytes = stats
                .candidate_bytes
                .saturating_add(output.work.candidate_bytes as u64);
            stats.candidates_applied = stats
                .candidates_applied
                .saturating_add(output.work.candidates_applied as u64);
            stats.observations_applied = stats
                .observations_applied
                .saturating_add(u64::from(output.work.observed));
            if let Some(reason) = output.work.clock_refusal {
                stats.clock_refusals = stats.clock_refusals.saturating_add(1);
                stats.last_clock_refusal = Some(reason);
                state.clock_failures = state.clock_failures.saturating_add(1);
            } else {
                state.clock_failures = 0;
            }
            match output.result {
                Ok(next) => {
                    stats.pages = stats.pages.saturating_add(1);
                    stats.sweeps = stats.sweeps.saturating_add(u64::from(next.is_none()));
                    state.next = next;
                    state.failures = 0;
                    let delay = if state.next.is_some() {
                        INTERVAL
                    } else if state.clock_failures > 0 {
                        IDLE.max(backoff(state.clock_failures))
                    } else {
                        IDLE
                    };
                    state.due = Instant::now() + delay;
                }
                Err(
                    Error::NotLeader { .. }
                    | Error::RawTtlRejected {
                        reason: kv9_common::raw_ttl::TtlRejection::NotEnabled,
                        ..
                    },
                ) => {
                    stats.skipped = stats.skipped.saturating_add(1);
                    state.failures = 0;
                    state.due = Instant::now() + IDLE;
                }
                Err(error) => {
                    // No cursor advancement on any unconfirmed or failed page.
                    state.failures = state.failures.saturating_add(1);
                    state.due = Instant::now() + backoff(state.failures);
                    stats.failures = stats.failures.saturating_add(1);
                    stats.backpressure = stats.backpressure.saturating_add(u64::from(matches!(
                        error,
                        Error::WriteBackpressure { .. }
                    )));
                    stats.last_error = Some(error.to_string().chars().take(160).collect());
                }
            }
            let id = group.range_binding().region;
            if directory
                .by_region(id)
                .is_some_and(|current| group.same_owner(&current))
            {
                remember(&mut states, id, state, &mut stats);
            } else {
                stats.owner_changed = stats.owner_changed.saturating_add(1);
            }
        }
        states.retain(|id, state| {
            state.owner.strong_count() > 0 && directory.by_region(*id).is_some()
        });
        let mut waiting_for_budget = false;
        if !cancel.is_cancelled() && !failed {
            let mut first = None;
            // Bounded traversal and bounded starts, even if all groups refuse
            // instantly. A stalled group occupies only its one shared slot.
            for _ in 0..VISITS_PER_TICK {
                if flights.len() == WORKERS {
                    break;
                }
                let Some(group) = directory.next_after(after) else {
                    break;
                };
                let id = group.range_binding().region;
                if first == Some(id) {
                    break;
                }
                first.get_or_insert(id);
                after = Some(id);
                if flights.iter().any(|f| f.group.range_binding().region == id) {
                    continue;
                }
                let now = Instant::now();
                let state = states.entry(id).or_insert_with(|| State::new(&group, now));
                if !state.matches(&group) {
                    *state = State::new(&group, now);
                }
                if state.due > now {
                    continue;
                }
                let Some(permit) = lane.try_acquire() else {
                    waiting_for_budget = true;
                    break;
                };
                let mut state = states.remove(&id).expect("inserted");
                sequence = sequence.saturating_add(1);
                state.visited = sequence;
                let job_group = group.clone();
                let directory = directory.clone();
                let ready = ready.clone();
                let cancel = cancel.clone();
                let next = state.next.clone();
                let pending = state.pending;
                let deadline = now + DEADLINE;
                let job = tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    page(
                        &directory, &job_group, &ready, &cancel, next, pending, deadline,
                    )
                });
                flights.push(Flight { group, state, job });
            }
        }
        if !waiting_for_budget {
            lane.withdraw();
        }
        stats.running = !cancel.is_cancelled() && !failed;
        stats.in_flight = flights.len();
        stats.peak_in_flight = stats.peak_in_flight.max(flights.len());
        stats.tracked_groups = states.len() + flights.len();
        stats.groups_with_cursor = states.values().filter(|s| s.next.is_some()).count()
            + flights.iter().filter(|f| f.state.next.is_some()).count();
        stats.groups_backing_off = states
            .values()
            .filter(|s| s.failures > 0 || s.clock_failures > 0)
            .count();
        stats.groups_awaiting_apply = states.values().filter(|s| s.pending.is_some()).count()
            + flights.iter().filter(|f| f.state.pending.is_some()).count();
        let now = Instant::now();
        stats.oldest_due_millis = states
            .values()
            .map(|s| {
                now.saturating_duration_since(s.due)
                    .as_millis()
                    .min(u64::MAX as u128) as u64
            })
            .max()
            .unwrap_or(0);
        *shared.lock().expect("TTL maintenance stats") = stats.clone();
        if (cancel.is_cancelled() || failed) && flights.is_empty() {
            return;
        }
        interval.tick().await;
    }
}

fn page(
    directory: &RawDirectory,
    group: &RawGroup,
    ready: &Ready,
    cancel: &CancellationToken,
    next: Option<UserKey>,
    pending: Option<ProposedAt>,
    deadline: Instant,
) -> PageResult {
    let mut work = Work {
        pending,
        ..Work::default()
    };
    let result = (|| {
        let (driver, engine, range) = group.capture_parts();
        let check = || -> Result<Duration> {
            if cancel.is_cancelled() {
                return Err(Error::Raft("TTL maintenance cancelled".into()));
            }
            ready(range.region)?;
            if !directory
                .by_region(range.region)
                .is_some_and(|current| group.same_owner(&current))
            {
                return Err(Error::StaleEpoch {
                    region: range.region,
                });
            }
            deadline
                .checked_duration_since(Instant::now())
                .filter(|d| !d.is_zero())
                .ok_or_else(|| Error::Raft("TTL maintenance deadline expired".into()))
        };
        check()?;
        if let Some(pending) = work.pending {
            // Timeout releases the worker, not this group's outstanding append
            // budget. Do not accumulate more commands behind an unknown one.
            if driver
                .driver_applied()
                .is_none_or(|at| at.index < pending.index.0)
            {
                return Err(Error::Raft(
                    "TTL maintenance awaits an unresolved log position".into(),
                ));
            }
            // Consuming the position is enough to permit a fresh ReadIndex and
            // rescan; it is NOT retrospectively an exact success receipt.
            work.pending = None;
        }
        // Local state may only decline work. ReadIndex below still establishes
        // authority before any observation or candidate collection.
        let (binding, _) = driver.raw_ttl_state()?;
        if binding != *range || range.sealed {
            return Err(Error::StaleEpoch {
                region: range.region,
            });
        }
        driver.read_barrier(check()?)?;
        group.write_pressure()?;
        let view = engine.snapshot()?;
        let committed = authorize(view.as_ref(), range)?;
        drop(view);
        let observed = directory
            .clock()
            .ok_or(ClockRefusal::Unavailable)
            .and_then(|clock| clock.sample())
            .and_then(|sample| {
                sample
                    .advance(committed)
                    .map_err(|_| ClockRefusal::BehindWatermark)
            });
        match observed {
            Ok(observed) if observed > committed => {
                check()?;
                group.write_pressure()?;
                let pending = driver.propose_raw_ttl_observation(range, observed, deadline)?;
                await_submission(pending, &mut work)?;
                work.observed = true;
                driver.read_barrier(check()?)?;
            }
            Ok(_) => {}
            Err(reason) => work.clock_refusal = Some(reason),
        }
        check()?;
        let view = engine.snapshot()?;
        authorize(view.as_ref(), range)?;
        let read = LeaderRead::new(view.as_ref(), true, None)?;
        let page = TtlRead::from_leader(&read)?.cleanup_page(
            range.keyspace,
            next.as_deref().unwrap_or(&range.start),
            &range.end,
            CANDIDATES,
            EXAMINED,
            CANDIDATE_BYTES,
        )?;
        work.examined = page.examined;
        work.candidate_bytes = page.candidate_bytes;
        drop(view);
        check()?;
        if !page.candidates.is_empty() {
            group.write_pressure()?;
            let count = page.candidates.len();
            let command = RawTtlCommand::new(
                range,
                RawTtlAction::Cleanup {
                    candidates: page.candidates,
                },
            )?;
            check()?;
            let pending = driver.propose_raw_ttl_command(command, deadline)?;
            await_submission(pending, &mut work)?;
            work.candidates_applied = count;
        }
        Ok(page.next)
    })();
    PageResult { work, result }
}

fn await_submission(pending: kv9_raft::driver::RawTtlSubmission, work: &mut Work) -> Result<()> {
    let at = pending.proposed_at();
    work.pending = Some(at);
    let outcome = tokio::runtime::Handle::current().block_on(pending.wait());
    // Applied rejections and replacement are definite terminal outcomes too.
    if outcome.is_ok() {
        work.pending = None;
    }
    let result = settle_proposal(at, outcome, true);
    match result {
        Ok(Some(_)) => {
            work.pending = None;
            Ok(())
        }
        Ok(None) => unreachable!("maintenance never re-proposes automatically"),
        Err(error) => {
            work.unconfirmed = work.pending.is_some();
            Err(error)
        }
    }
}

fn authorize(view: &dyn ReadView, range: &DataRange) -> Result<ClockWatermark> {
    let binding = view
        .get(ColumnFamily::Default, RANGE_KEY)?
        .map(|bytes| DataRange::decode(&bytes))
        .transpose()?;
    if binding.as_ref() != Some(range) || range.sealed {
        return Err(Error::StaleEpoch {
            region: range.region,
        });
    }
    let clock = view
        .get(ColumnFamily::Default, CLOCK_KEY)?
        .ok_or(Error::RawTtlRejected {
            region: range.region,
            reason: kv9_common::raw_ttl::TtlRejection::NotEnabled,
        })?;
    ClockWatermark::decode(&clock)
}

#[cfg(test)]
mod tests;
