//! Drain a node (docs/DRAIN-NODE.md). A committed kind-112 row names a store
//! that leaves. Two roles act on it, both from the reconcile loop, both with at
//! most one blocking step in flight on a worker thread:
//!
//! - **the drained node itself** hands away every leadership it holds (data
//!   groups first, then the metadata group), repeatedly if re-elected (§2);
//! - **the metadata leader** moves the store's replicas one move plan at a time
//!   (at most `KV9_DRAIN_PARALLEL` unsettled), then takes the node out of the
//!   metadata group without ever leaving fewer than three voters (§3).
//!
//! `drained` is derived from committed state; nothing records completion.

use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use kv9_common::{Error, NodeId, RegionId, Result, RootDigest, META_REGION_0};
use kv9_meta::data_groups::addition::{committed_plans, PlanMode};
use kv9_meta::data_groups::drain::{committed_drains, is_draining, DrainIntent};
use kv9_meta::data_groups::InitialReplica;
use kv9_raft::Role;

use super::{NodeRuntime, RuntimeBackend};
use crate::api::AdminApi;

const EVENT_HISTORY: usize = 24;

#[derive(Debug, Clone, Copy)]
pub(super) struct Config {
    /// `KV9_DRAIN_PARALLEL` (default 1): unsettled move plans per drained store.
    pub(super) parallel: usize,
}

impl Config {
    pub(super) fn from_env() -> Self {
        Self {
            parallel: std::env::var("KV9_DRAIN_PARALLEL")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|n| *n >= 1)
                .unwrap_or(1),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
struct Event {
    at_unix: u64,
    node: u64,
    step: &'static str,
    outcome: String,
}

/// One drain as the last pass derived it (also the status line).
#[derive(Debug, Clone, serde::Serialize)]
struct DrainView {
    task: u64,
    node: u64,
    incarnation: String,
    memberships_left: Vec<u64>,
    plans_unsettled: Vec<u64>,
    meta_role: &'static str,
    finished: bool,
    waiting: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct StatusLine {
    parallel: usize,
    this_store_draining: bool,
    drains: Vec<DrainView>,
    in_flight: Option<String>,
    events: Vec<Event>,
    error: Option<String>,
}

#[derive(Debug)]
pub(super) struct State {
    config: Config,
    views: Vec<DrainView>,
    this_store_draining: bool,
    in_flight: Option<(u64, &'static str, String, std::thread::JoinHandle<Result<String>>)>,
    events: VecDeque<Event>,
    error: Option<String>,
}

impl State {
    pub(super) fn new(config: Config) -> Self {
        Self {
            config,
            views: Vec::new(),
            this_store_draining: false,
            in_flight: None,
            events: VecDeque::new(),
            error: None,
        }
    }

    fn record(&mut self, node: u64, step: &'static str, outcome: String) {
        if self.events.len() >= EVENT_HISTORY {
            self.events.pop_front();
        }
        self.events.push_back(Event {
            at_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            node,
            step,
            outcome,
        });
    }
}

/// What one pass derived: every drain's view and the first step to take.
type Derived = (Vec<DrainView>, Option<(u64, Step)>);

/// One blocking step, run on a worker thread.
enum Step {
    TransferGroup { region: RegionId, transferee: NodeId },
    TransferMetadata { transferee: NodeId },
    MovePlan { root: RootDigest, operation: [u8; 16], creation_task: u64, destination: NodeId, source: NodeId, region: RegionId },
    PromoteLearner { learner: NodeId },
    RemoveVoter { node: NodeId },
    RemoveLearner { node: NodeId },
}

impl Step {
    fn name(&self) -> &'static str {
        match self {
            Step::TransferGroup { .. } => "transfer_group_leader",
            Step::TransferMetadata { .. } => "transfer_metadata_leader",
            Step::MovePlan { .. } => "move_plan",
            Step::PromoteLearner { .. } => "promote_learner",
            Step::RemoveVoter { .. } => "remove_voter",
            Step::RemoveLearner { .. } => "remove_learner",
        }
    }
    fn label(&self) -> String {
        match self {
            Step::TransferGroup { region, transferee } => format!("region {} → node {}", region.0, transferee.0),
            Step::TransferMetadata { transferee } => format!("metadata → node {}", transferee.0),
            Step::MovePlan { region, source, destination, .. } => {
                format!("region {}: node {} → node {}", region.0, source.0, destination.0)
            }
            Step::PromoteLearner { learner } => format!("metadata learner {}", learner.0),
            Step::RemoveVoter { node } | Step::RemoveLearner { node } => format!("metadata member {}", node.0),
        }
    }
}

/// `sha16("kv9-drain-move/v1" ++ drain operation ++ region)`: one plan identity
/// per (drain, group), so a repeated decision confirms.
fn move_operation(drain: &DrainIntent, region: RegionId) -> [u8; 16] {
    let mut bytes = b"kv9-drain-move/v1".to_vec();
    bytes.extend_from_slice(&drain.operation());
    bytes.extend_from_slice(&region.0.to_be_bytes());
    let digest = RootDigest::sha256(&bytes);
    let mut out = [0u8; 16];
    out.copy_from_slice(&digest.as_bytes()[..16]);
    out
}

impl NodeRuntime {
    pub(super) fn reconcile_node_drain(&mut self) {
        // Observe a step an earlier pass started.
        if let Some((node, step, label, handle)) = self.node_drain.in_flight.take() {
            if !handle.is_finished() {
                self.node_drain.in_flight = Some((node, step, label, handle));
                return;
            }
            match handle.join() {
                Ok(Ok(outcome)) => {
                    self.node_drain.error = None;
                    self.node_drain.record(node, step, format!("{label}: {outcome}"));
                }
                Ok(Err(error)) => {
                    self.node_drain.error = Some(format!("{step} {label}: {error}"));
                    self.node_drain.record(node, step, format!("{label}: failed: {error}"));
                }
                Err(_) => {
                    self.node_drain.error = Some(format!("{step} worker panicked"));
                    self.node_drain.record(node, step, format!("{label}: worker panicked"));
                }
            }
        }
        let store = &self.node.meta_raft.store;
        let drains = match committed_drains(store) {
            Ok(drains) => drains,
            Err(_) => return, // no certified root yet
        };
        let me = InitialReplica {
            node: self.node.id,
            incarnation: self.store_identity.store_incarnation,
        };
        self.node_drain.this_store_draining = is_draining(&drains, me);
        if drains.is_empty() {
            self.node_drain.views.clear();
            return;
        }
        // Part A — this store is draining: hand away every leadership it holds.
        if self.node_drain.this_store_draining {
            if let Some(step) = self.handoff_step() {
                self.start(me.node.0, step);
                return;
            }
        }
        // Part B — the metadata leader drives every drain.
        let status = self.driver.status();
        if status.role != Role::Leader {
            self.node_drain.views.clear();
            return;
        }
        match self.plan_drain_steps(&drains, &status) {
            Ok((views, step)) => {
                self.node_drain.views = views;
                if let Some((node, step)) = step {
                    self.start(node, step);
                }
            }
            Err(error) => self.node_drain.error = Some(error.to_string()),
        }
    }

    /// §2: the first leadership this (draining) store still holds, and whom to
    /// hand it to: the caught-up voter with the highest `matched`.
    fn handoff_step(&self) -> Option<Step> {
        let me = self.node.id.0;
        for (creation, status, progress) in self.data_groups.led_groups() {
            let transferee = progress
                .iter()
                .filter(|p| p.voter && p.node.0 != me && status.voters.contains(&p.node.0) && p.recent_active)
                .max_by_key(|p| p.matched)
                .map(|p| p.node);
            if let Some(transferee) = transferee {
                return Some(Step::TransferGroup {
                    region: creation.region(),
                    transferee,
                });
            }
        }
        let status = self.driver.status();
        if status.role == Role::Leader {
            let transferee = self
                .driver
                .peer_progress()
                .iter()
                .filter(|p| p.voter && p.node.0 != me && status.voters.contains(&p.node.0) && p.recent_active)
                .max_by_key(|p| p.matched)
                .map(|p| p.node);
            if let Some(transferee) = transferee {
                return Some(Step::TransferMetadata { transferee });
            }
        }
        None
    }

    /// §3: derive every drain's view from committed state and the first step to
    /// take (one per pass, the oldest drain first).
    fn plan_drain_steps(
        &self,
        drains: &[DrainIntent],
        status: &kv9_raft::driver::NodeStatus,
    ) -> Result<Derived> {
        use kv9_meta::data_groups as dg;
        let store = &self.node.meta_raft.store;
        let txn = store.begin()?;
        let root = kv9_meta::root::certified_root(&txn)?
            .ok_or_else(|| Error::MetaNotReady("drain requires a certified root".into()))?
            .digest();
        let creations = dg::committed_creations(store)?;
        let plans = committed_plans(store)?;
        let aborts = dg::abort::committed_aborts(store)?;
        let evidence = dg::evidence::committed_install_evidence(store)?;
        let removals = dg::removal::committed_removals(store)?;
        let settled = |operation: [u8; 16], mode: PlanMode| {
            aborts.iter().any(|a| a.operation() == operation)
                || (evidence.iter().any(|e| e.operation() == operation)
                    && (mode == PlanMode::Add || removals.iter().any(|r| r.operation() == operation)))
        };
        let unsettled_regions: BTreeSet<u64> = plans
            .iter()
            .filter(|p| !settled(p.operation(), p.mode()))
            .map(|p| p.region().0)
            .collect();
        let mut views = Vec::new();
        let mut step: Option<(u64, Step)> = None;
        let me = self.node.id;
        for drain in drains {
            let store_id = drain.store();
            let node = store_id.node;
            let mut memberships: Vec<(&dg::CommittedCreation, Vec<(InitialReplica, u64)>)> = Vec::new();
            for creation in &creations {
                let members = dg::committed_members(&txn, root, creation.intent())?;
                if members.iter().any(|(m, _)| *m == store_id) {
                    memberships.push((creation, members));
                }
            }
            let unsettled: Vec<u64> = plans
                .iter()
                .filter(|p| p.source() == Some(store_id) && !settled(p.operation(), p.mode()))
                .map(|p| p.task())
                .collect();
            let meta_role = if status.voters.contains(&node.0) {
                "voter"
            } else if status.learners.contains(&node.0) {
                "learner"
            } else {
                "none"
            };
            let mut view = DrainView {
                task: drain.task(),
                node: node.0,
                incarnation: hex(store_id.incarnation.as_bytes()),
                memberships_left: memberships.iter().map(|(c, _)| c.intent().region().0).collect(),
                plans_unsettled: unsettled.clone(),
                meta_role,
                finished: memberships.is_empty() && meta_role == "none",
                waiting: None,
            };
            if view.finished || step.is_some() {
                views.push(view);
                continue;
            }
            if !memberships.is_empty() {
                if unsettled.len() >= self.node_drain.config.parallel {
                    view.waiting = Some(format!("{} move plan(s) in flight", unsettled.len()));
                } else if let Some((creation, members)) = memberships
                    .iter()
                    .find(|(c, _)| !unsettled_regions.contains(&c.intent().region().0))
                {
                    let region = creation.intent().region();
                    let mut exclude: BTreeSet<u64> = members.iter().map(|(m, _)| m.node.0).collect();
                    exclude.insert(node.0);
                    match super::placement::choose_destination(&txn, store, root, &exclude)? {
                        Some(destination) => {
                            step = Some((
                                node.0,
                                Step::MovePlan {
                                    root,
                                    operation: move_operation(drain, region),
                                    creation_task: creation.intent().task(),
                                    destination,
                                    source: node,
                                    region,
                                },
                            ));
                        }
                        None => {
                            view.waiting = Some(format!(
                                "no active, non-draining store outside region {} to move its replica to",
                                region.0
                            ));
                        }
                    }
                } else {
                    view.waiting = Some("every remaining group has another plan in flight".into());
                }
            } else if meta_role == "learner" {
                step = Some((node.0, Step::RemoveLearner { node }));
            } else if meta_role == "voter" {
                if node == me {
                    view.waiting = Some("this leader hands metadata leadership away first".into());
                } else if status.voters.len() > 3 {
                    step = Some((node.0, Step::RemoveVoter { node }));
                } else {
                    let lag = self.voter_replacement_catchup_lag();
                    let candidate = self
                        .driver
                        .peer_progress()
                        .iter()
                        .filter(|p| {
                            !p.voter
                                && status.learners.contains(&p.node.0)
                                && p.node != node
                                && p.recent_active
                                && p.matched.saturating_add(lag) >= status.raft_committed
                        })
                        .max_by_key(|p| p.matched)
                        .map(|p| p.node);
                    match candidate {
                        Some(learner) => step = Some((node.0, Step::PromoteLearner { learner })),
                        None => {
                            view.waiting = Some(
                                "needs a metadata learner: three voters remain; admit and join one more node".into(),
                            )
                        }
                    }
                }
            }
            views.push(view);
        }
        Ok((views, step))
    }

    fn start(&mut self, node: u64, step: Step) {
        let backend = self.rpc_backend.clone();
        let handle = self.grpc_runtime.handle().clone();
        let (name, label) = (step.name(), step.label());
        let worker = std::thread::Builder::new()
            .name("kv9-node-drain".into())
            .spawn(move || execute(&backend, handle, step))
            .expect("spawn drain worker");
        self.node_drain.in_flight = Some((node, name, label, worker));
    }

    /// The `node_drain=` status line.
    pub(super) fn node_drain_status(&self) -> String {
        let state = &self.node_drain;
        serde_json::to_string(&StatusLine {
            parallel: state.config.parallel,
            this_store_draining: state.this_store_draining,
            drains: state.views.clone(),
            in_flight: state.in_flight.as_ref().map(|(_, step, label, _)| format!("{step} {label}")),
            events: state.events.iter().cloned().collect(),
            error: state.error.clone(),
        })
        .expect("serialize node drain status")
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn execute(backend: &Arc<RuntimeBackend>, handle: tokio::runtime::Handle, step: Step) -> Result<String> {
    let _runtime = handle.enter();
    const CALLER: &str = "drain";
    match step {
        Step::TransferGroup { region, transferee } => {
            let r = AdminApi::transfer_leader(backend.as_ref(), CALLER, region, transferee)?;
            Ok(format!("leader now {:?} (completed {})", r.leader, r.completed))
        }
        Step::TransferMetadata { transferee } => {
            let r = AdminApi::transfer_leader(backend.as_ref(), CALLER, META_REGION_0, transferee)?;
            Ok(format!("leader now {:?} (completed {})", r.leader, r.completed))
        }
        Step::MovePlan { root, operation, creation_task, destination, source, .. } => {
            let r = AdminApi::add_replica(
                backend.as_ref(),
                CALLER,
                root,
                operation,
                creation_task,
                destination,
                PlanMode::Move,
                Some(source),
            )?;
            Ok(format!(
                "{} plan {} (migration {})",
                if r.changed { "planned" } else { "confirmed" },
                r.plan.task(),
                r.migration.task()
            ))
        }
        Step::PromoteLearner { learner } => {
            let r = AdminApi::promote_node(backend.as_ref(), CALLER, learner)?;
            Ok(format!("voters now {:?}", r.voters))
        }
        Step::RemoveVoter { node } => {
            let r = AdminApi::remove_node(backend.as_ref(), CALLER, node)?;
            Ok(format!("voters now {:?}", r.voters))
        }
        Step::RemoveLearner { node } => {
            let proposed = backend.driver.remove_voter(node)?;
            let receipt = backend
                .driver
                .wait_conf_applied(proposed, std::time::Duration::from_secs(10))?;
            Ok(format!("learners now {:?}", receipt.learners))
        }
    }
}
