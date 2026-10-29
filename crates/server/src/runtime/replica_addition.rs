//! The replica-plan driver (docs/REPLICA-ADDITION.md §2): each party derives, from
//! committed rows and the facts only it observes, the one migration verb it may take
//! next for every committed replica plan, and runs it on a worker thread — at most
//! one step in flight per process, never on the reconcile thread.
//!
//! Every step is an existing verb called in-process, so every guard those verbs
//! already have applies unchanged; this module adds no authority. A party that
//! crashes mid-step re-derives the same step from the same committed rows on its
//! next pass; the verbs are idempotent, so repeating one confirms. The install step
//! runs **inside the destination process** against its own store guard, with the
//! record from the committed capture descriptor and the in-process routed ledger.
//!
//! `KV9_REPLICA_DRIVER=0` disables the driver (the plan rows and verbs still work by
//! hand); it is on by default because a committed plan is the operator's request.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kv9_common::{Error, NodeId, Result, RootDescriptor, RootDigest};
use kv9_meta::data_groups::addition::{PlanMode, ReplicaPlan};
use kv9_raft::Role;

use crate::api::AdminApi;
use super::{NodeRuntime, RuntimeBackend};

const EVENT_HISTORY: usize = 16;
const DELIVERY_BUDGET: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub(super) enum Step {
    Attach,
    PlanImage,
    Bind,
    Capture,
    Install,
    Record,
    Promote,
    Decide,
    Remove,
}

impl Step {
    fn label(self) -> &'static str {
        match self {
            Step::Attach => "attach",
            Step::PlanImage => "plan_image",
            Step::Bind => "bind",
            Step::Capture => "capture",
            Step::Install => "install",
            Step::Record => "record_evidence",
            Step::Promote => "promote",
            Step::Decide => "decide_removal",
            Step::Remove => "remove",
        }
    }

    /// How long after an attempt (success or failure) the same step may run again.
    fn backoff(self) -> Duration {
        match self {
            Step::Capture => Duration::from_secs(10),
            Step::Install => Duration::from_secs(30),
            _ => Duration::from_secs(2),
        }
    }

    /// Steps whose success this process need not repeat (uploads, installs); the
    /// committed rows carry every other step's effect.
    fn once_per_process(self) -> bool {
        matches!(self, Step::Capture | Step::Install)
    }
}

type StepKey = ([u8; 16], Step);

#[derive(Debug, Clone, serde::Serialize)]
struct Event {
    at_unix: u64,
    operation: String,
    step: &'static str,
    outcome: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct StatusLine {
    enabled: bool,
    in_flight: Option<String>,
    events: Vec<Event>,
    error: Option<String>,
}

#[derive(Debug)]
pub(super) struct State {
    enabled: bool,
    in_flight: Option<(StepKey, std::thread::JoinHandle<Result<String>>)>,
    attempted: BTreeMap<StepKey, Instant>,
    done: BTreeSet<StepKey>,
    events: VecDeque<Event>,
    error: Option<String>,
}

impl State {
    pub(super) fn from_env() -> Self {
        Self {
            enabled: std::env::var("KV9_REPLICA_DRIVER").as_deref() != Ok("0"),
            in_flight: None,
            attempted: BTreeMap::new(),
            done: BTreeSet::new(),
            events: VecDeque::new(),
            error: None,
        }
    }

    fn record(&mut self, key: StepKey, outcome: String) {
        if self.events.len() >= EVENT_HISTORY {
            self.events.pop_front();
        }
        self.events.push_back(Event {
            at_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            operation: hex(&key.0),
            step: key.1.label(),
            outcome,
        });
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The committed facts of one plan, read from local applied metadata.
struct Facts {
    plan: ReplicaPlan,
    migration: kv9_meta::data_groups::migration::CommittedMigration,
    certified: RootDescriptor,
    capture: Option<kv9_raft::snapshot_install::capture::CaptureDescriptor>,
    pinned: bool,
    evidence: bool,
    decided: bool,
    aborted: bool,
}

impl NodeRuntime {
    /// One pass of the replica-plan driver; called from the reconcile loop.
    pub(super) fn reconcile_replica_plans(&mut self) {
        if !self.replica_plans.enabled {
            return;
        }
        // Observe the step an earlier pass started; one in flight at a time.
        if let Some((key, handle)) = self.replica_plans.in_flight.take() {
            if !handle.is_finished() {
                self.replica_plans.in_flight = Some((key, handle));
                return;
            }
            match handle.join() {
                Ok(Ok(outcome)) => {
                    if key.1.once_per_process() {
                        self.replica_plans.done.insert(key);
                    }
                    self.replica_plans.error = None;
                    self.replica_plans.record(key, outcome);
                }
                Ok(Err(error)) => {
                    self.replica_plans.error = Some(format!("{} {}: {error}", key.1.label(), hex(&key.0)));
                    self.replica_plans.record(key, format!("failed: {error}"));
                }
                Err(_) => {
                    self.replica_plans.error = Some(format!("{} worker panicked", key.1.label()));
                    self.replica_plans.record(key, "worker panicked".into());
                }
            }
        }
        let facts = match self.plan_facts() {
            Ok(facts) => facts,
            Err(error) => {
                self.replica_plans.error = Some(error.to_string());
                return;
            }
        };
        let now = Instant::now();
        for fact in facts {
            if fact.aborted {
                continue;
            }
            let Some(step) = self.next_step(&fact) else { continue };
            let key = (fact.plan.operation(), step);
            if self.replica_plans.done.contains(&key) {
                continue;
            }
            if self
                .replica_plans
                .attempted
                .get(&key)
                .is_some_and(|at| now.duration_since(*at) < step.backoff())
            {
                continue;
            }
            self.replica_plans.attempted.insert(key, now);
            let backend = self.rpc_backend.clone();
            let guard = self._store_guard.clone();
            let handle = self.grpc_runtime.handle().clone();
            let worker = std::thread::Builder::new()
                .name(format!("kv9-replica-plan-{}", step.label()))
                .spawn(move || execute(step, &backend, &guard, handle, fact))
                .expect("spawn replica plan worker");
            self.replica_plans.in_flight = Some((key, worker));
            return; // one step per pass, one in flight
        }
    }

    fn plan_facts(&self) -> Result<Vec<Facts>> {
        use kv9_meta::data_groups as dg;
        let store = &self.node.meta_raft.store;
        let plans = dg::addition::committed_plans(store)?;
        if plans.is_empty() {
            return Ok(Vec::new());
        }
        let certified = kv9_meta::root::certified_root(&store.begin()?)?
            .ok_or_else(|| Error::MetaNotReady("replica plans require a certified root".into()))?;
        let migrations = dg::migration::committed_migrations(store)?;
        let aborts = dg::abort::committed_aborts(store)?;
        let evidence = dg::evidence::committed_install_evidence(store)?;
        let removals = dg::removal::committed_removals(store)?;
        let view = store.begin()?.into_view();
        let mut out = Vec::new();
        for plan in plans {
            let operation = plan.operation();
            let Some(migration) = migrations
                .iter()
                .find(|m| m.intent().operation() == operation)
                .cloned()
            else {
                continue;
            };
            let capture = dg::capture::committed_capture(view.as_ref(), &certified, operation)?
                .map(|c| c.descriptor().clone());
            let (source_owner, _) = crate::migration_retention::migration_owner_ids(
                certified.digest(),
                operation,
                &migration.intent().destination().incarnation,
            )?;
            let pinned = kv9_meta::retention::retention_owner(view.as_ref(), &certified, source_owner)?
                .is_some_and(|o| o.phase == kv9_common::retention::PinPhase::Published);
            out.push(Facts {
                aborted: aborts.iter().any(|a| a.operation() == operation),
                evidence: evidence.iter().any(|e| e.operation() == operation),
                decided: removals.iter().any(|r| r.operation() == operation),
                pinned,
                capture,
                certified: certified.clone(),
                migration,
                plan,
            });
        }
        Ok(out)
    }

    /// The step table of docs/REPLICA-ADDITION.md §2, for the roles this process holds.
    fn next_step(&self, f: &Facts) -> Option<Step> {
        let region = f.migration.intent().region();
        let destination = f.migration.intent().destination();
        let meta_leader = self.driver.status().role == Role::Leader;
        let group = self
            .rpc_backend
            .raw_directory
            .by_region(region)
            .map(|g| g.capture_parts().0.status())
            .filter(|s| s.role == Role::Leader && s.fatal.is_none());
        let is_destination = destination.node == self.node.id
            && destination.incarnation == self.store_identity.store_incarnation;
        let adopted = self
            .rpc_backend
            .adoption_receipts
            .lock()
            .expect("receipts poisoned")
            .contains_key(&region);
        let is_move = f.plan.mode() == PlanMode::Move;

        if let Some(status) = &group {
            let learner = status.learners.contains(&destination.node.0);
            let voter = status.voters.contains(&destination.node.0);
            if !learner && !voter {
                return Some(Step::Attach);
            }
            if learner && f.capture.is_none() {
                return Some(Step::PlanImage);
            }
            if f.pinned && !f.evidence {
                return Some(Step::Capture);
            }
            if f.evidence && learner && !voter {
                return Some(Step::Promote);
            }
            if is_move && f.decided && voter {
                if let Some(source) = f.plan.source() {
                    if status.voters.contains(&source.node.0) {
                        return Some(Step::Remove);
                    }
                }
            }
        }
        if meta_leader {
            if f.capture.is_some() && !f.pinned {
                return Some(Step::Bind);
            }
            if is_move && f.evidence && !f.decided {
                return Some(Step::Decide);
            }
        }
        if is_destination {
            if f.pinned && !f.evidence && !adopted {
                return Some(Step::Install);
            }
            if adopted && !f.evidence {
                return Some(Step::Record);
            }
        }
        None
    }

    /// The `replica_plans=` status line.
    pub(super) fn replica_plans_status(&self) -> String {
        let state = &self.replica_plans;
        serde_json::to_string(&StatusLine {
            enabled: state.enabled,
            in_flight: state
                .in_flight
                .as_ref()
                .map(|(key, _)| format!("{} {}", key.1.label(), hex(&key.0))),
            events: state.events.iter().cloned().collect(),
            error: state.error.clone(),
        })
        .expect("serialize replica plan status")
    }
}

const CALLER: &str = "replica-plan";

/// Run one step with the existing verbs. Blocking; runs on the worker thread.
fn execute(
    step: Step,
    backend: &Arc<RuntimeBackend>,
    guard: &kv9_common::store_lifecycle::StoreGuard,
    handle: tokio::runtime::Handle,
    f: Facts,
) -> Result<String> {
    // The verbs assume a Tokio context (the public path runs them under
    // spawn_blocking inside the gRPC runtime); give this worker thread the same.
    let _runtime = handle.enter();
    let root = f.certified.digest();
    let operation = f.plan.operation();
    match step {
        Step::Attach => {
            let r = backend.attach_migration_learner(CALLER, root, operation)?;
            Ok(format!(
                "{} learner {} at cut {}/{}",
                if r.changed { "attached" } else { "confirmed" },
                r.destination.0,
                r.cut.term,
                r.cut.index
            ))
        }
        Step::PlanImage => {
            let r = backend.plan_migration_image(CALLER, root, operation)?;
            Ok(format!("capture planned at cut {}/{}", r.cut.term, r.cut.index))
        }
        Step::Bind => {
            let descriptor = f
                .capture
                .as_ref()
                .ok_or_else(|| Error::MetaNotReady("bind needs the committed capture".into()))?;
            let manifest = descriptor.manifest().encode()?;
            let r = backend.bind_migration_image(CALLER, root, operation, &manifest)?;
            Ok(format!(
                "pins published: source {} destination {}",
                hex(r.source_owner.as_bytes()),
                hex(r.destination_owner.as_bytes())
            ))
        }
        Step::Capture => {
            let r = backend.capture_migration_image(CALLER, root, operation)?;
            Ok(format!(
                "image {} uploaded: {} objects, {} bytes, cut {}/{}",
                r.image_digest, r.objects, r.object_bytes, r.cut.term, r.cut.index
            ))
        }
        Step::Install => install(backend, guard, handle, &f),
        Step::Record => {
            let region = f.migration.intent().region();
            let emitted = backend.emit_install_evidence(CALLER, root, region)?;
            // The metadata leader may be this very process.
            if backend.driver.status().role == Role::Leader {
                if let Ok(r) = AdminApi::record_install_evidence(backend.as_ref(), CALLER, root, &emitted.receipt) {
                    return Ok(format!(
                        "evidence task {} {} locally",
                        r.evidence.task(),
                        if r.changed { "committed" } else { "confirmed" }
                    ));
                }
            }
            deliver_evidence(backend, handle, root, &emitted.receipt)
        }
        Step::Promote => {
            let r = backend.promote_migration_voter(CALLER, root, operation)?;
            Ok(format!(
                "{} voter {}; voters {:?}",
                if r.changed { "promoted" } else { "confirmed" },
                r.destination.0,
                r.voters
            ))
        }
        Step::Decide => {
            let source = f
                .plan
                .source()
                .ok_or_else(|| Error::Config("an addition has no source to retire".into()))?;
            let r = backend.record_source_removal(CALLER, root, operation, source)?;
            Ok(format!(
                "removal of node {} {} as task {}",
                source.node.0,
                if r.changed { "decided" } else { "confirmed" },
                r.decision.task()
            ))
        }
        Step::Remove => {
            let r = backend.remove_source_replica(CALLER, root, operation)?;
            Ok(format!(
                "{} node {}; voters {:?}",
                if r.changed { "removed" } else { "confirmed" },
                r.removed.0,
                r.voters
            ))
        }
    }
}

/// Online install at the destination: the committed capture descriptor carries the
/// installer's record; the pins are read through the in-process routed ledger; the
/// joint installer runs against this process's own store guard. Adoption follows on
/// the ordinary reconcile pass.
fn install(
    backend: &Arc<RuntimeBackend>,
    guard: &kv9_common::store_lifecycle::StoreGuard,
    handle: tokio::runtime::Handle,
    f: &Facts,
) -> Result<String> {
    let descriptor = f
        .capture
        .as_ref()
        .ok_or_else(|| Error::MetaNotReady("install needs the committed capture".into()))?;
    let uploader = backend
        .uploader
        .as_deref()
        .ok_or_else(|| Error::Config("install requires configured object storage".into()))?;
    let identity = backend.ttl_context()?.local.identity();
    // The capture descriptor is committed before the group leader uploads the
    // image; installing an incomplete image would burn a retained generation
    // for nothing. Confirm every object first and retry later otherwise.
    let manifest = kv9_raft::snapshot_install::captured_record_manifest(&descriptor.image().record)?;
    uploader
        .verify_checkpoint_objects(&manifest)
        .map_err(|error| Error::MetaNotReady(format!("image not fully uploaded yet: {error}")))?;
    let (source_owner, destination_owner) = crate::migration_retention::migration_owner_ids(
        f.certified.digest(),
        f.plan.operation(),
        &f.migration.intent().destination().incarnation,
    )?;
    let ledger = super::checkpoint_retention::RoutedLedger::new(backend, handle);
    let reader = crate::installation_retention::RetainedImages::new(
        uploader,
        &ledger,
        &f.certified,
        &[source_owner, destination_owner],
    )?;
    let installed = kv9_raft::snapshot_install::install_captured_record_with_reader(
        guard,
        identity,
        &descriptor.image().record,
        &reader,
    )?;
    Ok(format!(
        "installed generation {} image {} cut {}/{} ({} records)",
        installed.generation, installed.image_digest, installed.position.term, installed.position.index, installed.records
    ))
}

/// Walk the metadata candidates with the node-authenticated internal service until
/// one commits the receipt; `NotLeader` moves on, anything else is final.
fn deliver_evidence(
    backend: &Arc<RuntimeBackend>,
    handle: tokio::runtime::Handle,
    root: RootDigest,
    receipt: &[u8],
) -> Result<String> {
    let context = backend.ttl_context()?;
    let me: NodeId = backend.node.id;
    let candidates = backend.ttl_metadata_candidates();
    handle.block_on(async {
        let mut last = Error::MetaNotReady("no metadata candidate answered".into());
        for address in candidates {
            let deadline = Instant::now() + DELIVERY_BUDGET;
            let channel = match context.channel(address, deadline).await {
                Ok(channel) => channel,
                Err(error) => {
                    last = error;
                    continue;
                }
            };
            match kv9_raft::grpc::install_evidence::deliver_on_channel(
                channel,
                root,
                receipt,
                me,
                Some(context.token.clone()),
                DELIVERY_BUDGET,
            )
            .await
            {
                Ok((task, changed, applied)) => {
                    return Ok(format!(
                        "evidence task {task} {} at {address} (applied {}/{})",
                        if changed { "committed" } else { "confirmed" },
                        applied.term,
                        applied.index
                    ))
                }
                Err(Error::NotLeader { .. }) | Err(Error::MetaNotReady(_)) => {
                    last = Error::MetaNotReady(format!("{address}: not the leader or unavailable"));
                }
                Err(error) => return Err(error),
            }
        }
        Err(last)
    })
}
