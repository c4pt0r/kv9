//! Automatic voter replacement for the metadata group while its quorum holds
//! (docs/VOTER-REPLACEMENT.md, phase 1).
//!
//! The leader is the only node whose view of a voter is both local and authoritative,
//! so the verdict comes from Raft progress: a voter that has shown no activity (no
//! response, no matched advance) for `KV9_VOTER_LOST_SECS` is **lost**. With
//! `KV9_AUTO_REPLACE_VOTER=1` the leader then repairs the group in the one order that
//! never risks the quorum — promote a caught-up learner first (`{A,B,X} → {A,B,X,R}`,
//! three live voters of four), remove the lost voter second (`→ {A,B,R}`). Each step is
//! one committed configuration change proposed from this pass and confirmed on a later
//! one; the pass never blocks on apply. The Raft configuration itself is the committed
//! record: a new leader re-derives the step from `voters`, `learners` and what it
//! observes, so a coordinator that dies between the two steps is resumed by whoever
//! leads next (an extra voter plus a lost voter means "remove"; a lost voter in a
//! baseline-sized set means "promote").
//!
//! Off by default: the verdict and the status line (`voter_replacement=`) ship either
//! way; decisions need the flag. A partitioned-but-alive voter looks lost; the cost of
//! that false verdict is bounded by design — it is removed and rejoins through the
//! ordinary learner path, and its log is never truncated on the strength of the verdict.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kv9_common::NodeId;
use kv9_raft::{ProposedAt, Role};

use super::NodeRuntime;

/// Fewer voters than this are never produced by a replacement step.
const MIN_VOTERS: usize = 3;
/// A proposed configuration change that has not applied within this long is
/// forgotten (raft will still apply it; the next pass re-derives the state).
const PENDING_DEADLINE: Duration = Duration::from_secs(30);
const EVENT_HISTORY: usize = 8;

#[derive(Debug, Clone, Copy)]
pub(super) struct Config {
    /// `KV9_AUTO_REPLACE_VOTER=1`: take decisions, not only verdicts.
    pub(super) enabled: bool,
    /// `KV9_VOTER_LOST_SECS` (default 300): inactivity after which a voter is lost.
    pub(super) lost_after: Duration,
    /// `KV9_VOTER_REPLACE_COOLDOWN_SECS` (default 600): minimum spacing between two
    /// promotions decided here, so that a flapping network cannot churn the voter set.
    pub(super) cooldown: Duration,
    /// How far (in log entries) a learner may trail the leader's commit index and still
    /// be promoted. The `AddNode` entry itself must replicate to it before it applies.
    pub(super) catchup_lag: u64,
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
            enabled: std::env::var("KV9_AUTO_REPLACE_VOTER").as_deref() == Ok("1"),
            lost_after: Duration::from_secs(secs("KV9_VOTER_LOST_SECS", 300)),
            cooldown: Duration::from_secs(secs("KV9_VOTER_REPLACE_COOLDOWN_SECS", 600)),
            catchup_lag: 1024,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Observation {
    last_active: Instant,
    matched: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Promote,
    Remove,
}

impl Step {
    fn label(self) -> &'static str {
        match self {
            Step::Promote => "promote",
            Step::Remove => "remove",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Pending {
    step: Step,
    node: u64,
    at: ProposedAt,
    since: Instant,
}

#[derive(Debug, Clone, serde::Serialize)]
struct Event {
    at_unix: u64,
    step: &'static str,
    node: u64,
    lost: Vec<u64>,
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
struct StatusLine {
    enabled: bool,
    lost_after_secs: u64,
    baseline_voters: usize,
    voters: Vec<VoterHealth>,
    pending: Option<String>,
    events: Vec<Event>,
    error: Option<String>,
}

#[derive(Debug)]
pub(super) struct State {
    config: Config,
    /// This leader's last observation of every other voter.
    observed: BTreeMap<u64, Observation>,
    /// The voter count the group is meant to have: the last count seen with no voter
    /// lost, never below three. A larger live set means a promotion already happened.
    baseline_voters: usize,
    pending: Option<Pending>,
    last_promotion: Option<Instant>,
    events: VecDeque<Event>,
    error: Option<String>,
}

impl State {
    pub(super) fn new(config: Config) -> Self {
        Self {
            config,
            observed: BTreeMap::new(),
            baseline_voters: MIN_VOTERS,
            pending: None,
            last_promotion: None,
            events: VecDeque::new(),
            error: None,
        }
    }

    fn record(&mut self, step: Step, node: u64, lost: &[u64], outcome: String) {
        if self.events.len() >= EVENT_HISTORY {
            self.events.pop_front();
        }
        self.events.push_back(Event {
            at_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            step: step.label(),
            node,
            lost: lost.to_vec(),
            outcome,
        });
    }

    fn lost(&self, now: Instant) -> Vec<u64> {
        self.observed
            .iter()
            .filter(|(_, o)| now.duration_since(o.last_active) >= self.config.lost_after)
            .map(|(node, _)| *node)
            .collect()
    }
}

fn majority(voters: usize) -> usize {
    voters / 2 + 1
}

impl NodeRuntime {
    /// One pass of the voter-replacement state machine; called from the leader-side
    /// reconcile loop. Never blocks on a Raft apply.
    pub(super) fn reconcile_voter_replacement(&mut self) {
        let status = self.driver.status();
        let me = self.node.id.0;
        let now = Instant::now();
        if status.role != Role::Leader {
            // Only a leader observes peers; a stale view must not survive a re-election.
            self.voter_replacement.observed.clear();
            self.voter_replacement.pending = None;
            return;
        }

        // Observe every other voter through the leader's replication progress.
        let progress = self.driver.peer_progress();
        let state = &mut self.voter_replacement;
        let mut current = BTreeSet::new();
        for p in &progress {
            if !p.voter || p.node.0 == me {
                continue;
            }
            current.insert(p.node.0);
            let entry = state.observed.entry(p.node.0).or_insert(Observation {
                last_active: now,
                matched: p.matched,
            });
            if p.recent_active || p.matched > entry.matched {
                entry.last_active = now;
            }
            entry.matched = p.matched;
        }
        state.observed.retain(|node, _| current.contains(node));

        // Settle a change proposed by an earlier pass before deciding anything new.
        if let Some(pending) = state.pending.take() {
            match self
                .driver
                .wait_conf_applied(pending.at, Duration::from_millis(1))
            {
                Ok(receipt) => {
                    let lost = state.lost(now);
                    state.record(
                        pending.step,
                        pending.node,
                        &lost,
                        format!(
                            "applied at index {}; voters {:?}",
                            receipt.conf_index, receipt.voters
                        ),
                    );
                    state.error = None;
                }
                Err(_) if pending.since.elapsed() < PENDING_DEADLINE => {
                    state.pending = Some(pending);
                    return;
                }
                Err(error) => {
                    let lost = state.lost(now);
                    state.record(
                        pending.step,
                        pending.node,
                        &lost,
                        format!("not applied: {error}"),
                    );
                    state.error = Some(error.to_string());
                }
            }
        }

        if !state.config.enabled {
            return;
        }
        let lost = state.lost(now);
        let voters = status.voters.len();
        if lost.is_empty() {
            state.baseline_voters = voters.max(MIN_VOTERS);
            return;
        }
        let live = voters - lost.len(); // this leader included

        // Step 2 — an earlier promotion left an extra voter: remove one lost voter, if the
        // result keeps at least three voters and the live ones still hold its majority.
        if voters > state.baseline_voters {
            let target = lost[0];
            let after = voters - 1;
            if after < MIN_VOTERS || live < majority(after) {
                return;
            }
            match self.driver.remove_voter(NodeId(target)) {
                Ok(at) => {
                    state.pending = Some(Pending {
                        step: Step::Remove,
                        node: target,
                        at,
                        since: now,
                    });
                }
                Err(error) => state.error = Some(format!("remove {target}: {error}")),
            }
            return;
        }

        // Step 1 — promote the best caught-up learner, if the four-voter configuration
        // would still have a live majority and the cooldown allows another decision.
        if state
            .last_promotion
            .is_some_and(|last| now.duration_since(last) < state.config.cooldown)
        {
            return;
        }
        if live + 1 < majority(voters + 1) {
            return; // no learner can restore a majority that is already gone
        }
        let candidate = progress
            .iter()
            .filter(|p| {
                !p.voter
                    && status.learners.contains(&p.node.0)
                    && p.recent_active
                    && p.matched.saturating_add(state.config.catchup_lag) >= status.raft_committed
            })
            .max_by_key(|p| p.matched);
        let Some(candidate) = candidate else {
            state.error = Some(format!(
                "voters {lost:?} lost and no caught-up learner to promote"
            ));
            return;
        };
        match self.driver.promote_voter(candidate.node) {
            Ok(at) => {
                state.pending = Some(Pending {
                    step: Step::Promote,
                    node: candidate.node.0,
                    at,
                    since: now,
                });
                state.last_promotion = Some(now);
            }
            Err(error) => state.error = Some(format!("promote {}: {error}", candidate.node.0)),
        }
    }

    /// The `voter_replacement=` status line: this leader's verdicts and decisions.
    pub(super) fn voter_replacement_status(&self) -> String {
        let state = &self.voter_replacement;
        let now = Instant::now();
        let suspect_after = state.config.lost_after / 4;
        let voters = state
            .observed
            .iter()
            .map(|(node, o)| {
                let inactive = now.duration_since(o.last_active);
                VoterHealth {
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
                }
            })
            .collect();
        serde_json::to_string(&StatusLine {
            enabled: state.config.enabled,
            lost_after_secs: state.config.lost_after.as_secs(),
            baseline_voters: state.baseline_voters,
            voters,
            pending: state
                .pending
                .map(|p| format!("{} {} at index {}", p.step.label(), p.node, p.at.index.0)),
            events: state.events.iter().cloned().collect(),
            error: state.error.clone(),
        })
        .expect("serialize voter replacement status")
    }
}
