//! The automatic split trigger (docs/RANGE-SPLIT.md §11), off unless
//! `KV9_AUTO_SPLIT_BYTES > 0`. Each data group's LEADER measures its own replica's
//! on-disk footprint (`data.wal` plus `data.segments/`, an honest write-volume
//! signal that counts overwrites) for every bound, unsealed range it leads; on a
//! breach it samples the median user key from a snapshot and commits one split
//! intent — through `SplitRange` locally when it leads the metadata group, else
//! through the node-authenticated `PlanSplit`. The intent's operation is derived
//! from `(root, region, bind task)`, so a region splits once per binding and any
//! repeated decision confirms; the planner's rules (no live split or merge on the
//! parent) and `KV9_AUTO_SPLIT_COOLDOWN_SECS` keep it from storming. The seal
//! window (§9) is this trigger's cost: it is deliberately off by default.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kv9_common::{Error, RegionId, Result, RootDigest};
use kv9_engine::Engine;
use kv9_raft::Role;
use kv9_raw::{LeaderRead, RawExecutor};

use super::{NodeRuntime, RuntimeBackend};
use crate::api::AdminApi;

const PROBE_EVERY: Duration = Duration::from_secs(5);
const DELIVERY_BUDGET: Duration = Duration::from_secs(10);
const PAGE: usize = 1024;
const EVENT_HISTORY: usize = 16;

#[derive(Debug, Clone, Copy)]
pub(super) struct Config {
    /// `KV9_AUTO_SPLIT_BYTES` (0 = off): the replica footprint that triggers a split.
    pub(super) bytes: u64,
    /// `KV9_AUTO_SPLIT_COOLDOWN_SECS` (600): spacing between two decisions on this node.
    pub(super) cooldown: Duration,
}

impl Config {
    pub(super) fn from_env() -> Self {
        let number = |name: &str, default: u64| {
            std::env::var(name)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        };
        Self {
            bytes: number("KV9_AUTO_SPLIT_BYTES", 0),
            cooldown: Duration::from_secs(number("KV9_AUTO_SPLIT_COOLDOWN_SECS", 600)),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
struct Event {
    at_unix: u64,
    region: u64,
    bytes: u64,
    outcome: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct GroupView {
    region: u64,
    bytes: u64,
    bound: bool,
    sealed: bool,
    eligible: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
struct StatusLine {
    enabled: bool,
    threshold_bytes: u64,
    groups: Vec<GroupView>,
    in_flight: Option<u64>,
    events: Vec<Event>,
    error: Option<String>,
}

#[derive(Debug)]
pub(super) struct State {
    config: Config,
    next_probe: Instant,
    last_decision: Option<Instant>,
    views: Vec<GroupView>,
    in_flight: Option<(u64, u64, std::thread::JoinHandle<Result<String>>)>,
    events: VecDeque<Event>,
    error: Option<String>,
}

impl State {
    pub(super) fn new(config: Config) -> Self {
        Self {
            config,
            next_probe: Instant::now(),
            last_decision: None,
            views: Vec::new(),
            in_flight: None,
            events: VecDeque::new(),
            error: None,
        }
    }
    fn record(&mut self, region: u64, bytes: u64, outcome: String) {
        if self.events.len() >= EVENT_HISTORY {
            self.events.pop_front();
        }
        self.events.push_back(Event {
            at_unix: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
            region,
            bytes,
            outcome,
        });
    }
}

struct Decision {
    root: RootDigest,
    operation: [u8; 16],
    region: RegionId,
    split_key: Vec<u8>,
    bytes: u64,
}

/// `sha16("kv9-auto-split/v1" ++ root ++ region ++ bind task)`.
fn auto_operation(root: RootDigest, region: RegionId, bind_task: u64) -> [u8; 16] {
    let mut bytes = b"kv9-auto-split/v1".to_vec();
    bytes.extend_from_slice(root.as_bytes());
    bytes.extend_from_slice(&region.0.to_be_bytes());
    bytes.extend_from_slice(&bind_task.to_be_bytes());
    let digest = RootDigest::sha256(&bytes);
    let mut out = [0u8; 16];
    out.copy_from_slice(&digest.as_bytes()[..16]);
    out
}

/// `data.wal` plus everything under `data.segments/` (recursively), next to the
/// engine's WAL.
fn footprint(wal_path: &std::path::Path) -> u64 {
    fn walk(dir: &std::path::Path, total: &mut u64, depth: usize) {
        if depth > 8 {
            return;
        }
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                match entry.metadata() {
                    Ok(m) if m.is_dir() => walk(&entry.path(), total, depth + 1),
                    Ok(m) => *total += m.len(),
                    Err(_) => {}
                }
            }
        }
    }
    let mut total = std::fs::metadata(wal_path).map(|m| m.len()).unwrap_or(0);
    walk(&wal_path.with_file_name("data.segments"), &mut total, 0);
    total
}

impl NodeRuntime {
    pub(super) fn reconcile_auto_split(&mut self) {
        if !super::range_split::range_driver_enabled() {
            return;
        }
        if let Some((region, bytes, handle)) = self.auto_split.in_flight.take() {
            if !handle.is_finished() {
                self.auto_split.in_flight = Some((region, bytes, handle));
                return;
            }
            match handle.join() {
                Ok(Ok(outcome)) => {
                    self.auto_split.error = None;
                    self.auto_split.record(region, bytes, outcome);
                }
                Ok(Err(error)) => {
                    self.auto_split.error = Some(format!("region {region}: {error}"));
                    self.auto_split.record(region, bytes, format!("failed: {error}"));
                }
                Err(_) => self.auto_split.error = Some("auto-split worker panicked".into()),
            }
        }
        let now = Instant::now();
        if now < self.auto_split.next_probe {
            return;
        }
        self.auto_split.next_probe = now + PROBE_EVERY;
        match self.plan_auto_split(now) {
            Ok((views, decision)) => {
                self.auto_split.views = views;
                if let Some(d) = decision {
                    let backend = self.rpc_backend.clone();
                    let handle = self.grpc_runtime.handle().clone();
                    let (region, bytes) = (d.region.0, d.bytes);
                    let worker = std::thread::Builder::new()
                        .name("kv9-auto-split".into())
                        .spawn(move || commit(&backend, handle, d))
                        .expect("spawn auto-split worker");
                    self.auto_split.in_flight = Some((region, bytes, worker));
                    self.auto_split.last_decision = Some(now);
                }
            }
            Err(error) => self.auto_split.error = Some(error.to_string()),
        }
    }

    fn plan_auto_split(&mut self, now: Instant) -> Result<(Vec<GroupView>, Option<Decision>)> {
        use kv9_meta::data_groups as dg;
        let store = &self.node.meta_raft.store;
        let bindings = dg::ranges::committed_ranges(store)?;
        let splits = dg::split::committed_splits(store).unwrap_or_default();
        let merges = dg::merge::committed_merges(store).unwrap_or_default();
        let plans = dg::addition::committed_plans(store).unwrap_or_default();
        let aborts = dg::abort::committed_aborts(store).unwrap_or_default();
        let evidence = dg::evidence::committed_install_evidence(store).unwrap_or_default();
        let removals = dg::removal::committed_removals(store).unwrap_or_default();
        let root = kv9_meta::root::certified_root(&store.begin()?)?
            .ok_or_else(|| Error::MetaNotReady("auto-split requires a certified root".into()))?
            .digest();
        let enabled = self.auto_split.config.bytes > 0;
        let cooled = self
            .auto_split
            .last_decision
            .is_none_or(|last| now.duration_since(last) >= self.auto_split.config.cooldown);
        let mut views = Vec::new();
        let mut decision = None;
        for (creation, status, _) in self.data_groups.led_groups() {
            let region = creation.region();
            let Some((_, engine, driver)) = self.data_groups.local_parts(region) else { continue };
            let bytes = footprint(&engine.path());
            let binding = bindings.iter().find(|b| b.range().region == region).map(|b| (b.range().clone(), b.bind_task()));
            let (bound, sealed) = binding.as_ref().map_or((false, false), |(r, _)| (true, r.sealed));
            let busy = splits.iter().any(|s| s.parent_region() == region)
                || merges.iter().any(|m| m.low_region() == region || m.high_region() == region)
                || plans.iter().any(|p| {
                    p.region() == region
                        && !(aborts.iter().any(|a| a.operation() == p.operation())
                            || (evidence.iter().any(|e| e.operation() == p.operation())
                                && (p.mode() == dg::addition::PlanMode::Add
                                    || removals.iter().any(|r| r.operation() == p.operation()))))
                });
            let eligible = bound && !sealed && !busy && status.role == Role::Leader;
            views.push(GroupView { region: region.0, bytes, bound, sealed, eligible });
            if !enabled || !cooled || !eligible || bytes < self.auto_split.config.bytes || decision.is_some() {
                continue;
            }
            let (range, bind_task) = binding.expect("bound");
            // The median user key of the range, from one snapshot: strictly inside.
            let snapshot = engine.snapshot()?;
            let read = LeaderRead::new(snapshot.as_ref(), true, None)?;
            let mut keys: Vec<Vec<u8>> = Vec::new();
            let mut from = range.start.clone();
            let mut first = true;
            loop {
                let start = if first { from.clone() } else { super::range_split::successor(&from) };
                first = false;
                let page = RawExecutor.scan(&read, range.keyspace, &start, &range.end, PAGE)?;
                if page.is_empty() {
                    break;
                }
                from = page.last().map(|(k, _)| k.clone()).expect("non-empty");
                keys.extend(page.into_iter().map(|(k, _)| k));
                if keys.len() >= 1 << 20 {
                    break; // a bounded sample is a fine median for this purpose
                }
            }
            if keys.len() < 2 {
                continue;
            }
            let candidate = keys[keys.len() / 2].clone();
            let inside = (range.start.is_empty() || candidate > range.start) && (range.end.is_empty() || candidate < range.end);
            if !inside {
                continue;
            }
            let _ = driver;
            decision = Some(Decision {
                root,
                operation: auto_operation(root, region, bind_task),
                region,
                split_key: candidate,
                bytes,
            });
        }
        Ok((views, decision))
    }

    pub(super) fn auto_split_status(&self) -> String {
        let state = &self.auto_split;
        serde_json::to_string(&StatusLine {
            enabled: state.config.bytes > 0,
            threshold_bytes: state.config.bytes,
            groups: state.views.clone(),
            in_flight: state.in_flight.as_ref().map(|(region, _, _)| *region),
            events: state.events.iter().cloned().collect(),
            error: state.error.clone(),
        })
        .expect("serialize auto split status")
    }
}

fn commit(backend: &Arc<RuntimeBackend>, handle: tokio::runtime::Handle, d: Decision) -> Result<String> {
    let _runtime = handle.enter();
    let shown = String::from_utf8_lossy(&d.split_key).into_owned();
    if backend.driver.status().role == Role::Leader {
        let r = AdminApi::split_range(backend.as_ref(), "auto-split", d.root, d.operation, d.region, &d.split_key)?;
        return Ok(format!(
            "{} intent {} at {shown} (children {} / {}) locally",
            if r.changed { "committed" } else { "confirmed" },
            r.intent.task(),
            r.child_low.task(),
            r.child_high.task()
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
            match kv9_raft::grpc::plan_split::deliver_on_channel(
                channel,
                d.root,
                d.operation,
                d.region,
                &d.split_key,
                me,
                Some(context.token.clone()),
                DELIVERY_BUDGET,
            )
            .await
            {
                Ok((intent, low, high, changed)) => {
                    return Ok(format!(
                        "{} intent {intent} at {shown} (children {low} / {high}) at {address}",
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
