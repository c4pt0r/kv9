//! Bounded per-node checkpoint scheduling over the actual data-group directory.
//! Persistent plans and reservations survive failed passes. Execution state is
//! kept once per live driver; retirement removes it only after the job exits.
use super::range_api::RawGroup;
use super::*;
use kv9_engine::checkpoint::RemoteUploader;
use std::sync::Weak;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
mod group;
pub(super) use group::owner_matches;
mod worker;
use group::Group;
use worker::{Context, Worker};
const WORKERS: usize = 2;
const TRACKED: usize = 255;
const VISITS: usize = 32;
const TICK: Duration = Duration::from_millis(50);
const PASS: Duration = Duration::from_secs(30);
type Driver = NodeDriver<DiskRaftStorage, WalEngine>;
fn invalid(message: &str) -> Error {
    Error::Engine(format!("data checkpoint: {message}"))
}

pub(super) fn publish_from_env() -> Result<bool> {
    match std::env::var("KV9_DATA_CHECKPOINTS") {
        Ok(value) if value == "0" => Ok(false),
        Ok(value) if value == "1" => Ok(true),
        Err(std::env::VarError::NotPresent) => Ok(true),
        _ => Err(Error::Config("KV9_DATA_CHECKPOINTS must be 0 or 1".into())),
    }
}
#[derive(Clone, Default, serde::Serialize)]
pub(super) struct Stats {
    pub running: bool,
    pub publish_new: bool,
    pub in_flight: usize,
    pub peak_in_flight: usize,
    pub tracked_groups: usize,
    pub plans: u64,
    pub settled_positive: u64,
    pub adoptions: u64,
    pub unknown: u64,
    pub refusals: u64,
    pub failures: u64,
    pub retired_states: u64,
    pub last_error: Option<String>,
}
#[derive(Default)]
struct Work {
    planned: u64,
    settled_positive: u64,
    adopted: u64,
    unknown: u64,
    refused: u64,
}
pub(super) struct Maintenance {
    cancel: CancellationToken,
    task: Option<JoinHandle<()>>,
    stats: Arc<Mutex<Stats>>,
}
impl Maintenance {
    pub(super) fn start(
        backend: Arc<RuntimeBackend>,
        uploader: Arc<RemoteUploader>,
        publish_new: bool,
        interval: Duration,
        runtime: &tokio::runtime::Handle,
    ) -> Self {
        let cancel = CancellationToken::new();
        let stats = Arc::new(Mutex::new(Stats::default()));
        let task = runtime.spawn(run(
            backend,
            uploader,
            publish_new,
            interval,
            cancel.clone(),
            stats.clone(),
        ));
        Self {
            cancel,
            task: Some(task),
            stats,
        }
    }
    pub(super) fn snapshot(&self) -> Stats {
        self.stats.lock().expect("checkpoint stats").clone()
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
    owner: Weak<Driver>,
    engine: Weak<WalEngine>,
    worker: Option<Worker>,
    due: Instant,
    failures: u32,
}
impl State {
    fn matches(&self, group: &Group) -> bool {
        self.owner
            .upgrade()
            .is_some_and(|d| Arc::ptr_eq(&d, group.capture_parts().0))
            && self
                .engine
                .upgrade()
                .is_some_and(|e| Arc::ptr_eq(&e, group.capture_parts().1))
    }
}
struct Flight {
    group: Arc<Group>,
    job: JoinHandle<(State, Work, Result<()>)>,
}
async fn run(
    backend: Arc<RuntimeBackend>,
    uploader: Arc<RemoteUploader>,
    publish_new: bool,
    interval: Duration,
    cancel: CancellationToken,
    shared: Arc<Mutex<Stats>>,
) {
    let mut states = BTreeMap::<RegionId, State>::new();
    let mut flights = Vec::<Flight>::new();
    let mut after = None;
    let mut fatal = false;
    let mut lane = backend
        .maintenance
        .lane(crate::maintenance::Class::DataCheckpoint);
    let mut stats = Stats {
        running: true,
        publish_new,
        ..Stats::default()
    };
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        let mut i = 0;
        while i < flights.len() {
            if !flights[i].job.is_finished() {
                i += 1;
                continue;
            }
            let Flight { group, job } = flights.swap_remove(i);
            match job.await {
                Ok((mut state, work, result)) => {
                    stats.plans = stats.plans.saturating_add(work.planned);
                    stats.settled_positive =
                        stats.settled_positive.saturating_add(work.settled_positive);
                    stats.adoptions = stats.adoptions.saturating_add(work.adopted);
                    stats.unknown = stats.unknown.saturating_add(work.unknown);
                    stats.refusals = stats.refusals.saturating_add(work.refused);
                    match result {
                        Ok(()) => {
                            state.failures = 0;
                            state.due = Instant::now() + interval;
                        }
                        Err(error) => {
                            if state.worker.is_none() {
                                fatal = true;
                            }
                            state.failures = state.failures.saturating_add(1);
                            state.due = Instant::now()
                                + Duration::from_millis(
                                    250_u64.saturating_mul(1 << state.failures.min(7)),
                                )
                                .min(Duration::from_secs(30));
                            stats.failures = stats.failures.saturating_add(1);
                            stats.last_error = Some(error.to_string().chars().take(160).collect());
                        }
                    }
                    if group::owner_matches(
                        &backend,
                        group.target().region,
                        group.capture_parts().0,
                        group.capture_parts().1,
                    ) {
                        states.insert(group.target().region, state);
                    } else {
                        stats.retired_states = stats.retired_states.saturating_add(1);
                    }
                }
                Err(error) => {
                    // The unique seam/candidate may have been unwound. Never
                    // remint it against a still-live driver after a panic.
                    fatal = true;
                    stats.failures = stats.failures.saturating_add(1);
                    stats.last_error = Some(
                        format!("checkpoint worker: {error}")
                            .chars()
                            .take(160)
                            .collect(),
                    );
                }
            }
        }
        states.retain(|region, state| {
            let keep = state
                .owner
                .upgrade()
                .zip(state.engine.upgrade())
                .is_some_and(|(driver, engine)| {
                    group::owner_matches(&backend, *region, &driver, &engine)
                });
            if !keep {
                stats.retired_states = stats.retired_states.saturating_add(1);
            }
            keep
        });
        let mut waiting_for_budget = false;
        if !fatal && !cancel.is_cancelled() {
            let mut first = None;
            for _ in 0..VISITS {
                if flights.len() == WORKERS {
                    break;
                }
                let Some((region, candidate)) = group::next(&backend, after) else {
                    break;
                };
                if first == Some(region) {
                    break;
                }
                first.get_or_insert(region);
                after = Some(region);
                let group = match candidate {
                    Ok(Some(group)) => group,
                    Ok(None) => continue,
                    Err(error) => {
                        stats.failures = stats.failures.saturating_add(1);
                        stats.last_error = Some(error.to_string().chars().take(160).collect());
                        continue;
                    }
                };
                if flights.iter().any(|f| f.group.target().region == region) {
                    continue;
                }
                if states.get(&region).is_some_and(|s| s.due > Instant::now()) {
                    continue;
                }
                let Some(permit) = lane.try_acquire() else {
                    waiting_for_budget = true;
                    break;
                };
                let state = match states.remove(&region) {
                    Some(s) if s.matches(&group) => s,
                    _ => {
                        if states.len() + flights.len() >= TRACKED {
                            break;
                        }
                        State {
                            owner: Arc::downgrade(group.capture_parts().0),
                            engine: Arc::downgrade(group.capture_parts().1),
                            worker: None,
                            due: Instant::now(),
                            failures: 0,
                        }
                    }
                };
                let backend = backend.clone();
                let uploader = uploader.clone();
                let cancel = cancel.clone();
                let job_group = group.clone();
                let job = tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    let mut state = state;
                    let mut work = Work::default();
                    let executed = uploader.with_local_execution_report(|uploader| {
                        if state.worker.is_none() {
                            state.worker = Some(Worker::new(&job_group, uploader)?);
                        }
                        let root = backend.ttl_context()?.root.clone();
                        let deadline = Instant::now() + PASS;
                        let ledger = super::checkpoint_retention::RoutedLedger::until(
                            &backend,
                            tokio::runtime::Handle::current(),
                            deadline,
                        );
                        let context = Context {
                            backend: &backend,
                            group: &job_group,
                            root: &root,
                            uploader,
                            ledger,
                            publish_new,
                            cancel: &cancel,
                            deadline,
                        };
                        state.worker.as_mut().unwrap().tick(&context, &mut work)
                    });
                    let result = match executed {
                        Ok((result, report)) => state
                            .worker
                            .as_mut()
                            .map_or(Ok(()), |worker| worker.observe_drain(report))
                            .and(result),
                        Err(error) => Err(error),
                    };
                    // Scope return proves local queue/future drain, including
                    // caller timeout. Ledger uncertainty retains the executor.
                    let close = state.worker.as_mut().map_or(Ok(()), |worker| {
                        worker.close_execution(&super::checkpoint_retention::RoutedLedger::new(
                            &backend,
                            tokio::runtime::Handle::current(),
                        ))
                    });
                    let result = close.and(result);
                    (state, work, result)
                });
                flights.push(Flight { group, job });
                stats.peak_in_flight = stats.peak_in_flight.max(flights.len());
            }
        }
        if !waiting_for_budget {
            lane.withdraw();
        }
        stats.in_flight = flights.len();
        stats.tracked_groups = states.len() + flights.len();
        stats.running = !fatal && !cancel.is_cancelled();
        *shared.lock().expect("checkpoint stats") = stats.clone();
        if (fatal || cancel.is_cancelled()) && flights.is_empty() {
            return;
        }
        tick.tick().await;
    }
}
