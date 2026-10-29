//! Range split, parts 2 and 3 (docs/RANGE-SPLIT.md §3–§4): from a committed
//! intent to two populated children, with nothing rerouted.
//!
//! Every pass, from committed state only, each node does what its own roles
//! allow, one blocking step in flight on a worker thread:
//!
//! - **a child's leader** installs the child's half descriptor through the
//!   child's own log (initial publication, so the child's fence admits exactly
//!   its half and nothing is routable to it: the catalog still names the parent);
//! - **the parent's leader** seals the parent (the `DataRange` CAS, version+1)
//!   once both children serve behind a leader;
//! - **a child's leader** copies its half from the LOCAL parent replica — the
//!   children live on the parent's stores — after that replica has applied the
//!   seal: page by page through the child's own log as ordinary raw writes, then
//!   verifies child rows == parent half rows and commits the kind-114 evidence
//!   (locally at the metadata leader, or over the internal service).
//!
//! Population is idempotent (puts of an immutable source); the cursor lives in
//! memory and a restart copies again from the first key.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kv9_common::data_range::{DataRange, RANGE_KEY};
use kv9_common::{Error, KeyspaceId, RegionId, Result, RootDigest, UserKey, Value};
use kv9_engine::{ColumnFamily, Engine, WalEngine};
use kv9_meta::data_groups::split::SplitIntent;
use kv9_meta::data_groups::split_evidence::SplitEvidence;
use kv9_raft::driver::NodeDriver;
use kv9_raft::storage::DiskRaftStorage;
use kv9_raft::{Command, Role};
use kv9_raw::{LeaderRead, RawExecutor};

use super::range_api::RawGroup;
use super::{NodeRuntime, RuntimeBackend};
use crate::api::{RawApi, RequestContext, RequestOrigin};

/// `KV9_RANGE_DRIVER=0` pauses the split, merge and auto-split drivers on this node
/// (an operator's brake and the e2e's way to abort before a seal deterministically).
pub(super) fn range_driver_enabled() -> bool {
    std::env::var("KV9_RANGE_DRIVER").as_deref() != Ok("0")
}

pub(super) const PAGE: usize = 512;
pub(super) const SEAL_WAIT: Duration = Duration::from_secs(10);
pub(super) const DELIVERY_BUDGET: Duration = Duration::from_secs(10);
const EVENT_HISTORY: usize = 32;

#[derive(Debug, Clone, serde::Serialize)]
struct Event {
    at_unix: u64,
    region: u64,
    step: &'static str,
    outcome: String,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
struct Population {
    rows_copied: u64,
    cursor: Option<String>,
    done: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
struct IntentView {
    task: u64,
    parent_region: u64,
    children: [u64; 2],
    parent_sealed_locally: Option<bool>,
    lead_parent: bool,
    lead_children: Vec<u64>,
    evidenced: Vec<u64>,
    waiting: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct StatusLine {
    intents: Vec<IntentView>,
    populations: BTreeMap<u64, Population>,
    in_flight: Option<String>,
    events: Vec<Event>,
    error: Option<String>,
}

#[derive(Debug, Default)]
pub(super) struct State {
    views: Vec<IntentView>,
    /// Child region → copy progress on this node (meaningful while it leads the child).
    populations: BTreeMap<u64, Population>,
    /// Descriptor proposals per child region, once per term.
    descriptor_proposals: BTreeMap<u64, u64>,
    in_flight: Option<(u64, &'static str, String, std::thread::JoinHandle<Result<Outcome>>)>,
    events: VecDeque<Event>,
    error: Option<String>,
}

impl State {
    fn record(&mut self, region: u64, step: &'static str, outcome: String) {
        if self.events.len() >= EVENT_HISTORY {
            self.events.pop_front();
        }
        self.events.push_back(Event {
            at_unix: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
            region,
            step,
            outcome,
        });
    }
}

/// What a worker step produced.
pub(super) enum Outcome {
    Sealed(String),
    Published(String),
    Copied { rows: u64, last: Vec<u8> },
    Evidenced { rows: u64, digest: RootDigest, message: String },
}

enum Step {
    Publish {
        parent_region: RegionId,
        root: RootDigest,
        operation: [u8; 16],
    },
    Seal {
        region: RegionId,
        driver: Arc<NodeDriver<DiskRaftStorage, WalEngine>>,
        current: DataRange,
        root: RootDigest,
        operation: [u8; 16],
    },
    Populate {
        intent: SplitIntent,
        child_task: u64,
        child_region: RegionId,
        child: Arc<RawGroup>,
        parent_engine: Arc<WalEngine>,
        half: DataRange,
        cursor: Option<Vec<u8>>,
    },
}

impl Step {
    fn name(&self) -> &'static str {
        match self {
            Step::Publish { .. } => "publish",
            Step::Seal { .. } => "seal",
            Step::Populate { .. } => "populate",
        }
    }
    fn region(&self) -> u64 {
        match self {
            Step::Publish { parent_region, .. } => parent_region.0,
            Step::Seal { region, .. } => region.0,
            Step::Populate { child_region, .. } => child_region.0,
        }
    }
    fn label(&self) -> String {
        match self {
            Step::Publish { parent_region, .. } => format!("split of region {}", parent_region.0),
            Step::Seal { region, .. } => format!("parent region {}", region.0),
            Step::Populate { child_region, cursor, .. } => format!(
                "child region {} from {}",
                child_region.0,
                cursor.as_ref().map_or("start".to_string(), |c| printable(c))
            ),
        }
    }
}

pub(super) fn printable(key: &[u8]) -> String {
    String::from_utf8_lossy(key).into_owned()
}

/// The half descriptor a child carries before publication (RANGE-SPLIT §4).
fn half_descriptor(parent: &DataRange, child: &kv9_meta::data_groups::CreationIntent, low: bool, key: &[u8]) -> DataRange {
    DataRange {
        root: parent.root,
        creation: child.digest(),
        region: child.region(),
        keyspace: parent.keyspace,
        tenant: parent.tenant,
        conf_ver: 1,
        version: 1,
        start: if low { parent.start.clone() } else { key.to_vec() },
        end: if low { key.to_vec() } else { parent.end.clone() },
        sealed: false,
    }
}

pub(super) fn applied_descriptor(engine: &WalEngine) -> Result<Option<DataRange>> {
    engine
        .get(ColumnFamily::Default, RANGE_KEY)?
        .map(|b| DataRange::decode(&b))
        .transpose()
}

impl NodeRuntime {
    pub(super) fn reconcile_range_split(&mut self) {
        if !range_driver_enabled() {
            return; // KV9_RANGE_DRIVER=0: committed intents wait; nothing is lost
        }
        if let Some((region, step, label, handle)) = self.range_split.in_flight.take() {
            if !handle.is_finished() {
                self.range_split.in_flight = Some((region, step, label, handle));
                return;
            }
            match handle.join() {
                Ok(Ok(Outcome::Sealed(message))) | Ok(Ok(Outcome::Published(message))) => {
                    self.range_split.error = None;
                    self.range_split.record(region, step, format!("{label}: {message}"));
                }
                Ok(Ok(Outcome::Copied { rows, last })) => {
                    self.range_split.error = None;
                    let p = self.range_split.populations.entry(region).or_default();
                    p.rows_copied += rows;
                    p.cursor = Some(printable(&last));
                }
                Ok(Ok(Outcome::Evidenced { rows, digest, message })) => {
                    self.range_split.error = None;
                    let p = self.range_split.populations.entry(region).or_default();
                    p.done = true;
                    self.range_split.record(
                        region,
                        step,
                        format!("{label}: {rows} rows, digest {}: {message}", &digest.to_string()[..16]),
                    );
                }
                Ok(Err(error)) => {
                    self.range_split.error = Some(format!("{step} {label}: {error}"));
                    self.range_split.record(region, step, format!("{label}: failed: {error}"));
                }
                Err(_) => {
                    self.range_split.error = Some(format!("{step} worker panicked"));
                }
            }
        }
        let store = &self.node.meta_raft.store;
        let intents = match kv9_meta::data_groups::split::committed_splits(store) {
            Ok(intents) => intents,
            Err(error) => {
                // A row family that no longer validates freezes this driver; say so.
                self.range_split.error = Some(format!("split intents do not read back: {error}"));
                return;
            }
        };
        if intents.is_empty() {
            self.range_split.views.clear();
            return;
        }
        match self.plan_split_steps(&intents) {
            Ok((views, step)) => {
                self.range_split.views = views;
                if let Some(step) = step {
                    self.start_split_step(step);
                }
            }
            Err(error) => self.range_split.error = Some(error.to_string()),
        }
    }

    fn plan_split_steps(&mut self, intents: &[SplitIntent]) -> Result<(Vec<IntentView>, Option<Step>)> {
        use kv9_meta::data_groups as dg;
        let store = &self.node.meta_raft.store;
        let bindings = dg::ranges::committed_ranges(store)?;
        let evidence = dg::split_evidence::committed_split_evidence(store).unwrap_or_default();
        let aborted = dg::range_abort::effective_aborts(store).unwrap_or_default();
        let me = self.node.id.0;
        let mut views = Vec::new();
        let mut step: Option<Step> = None;
        for intent in intents {
            if aborted.contains(&intent.operation()) {
                continue; // history: its children retire through the retirement pass
            }
            let Some(parent_binding) = bindings.iter().find(|b| b.range().region == intent.parent_region()) else {
                continue;
            };
            let parent_range = parent_binding.range().clone();
            if parent_range.sealed {
                continue; // published: the directory does the rest
            }
            let children: Vec<(u64, bool)> = vec![(intent.child_low(), true), (intent.child_high(), false)];
            let mut view = IntentView {
                task: intent.task(),
                parent_region: intent.parent_region().0,
                children: [intent.child_low(), intent.child_high()],
                parent_sealed_locally: None,
                lead_parent: false,
                lead_children: Vec::new(),
                evidenced: evidence
                    .iter()
                    .filter(|e| e.operation() == intent.operation())
                    .map(|e| e.child_task())
                    .collect(),
                waiting: None,
            };
            let parent_local = self.data_groups.local_parts(intent.parent_region());
            if let Some((_, engine, driver)) = &parent_local {
                view.parent_sealed_locally = applied_descriptor(engine)?.map(|d| d.sealed);
                view.lead_parent = driver.status().role == Role::Leader;
            }
            // Children: creation rows, local parts, descriptors.
            let mut child_parts = Vec::new();
            for (task, low) in &children {
                let creation = dg::committed_creation(store, *task)?
                    .ok_or_else(|| Error::MetaNotReady("split child creation is not applied yet".into()))?;
                let region = creation.intent().region();
                let half = half_descriptor(&parent_range, creation.intent(), *low, intent.split_key());
                let local = self.data_groups.local_parts(region);
                child_parts.push((*task, region, half, local));
            }
            if step.is_some() {
                views.push(view);
                continue;
            }
            // (0) The metadata leader publishes once both children are evidenced.
            if self.driver.status().role == Role::Leader
                && view.evidenced.contains(&intent.child_low())
                && view.evidenced.contains(&intent.child_high())
            {
                step = Some(Step::Publish {
                    parent_region: intent.parent_region(),
                    root: intent.root(),
                    operation: intent.operation(),
                });
                views.push(view);
                continue;
            }
            // (1) A child this node leads without its descriptor: propose it.
            for (_, region, half, local) in &child_parts {
                let Some((_, engine, driver)) = local else { continue };
                let status = driver.status();
                if status.role != Role::Leader {
                    continue;
                }
                view.lead_children.push(region.0);
                if applied_descriptor(engine)?.is_none()
                    && self.range_split.descriptor_proposals.get(&region.0) != Some(&status.term)
                {
                    match driver.propose(&Command::DataRange {
                        expected: None,
                        next: half.clone(),
                    }) {
                        Ok(_) => {
                            self.range_split.descriptor_proposals.insert(region.0, status.term);
                            self.range_split.record(region.0, "child_descriptor", format!(
                                "proposed [{}, {}) in term {}",
                                printable(&half.start),
                                printable(&half.end),
                                status.term
                            ));
                        }
                        Err(error) => self.range_split.error = Some(format!("child descriptor {}: {error}", region.0)),
                    }
                }
            }
            // (2) This node leads the parent: seal it once both children serve.
            if let Some((_, engine, driver)) = &parent_local {
                if view.lead_parent {
                    match applied_descriptor(engine)? {
                        Some(current) if !current.sealed => {
                            let children_serve = child_parts
                                .iter()
                                .all(|(_, _, _, local)| local.as_ref().is_some_and(|(_, e, d)| {
                                    d.status().leader_id.is_some() && applied_descriptor(e).ok().flatten().is_some()
                                }));
                            if children_serve {
                                step = Some(Step::Seal {
                                    region: intent.parent_region(),
                                    driver: driver.clone(),
                                    current,
                                    root: intent.root(),
                                    operation: intent.operation(),
                                });
                                views.push(view);
                                continue;
                            }
                            view.waiting = Some("waiting for both children to serve behind a leader with their descriptors".into());
                        }
                        Some(_) => {}
                        None => view.waiting = Some("parent has no applied descriptor".into()),
                    }
                }
            }
            // (3) This node leads a child whose local parent replica is sealed: copy.
            let parent_sealed_here = view.parent_sealed_locally == Some(true);
            if parent_sealed_here {
                for (task, region, half, local) in &child_parts {
                    if view.evidenced.contains(task) {
                        continue;
                    }
                    let Some((_, engine, driver)) = local else { continue };
                    if driver.status().role != Role::Leader {
                        continue;
                    }
                    if applied_descriptor(engine)?.as_ref() != Some(half) {
                        continue; // descriptor not applied yet (or differs: never write)
                    }
                    if self.range_split.populations.get(&region.0).is_some_and(|p| p.done) {
                        continue; // evidence is on its way through the catalog
                    }
                    let (_, parent_engine, _) = parent_local.as_ref().expect("sealed locally implies local parts");
                    let child = RawGroup::new(half.clone(), engine.clone(), driver.clone(), self.rpc_backend.raw_directory.clock());
                    let cursor = self
                        .range_split
                        .populations
                        .get(&region.0)
                        .and_then(|p| p.cursor.as_ref().map(|c| c.as_bytes().to_vec()));
                    step = Some(Step::Populate {
                        intent: intent.clone(),
                        child_task: *task,
                        child_region: *region,
                        child,
                        parent_engine: parent_engine.clone(),
                        half: half.clone(),
                        cursor,
                    });
                    break;
                }
            } else if !view.lead_children.is_empty() && view.waiting.is_none() {
                view.waiting = Some(if parent_local.is_some() {
                    "waiting for the local parent replica to apply the seal".into()
                } else {
                    format!("node {me} holds no parent replica")
                });
            }
            views.push(view);
        }
        Ok((views, step))
    }

    fn start_split_step(&mut self, step: Step) {
        let backend = self.rpc_backend.clone();
        let handle = self.grpc_runtime.handle().clone();
        let (name, region, label) = (step.name(), step.region(), step.label());
        let worker = std::thread::Builder::new()
            .name("kv9-range-split".into())
            .spawn(move || execute(&backend, handle, step))
            .expect("spawn split worker");
        self.range_split.in_flight = Some((region, name, label, worker));
    }

    pub(super) fn range_split_status(&self) -> String {
        let state = &self.range_split;
        serde_json::to_string(&StatusLine {
            intents: state.views.clone(),
            populations: state.populations.clone(),
            in_flight: state.in_flight.as_ref().map(|(_, step, label, _)| format!("{step} {label}")),
            events: state.events.iter().cloned().collect(),
            error: state.error.clone(),
        })
        .expect("serialize range split status")
    }
}

/// Seal one parent: re-check the abort right before proposing (an abort that
/// lands after this check is decided by the seal evidence below), propose the
/// CAS, then commit the seal evidence that makes the operation irrevocable.
pub(super) fn seal_parent(
    backend: &Arc<RuntimeBackend>,
    handle: &tokio::runtime::Handle,
    region: RegionId,
    driver: &Arc<NodeDriver<DiskRaftStorage, WalEngine>>,
    current: &DataRange,
    root: RootDigest,
    operation: [u8; 16],
) -> Result<Outcome> {
    if kv9_meta::data_groups::range_abort::effective_aborts(&backend.node.meta_raft.store)?.contains(&operation) {
        return Err(Error::Config("the operation was aborted before its seal".into()));
    }
    let mut sealed = current.clone();
    sealed.version = current
        .version
        .checked_add(1)
        .ok_or_else(|| Error::Config("range version exhausted".into()))?;
    sealed.sealed = true;
    let applied = super::propose_and_wait(
        driver,
        &Command::DataRange {
            expected: Some(current.digest()),
            next: sealed,
        },
        SEAL_WAIT,
    )?;
    let receipt = kv9_meta::data_groups::range_abort::SealEvidence::receipt(root, operation, region, applied)?;
    let evidence = deliver(backend, handle, root, &receipt)?;
    Ok(Outcome::Sealed(format!(
        "sealed at {}/{} (version {}); {evidence}",
        applied.term,
        applied.index,
        current.version + 1
    )))
}

/// The successor of `key` in scan order: the smallest key greater than it.
pub(super) fn successor(key: &[u8]) -> Vec<u8> {
    let mut next = key.to_vec();
    next.push(0);
    next
}

/// Scan `[start, end)` of `keyspace` from a snapshot of `engine`, one page.
pub(super) fn scan_page(engine: &WalEngine, keyspace: KeyspaceId, start: &[u8], end: &[u8], limit: usize) -> Result<Vec<(UserKey, Value)>> {
    let snapshot = engine.snapshot()?;
    let read = LeaderRead::new(snapshot.as_ref(), true, None)?;
    RawExecutor.scan(&read, keyspace, start, end, limit)
}

/// sha256 over every `(key, value)` of the half, in order, length-prefixed.
pub(super) fn digest_half(page: impl FnMut(&[u8]) -> Result<Vec<(UserKey, Value)>>, start: &[u8]) -> Result<(RootDigest, u64)> {
    let mut hasher = Vec::new();
    let rows = digest_into(&mut hasher, page, start)?;
    Ok((RootDigest::sha256(&hasher), rows))
}

/// Feed every `(key, value)` of one range into `hasher`, length-prefixed, in order;
/// returns the row count. A merge chains two ranges through one hasher.
pub(super) fn digest_into(hasher: &mut Vec<u8>, mut page: impl FnMut(&[u8]) -> Result<Vec<(UserKey, Value)>>, start: &[u8]) -> Result<u64> {
    let mut rows = 0u64;
    let mut cursor = start.to_vec();
    let mut first = true;
    loop {
        let from = if first { cursor.clone() } else { successor(&cursor) };
        first = false;
        let pairs = page(&from)?;
        if pairs.is_empty() {
            break;
        }
        for (k, v) in &pairs {
            hasher.extend_from_slice(&(k.len() as u32).to_be_bytes());
            hasher.extend_from_slice(k);
            hasher.extend_from_slice(&(v.len() as u32).to_be_bytes());
            hasher.extend_from_slice(v);
            rows += 1;
        }
        cursor = pairs.last().map(|(k, _)| k.clone()).expect("non-empty page");
        if pairs.len() < PAGE {
            break;
        }
    }
    Ok(rows)
}

fn execute(backend: &Arc<RuntimeBackend>, handle: tokio::runtime::Handle, step: Step) -> Result<Outcome> {
    let _runtime = handle.enter();
    match step {
        Step::Publish { root, operation, .. } => {
            let (changed, applied) = backend.commit_split_publication(root, operation)?;
            Ok(Outcome::Published(format!(
                "{} at {}/{}",
                if changed { "published" } else { "confirmed" },
                applied.term,
                applied.index
            )))
        }
        Step::Seal { region, driver, current, root, operation } => {
            seal_parent(backend, &handle, region, &driver, &current, root, operation)
        }
        Step::Populate { intent, child_task, child, parent_engine, half, cursor, .. } => {
            let ctx = RequestContext {
                keyspace: half.keyspace,
                region_epoch: kv9_region::RegionEpoch { conf_ver: 1, version: 1 },
                origin: RequestOrigin::from_transport("range-split"),
            };
            let from = cursor.as_ref().map_or(half.start.clone(), |c| successor(c));
            let pairs = scan_page(&parent_engine, half.keyspace, &from, &half.end, PAGE)?;
            if !pairs.is_empty() {
                child.raw_batch_put(&ctx, &pairs)?;
                let last = pairs.last().map(|(k, _)| k.clone()).expect("non-empty");
                return Ok(Outcome::Copied { rows: pairs.len() as u64, last });
            }
            // The half is exhausted: verify row for row, then report.
            let (parent_digest, parent_rows) =
                digest_half(|from| scan_page(&parent_engine, half.keyspace, from, &half.end, PAGE), &half.start)?;
            let (child_digest, child_rows) =
                digest_half(|from| child.raw_scan(&ctx, from, &half.end, PAGE), &half.start)?;
            if parent_digest != child_digest || parent_rows != child_rows {
                return Err(Error::Config(format!(
                    "child rows differ from the sealed parent half ({child_rows} vs {parent_rows} rows)"
                )));
            }
            let receipt = SplitEvidence::receipt(&intent, child_task, child_digest, child_rows)?;
            let message = deliver(backend, &handle, intent.root(), &receipt)?;
            Ok(Outcome::Evidenced { rows: child_rows, digest: child_digest, message })
        }
    }
}

/// Commit the evidence locally when this node leads the metadata group, else at a
/// metadata candidate over the node-authenticated internal service. Split and
/// merge receipts share this path; the leader dispatches on the receipt's magic.
pub(super) fn deliver(backend: &Arc<RuntimeBackend>, handle: &tokio::runtime::Handle, root: RootDigest, receipt: &[u8]) -> Result<String> {
    if backend.driver.status().role == Role::Leader {
        let (task, changed, _) = backend.commit_split_evidence(root, receipt)?;
        return Ok(format!("evidence task {task} {} locally", if changed { "committed" } else { "confirmed" }));
    }
    let context = backend.ttl_context()?;
    let me = backend.node.id;
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
            match kv9_raft::grpc::split_evidence::deliver_on_channel(
                channel,
                root,
                receipt,
                me,
                Some(context.token.clone()),
                DELIVERY_BUDGET,
            )
            .await
            {
                Ok((task, changed, _)) => {
                    return Ok(format!(
                        "evidence task {task} {} at {address}",
                        if changed { "committed" } else { "confirmed" }
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

impl RuntimeBackend {
    /// Publish the split (RANGE-SPLIT §5): one catalog transaction, metadata leader only.
    fn commit_split_publication(&self, root: RootDigest, operation: [u8; 16]) -> Result<(bool, kv9_common::AppliedPosition)> {
        self.ensure_serving()?;
        let _guard = self.node.meta_raft.lock_catalog_txn();
        let term = self.prepare_catalog()?;
        let mut txn = self.node.meta_raft.store.begin()?;
        if kv9_meta::root::certified_root(&txn)?.map(|r| r.digest()) != Some(root) {
            return Err(Error::Config("split publication root differs".into()));
        }
        let (_, changed) = kv9_meta::data_groups::split::plan_split_publication(&mut txn, operation)?;
        let command = if changed {
            Command::from_batch(&txn.into_batch())
        } else {
            Command::Noop
        };
        let applied = self.commit_catalog(&command, term)?;
        Ok((changed, applied))
    }

    /// Commit a kind-114 row from a child's receipt (metadata leader only).
    pub(super) fn commit_split_evidence(&self, root: RootDigest, receipt: &[u8]) -> Result<(u64, bool, kv9_common::AppliedPosition)> {
        self.ensure_serving()?;
        let _guard = self.node.meta_raft.lock_catalog_txn();
        let term = self.prepare_catalog()?;
        let mut txn = self.node.meta_raft.store.begin()?;
        if kv9_meta::root::certified_root(&txn)?.map(|r| r.digest()) != Some(root) {
            return Err(Error::Config("split evidence root differs".into()));
        }
        let (task, changed) = if kv9_meta::data_groups::range_abort::is_seal_receipt(receipt) {
            let (e, changed) = kv9_meta::data_groups::range_abort::plan_seal_evidence(&mut txn, receipt)?;
            (e.task(), changed)
        } else if kv9_meta::data_groups::merge::is_merge_receipt(receipt) {
            let (e, changed) = kv9_meta::data_groups::merge::plan_merge_evidence(&mut txn, receipt)?;
            (e.task(), changed)
        } else {
            let (e, changed) = kv9_meta::data_groups::split_evidence::plan_split_evidence(&mut txn, receipt)?;
            (e.task(), changed)
        };
        let command = if changed {
            Command::from_batch(&txn.into_batch())
        } else {
            Command::Noop
        };
        let applied = self.commit_catalog(&command, term)?;
        Ok((task, changed, applied))
    }
}
