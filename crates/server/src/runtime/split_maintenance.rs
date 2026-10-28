//! Continue committed splits independently of new-trigger policy. One actual
//! blocking job owns one shared heavy permit; control ticks never wait on it.
use super::*;
use std::sync::atomic::AtomicU64;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const TICK: Duration = Duration::from_millis(100);
const VISITS: usize = 32;

#[derive(Clone, Default, serde::Serialize)]
pub(super) struct Stats {
    pub running: bool,
    pub allowed: bool,
    pub trigger_bytes: u64,
    pub in_flight: bool,
    pub passes: u64,
    pub seals: u64,
    pub populations: u64,
    pub publications: u64,
    pub triggers: u64,
    pub failures: u64,
    pub last_error: Option<String>,
}
#[derive(Clone, Copy, Default)]
struct Cursor {
    intent: Option<(u64, u8)>,
    trigger: Option<u64>,
}
#[derive(Clone, Copy)]
enum Step {
    Idle,
    Seal,
    Populate,
    Publish,
    Trigger,
}

pub(super) struct Maintenance {
    cancel: CancellationToken,
    allowed: Arc<AtomicBool>,
    threshold: Arc<AtomicU64>,
    stats: Arc<Mutex<Stats>>,
    task: Option<JoinHandle<()>>,
}
impl Maintenance {
    pub(super) fn start(
        backend: Arc<RuntimeBackend>,
        threshold: u64,
        runtime: &tokio::runtime::Handle,
    ) -> Self {
        let cancel = CancellationToken::new();
        let allowed = Arc::new(AtomicBool::new(false));
        let threshold = Arc::new(AtomicU64::new(threshold));
        let stats = Arc::new(Mutex::new(Stats::default()));
        let task = runtime.spawn(run(
            backend,
            cancel.clone(),
            allowed.clone(),
            threshold.clone(),
            stats.clone(),
        ));
        Self {
            cancel,
            allowed,
            threshold,
            stats,
            task: Some(task),
        }
    }
    pub(super) fn configure(&self, allowed: bool, threshold: u64) {
        self.threshold.store(threshold, Ordering::Release);
        self.allowed.store(allowed, Ordering::Release);
    }
    pub(super) fn snapshot(&self) -> Stats {
        self.stats.lock().expect("split stats").clone()
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

async fn run(
    backend: Arc<RuntimeBackend>,
    cancel: CancellationToken,
    allowed: Arc<AtomicBool>,
    threshold: Arc<AtomicU64>,
    shared: Arc<Mutex<Stats>>,
) {
    let mut lane = backend.maintenance.lane(crate::maintenance::Class::Split);
    let mut job: Option<JoinHandle<(Cursor, Result<Step>)>> = None;
    let mut cursor = Cursor::default();
    let mut stats = Stats::default();
    let mut failures = 0u32;
    let mut due = Instant::now();
    let mut fatal = false;
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        if job.as_ref().is_some_and(JoinHandle::is_finished) {
            match job.take().unwrap().await {
                Ok((next, result)) => {
                    cursor = next; // Advance even after a refusal; another intent gets a turn.
                    stats.passes = stats.passes.saturating_add(1);
                    match result {
                        Ok(step) => {
                            failures = 0;
                            stats.last_error = None;
                            match step {
                                Step::Idle => {}
                                Step::Seal => stats.seals = stats.seals.saturating_add(1),
                                Step::Populate => {
                                    stats.populations = stats.populations.saturating_add(1)
                                }
                                Step::Publish => {
                                    stats.publications = stats.publications.saturating_add(1)
                                }
                                Step::Trigger => stats.triggers = stats.triggers.saturating_add(1),
                            }
                            due = Instant::now() + TICK;
                        }
                        Err(error) => {
                            failures = failures.saturating_add(1);
                            stats.failures = stats.failures.saturating_add(1);
                            stats.last_error = Some(error.to_string().chars().take(192).collect());
                            due = Instant::now() + Duration::from_millis(100 << failures.min(5));
                        }
                    }
                }
                Err(error) => {
                    fatal = true;
                    stats.failures = stats.failures.saturating_add(1);
                    stats.last_error = Some(
                        format!("split worker failed: {error}")
                            .chars()
                            .take(192)
                            .collect(),
                    );
                }
            }
        }
        let enabled = allowed.load(Ordering::Acquire) && !cancel.is_cancelled() && !fatal;
        let mut waiting = false;
        if enabled && job.is_none() && Instant::now() >= due {
            if let Some(permit) = lane.try_acquire() {
                let backend = backend.clone();
                let cancel = cancel.clone();
                let allowed = allowed.clone();
                let threshold = threshold.clone();
                job = Some(tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    let mut next = cursor;
                    let result = if cancel.is_cancelled() || !allowed.load(Ordering::Acquire) {
                        Ok(Step::Idle)
                    } else {
                        backend.split_maintenance_step(&mut next, threshold.load(Ordering::Acquire))
                    };
                    (next, result)
                }));
            } else {
                waiting = true;
            }
        }
        if !waiting {
            lane.withdraw();
        }
        stats.running = !cancel.is_cancelled() && !fatal;
        stats.allowed = enabled;
        stats.trigger_bytes = threshold.load(Ordering::Acquire);
        stats.in_flight = job.is_some();
        *shared.lock().expect("split stats") = stats.clone();
        if (cancel.is_cancelled() || fatal) && job.is_none() {
            return;
        }
        tick.tick().await;
    }
}

impl RuntimeBackend {
    fn split_maintenance_step(&self, cursor: &mut Cursor, threshold: u64) -> Result<Step> {
        self.ensure_serving()?;
        let store = &self.node.meta_raft.store;
        let root = store
            .begin()
            .and_then(|txn| kv9_meta::root::certified_root(&txn))?
            .ok_or_else(|| Error::MetaNotReady("split maintenance needs a certified root".into()))?
            .digest();
        let mut bindings = kv9_meta::data_groups::ranges::committed_ranges(store)?;
        bindings.sort_by_key(|b| b.range().region);
        let mut intents = kv9_meta::data_groups::split::committed_splits(store)?;
        intents.sort_by_key(|i| i.task());
        let after = cursor.intent;
        let candidates = || {
            intents
                .iter()
                .flat_map(|i| (0..4).map(move |stage| (i, stage)))
        };
        // Control state is an ephemeral fairness cursor. Protocol progress is
        // always re-read from committed metadata and each group's actual state.
        for (intent, stage) in candidates()
            .filter(|(i, s)| after.is_none_or(|a| (i.task(), *s) > a))
            .chain(candidates().filter(|(i, s)| after.is_some_and(|a| (i.task(), *s) <= a)))
            .take(VISITS)
        {
            cursor.intent = Some((intent.task(), stage));
            if bindings
                .iter()
                .any(|b| b.range().region == intent.parent_region() && b.range().sealed)
            {
                continue;
            }
            let parent = {
                self.group_handles
                    .lock()
                    .expect("group handles")
                    .get(&intent.parent_region())
                    .cloned()
            };
            let Some((engine, driver)) = parent else {
                continue;
            };
            let range = engine
                .get(
                    kv9_engine::ColumnFamily::Default,
                    kv9_common::data_range::RANGE_KEY,
                )?
                .map(|b| kv9_common::data_range::DataRange::decode(&b))
                .transpose()?;
            let Some(range) = range else { continue };
            match stage {
                0 if !range.sealed && driver.status().role == Role::Leader => {
                    self.seal_split_parent("split-maintenance", root, intent.operation())?;
                    return Ok(Step::Seal);
                }
                1 | 2 if range.sealed => {
                    let high = stage == 2;
                    let task = if high {
                        intent.child_high()
                    } else {
                        intent.child_low()
                    };
                    let Some(creation) = kv9_meta::data_groups::committed_creation(store, task)?
                    else {
                        continue;
                    };
                    let child = {
                        self.group_handles
                            .lock()
                            .expect("group handles")
                            .get(&creation.intent().region())
                            .cloned()
                    };
                    let Some((engine, driver)) = child else {
                        continue;
                    };
                    if driver.status().role != Role::Leader {
                        continue;
                    }
                    if engine
                        .get(
                            kv9_engine::ColumnFamily::Default,
                            kv9_common::data_range::RANGE_KEY,
                        )?
                        .is_some()
                    {
                        continue;
                    }
                    self.populate_split_child("split-maintenance", root, intent.operation(), high)?;
                    return Ok(Step::Populate);
                }
                3 if range.sealed && self.driver.status().role == Role::Leader => {
                    self.publish_split("split-maintenance", root, intent.operation())?;
                    return Ok(Step::Publish);
                }
                _ => {}
            }
        }
        // Zero affects only brand-new triggers. It cannot cancel an intent or
        // stop the durable seal/populate/publish protocol above.
        if threshold == 0 || self.driver.status().role != Role::Leader {
            return Ok(Step::Idle);
        }
        let after = cursor.trigger;
        for binding in bindings
            .iter()
            .filter(|b| after.is_none_or(|a| b.range().region.0 > a))
            .chain(
                bindings
                    .iter()
                    .filter(|b| after.is_some_and(|a| b.range().region.0 <= a)),
            )
            .take(VISITS)
        {
            let range = binding.range();
            cursor.trigger = Some(range.region.0);
            if range.sealed || intents.iter().any(|i| i.parent_region() == range.region) {
                continue;
            }
            let handle = {
                self.group_handles
                    .lock()
                    .expect("group handles")
                    .get(&range.region)
                    .cloned()
            };
            let Some((engine, _)) = handle else { continue };
            if NodeRuntime::engine_disk_bytes(&engine.path()) < threshold {
                continue;
            }
            let Some(split_key) = NodeRuntime::sampled_median_key(engine.as_ref(), range) else {
                continue;
            };
            let derive = |operation: [u8; 16], salt: &[u8]| {
                let mut bytes = b"kv9-auto-split-v1".to_vec();
                bytes.extend_from_slice(root.as_bytes());
                bytes.extend_from_slice(&operation);
                bytes.extend_from_slice(salt);
                RootDigest::sha256(&bytes).as_bytes()[..16]
                    .try_into()
                    .expect("16 bytes")
            };
            let operation = derive([0; 16], &range.region.0.to_be_bytes());
            let voters = binding
                .creation()
                .intent()
                .replicas()
                .iter()
                .map(|r| r.node)
                .collect::<Vec<_>>();
            let low =
                self.create_data_group("auto-split", root, derive(operation, b"low"), &voters)?;
            let high =
                self.create_data_group("auto-split", root, derive(operation, b"high"), &voters)?;
            self.record_split_intent(
                "auto-split",
                root,
                operation,
                range.region,
                &split_key,
                low.intent.task(),
                high.intent.task(),
            )?;
            return Ok(Step::Trigger);
        }
        Ok(Step::Idle)
    }
}
