//! Backup of a keyspace (docs/BACKUP-RESTORE.md §3). A committed kind-119 intent
//! names a keyspace's ranges; from then on two roles act on it, both from the
//! reconcile loop, both with at most one blocking step in flight on a worker:
//!
//! - **each range's leader** plans the capture at its own cut (`plan_image`),
//!   uploads and verifies the image once the pins are published (`capture`),
//!   digests the rows from the image, and commits the range's anchor (kind 120)
//!   through the shared evidence path;
//! - **the metadata leader** publishes the backup owners for a planned image
//!   (`bind`) and, once every range is anchored, writes the bundle object,
//!   verifies the whole closure and commits the completion (kind 122, `bundle`).
//!
//! Every step is derived from committed rows and the applied ledger; a crashed
//! node resumes from them, and a leader change moves the step to the new leader.
//! `KV9_RANGE_DRIVER=0` pauses this driver too.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kv9_common::{Error, KeyspaceId, RegionId, Result, RootDigest};
use kv9_engine::checkpoint::CheckpointManifest;
use kv9_engine::Engine;
use kv9_meta::data_groups::backup::{BackupAnchor, BackupBundle, BackupIntent, BackupRange, BackupRows, BackupStage, BundleRange};
use kv9_meta::data_groups::CaptureSubject;
use kv9_raft::snapshot_install::capture::CaptureDescriptor;
use kv9_raft::Role;
use kv9_raw::{LeaderRead, RawExecutor};

use super::range_split::{digest_into, range_driver_enabled};
use super::{NodeRuntime, RuntimeBackend};
use crate::api::AdminApi;

const EVENT_HISTORY: usize = 24;
/// Rows per page when digesting a restored image.
const PAGE: usize = 512;

#[derive(Debug, Clone, serde::Serialize)]
struct Event {
    at_unix: u64,
    backup: u64,
    region: u64,
    step: &'static str,
    outcome: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct RangeView {
    region: u64,
    operation: String,
    /// `planned` (capture descriptor committed), `pinned` (owners published),
    /// `anchored`, or `-`.
    state: &'static str,
    lead: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
struct BackupView {
    task: u64,
    keyspace: u32,
    label: String,
    stage: &'static str,
    ranges: Vec<RangeView>,
    waiting: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct StatusLine {
    backups: Vec<BackupView>,
    in_flight: Option<String>,
    events: Vec<Event>,
    error: Option<String>,
}

#[derive(Debug, Default)]
pub(super) struct State {
    views: Vec<BackupView>,
    in_flight: Option<(u64, u64, &'static str, String, std::thread::JoinHandle<Result<String>>)>,
    /// Last attempt per (backup, region, step): a failed step is retried after its
    /// backoff, not every pass.
    attempted: BTreeMap<(u64, u64, &'static str), Instant>,
    events: VecDeque<Event>,
    error: Option<String>,
}

impl State {
    fn record(&mut self, backup: u64, region: u64, step: &'static str, outcome: String) {
        if self.events.len() >= EVENT_HISTORY {
            self.events.pop_front();
        }
        self.events.push_back(Event {
            at_unix: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
            backup,
            region,
            step,
            outcome,
        });
    }
}

enum Step {
    PlanImage { root: RootDigest, operation: [u8; 16] },
    Bind { root: RootDigest, operation: [u8; 16], manifest: Vec<u8> },
    Capture { root: RootDigest, intent: BackupIntent, range: BackupRange, keyspace: KeyspaceId, start: Vec<u8>, end: Vec<u8>, manifest: CheckpointManifest },
    Bundle { root: RootDigest, intent: BackupIntent },
}

impl Step {
    fn name(&self) -> &'static str {
        match self {
            Step::PlanImage { .. } => "plan_image",
            Step::Bind { .. } => "bind",
            Step::Capture { .. } => "capture",
            Step::Bundle { .. } => "bundle",
        }
    }
    /// How long after an attempt the same step may run again.
    fn backoff(&self) -> Duration {
        match self {
            Step::Capture { .. } => Duration::from_secs(10),
            Step::Bundle { .. } => Duration::from_secs(5),
            _ => Duration::from_secs(2),
        }
    }
    fn label(&self) -> String {
        match self {
            Step::PlanImage { operation, .. } | Step::Bind { operation, .. } => format!("operation {}", hex(&operation[..4])),
            Step::Capture { range, .. } => format!("region {}", range.region().0),
            Step::Bundle { intent, .. } => format!("backup {} ({} ranges)", intent.task(), intent.ranges().len()),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

impl NodeRuntime {
    pub(super) fn reconcile_backup(&mut self) {
        if let Some((backup, region, step, label, handle)) = self.backup.in_flight.take() {
            if !handle.is_finished() {
                self.backup.in_flight = Some((backup, region, step, label, handle));
                return;
            }
            match handle.join() {
                Ok(Ok(outcome)) => {
                    self.backup.error = None;
                    self.backup.record(backup, region, step, format!("{label}: {outcome}"));
                }
                Ok(Err(error)) => {
                    self.backup.error = Some(format!("{step} {label}: {error}"));
                    self.backup.record(backup, region, step, format!("{label}: failed: {error}"));
                }
                Err(_) => {
                    self.backup.error = Some(format!("{step} worker panicked"));
                    self.backup.record(backup, region, step, format!("{label}: worker panicked"));
                }
            }
        }
        if !range_driver_enabled() {
            return;
        }
        match self.plan_backup_steps() {
            Ok((views, step)) => {
                self.backup.views = views;
                if let Some((backup, region, step)) = step {
                    let key = (backup, region, step.name());
                    let now = Instant::now();
                    if self.backup.attempted.get(&key).is_some_and(|at| now.duration_since(*at) < step.backoff()) {
                        return;
                    }
                    self.backup.attempted.insert(key, now);
                    self.start_backup_step(backup, region, step);
                }
            }
            Err(Error::MetaNotReady(_)) => {}
            Err(error) => self.backup.error = Some(error.to_string()),
        }
    }

    /// Every live backup's view and the first step this node should take.
    #[allow(clippy::type_complexity)]
    fn plan_backup_steps(&self) -> Result<(Vec<BackupView>, Option<(u64, u64, Step)>)> {
        use kv9_meta::data_groups as dg;
        let store = &self.node.meta_raft.store;
        let txn = store.begin()?;
        let certified = kv9_meta::root::certified_root(&txn)?
            .ok_or_else(|| Error::MetaNotReady("backup requires a certified root".into()))?;
        let root = certified.digest();
        let rows: BackupRows = dg::backup::committed_backups_in_txn(&txn)?;
        let live: Vec<&BackupIntent> = rows
            .intents
            .iter()
            .filter(|i| matches!(rows.stage(i), BackupStage::Intended | BackupStage::Capturing | BackupStage::Anchored))
            .collect();
        if live.is_empty() {
            return Ok((Vec::new(), None));
        }
        let bindings = dg::ranges::committed_ranges_in_txn(&txn)?;
        let view = txn.into_view();
        let meta_leader = self.driver.status().role == Role::Leader;
        let led: Vec<RegionId> = self.data_groups.led_groups().iter().map(|(c, _, _)| c.region()).collect();
        let mut views = Vec::new();
        let mut step: Option<(u64, u64, Step)> = None;
        for intent in live {
            let stage = rows.stage(intent);
            let mut out = BackupView {
                task: intent.task(),
                keyspace: intent.keyspace().0,
                label: intent.label().to_owned(),
                stage: stage.label(),
                ranges: Vec::new(),
                waiting: None,
            };
            for range in intent.ranges() {
                let operation = range.operation();
                let lead = led.contains(&range.region());
                let anchored = rows.anchor_of_range(range).is_some();
                let capture = dg::capture::committed_capture(view.as_ref(), &certified, operation)?.map(|c| c.descriptor().clone());
                let subject = CaptureSubject::Backup { intent: intent.clone(), range: range.clone() };
                let (source_owner, _) = crate::migration_retention::backup_owner_ids(root, &subject)?;
                let pinned = kv9_meta::retention::retention_owner(view.as_ref(), &certified, source_owner)
                    .ok()
                    .flatten()
                    .is_some_and(|o| o.phase == kv9_common::retention::PinPhase::Published);
                out.ranges.push(RangeView {
                    region: range.region().0,
                    operation: hex(&operation),
                    state: if anchored {
                        "anchored"
                    } else if pinned {
                        "pinned"
                    } else if capture.is_some() {
                        "planned"
                    } else {
                        "-"
                    },
                    lead,
                });
                if anchored || step.is_some() {
                    continue;
                }
                match (&capture, pinned) {
                    (None, _) if lead => {
                        step = Some((intent.task(), range.region().0, Step::PlanImage { root, operation }));
                    }
                    (Some(descriptor), false) if meta_leader => {
                        let manifest = descriptor.manifest().encode()?;
                        step = Some((intent.task(), range.region().0, Step::Bind { root, operation, manifest }));
                    }
                    (Some(descriptor), true) if lead => {
                        let binding = bindings
                            .iter()
                            .find(|b| b.range().region == range.region())
                            .ok_or_else(|| Error::MetaNotReady("backup range binding missing".into()))?;
                        step = Some((
                            intent.task(),
                            range.region().0,
                            Step::Capture {
                                root,
                                intent: intent.clone(),
                                range: range.clone(),
                                keyspace: binding.range().keyspace,
                                start: binding.range().start.clone(),
                                end: binding.range().end.clone(),
                                manifest: descriptor.manifest().clone(),
                            },
                        ));
                    }
                    (None, _) => out.waiting = Some(format!("region {}: waiting for its leader to plan the image", range.region().0)),
                    (Some(_), false) => out.waiting = Some(format!("region {}: waiting for the metadata leader to publish the pins", range.region().0)),
                    (Some(_), true) => out.waiting = Some(format!("region {}: waiting for its leader to upload and anchor", range.region().0)),
                }
            }
            if stage == BackupStage::Anchored && step.is_none() {
                if meta_leader {
                    step = Some((intent.task(), 0, Step::Bundle { root, intent: intent.clone() }));
                } else {
                    out.waiting = Some("waiting for the metadata leader to write the bundle".into());
                }
            }
            views.push(out);
        }
        Ok((views, step))
    }

    fn start_backup_step(&mut self, backup: u64, region: u64, step: Step) {
        let backend = self.rpc_backend.clone();
        let handle = self.grpc_runtime.handle().clone();
        let (name, label) = (step.name(), step.label());
        let worker = std::thread::Builder::new()
            .name("kv9-backup".into())
            .spawn(move || execute(&backend, handle, step))
            .expect("spawn backup worker");
        self.backup.in_flight = Some((backup, region, name, label, worker));
    }

    /// The `backup=` status line.
    pub(super) fn backup_status(&self) -> String {
        let state = &self.backup;
        serde_json::to_string(&StatusLine {
            backups: state.views.clone(),
            in_flight: state.in_flight.as_ref().map(|(_, _, step, label, _)| format!("{step} {label}")),
            events: state.events.iter().cloned().collect(),
            error: state.error.clone(),
        })
        .expect("serialize backup status")
    }
}

fn execute(backend: &Arc<RuntimeBackend>, handle: tokio::runtime::Handle, step: Step) -> Result<String> {
    let name = step.name();
    super::fault::point(&format!("backup:{name}:before"));
    let out = execute_step(backend, handle, step);
    if out.is_ok() {
        super::fault::point(&format!("backup:{name}:after"));
    }
    out
}

fn execute_step(backend: &Arc<RuntimeBackend>, handle: tokio::runtime::Handle, step: Step) -> Result<String> {
    let _runtime = handle.enter();
    const CALLER: &str = "backup";
    match step {
        Step::PlanImage { root, operation } => {
            let r = AdminApi::plan_migration_image(backend.as_ref(), CALLER, root, operation)?;
            Ok(format!("capture planned at cut {}/{}", r.cut.term, r.cut.index))
        }
        Step::Bind { root, operation, manifest } => {
            let r = AdminApi::bind_migration_image(backend.as_ref(), CALLER, root, operation, &manifest)?;
            Ok(format!("pins published: source {} backup {}", hex(r.source_owner.as_bytes()), hex(r.destination_owner.as_bytes())))
        }
        Step::Capture { root, intent, range, keyspace, start, end, manifest } => {
            let r = AdminApi::capture_migration_image(backend.as_ref(), CALLER, root, range.operation())?;
            // The rows and their digest come from the image itself, restored under
            // the image budget; an image too large for it is kept but not restorable
            // (docs/BACKUP-RESTORE.md §11).
            let rows = match backend.uploader.as_deref() {
                Some(uploader) => match uploader.restore(&manifest) {
                    Ok(engine) => Some(digest_image(&engine, keyspace, &start, &end)?),
                    Err(error) => {
                        let _ = error;
                        None
                    }
                },
                None => None,
            };
            let manifest_digest = RootDigest::sha256(&manifest.encode()?);
            let receipt = BackupAnchor::receipt(
                &intent,
                &range,
                r.cut,
                r.configuration_applied_at,
                r.image_digest,
                manifest_digest,
                r.objects,
                r.object_bytes,
                rows,
                *r.destination_owner.as_bytes(),
            )?;
            let message = super::range_split::deliver(backend, &handle, root, &receipt)?;
            Ok(format!(
                "image {} uploaded ({} objects, {} bytes, cut {}/{}); rows {}; anchor: {message}",
                r.image_digest,
                r.objects,
                r.object_bytes,
                r.cut.term,
                r.cut.index,
                rows.map_or("not digested (image exceeds the budget)".to_owned(), |(n, _)| n.to_string())
            ))
        }
        Step::Bundle { root, intent } => backend.write_backup_bundle(root, &intent),
    }
}

/// sha256 over every `(key, value)` of `[start, end)` in the restored image, in
/// order, length-prefixed (the split's digest), with the row count.
fn digest_image<E: Engine>(engine: &E, keyspace: KeyspaceId, start: &[u8], end: &[u8]) -> Result<(u64, RootDigest)> {
    let mut hasher = Vec::new();
    let rows = digest_into(
        &mut hasher,
        |from| {
            let snapshot = engine.snapshot()?;
            let read = LeaderRead::new(snapshot.as_ref(), true, None)?;
            RawExecutor.scan(&read, keyspace, from, end, PAGE)
        },
        start,
    )?;
    Ok((rows, RootDigest::sha256(&hasher)))
}

impl RuntimeBackend {
    /// The metadata leader's last step: assemble the bundle from committed rows,
    /// PUT it, GET-verify every range's closure, commit the completion.
    fn write_backup_bundle(&self, root: RootDigest, intent: &BackupIntent) -> Result<String> {
        use kv9_meta::data_groups as dg;
        let uploader = self
            .uploader
            .as_deref()
            .ok_or_else(|| Error::Config("backup requires configured object storage".into()))?;
        let store = &self.node.meta_raft.store;
        let txn = store.begin()?;
        let certified = kv9_meta::root::certified_root(&txn)?
            .ok_or_else(|| Error::MetaNotReady("backup requires a certified root".into()))?;
        if certified.digest() != root {
            return Err(Error::Config("backup bundle root differs".into()));
        }
        let rows = dg::backup::committed_backups_in_txn(&txn)?;
        let intent = rows
            .intent_by_operation(intent.operation())
            .ok_or_else(|| Error::Config("backup intent is not committed".into()))?;
        if rows.stage(intent) != BackupStage::Anchored {
            return Err(Error::Config(format!("backup is {}; the bundle needs every range anchored", rows.stage(intent).label())));
        }
        let keyspace_name = dg::backup::keyspace_name_in(&txn, intent.keyspace())?.unwrap_or_default();
        let bindings = dg::ranges::committed_ranges_in_txn(&txn)?;
        let view = txn.into_view();
        let mut ranges = Vec::with_capacity(intent.ranges().len());
        let mut manifests = Vec::with_capacity(intent.ranges().len());
        for range in intent.ranges() {
            let anchor = rows
                .anchor_of_range(range)
                .ok_or_else(|| Error::Config("backup range lost its anchor".into()))?
                .clone();
            let descriptor: CaptureDescriptor = dg::capture::committed_capture(view.as_ref(), &certified, range.operation())?
                .ok_or_else(|| Error::Config("anchored range has no committed capture".into()))?
                .descriptor()
                .clone();
            let binding = bindings
                .iter()
                .find(|b| b.bind_task() == range.bind_task())
                .ok_or_else(|| Error::Config("backup range binding is missing".into()))?;
            manifests.push(descriptor.manifest().clone());
            ranges.push(BundleRange {
                region: range.region(),
                creation_task: range.creation_task(),
                bind_task: range.bind_task(),
                conf_ver: range.conf_ver(),
                version: range.version(),
                start: binding.range().start.clone(),
                end: binding.range().end.clone(),
                anchor,
                descriptor: descriptor.encode().to_vec(),
            });
        }
        let bundle = BackupBundle {
            cluster_id: *certified.cluster_id.as_bytes(),
            root,
            task: intent.task(),
            operation: intent.operation(),
            keyspace: intent.keyspace(),
            tenant: intent.tenant(),
            sequence: intent.sequence(),
            requested_unix: intent.requested_unix(),
            label: intent.label().to_owned(),
            keyspace_name,
            ranges,
        };
        let bytes = bundle.encode();
        let key = BackupBundle::object_key(&bundle.cluster_id, intent.task());
        let object_key = kv9_engine::ObjectKey::new(key.clone())?;
        uploader.put_object(&object_key, &bytes)?;
        // The closure must GET-verify after the PUT: a bundle that references a
        // missing object is not a backup.
        for manifest in &manifests {
            uploader.verify_checkpoint_objects(manifest)?;
        }
        let digest = RootDigest::sha256(&bytes);
        let (completion, changed) = self.commit_backup_completion(intent.task(), &key, digest, bytes.len() as u64)?;
        Ok(format!(
            "bundle {key} ({} bytes, {} ranges verified) {} as task {}",
            bytes.len(),
            manifests.len(),
            if changed { "committed" } else { "confirmed" },
            completion.task()
        ))
    }

    fn commit_backup_completion(&self, backup_task: u64, key: &str, digest: RootDigest, bytes: u64) -> Result<(kv9_meta::data_groups::backup::BackupCompletion, bool)> {
        use kv9_raft::Command;
        self.ensure_serving()?;
        let _guard = self.node.meta_raft.lock_catalog_txn();
        let term = self.prepare_catalog()?;
        let mut txn = self.node.meta_raft.store.begin()?;
        let (completion, changed) = kv9_meta::data_groups::backup::plan_backup_completion(&mut txn, backup_task, key, digest, bytes, unix_now())?;
        let command = if changed {
            Command::from_batch(&txn.into_batch())
        } else {
            Command::Noop
        };
        self.commit_catalog(&command, term)?;
        Ok((completion, changed))
    }
}
