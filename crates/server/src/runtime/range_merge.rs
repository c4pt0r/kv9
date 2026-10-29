//! Range merge (docs/RANGE-SPLIT.md §6): the split pipeline backwards, from a
//! committed merge intent to one published child, with the same three roles and
//! the same discipline as `range_split.rs`:
//!
//! - **the child's leader** installs the child's descriptor `[low.start, high.end)`;
//! - **each parent's leader** seals its parent once the child serves;
//! - **the child's leader** copies the low parent, then the high parent, from the
//!   LOCAL sealed replicas, verifies the child against both halves chained through
//!   one digest, and commits the kind-116 evidence over the shared evidence RPC;
//! - **the metadata leader** publishes: both parent bindings sealed in place, the
//!   child bound, in one transaction.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use kv9_common::data_range::DataRange;
use kv9_common::{Error, RegionId, Result, RootDigest};
use kv9_engine::WalEngine;
use kv9_meta::data_groups::merge::{MergeEvidence, MergeIntent};
use kv9_raft::driver::NodeDriver;
use kv9_raft::storage::DiskRaftStorage;
use kv9_raft::{Command, Role};

use super::range_api::RawGroup;
use super::range_split::{applied_descriptor, deliver, digest_into, populate_delay, printable, range_driver_enabled, scan_page, seal_parent, successor, PAGE};
use super::{NodeRuntime, RuntimeBackend};
use crate::api::{RawApi, RequestContext, RequestOrigin};

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
    /// `(parent index: 0 low / 1 high, last key)`.
    cursor: Option<(u8, String)>,
    done: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
struct IntentView {
    task: u64,
    parents: [u64; 2],
    child: u64,
    parents_sealed_locally: [Option<bool>; 2],
    lead_parents: Vec<u64>,
    lead_child: bool,
    evidenced: bool,
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
    populations: BTreeMap<u64, Population>,
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

enum Outcome {
    Done(String),
    Copied { rows: u64, parent: u8, last: Vec<u8> },
    Evidenced { rows: u64, digest: RootDigest, message: String },
}

enum Step {
    Publish { low_region: RegionId, root: RootDigest, operation: [u8; 16] },
    SealEvidence { region: RegionId, root: RootDigest, operation: [u8; 16], position: kv9_common::AppliedPosition },
    Seal { region: RegionId, driver: Arc<NodeDriver<DiskRaftStorage, WalEngine>>, current: DataRange, root: RootDigest, operation: [u8; 16] },
    Populate(Box<Populate>),
}

struct Populate {
    intent: MergeIntent,
    child_region: RegionId,
    child: Arc<RawGroup>,
    child_range: DataRange,
    parents: [(Arc<WalEngine>, DataRange); 2],
    cursor: Option<(u8, Vec<u8>)>,
}

impl Step {
    fn name(&self) -> &'static str {
        match self {
            Step::Publish { .. } => "publish",
            Step::SealEvidence { .. } => "seal_evidence",
            Step::Seal { .. } => "seal",
            Step::Populate { .. } => "populate",
        }
    }
    fn region(&self) -> u64 {
        match self {
            Step::Publish { low_region, .. } => low_region.0,
            Step::SealEvidence { region, .. } => region.0,
            Step::Seal { region, .. } => region.0,
            Step::Populate(p) => p.child_region.0,
        }
    }
    fn label(&self) -> String {
        match self {
            Step::Publish { low_region, .. } => format!("merge of region {}", low_region.0),
            Step::SealEvidence { region, .. } => format!("seal evidence for region {}", region.0),
            Step::Seal { region, .. } => format!("parent region {}", region.0),
            Step::Populate(p) => format!(
                "child region {} from {}",
                p.child_region.0,
                p.cursor.as_ref().map_or("start".to_string(), |(i, k)| format!("parent {i} after {}", printable(k)))
            ),
        }
    }
}

fn child_descriptor(low: &DataRange, high: &DataRange, child: &kv9_meta::data_groups::CreationIntent) -> DataRange {
    DataRange {
        root: low.root,
        creation: child.digest(),
        region: child.region(),
        keyspace: low.keyspace,
        tenant: low.tenant,
        conf_ver: 1,
        version: 1,
        start: low.start.clone(),
        end: high.end.clone(),
        sealed: false,
    }
}

impl NodeRuntime {
    pub(super) fn reconcile_range_merge(&mut self) {
        if !range_driver_enabled() {
            return;
        }
        if let Some((region, step, label, handle)) = self.range_merge.in_flight.take() {
            if !handle.is_finished() {
                self.range_merge.in_flight = Some((region, step, label, handle));
                return;
            }
            match handle.join() {
                Ok(Ok(Outcome::Done(message))) => {
                    self.range_merge.error = None;
                    self.range_merge.record(region, step, format!("{label}: {message}"));
                }
                Ok(Ok(Outcome::Copied { rows, parent, last })) => {
                    self.range_merge.error = None;
                    let p = self.range_merge.populations.entry(region).or_default();
                    p.rows_copied += rows;
                    p.cursor = Some((parent, printable(&last)));
                }
                Ok(Ok(Outcome::Evidenced { rows, digest, message })) => {
                    self.range_merge.error = None;
                    self.range_merge.populations.entry(region).or_default().done = true;
                    self.range_merge.record(region, step, format!("{label}: {rows} rows, digest {}: {message}", &digest.to_string()[..16]));
                }
                Ok(Err(error)) => {
                    self.range_merge.error = Some(format!("{step} {label}: {error}"));
                    self.range_merge.record(region, step, format!("{label}: failed: {error}"));
                }
                Err(_) => self.range_merge.error = Some(format!("{step} worker panicked")),
            }
        }
        let store = &self.node.meta_raft.store;
        let intents = match kv9_meta::data_groups::merge::committed_merges(store) {
            Ok(intents) => intents,
            Err(error) => {
                self.range_merge.error = Some(format!("merge intents do not read back: {error}"));
                return;
            }
        };
        if intents.is_empty() {
            self.range_merge.views.clear();
            return;
        }
        match self.plan_merge_steps(&intents) {
            Ok((views, step)) => {
                self.range_merge.views = views;
                if let Some(step) = step {
                    self.start_merge_step(step);
                }
            }
            Err(error) => self.range_merge.error = Some(error.to_string()),
        }
    }

    fn plan_merge_steps(&mut self, intents: &[MergeIntent]) -> Result<(Vec<IntentView>, Option<Step>)> {
        use kv9_meta::data_groups as dg;
        let store = &self.node.meta_raft.store;
        let bindings = dg::ranges::committed_ranges(store)?;
        let evidence = dg::merge::committed_merge_evidence(store).unwrap_or_default();
        let aborted = dg::range_abort::effective_aborts(store).unwrap_or_default();
        let seals = dg::range_abort::committed_seal_evidence(store).unwrap_or_default();
        let mut views = Vec::new();
        let mut step: Option<Step> = None;
        for intent in intents {
            if aborted.contains(&intent.operation()) {
                continue;
            }
            let binding = |region: RegionId| bindings.iter().find(|b| b.range().region == region).map(|b| b.range().clone());
            let (Some(low), Some(high)) = (binding(intent.low_region()), binding(intent.high_region())) else { continue };
            if low.sealed && high.sealed {
                continue; // published
            }
            let child_creation = dg::committed_creation(store, intent.child())?
                .ok_or_else(|| Error::MetaNotReady("merge child creation is not applied yet".into()))?;
            let child_region = child_creation.intent().region();
            let child_range = child_descriptor(&low, &high, child_creation.intent());
            let evidenced = evidence.iter().any(|e| e.operation() == intent.operation());
            let parents_local = [self.data_groups.local_parts(intent.low_region()), self.data_groups.local_parts(intent.high_region())];
            let child_local = self.data_groups.local_parts(child_region);
            let mut view = IntentView {
                task: intent.task(),
                parents: [intent.low_region().0, intent.high_region().0],
                child: child_region.0,
                parents_sealed_locally: [None, None],
                lead_parents: Vec::new(),
                lead_child: false,
                evidenced,
                waiting: None,
            };
            for (i, local) in parents_local.iter().enumerate() {
                if let Some((_, engine, driver)) = local {
                    view.parents_sealed_locally[i] = applied_descriptor(engine)?.map(|d| d.sealed);
                    if driver.status().role == Role::Leader {
                        view.lead_parents.push(view.parents[i]);
                    }
                }
            }
            if step.is_some() {
                views.push(view);
                continue;
            }
            // (0) Publish once evidenced.
            if evidenced && self.driver.status().role == Role::Leader {
                step = Some(Step::Publish { low_region: intent.low_region(), root: intent.root(), operation: intent.operation() });
                views.push(view);
                continue;
            }
            // (1) Child descriptor.
            let child_serves = child_local.as_ref().is_some_and(|(_, e, d)| d.status().leader_id.is_some() && applied_descriptor(e).ok().flatten().is_some());
            if let Some((_, engine, driver)) = &child_local {
                let status = driver.status();
                if status.role == Role::Leader {
                    view.lead_child = true;
                    if applied_descriptor(engine)?.is_none() && self.range_merge.descriptor_proposals.get(&child_region.0) != Some(&status.term) {
                        match driver.propose(&Command::DataRange { expected: None, next: child_range.clone() }) {
                            Ok(_) => {
                                self.range_merge.descriptor_proposals.insert(child_region.0, status.term);
                                self.range_merge.record(child_region.0, "child_descriptor", format!("proposed [{}, {}) in term {}", printable(&child_range.start), printable(&child_range.end), status.term));
                            }
                            Err(error) => self.range_merge.error = Some(format!("child descriptor {}: {error}", child_region.0)),
                        }
                    }
                }
            }
            // (2) Seal a parent this node leads, once the child serves.
            for (i, local) in parents_local.iter().enumerate() {
                let Some((_, engine, driver)) = local else { continue };
                if driver.status().role != Role::Leader {
                    continue;
                }
                match applied_descriptor(engine)? {
                    Some(current)
                        if current.sealed
                            && !seals.iter().any(|e| e.operation() == intent.operation() && e.region() == RegionId(view.parents[i])) =>
                    {
                        let status = driver.status();
                        step = Some(Step::SealEvidence {
                            region: RegionId(view.parents[i]),
                            root: intent.root(),
                            operation: intent.operation(),
                            position: kv9_common::AppliedPosition { term: status.term, index: status.applied_index },
                        });
                        break;
                    }
                    Some(current) if !current.sealed => {
                        if child_serves {
                            step = Some(Step::Seal { region: RegionId(view.parents[i]), driver: driver.clone(), current, root: intent.root(), operation: intent.operation() });
                            break;
                        }
                        view.waiting = Some("waiting for the merged child to serve behind a leader with its descriptor".into());
                    }
                    Some(_) => {}
                    None => view.waiting = Some(format!("parent {} has no applied descriptor", view.parents[i])),
                }
            }
            if step.is_some() {
                views.push(view);
                continue;
            }
            // (3) Copy both sealed parents into the child this node leads.
            let both_sealed_here = view.parents_sealed_locally == [Some(true), Some(true)];
            if view.lead_child && !evidenced && both_sealed_here && !self.range_merge.populations.get(&child_region.0).is_some_and(|p| p.done) {
                let (_, engine, driver) = child_local.as_ref().expect("lead child implies local");
                if applied_descriptor(engine)?.as_ref() == Some(&child_range) {
                    let child = RawGroup::new(child_range.clone(), engine.clone(), driver.clone(), self.rpc_backend.raw_directory.clock());
                    let parents = [
                        (parents_local[0].as_ref().expect("sealed locally").1.clone(), low.clone()),
                        (parents_local[1].as_ref().expect("sealed locally").1.clone(), high.clone()),
                    ];
                    let cursor = self
                        .range_merge
                        .populations
                        .get(&child_region.0)
                        .and_then(|p| p.cursor.as_ref().map(|(i, k)| (*i, k.as_bytes().to_vec())));
                    step = Some(Step::Populate(Box::new(Populate { intent: intent.clone(), child_region, child, child_range: child_range.clone(), parents, cursor })));
                }
            } else if view.lead_child && !evidenced && view.waiting.is_none() {
                view.waiting = Some("waiting for both local parent replicas to apply their seals".into());
            }
            views.push(view);
        }
        Ok((views, step))
    }

    fn start_merge_step(&mut self, step: Step) {
        let backend = self.rpc_backend.clone();
        let handle = self.grpc_runtime.handle().clone();
        let (name, region, label) = (step.name(), step.region(), step.label());
        let worker = std::thread::Builder::new()
            .name("kv9-range-merge".into())
            .spawn(move || execute(&backend, handle, step))
            .expect("spawn merge worker");
        self.range_merge.in_flight = Some((region, name, label, worker));
    }

    pub(super) fn range_merge_status(&self) -> String {
        let state = &self.range_merge;
        serde_json::to_string(&StatusLine {
            intents: state.views.clone(),
            populations: state.populations.clone(),
            in_flight: state.in_flight.as_ref().map(|(_, step, label, _)| format!("{step} {label}")),
            events: state.events.iter().cloned().collect(),
            error: state.error.clone(),
        })
        .expect("serialize range merge status")
    }
}

fn execute(backend: &Arc<RuntimeBackend>, handle: tokio::runtime::Handle, step: Step) -> Result<Outcome> {
    let name = step.name();
    super::fault::point(&format!("merge:{name}:before"));
    let out = execute_step(backend, handle, step);
    if out.is_ok() {
        super::fault::point(&format!("merge:{name}:after"));
    }
    out
}

fn execute_step(backend: &Arc<RuntimeBackend>, handle: tokio::runtime::Handle, step: Step) -> Result<Outcome> {
    let _runtime = handle.enter();
    match step {
        Step::SealEvidence { region, root, operation, position } => {
            let receipt = kv9_meta::data_groups::range_abort::SealEvidence::receipt(root, operation, region, position)?;
            let message = deliver(backend, &handle, root, &receipt)?;
            Ok(Outcome::Done(format!("seal evidence recovered: {message}")))
        }
        Step::Publish { root, operation, .. } => {
            let (changed, applied) = backend.commit_merge_publication(root, operation)?;
            Ok(Outcome::Done(format!("{} at {}/{}", if changed { "published" } else { "confirmed" }, applied.term, applied.index)))
        }
        Step::Seal { region, driver, current, root, operation } => {
            match seal_parent(backend, &handle, region, &driver, &current, root, operation)? {
                super::range_split::Outcome::Sealed(message) => Ok(Outcome::Done(message)),
                _ => Err(Error::Config("unexpected seal outcome".into())),
            }
        }
        Step::Populate(p) => {
            let Populate { intent, child, child_range, parents, cursor, .. } = *p;
            let ctx = RequestContext {
                keyspace: child_range.keyspace,
                region_epoch: kv9_region::RegionEpoch { conf_ver: 1, version: 1 },
                origin: RequestOrigin::from_transport("range-merge"),
            };
            // Copy the low parent to its end, then the high parent.
            let (mut index, mut from) = match &cursor {
                None => (0u8, parents[0].1.start.clone()),
                Some((i, k)) => (*i, successor(k)),
            };
            loop {
                let (engine, range) = &parents[index as usize];
                let pairs = scan_page(engine, range.keyspace, &from, &range.end, PAGE)?;
                if !pairs.is_empty() {
                    populate_delay();
                    child.raw_batch_put(&ctx, &pairs)?;
                    let last = pairs.last().map(|(k, _)| k.clone()).expect("non-empty");
                    return Ok(Outcome::Copied { rows: pairs.len() as u64, parent: index, last });
                }
                if index == 0 {
                    index = 1;
                    from = parents[1].1.start.clone();
                    continue;
                }
                break;
            }
            // Verify: both parents chained through one digest == the child's whole range.
            let mut hasher = Vec::new();
            let mut parent_rows = 0u64;
            for (engine, range) in &parents {
                parent_rows += digest_into(&mut hasher, |from| scan_page(engine, range.keyspace, from, &range.end, PAGE), &range.start)?;
            }
            let parent_digest = RootDigest::sha256(&hasher);
            let mut hasher = Vec::new();
            let child_rows = digest_into(&mut hasher, |from| child.raw_scan(&ctx, from, &child_range.end, PAGE), &child_range.start)?;
            let child_digest = RootDigest::sha256(&hasher);
            if parent_digest != child_digest || parent_rows != child_rows {
                return Err(Error::Config(format!("merged child rows differ from the sealed parents ({child_rows} vs {parent_rows} rows)")));
            }
            let receipt = MergeEvidence::receipt(&intent, child_digest, child_rows)?;
            let message = deliver(backend, &handle, intent.root(), &receipt)?;
            Ok(Outcome::Evidenced { rows: child_rows, digest: child_digest, message })
        }
    }
}

impl RuntimeBackend {
    fn commit_merge_publication(&self, root: RootDigest, operation: [u8; 16]) -> Result<(bool, kv9_common::AppliedPosition)> {
        self.ensure_serving()?;
        let _guard = self.node.meta_raft.lock_catalog_txn();
        let term = self.prepare_catalog()?;
        let mut txn = self.node.meta_raft.store.begin()?;
        if kv9_meta::root::certified_root(&txn)?.map(|r| r.digest()) != Some(root) {
            return Err(Error::Config("merge publication root differs".into()));
        }
        let (_, changed) = kv9_meta::data_groups::merge::plan_merge_publication(&mut txn, operation)?;
        let command = if changed { Command::from_batch(&txn.into_batch()) } else { Command::Noop };
        let applied = self.commit_catalog(&command, term)?;
        Ok((changed, applied))
    }
}
