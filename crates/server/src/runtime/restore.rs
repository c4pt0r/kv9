//! Restore of a backup into a new keyspace (docs/BACKUP-RESTORE.md §5). A
//! committed kind-123 intent names the targets; from then on:
//!
//! - **each target's leader** proposes the target's descriptor (the reserved
//!   keyspace over the range's boundaries) once per term, then copies its range
//!   from the anchored image — restored under the image budget — through the
//!   target's own log in pages, digests the target row by row, and commits the
//!   evidence (kind 124) through the shared evidence path; the metadata leader
//!   refuses a digest that differs from the anchor's (B5);
//! - **the metadata leader** publishes the keyspace, its bindings and routing rows
//!   in one transaction once every target is evidenced (B4).
//!
//! Nothing here touches the source keyspace or the backup (B3). An aborted
//! restore's targets retire through the retirement pass like an aborted split's
//! children (B6). `KV9_RANGE_DRIVER=0` pauses this driver too.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kv9_common::data_range::DataRange;
use kv9_common::{Error, KeyspaceId, RegionId, Result, RootDigest};
use kv9_engine::checkpoint::CheckpointManifest;
use kv9_engine::Engine;
use kv9_meta::data_groups::restore::{committed_restores, target_descriptor, RestoreEvidence, RestoreIntent, RestoreStage, RestoreTarget};
use kv9_raft::{Command, Role};
use kv9_raw::{LeaderRead, RawExecutor};

use super::range_api::RawGroup;
use super::range_split::{applied_descriptor, deliver, digest_half, printable, range_driver_enabled, PAGE};
use super::{NodeRuntime, RuntimeBackend};
use crate::api::{RawApi, RequestContext, RequestOrigin};

const EVENT_HISTORY: usize = 24;

#[derive(Debug, Clone, serde::Serialize)]
struct Event {
    at_unix: u64,
    restore: u64,
    region: u64,
    step: &'static str,
    outcome: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct TargetView {
    region: u64,
    source_region: u64,
    /// `-`, `descriptor`, `copied`, `evidenced`.
    state: &'static str,
    lead: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
struct RestoreView {
    task: u64,
    backup: u64,
    name: String,
    keyspace: u32,
    stage: &'static str,
    targets: Vec<TargetView>,
    waiting: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct StatusLine {
    restores: Vec<RestoreView>,
    in_flight: Option<String>,
    events: Vec<Event>,
    error: Option<String>,
}

#[derive(Debug, Default)]
pub(super) struct State {
    views: Vec<RestoreView>,
    /// Target region → the term its descriptor was proposed in on this node.
    descriptor_proposals: BTreeMap<u64, u64>,
    /// Target region → copied and digested on this node (evidence on its way).
    copied: BTreeMap<u64, u64>,
    attempted: BTreeMap<(u64, u64, &'static str), Instant>,
    in_flight: Option<(u64, u64, &'static str, String, std::thread::JoinHandle<Result<String>>)>,
    events: VecDeque<Event>,
    error: Option<String>,
}

impl State {
    fn record(&mut self, restore: u64, region: u64, step: &'static str, outcome: String) {
        if self.events.len() >= EVENT_HISTORY {
            self.events.pop_front();
        }
        self.events.push_back(Event {
            at_unix: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
            restore,
            region,
            step,
            outcome,
        });
    }
}

enum Step {
    Populate {
        intent: RestoreIntent,
        target: RestoreTarget,
        source_keyspace: KeyspaceId,
        manifest: CheckpointManifest,
        descriptor: DataRange,
        group: Arc<RawGroup>,
    },
    Publish {
        root: RootDigest,
        operation: [u8; 16],
    },
}

impl Step {
    fn name(&self) -> &'static str {
        match self {
            Step::Populate { .. } => "populate",
            Step::Publish { .. } => "publish",
        }
    }
    fn backoff(&self) -> Duration {
        match self {
            Step::Populate { .. } => Duration::from_secs(10),
            Step::Publish { .. } => Duration::from_secs(3),
        }
    }
    fn label(&self) -> String {
        match self {
            Step::Populate { target, .. } => format!("region {} from region {}", target.region().0, target.source_region().0),
            Step::Publish { operation, .. } => format!("operation {}", hex(&operation[..4])),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl NodeRuntime {
    pub(super) fn reconcile_restore(&mut self) {
        if let Some((restore, region, step, label, handle)) = self.restore.in_flight.take() {
            if !handle.is_finished() {
                self.restore.in_flight = Some((restore, region, step, label, handle));
                return;
            }
            match handle.join() {
                Ok(Ok(outcome)) => {
                    self.restore.error = None;
                    if step == "populate" {
                        self.restore.copied.insert(region, restore);
                    }
                    self.restore.record(restore, region, step, format!("{label}: {outcome}"));
                }
                Ok(Err(error)) => {
                    self.restore.error = Some(format!("{step} {label}: {error}"));
                    self.restore.record(restore, region, step, format!("{label}: failed: {error}"));
                }
                Err(_) => {
                    self.restore.error = Some(format!("{step} worker panicked"));
                    self.restore.record(restore, region, step, format!("{label}: worker panicked"));
                }
            }
        }
        if !range_driver_enabled() {
            return;
        }
        match self.plan_restore_steps() {
            Ok((views, step)) => {
                self.restore.views = views;
                if let Some((restore, region, step)) = step {
                    let key = (restore, region, step.name());
                    let now = Instant::now();
                    if self.restore.attempted.get(&key).is_some_and(|at| now.duration_since(*at) < step.backoff()) {
                        return;
                    }
                    self.restore.attempted.insert(key, now);
                    self.start_restore_step(restore, region, step);
                }
            }
            Err(Error::MetaNotReady(_)) => {}
            Err(error) => self.restore.error = Some(error.to_string()),
        }
    }

    #[allow(clippy::type_complexity)]
    fn plan_restore_steps(&mut self) -> Result<(Vec<RestoreView>, Option<(u64, u64, Step)>)> {
        use kv9_meta::data_groups as dg;
        let store = &self.node.meta_raft.store;
        let rows = committed_restores(store)?;
        let live: Vec<RestoreIntent> = rows.intents.iter().filter(|i| rows.stage(i).live()).cloned().collect();
        if live.is_empty() {
            self.restore.copied.clear();
            return Ok((Vec::new(), None));
        }
        let txn = store.begin()?;
        let certified = kv9_meta::root::certified_root(&txn)?
            .ok_or_else(|| Error::MetaNotReady("restore requires a certified root".into()))?;
        let backups = dg::backup::committed_backups_in_txn(&txn)?;
        let view = txn.into_view();
        let meta_leader = self.driver.status().role == Role::Leader;
        let mut views = Vec::new();
        let mut step: Option<(u64, u64, Step)> = None;
        for intent in &live {
            let stage = rows.stage(intent);
            let mut out = RestoreView {
                task: intent.task(),
                backup: intent.backup_task(),
                name: intent.name().to_owned(),
                keyspace: intent.keyspace().0,
                stage: stage.label(),
                targets: Vec::new(),
                waiting: None,
            };
            if stage == RestoreStage::Populated {
                if meta_leader && step.is_none() {
                    step = Some((intent.task(), 0, Step::Publish { root: intent.root(), operation: intent.operation() }));
                } else if !meta_leader {
                    out.waiting = Some("waiting for the metadata leader to publish".into());
                }
                for t in intent.targets() {
                    out.targets.push(TargetView { region: t.region().0, source_region: t.source_region().0, state: "evidenced", lead: false });
                }
                views.push(out);
                continue;
            }
            let Some(backup) = backups.intent_by_operation(intent.backup_operation()) else {
                out.waiting = Some("the backup intent is not applied here yet".into());
                views.push(out);
                continue;
            };
            for target in intent.targets() {
                let evidenced = rows.evidence_of(intent, target).is_some();
                let local = self.data_groups.local_parts(target.region());
                let lead = local.as_ref().is_some_and(|(_, _, d)| d.status().role == Role::Leader);
                let mut state = if evidenced { "evidenced" } else { "-" };
                if !evidenced {
                    if let Some((creation, engine, driver)) = &local {
                        let descriptor = target_descriptor(intent, target, creation);
                        let applied = applied_descriptor(engine)?;
                        if applied.is_some() {
                            state = "descriptor";
                        }
                        if self.restore.copied.get(&target.region().0) == Some(&intent.task()) {
                            state = "copied";
                        }
                        if lead {
                            let status = driver.status();
                            if applied.is_none() && self.restore.descriptor_proposals.get(&target.region().0) != Some(&status.term) {
                                super::fault::point("restore:descriptor:before");
                                match driver.propose(&Command::DataRange { expected: None, next: descriptor.clone() }) {
                                    Ok(_) => {
                                        self.restore.descriptor_proposals.insert(target.region().0, status.term);
                                        self.restore.record(intent.task(), target.region().0, "descriptor", format!(
                                            "proposed keyspace {} [{}, {}) in term {}",
                                            intent.keyspace().0,
                                            printable(target.start()),
                                            printable(target.end()),
                                            status.term
                                        ));
                                    }
                                    Err(error) => self.restore.error = Some(format!("restore descriptor {}: {error}", target.region().0)),
                                }
                            } else if applied.as_ref() == Some(&descriptor)
                                && self.restore.copied.get(&target.region().0) != Some(&intent.task())
                                && step.is_none()
                            {
                                let manifest = dg::capture::committed_capture(view.as_ref(), &certified, target.source_operation())?
                                    .ok_or_else(|| Error::MetaNotReady("the source range's capture descriptor is not applied here".into()))?
                                    .descriptor()
                                    .manifest()
                                    .clone();
                                let group = RawGroup::new(descriptor.clone(), engine.clone(), driver.clone(), self.rpc_backend.raw_directory.clock());
                                step = Some((
                                    intent.task(),
                                    target.region().0,
                                    Step::Populate {
                                        intent: intent.clone(),
                                        target: target.clone(),
                                        source_keyspace: backup.keyspace(),
                                        manifest,
                                        descriptor,
                                        group,
                                    },
                                ));
                            }
                        }
                    } else if out.waiting.is_none() {
                        out.waiting = Some(format!("region {}: waiting for its leader to copy", target.region().0));
                    }
                }
                out.targets.push(TargetView { region: target.region().0, source_region: target.source_region().0, state, lead });
            }
            views.push(out);
        }
        Ok((views, step))
    }

    fn start_restore_step(&mut self, restore: u64, region: u64, step: Step) {
        let backend = self.rpc_backend.clone();
        let handle = self.grpc_runtime.handle().clone();
        let (name, label) = (step.name(), step.label());
        let worker = std::thread::Builder::new()
            .name("kv9-restore".into())
            .spawn(move || execute(&backend, handle, step))
            .expect("spawn restore worker");
        self.restore.in_flight = Some((restore, region, name, label, worker));
    }

    /// The `restore=` status line.
    pub(super) fn restore_status(&self) -> String {
        let state = &self.restore;
        serde_json::to_string(&StatusLine {
            restores: state.views.clone(),
            in_flight: state.in_flight.as_ref().map(|(_, _, step, label, _)| format!("{step} {label}")),
            events: state.events.iter().cloned().collect(),
            error: state.error.clone(),
        })
        .expect("serialize restore status")
    }
}

fn execute(backend: &Arc<RuntimeBackend>, handle: tokio::runtime::Handle, step: Step) -> Result<String> {
    let name = step.name();
    super::fault::point(&format!("restore:{name}:before"));
    let out = execute_step(backend, handle, step);
    if out.is_ok() {
        super::fault::point(&format!("restore:{name}:after"));
    }
    out
}

fn execute_step(backend: &Arc<RuntimeBackend>, handle: tokio::runtime::Handle, step: Step) -> Result<String> {
    let _runtime = handle.enter();
    match step {
        Step::Populate { intent, target, source_keyspace, manifest, descriptor, group } => {
            let uploader = backend
                .uploader
                .as_deref()
                .ok_or_else(|| Error::Config("restore requires configured object storage".into()))?;
            // The anchored image, restored under the image budget: the exact rows the
            // backup verified, nothing the source wrote since.
            let image = uploader.restore(&manifest)?;
            let ctx = RequestContext {
                keyspace: intent.keyspace(),
                region_epoch: kv9_region::RegionEpoch { conf_ver: 1, version: 1 },
                origin: RequestOrigin::from_transport("restore"),
            };
            let mut cursor = descriptor.start.clone();
            let mut first = true;
            let mut copied = 0u64;
            loop {
                let from = if first { cursor.clone() } else { super::range_split::successor(&cursor) };
                first = false;
                let snapshot = image.snapshot()?;
                let read = LeaderRead::new(snapshot.as_ref(), true, None)?;
                let pairs = RawExecutor.scan(&read, source_keyspace, &from, &descriptor.end, PAGE)?;
                if pairs.is_empty() {
                    break;
                }
                cursor = pairs.last().map(|(k, _)| k.clone()).expect("non-empty");
                copied += pairs.len() as u64;
                group.raw_batch_put(&ctx, &pairs)?;
            }
            // Verify the target row by row against what the image holds, then let the
            // metadata leader compare with the anchor (its own digest of the same image).
            let (image_digest, image_rows) = digest_half(
                |from| {
                    let snapshot = image.snapshot()?;
                    let read = LeaderRead::new(snapshot.as_ref(), true, None)?;
                    RawExecutor.scan(&read, source_keyspace, from, &descriptor.end, PAGE)
                },
                &descriptor.start,
            )?;
            let (target_digest, target_rows) = digest_half(|from| group.raw_scan(&ctx, from, &descriptor.end, PAGE), &descriptor.start)?;
            if image_digest != target_digest || image_rows != target_rows {
                return Err(Error::Config(format!(
                    "target rows differ from the image ({target_rows} vs {image_rows} rows)"
                )));
            }
            let receipt = RestoreEvidence::receipt(&intent, &target, target_rows, target_digest)?;
            let message = deliver(backend, &handle, intent.root(), &receipt)?;
            Ok(format!("copied {copied} rows, {target_rows} verified, digest {}: {message}", &target_digest.to_string()[..16]))
        }
        Step::Publish { root, operation } => {
            let (changed, applied) = backend.commit_restore_publication(root, operation)?;
            Ok(format!("{} at {}/{}", if changed { "published" } else { "confirmed" }, applied.term, applied.index))
        }
    }
}

impl RuntimeBackend {
    /// Publish the restore (B4): one catalog transaction, metadata leader only.
    fn commit_restore_publication(&self, root: RootDigest, operation: [u8; 16]) -> Result<(bool, kv9_common::AppliedPosition)> {
        self.ensure_serving()?;
        let _guard = self.node.meta_raft.lock_catalog_txn();
        let term = self.prepare_catalog()?;
        let mut txn = self.node.meta_raft.store.begin()?;
        if kv9_meta::root::certified_root(&txn)?.map(|r| r.digest()) != Some(root) {
            return Err(Error::Config("restore publication root differs".into()));
        }
        let (_, changed) = kv9_meta::data_groups::restore::plan_restore_publication(&mut txn, operation)?;
        let command = if changed {
            Command::from_batch(&txn.into_batch())
        } else {
            Command::Noop
        };
        let applied = self.commit_catalog(&command, term)?;
        Ok((changed, applied))
    }
}

/// The region ids of every target of an effectively aborted restore: abandoned
/// empty groups that retire like an aborted split's children.
pub(super) fn aborted_targets<E: kv9_engine::Engine>(store: &kv9_meta::store::MetaStore<E>) -> Result<Vec<RegionId>> {
    let rows = committed_restores(store)?;
    Ok(rows
        .intents
        .iter()
        .filter(|i| rows.stage(i) == RestoreStage::Aborted)
        .flat_map(|i| i.targets().iter().map(|t| t.region()))
        .collect())
}
