//! Automatic data-group repair (docs/REPLICA-REPAIR.md): each data group's leader
//! keeps the voter verdict `voter_replacement.rs` keeps for the metadata group — a
//! voter silent for `KV9_REPLICA_LOST_SECS` is **lost** — and, with
//! `KV9_AUTO_REPAIR_REPLICA=1`, repairs the loss by committing one **move plan**
//! (lost voter → the least-loaded active store outside the group) that the
//! replica-plan driver then carries out: promote the replacement first, remove the
//! lost voter second. The decision is a derived, idempotent row; it is committed
//! locally when this node leads the metadata group and over the node-authenticated
//! internal service otherwise, on a worker thread, never on the reconcile thread.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kv9_common::{Error, NodeId, RegionId, Result, RootDigest};
use kv9_meta::data_groups::addition::PlanMode;
use kv9_meta::data_groups::InitialReplica;
use kv9_raft::Role;

use crate::api::AdminApi;
use super::{NodeRuntime, RuntimeBackend};

const EVENT_HISTORY: usize = 16;
const DELIVERY_BUDGET: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy)]
pub(super) struct Config {
    pub(super) enabled: bool,
    pub(super) lost_after: Duration,
    pub(super) cooldown: Duration,
}

impl Config {
    pub(super) fn from_env() -> Self {
        let secs = |name: &str, default: u64| {
            std::env::var(name)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        };
        Self {
            enabled: std::env::var("KV9_AUTO_REPAIR_REPLICA").as_deref() == Ok("1"),
            lost_after: Duration::from_secs(secs("KV9_REPLICA_LOST_SECS", 300)),
            cooldown: Duration::from_secs(secs("KV9_REPLICA_REPAIR_COOLDOWN_SECS", 600)),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Observation {
    last_active: Instant,
    matched: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
struct Event {
    at_unix: u64,
    region: u64,
    lost: u64,
    destination: u64,
    outcome: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct VoterHealth {
    node: u64,
    state: &'static str,
    inactive_secs: u64,
    matched: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
struct GroupLine {
    region: u64,
    voters: Vec<VoterHealth>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct StatusLine {
    enabled: bool,
    lost_after_secs: u64,
    groups: Vec<GroupLine>,
    in_flight: Option<String>,
    events: Vec<Event>,
    error: Option<String>,
}

/// One decision, carried to the worker thread.
struct Decision {
    creation_task: u64,
    root: RootDigest,
    operation: [u8; 16],
    lost: InitialReplica,
    destination: NodeId,
}

#[derive(Debug)]
pub(super) struct State {
    config: Config,
    /// This leader's last observation of every other voter, per led group.
    observed: BTreeMap<(RegionId, u64), Observation>,
    last_decision: BTreeMap<RegionId, Instant>,
    in_flight: Option<(RegionId, u64, u64, std::thread::JoinHandle<Result<String>>)>,
    events: VecDeque<Event>,
    error: Option<String>,
}

impl State {
    pub(super) fn new(config: Config) -> Self {
        Self {
            config,
            observed: BTreeMap::new(),
            last_decision: BTreeMap::new(),
            in_flight: None,
            events: VecDeque::new(),
            error: None,
        }
    }

    fn record(&mut self, region: RegionId, lost: u64, destination: u64, outcome: String) {
        if self.events.len() >= EVENT_HISTORY {
            self.events.pop_front();
        }
        self.events.push_back(Event {
            at_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            region: region.0,
            lost,
            destination,
            outcome,
        });
    }
}

/// `sha16("kv9-replica-repair/v1" ++ root ++ region ++ lost incarnation)`: one
/// identity per loss, so concurrent or repeated decisions confirm one row.
fn repair_operation(root: RootDigest, region: RegionId, lost: &InitialReplica) -> [u8; 16] {
    let mut bytes = b"kv9-replica-repair/v1".to_vec();
    bytes.extend_from_slice(root.as_bytes());
    bytes.extend_from_slice(&region.0.to_be_bytes());
    bytes.extend_from_slice(lost.incarnation.as_bytes());
    let digest = RootDigest::sha256(&bytes);
    let mut out = [0u8; 16];
    out.copy_from_slice(&digest.as_bytes()[..16]);
    out
}

impl NodeRuntime {
    /// One pass of the data-group repair verdict; called from the reconcile loop.
    pub(super) fn reconcile_replica_repair(&mut self) {
        // Observe a decision an earlier pass started.
        if let Some((region, lost, destination, handle)) = self.replica_repair.in_flight.take() {
            if !handle.is_finished() {
                self.replica_repair.in_flight = Some((region, lost, destination, handle));
                return;
            }
            match handle.join() {
                Ok(Ok(outcome)) => {
                    self.replica_repair.error = None;
                    self.replica_repair.record(region, lost, destination, outcome);
                }
                Ok(Err(error)) => {
                    self.replica_repair.error = Some(format!("region {}: {error}", region.0));
                    self.replica_repair.record(region, lost, destination, format!("failed: {error}"));
                }
                Err(_) => {
                    self.replica_repair.error = Some("repair worker panicked".into());
                    self.replica_repair.record(region, lost, destination, "worker panicked".into());
                }
            }
        }
        let now = Instant::now();
        let me = self.node.id.0;
        let led = self.data_groups.led_groups();
        // Observe every other voter of every group this node leads.
        let (enabled, lost_after, cooldown) = {
            let state = &mut self.replica_repair;
            let mut current = BTreeSet::new();
            for (creation, _status, progress) in &led {
                for p in progress {
                    if !p.voter || p.node.0 == me {
                        continue;
                    }
                    let key = (creation.region(), p.node.0);
                    current.insert(key);
                    let entry = state.observed.entry(key).or_insert(Observation {
                        last_active: now,
                        matched: p.matched,
                    });
                    if p.recent_active || p.matched > entry.matched {
                        entry.last_active = now;
                    }
                    entry.matched = p.matched;
                }
            }
            state.observed.retain(|key, _| current.contains(key));
            (state.config.enabled, state.config.lost_after, state.config.cooldown)
        };
        if !enabled {
            return;
        }
        for (creation, status, _) in &led {
            let region = creation.region();
            let lost: Vec<u64> = self
                .replica_repair
                .observed
                .iter()
                .filter(|((r, _), o)| *r == region && now.duration_since(o.last_active) >= lost_after)
                .map(|((_, node), _)| *node)
                .collect();
            if lost.is_empty() {
                continue;
            }
            if self
                .replica_repair
                .last_decision
                .get(&region)
                .is_some_and(|at| now.duration_since(*at) < cooldown)
            {
                continue;
            }
            // The live majority must hold after promotion, as for the metadata group.
            let voters = status.voters.len();
            let live = voters.saturating_sub(lost.len());
            if live + 1 < voters.div_ceil(2) + 1 {
                continue;
            }
            match self.plan_repair(creation, lost[0]) {
                Ok(Some(decision)) => {
                    let backend = self.rpc_backend.clone();
                    let handle = self.grpc_runtime.handle().clone();
                    let (lost_node, destination) = (decision.lost.node.0, decision.destination.0);
                    let worker = std::thread::Builder::new()
                        .name("kv9-replica-repair".into())
                        .spawn(move || commit_decision(&backend, handle, decision))
                        .expect("spawn repair worker");
                    self.replica_repair.in_flight = Some((region, lost_node, destination, worker));
                    self.replica_repair.last_decision.insert(region, now);
                    return; // one decision per pass
                }
                Ok(None) => {}
                Err(error) => self.replica_repair.error = Some(format!("region {}: {error}", region.0)),
            }
        }
    }

    /// The decision for one lost voter, from committed state: the exact lost
    /// incarnation and the least-loaded active store outside the group.
    fn plan_repair(
        &mut self,
        creation: &kv9_meta::data_groups::CreationIntent,
        lost_node: u64,
    ) -> Result<Option<Decision>> {
        let store = &self.node.meta_raft.store;
        let txn = store.begin()?;
        let certified = kv9_meta::root::certified_root(&txn)?
            .ok_or_else(|| Error::MetaNotReady("repair requires a certified root".into()))?;
        let root = certified.digest();
        let members = kv9_meta::data_groups::committed_members(&txn, root, creation)?;
        let Some((lost, _)) = members.iter().find(|(m, _)| m.node.0 == lost_node).copied() else {
            // Already moved by an earlier plan (or an operator): nothing to decide.
            return Ok(None);
        };
        let operation = repair_operation(root, creation.region(), &lost);
        // One row per loss: while the plan for this exact incarnation exists —
        // in flight, or aborted by an operator — there is nothing new to commit.
        if kv9_meta::data_groups::addition::committed_plans(store)?
            .iter()
            .any(|plan| plan.operation() == operation)
        {
            return Ok(None);
        }
        // Destination: the least-loaded active store outside the group that is
        // not draining and not the lost node (runtime/placement.rs).
        let mut exclude = super::placement::never_again(&txn, store, root, creation)?;
        exclude.insert(lost_node);
        let Some(destination) = super::placement::choose_destination(&txn, store, root, &exclude)? else {
            self.replica_repair.error = Some(format!(
                "region {}: voter {} lost and no active store outside the group to move it to",
                creation.region().0,
                lost_node
            ));
            return Ok(None);
        };
        Ok(Some(Decision {
            creation_task: creation.task(),
            root,
            operation,
            lost,
            destination,
        }))
    }

    /// The `replica_repair=` status line: the verdicts of every group this node leads.
    pub(super) fn replica_repair_status(&self) -> String {
        let state = &self.replica_repair;
        let now = Instant::now();
        let suspect_after = state.config.lost_after / 4;
        let mut groups: BTreeMap<u64, Vec<VoterHealth>> = BTreeMap::new();
        for ((region, node), o) in &state.observed {
            let inactive = now.duration_since(o.last_active);
            groups.entry(region.0).or_default().push(VoterHealth {
                node: *node,
                state: if inactive >= state.config.lost_after {
                    "lost"
                } else if inactive >= suspect_after {
                    "suspected"
                } else {
                    "active"
                },
                inactive_secs: inactive.as_secs(),
                matched: o.matched,
            });
        }
        serde_json::to_string(&StatusLine {
            enabled: state.config.enabled,
            lost_after_secs: state.config.lost_after.as_secs(),
            groups: groups.into_iter().map(|(region, voters)| GroupLine { region, voters }).collect(),
            in_flight: state
                .in_flight
                .as_ref()
                .map(|(region, lost, destination, _)| format!("region {} lost {lost} → {destination}", region.0)),
            events: state.events.iter().cloned().collect(),
            error: state.error.clone(),
        })
        .expect("serialize replica repair status")
    }
}

/// Commit the move plan: locally when this node leads the metadata group, else
/// through the node-authenticated internal service at a metadata candidate.
fn commit_decision(backend: &Arc<RuntimeBackend>, handle: tokio::runtime::Handle, d: Decision) -> Result<String> {
    let _runtime = handle.enter();
    if backend.driver.status().role == Role::Leader {
        let r = AdminApi::add_replica(
            backend.as_ref(),
            "repair",
            d.root,
            d.operation,
            d.creation_task,
            d.destination,
            PlanMode::Move,
            Some(d.lost.node),
        )?;
        return Ok(format!(
            "{} plan {} (migration {}) moving node {} → {} locally",
            if r.changed { "planned" } else { "confirmed" },
            r.plan.task(),
            r.migration.task(),
            d.lost.node.0,
            d.destination.0
        ));
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
            match kv9_raft::grpc::plan_replica::deliver_on_channel(
                channel,
                d.root,
                d.operation,
                d.creation_task,
                d.destination,
                "move",
                Some(d.lost.node),
                me,
                Some(context.token.clone()),
                DELIVERY_BUDGET,
            )
            .await
            {
                Ok((plan, migration, changed)) => {
                    return Ok(format!(
                        "{} plan {plan} (migration {migration}) moving node {} → {} at {address}",
                        if changed { "planned" } else { "confirmed" },
                        d.lost.node.0,
                        d.destination.0
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
