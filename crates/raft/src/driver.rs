//! Node runtime driver (Phase 1-final): the loop that turns a [`RaftPeer`],
//! a [`RaftTransport`] and a state machine into a running meta node.
//!
//! The server owns process residency (signals, lifecycle); this type owns the
//! pump: drain transport → step raft → persist/send via `pump()` → apply
//! committed entries → expose queryable [`NodeStatus`]. Acceptance criteria
//! read `status()` — never logs, never sleeps-as-proof.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kv9_engine::MemEngine;
use raft::storage::MemStorage;

use kv9_common::metrics::{Latency, NamedLatency, Outcome};
use kv9_common::{Error, NodeId, Result};

use raft::eraftpb::{ConfChangeSingle, ConfChangeType, ConfChangeV2};

use crate::rawnode::{PersistentRaftStorage, ProposedAt, RaftPeer};
use crate::transport::RaftTransport;
use crate::ReadyConsume;
use crate::{Command, EntryKind, MemStateMachine, Role, StateMachine};

/// Queryable node state (the server's `status` surface, agreed seam with the
/// acceptance harness: success is judged on these fields, not on log text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeStatus {
    pub node_id: NodeId,
    pub leader_id: Option<NodeId>,
    pub role: Role,
    pub term: u64,
    /// Highest raft-committed log index. NEVER wait for
    /// `applied_index == raft_committed`: every leader election appends a
    /// no-op barrier that is committed but carries nothing to apply, so the
    /// gap is PERMANENT after the first election — such a wait hangs forever.
    /// Correct catch-up criteria: wait for a SPECIFIC write's (term, index)
    /// to apply, or assert applied_index made absolute progress.
    pub raft_committed: u64,
    /// Highest log index applied to the state machine (empty/no-op entries
    /// are consumed by raft but never reach the state machine — see
    /// `raft_committed`'s warning before comparing the two).
    pub applied_index: u64,
    /// Term paired with `applied_index`; together they identify the last real
    /// state-machine command across leader failover.
    pub applied_term: u64,
    /// A fatal persistence or apply failure (Raft I/O, undecodable committed
    /// entry, or engine apply error). Once set, the pump has stopped: continuing past a hole would
    /// silently diverge this replica from the group. The server surfaces this
    /// and exits non-zero.
    pub fatal: Option<String>,
    /// Inbound messages rejected by raft `step` (dropped, sender retransmits).
    /// Diagnostic: persistent growth signals stale peers / version skew.
    pub step_errors: u64,
    /// Highest conf-change log index applied here (0 = still on the seeded
    /// configuration). The pair (`voters`,`learners`) took effect at exactly
    /// this index.
    pub conf_index: u64,
    /// Current raft voter set (sorted node ids), from the live ConfState.
    /// Post-initialization membership authority is THIS (the raft-committed
    /// configuration), never the boot-time declared seed list (task #24).
    pub voters: Vec<u64>,
    /// Current raft learner set (sorted node ids). A learner replicates the
    /// log but never votes or campaigns.
    pub learners: Vec<u64>,
    /// The unified driver-applied watermark, one atomic snapshot (see
    /// [`DriverAppliedPosition`]). A SINGLE Option so downstream rendering can
    /// never mix "none" for one component with a number for the other — the
    /// frozen status contract renders both `driver_applied_*` fields from
    /// this one value: both `none`, or both decimal, never mixed. Distinct
    /// from the command-scoped `applied_index`/`applied_term` pair above,
    /// whose semantics are unchanged.
    pub driver_applied: Option<DriverAppliedPosition>,
}

/// How many recently applied `(index, term)` pairs are retained for proposal
/// verification (correlation is by term+index, never position alone).
const APPLIED_RING: usize = 1024;

/// How many conf-change receipts are retained (membership changes are rare;
/// a waiter that lags 64 changes behind has bigger problems).
const CONF_RECEIPTS: usize = 64;

/// One command-ring record: the exact applied position and what applying it
/// MEANT — the verdict is apply-time fact, never re-derived from the command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RingEntry {
    index: u64,
    term: u64,
    /// What applying this entry MEANT — exclusive by TYPE (review round:
    /// two Options + a comment claiming exclusivity let the consumer
    /// wildcard the impossible dual-verdict state; the enum deletes the
    /// state instead of trusting it away).
    outcome: crate::ApplyOutcome,
}

/// A conf change applied HERE: its exact position and the membership
/// `apply_conf_change` actually produced at that moment.
#[derive(Debug, Clone)]
struct ConfReceiptEntry {
    index: u64,
    term: u64,
    voters: Vec<u64>,
    learners: Vec<u64>,
}

/// The unified driver-applied position: the highest CONTIGUOUS raft log
/// position such that every entry at or below it — Noop, Command, and
/// ConfChange alike — has been fully and successfully processed by this
/// driver. It conservatively lags and never leads; the fatal path never
/// advances it past a failed item.
///
/// This is a SEPARATE quantity from the command-scoped
/// `MemStateMachine::applied_index` (which only Commands advance) and from
/// the command receipt ring. Never mix components across the two: a term from
/// one paired with an index from the other fabricates a position that never
/// existed (the e2ecc5a bug). Both members here come from the SAME entry.
///
/// Consumers: the bootstrap current-term barrier (task #40), the #28
/// ReadIndex wait, and the raw linearizable read path (task #27) — each with
/// its own receipt semantics; they share only this watermark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DriverAppliedPosition {
    pub term: u64,
    pub index: u64,
}

/// Observers are leaf locks: no operation runs while their locks are held.
#[derive(Default)]
pub struct DriverMetrics {
    pub proposal_submission: Latency,
    pub logical_proposal_wait: Latency,
    pub application_wait: Latency,
    pub read_establishment: Latency,
    pub command_apply: Latency,
    pub pump_service: Latency,
    pub pump_idle_wait: Latency,
    pub pump_iteration_spacing: Latency,
}

impl DriverMetrics {
    pub fn snapshots(&self) -> Vec<NamedLatency> {
        vec![
            NamedLatency::new("raft_proposal_submission", &self.proposal_submission),
            NamedLatency::new("runtime_logical_proposal_wait", &self.logical_proposal_wait),
            NamedLatency::new("raft_application_wait", &self.application_wait),
            NamedLatency::new("raft_read_establishment", &self.read_establishment),
            NamedLatency::new("raft_command_apply", &self.command_apply),
            NamedLatency::new("raft_pump_service", &self.pump_service),
            NamedLatency::new("raft_pump_idle_wait", &self.pump_idle_wait),
            NamedLatency::new("raft_pump_iteration_spacing", &self.pump_iteration_spacing),
        ]
    }
}

/// Two peer observations bracket one contiguous driver watermark. A derived
/// lag is available only when the commit index and current term were stable.
/// No peer lock nests with a driver lock. This is diagnostic, never authority.
#[derive(Debug)]
pub struct ApplyLagObservation {
    pub term_before: u64,
    pub term_after: u64,
    pub committed_before: u64,
    pub committed_after: u64,
    pub driver_applied: Option<DriverAppliedPosition>,
}

impl ApplyLagObservation {
    pub fn lag(&self) -> Option<u64> {
        if self.term_before != self.term_after || self.committed_before != self.committed_after {
            return None;
        }
        let applied = self.driver_applied?;
        if applied.term > self.term_after {
            return None;
        }
        self.committed_after.checked_sub(applied.index)
    }
}

pub struct NodeDriver<S: PersistentRaftStorage = MemStorage, E: crate::ApplyStore = MemEngine> {
    peer: Arc<RaftPeer<S>>,
    /// THE consume face over `peer` (task #5): minted exactly once, held
    /// privately here for the life of the driver. `peer()` keeps handing out
    /// the peer — which can propose and observe but not drain.
    drain: crate::DrainToken<S>,
    transport: Arc<dyn RaftTransport>,
    /// Generic over the engine so the durable `WalEngine` (or any other) sits
    /// directly in the apply downstream — the server passes a state machine
    /// sharing ONE engine instance with its MetaStore, or catalog reads and
    /// applied writes would land in different engines.
    /// LOCK ORDER: `applied` before `sm`, always. Every site that holds both
    /// (`step`, `status`, `wait_applied`) must acquire in that order — the
    /// pump and the status/wait readers run on different threads, and one
    /// reversed pair is an AB-BA deadlock that freezes the whole node
    /// silently (found live under load; the acceptance-flake root cause).
    /// `fatal` is a leaf: never held while acquiring either of the others.
    sm: Mutex<MemStateMachine<E>>,
    /// Recently applied entries — exact `(index, term)` PLUS the apply
    /// verdict — for proposal verification. ONLY successfully applied entries
    /// enter this ring; a fence-rejected entry applies successfully (watermark
    /// advanced, nothing written) and enters WITH its rejection verdict, so
    /// the receipt reaches the proposer instead of dying at this boundary
    /// (the silent-lost-write blocker Ren's layer-3 test caught).
    applied: Mutex<Vec<RingEntry>>,
    /// Conf-change receipts by exact (index, term) — the correlation store for
    /// [`Self::wait_conf_applied`]. Conf entries NEVER enter the command ring:
    /// `applied_index`/`applied_term` must remain a same-entry pair.
    /// Lock order: leaf — never held while acquiring `applied`/`sm`/peer.
    conf_receipts: Mutex<Vec<ConfReceiptEntry>>,
    /// Quorum-confirmed read receipts (task #28): `(request_ctx, index)` pairs
    /// drained from the peer's Ready loop, correlated by EXACT context bytes.
    /// Bounded like the command ring; eviction is FAIL-CLOSED (Cindy's review
    /// boundary, written down so a load test does not send someone chasing the
    /// quorum round-trip): if more than APPLIED_RING confirmations arrive
    /// between a request and its poll, a genuinely-confirmed read can be
    /// evicted and reported `Unconfirmed{QuorumConfirmation}` — one spurious
    /// retry, never a false confirmation (contexts are incarnation ++
    /// monotonic seq, never reused, so a hit can only be OUR receipt).
    /// Receipts are not deleted on hit — deletion would open a window for a
    /// concurrent second lookup of the same rctx; aging out is the only exit.
    /// Lock order: leaf — never held while acquiring any other lock.
    read_receipts: Mutex<Vec<(Vec<u8>, u64)>>,
    /// This driver's boot incarnation: 16 random bytes minted at construction.
    /// Every read context is `incarnation ++ counter`, so a receipt minted in
    /// a previous process life (same node id, restarted) can never satisfy a
    /// wait in this one — position alone never confirms a read, the same rule
    /// the command ring enforces for proposals.
    read_incarnation: [u8; 16],
    /// Monotonic per-incarnation read sequence (uniqueness within a life).
    read_seq: std::sync::atomic::AtomicU64,
    /// Observe a completed submission attempt without timing sleeps in tests.
    #[cfg(test)]
    read_attempt_observer: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    /// First fatal persistence/apply error; poisons the driver (pump stops).
    fatal: Mutex<Option<String>>,
    /// Whether THE manifest seam over this node has been minted (task #9
    /// review round 1): slot state must be process-unique per node, so seam
    /// construction is once-CAS here — a second seam instance would carry a
    /// second slot table and let two proposers race the same region.
    manifest_seam_minted: std::sync::atomic::AtomicBool,
    /// Testing-only: freeze the APPLY half of the pump (committed entries
    /// stay queued in the peer) while raft itself keeps electing/committing.
    /// This is the deterministic construction of the committed-but-unapplied
    /// window that the bootstrap current-term barrier exists for (and that
    /// #28's ReadIndex tests will reuse). Never compiled into production.
    #[cfg(any(test, feature = "testing"))]
    apply_paused: std::sync::atomic::AtomicBool,
    /// The unified driver-applied watermark (see [`DriverAppliedPosition`]).
    /// Lock order: leaf — never held while acquiring any other lock. The pair
    /// is read as one snapshot under this single lock; reading the two
    /// components separately is forbidden (torn pair = e2ecc5a again).
    driver_applied: Mutex<Option<DriverAppliedPosition>>,
    stop: AtomicBool,
    pump_started: AtomicBool,
    pump_gate: Mutex<()>,
    completion: crate::work::CompletionSignal,
    proposals: crate::proposal_queue::ProposalQueue,
    metrics: DriverMetrics,
}

impl<S: PersistentRaftStorage, E: crate::ApplyStore + 'static> NodeDriver<S, E> {
    /// Wire a driver over `peer`. Mints THE drain token for the peer — a
    /// typed refusal if one was already minted: two drivers over one peer
    /// would be two destructive Ready consumers, the exact hole task #5
    /// closes.
    pub fn new(
        peer: Arc<RaftPeer<S>>,
        transport: Arc<dyn RaftTransport>,
        sm: MemStateMachine<E>,
    ) -> Result<Arc<NodeDriver<S, E>>> {
        let drain = crate::DrainToken::mint(&peer)?;
        transport.set_work_signal(peer.work_signal.clone());
        let proposals = crate::proposal_queue::ProposalQueue::new(peer.work_signal.clone());
        Ok(Arc::new(NodeDriver {
            peer,
            drain,
            transport,
            sm: Mutex::new(sm),
            applied: Mutex::new(Vec::new()),
            conf_receipts: Mutex::new(Vec::new()),
            read_receipts: Mutex::new(Vec::new()),
            read_incarnation: {
                use std::io::Read;
                let mut bytes = [0u8; 16];
                std::fs::File::open("/dev/urandom")
                    .and_then(|mut f| f.read_exact(&mut bytes))
                    .expect("read-incarnation entropy");
                bytes
            },
            read_seq: std::sync::atomic::AtomicU64::new(0),
            #[cfg(test)]
            read_attempt_observer: Mutex::new(None),
            fatal: Mutex::new(None),
            manifest_seam_minted: std::sync::atomic::AtomicBool::new(false),
            #[cfg(any(test, feature = "testing"))]
            apply_paused: std::sync::atomic::AtomicBool::new(false),
            // None = no position PROVEN yet this run — distinct from "position
            // zero". Restart replays the log from 0 and re-proves; when
            // snapshots land, this must be restored from the snapshot's
            // unified position — never guessed from the command watermark,
            // commit index, or conf index (missing that restore fail-closes:
            // consumers keep waiting instead of trusting a fabricated 0).
            driver_applied: Mutex::new(None),
            stop: AtomicBool::new(false),
            pump_started: AtomicBool::new(false),
            pump_gate: Mutex::new(()),
            completion: crate::work::CompletionSignal::default(),
            proposals,
            metrics: DriverMetrics::default(),
        }))
    }

    /// Mint THE seam handle for this node (once-CAS; the second mint is a
    /// typed refusal). The handle is BOUND to this driver — it is the node
    /// reference and the authority in one non-duplicable value, so there is
    /// nothing to cross-pair (task #9 round 4).
    pub fn mint_seam_handle(self: &Arc<Self>) -> Result<SeamHandle> {
        use std::sync::atomic::Ordering;
        if self
            .manifest_seam_minted
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(Error::Raft(
                "manifest seam already minted for this node — one slot table per \
                 node (task #9); a second seam would let two proposers race one \
                 region's in-flight window"
                    .into(),
            ));
        }
        Ok(SeamHandle {
            node: Arc::clone(self) as Arc<dyn ManifestNode>,
        })
    }

    /// The peer: the propose/observe face. Holding it does NOT confer drain —
    /// `take_ready` lives on the private [`crate::DrainToken`] minted in
    /// [`Self::new`] (task #5).
    pub fn peer(&self) -> &Arc<RaftPeer<S>> {
        &self.peer
    }

    /// One pump iteration WITHOUT a tick: deliver inbound, persist+send
    /// outbound, apply committed entries.
    ///
    /// A Raft persistence failure stops the peer before publishing that Ready
    /// and reaches the same observable fatal state without a mutex-poisoning
    /// panic. A committed entry that fails to decode or apply is **fatal**: every
    /// replica must apply the same committed sequence, so skipping one and
    /// continuing would silently diverge this node from the group. On error
    /// the driver poisons itself (pump stops, `status().fatal` set) and the
    /// failed entry never enters the success-correlation ring.
    pub fn step(&self) -> Result<()> {
        let _owner = self.pump_gate.lock().expect("pump gate poisoned");
        self.step_observed()
    }

    fn step_observed(&self) -> Result<()> {
        let result = self.metrics.pump_service.observe(
            || self.step_inner(),
            |result| {
                if result.is_ok() {
                    Outcome::Success
                } else {
                    Outcome::Error
                }
            },
        );
        // Receipt/read-state/watermark publication precedes this notification.
        // Waiters still validate their exact identity and original deadline.
        if let Err(cause) = self.completion.publish() {
            if result.is_ok() {
                return Err(self.poison_persistence(&cause));
            }
        }
        result
    }

    fn step_inner(&self) -> Result<()> {
        if let Some(f) = self.fatal.lock().expect("fatal poisoned").as_ref() {
            return Err(Error::Raft(f.clone()));
        }
        for msg in self.transport.drain() {
            self.peer.step_message(msg);
        }
        // Incoming term changes precede queued submission. Every request is
        // checked against actual leadership/planning term at this handoff.
        // Queue guards are released before the peer lock is taken. Submitting
        // an available prefix before pump() lets one Ready contain its entries.
        self.proposals
            .drain(|data, term| self.peer.propose_in_term(data, term));
        let messages = self
            .peer
            .pump()
            .map_err(|cause| self.poison_persistence(&cause))?;
        for msg in messages {
            let to = NodeId(msg.to);
            self.transport.send(to, msg);
        }
        {
            let states = self.peer.take_read_states();
            if !states.is_empty() {
                let mut receipts = self.read_receipts.lock().expect("read receipts poisoned");
                for st in states {
                    receipts.push((st.request_ctx, st.index));
                }
                let len = receipts.len();
                if len > APPLIED_RING {
                    receipts.drain(..len - APPLIED_RING);
                }
            }
        }
        #[cfg(any(test, feature = "testing"))]
        if self.apply_paused.load(std::sync::atomic::Ordering::Relaxed) {
            // Apply frozen: leave committed entries queued (they drain in
            // order on unpause). Raft above keeps running — elections and
            // commits proceed, driver_applied does not.
            return Ok(());
        }
        let entries = self
            .drain
            .take_ready()
            .map_err(|cause| self.poison_persistence(&cause))?;
        if entries.is_empty() {
            return Ok(());
        }
        // Items are processed strictly in log order. Locks are taken per apply group
        // (always `applied` then `sm`, per the declared order) and NEVER held
        // across a peer call: conf changes go through `peer.inner`, and peer
        // must not nest with driver locks in either direction.
        let mut last_seen: u64 = 0;
        let mut last_term: u64 = 0;
        let mut entries = entries.into_iter().peekable();
        while let Some(entry) = entries.next() {
            last_seen = entry.index.0;
            last_term = entry.term;
            match entry.kind {
                // A no-op barrier never reaches the state machine or the
                // command-only watermark — but it DOES advance the unified
                // driver watermark at the batch tail below (that is the
                // current-term barrier's liveness anchor).
                EntryKind::Noop => {}
                EntryKind::Command => {
                    let cmd = match Command::decode(&entry.data) {
                        Ok(c) => c,
                        Err(e) => return Err(self.poison(entry.term, entry.index.0, &e)),
                    };
                    if matches!(cmd, Command::ConfChange { .. }) {
                        // Loud-fail (task #24): the legacy app-level tag was an
                        // unwired placeholder that applied as an empty batch —
                        // "committed but configuration unchanged" IS divergence.
                        // Real membership changes travel as raft EntryConfChange.
                        let e = Error::Raft(
                            "legacy Command::ConfChange is unwired; membership \
                             changes must use raft conf-change entries"
                                .into(),
                        );
                        return Err(self.poison(entry.term, entry.index.0, &e));
                    }
                    let mut applied = self.applied.lock().expect("applied poisoned");
                    let mut sm = self.sm.lock().expect("sm poisoned");
                    let mut bytes = entry.data.len();
                    let mut commands = vec![(
                        kv9_common::AppliedPosition {
                            term: entry.term,
                            index: entry.index.0,
                        },
                        cmd,
                    )];
                    // Coalesce only an already-queued Raw prefix. An oversized
                    // legal command remains a singleton; no timer or queue is added.
                    // Catalog, manifest, no-op and configuration entries are barriers.
                    use crate::state_machine::raw_group::{
                        MAX_RAW_GROUP_BYTES, MAX_RAW_GROUP_ENTRIES,
                    };
                    if bytes <= MAX_RAW_GROUP_BYTES
                        && entry.index > sm.applied_index()
                        && sm.can_group_raw(&commands[0].1)
                    {
                        while commands.len() < MAX_RAW_GROUP_ENTRIES {
                            let Some(next) = entries.peek() else { break };
                            if next.kind != EntryKind::Command
                                || next.data.len() > MAX_RAW_GROUP_BYTES - bytes
                            {
                                break;
                            }
                            let Ok(next_cmd) = Command::decode(&next.data) else {
                                // Apply the preceding valid group first; the next
                                // iteration reports the undecodable entry itself.
                                break;
                            };
                            if !sm.can_group_raw(&next_cmd) {
                                break;
                            }
                            bytes += next.data.len();
                            last_seen = next.index.0;
                            last_term = next.term;
                            commands.push((
                                kv9_common::AppliedPosition {
                                    term: next.term,
                                    index: next.index.0,
                                },
                                next_cmd,
                            ));
                            entries.next();
                        }
                    }
                    // Each sample remains a command's complete apply interval;
                    // commands in one group share its persistence wait.
                    let apply_timers: Vec<_> = commands
                        .iter()
                        .map(|_| self.metrics.command_apply.start())
                        .collect();
                    let outcome = if commands.len() == 1 {
                        sm.apply_at(commands[0].0, &commands[0].1).map(|r| vec![r])
                    } else {
                        sm.apply_raw_group(&commands)
                    };
                    let results = match outcome {
                        Ok(r) => {
                            for timer in apply_timers {
                                timer.finish(Outcome::Success);
                            }
                            r
                        }
                        Err(e) => {
                            for timer in apply_timers {
                                timer.finish(Outcome::Error);
                            }
                            drop(sm);
                            drop(applied);
                            return Err(self.poison(last_term, last_seen, &e));
                        }
                    };
                    for ((at, _), result) in commands.iter().zip(results) {
                        push_ring(&mut applied, at.index, at.term, result.outcome);
                    }
                }
                EntryKind::ConfChangeV1 | EntryKind::ConfChangeV2 => {
                    // Peer call first (no driver locks held). The result goes
                    // into the CONF receipt ring only — never the command
                    // ring: `applied_index`/`applied_term` must stay a
                    // same-entry pair from the last Command, and a conf term
                    // paired with an older command index would fabricate a
                    // position that never existed (Tess's review).
                    match self
                        .peer
                        .apply_conf_change_bytes(entry.kind, &entry.data, entry.index.0)
                    {
                        Ok((voters, learners)) => {
                            let mut receipts =
                                self.conf_receipts.lock().expect("receipts poisoned");
                            receipts.push(ConfReceiptEntry {
                                index: entry.index.0,
                                term: entry.term,
                                voters,
                                learners,
                            });
                            let len = receipts.len();
                            if len > CONF_RECEIPTS {
                                receipts.drain(..len - CONF_RECEIPTS);
                            }
                        }
                        Err(e) => return Err(self.poison(entry.term, entry.index.0, &e)),
                    }
                }
            }
        }
        // Report REAL apply progress to raft — only now, after every entry up
        // to `last_seen` has actually been applied. raft-rs gates the next
        // one-at-a-time conf change on this. No driver locks are held here.
        self.peer.applied_to(last_seen);
        // Publish the unified driver-applied watermark from the SAME batch
        // tail. Both members come from the last entry of a batch in which
        // EVERY item succeeded — the fatal paths above return before reaching
        // this line, so the watermark never advances past a failed item.
        //
        // LOAD-BEARING — contiguity is what the bootstrap current-term
        // barrier proof stands on: `pair.term == T` proves "term T's barrier
        // is processed" ONLY because everything at or below `pair.index` is
        // processed too (T's election no-op is T's first entry). Turning any
        // `return Err(self.poison(..))` above into a skip/continue silently
        // breaks that proof — a WaitForBootstrap leader could then mint over
        // a committed-but-unprocessed init and poison the cluster. Weakening
        // this is weakening bootstrap safety, not a cleanup.
        //
        // LIVENESS anchor: `pair.term` always reaches the current leader term
        // without any application proposal, because raft-rs `become_leader`
        // unconditionally appends an empty entry at the new term
        // (raft-0.7.0/src/raft.rs:1236, panics if refused) and that entry
        // flows through `EntryKind::Noop` above.
        self.publish_driver_applied(DriverAppliedPosition {
            term: last_term,
            index: last_seen,
        });
        Ok(())
    }

    /// The unified driver-applied watermark, as one atomic snapshot. `None`
    /// means no position has been proven this run (fail-closed: consumers
    /// wait; they never treat it as position zero).
    pub fn driver_applied(&self) -> Option<DriverAppliedPosition> {
        *self.driver_applied.lock().expect("driver_applied poisoned")
    }

    /// Testing-only control for [`Self`]'s apply freeze — see `apply_paused`.
    #[cfg(any(test, feature = "testing"))]
    pub fn pause_apply(&self, paused: bool) {
        self.apply_paused
            .store(paused, std::sync::atomic::Ordering::Relaxed);
        self.peer.work_signal.notify();
    }

    /// The single publication point for the unified watermark. Monotonic
    /// guard — same shape as rawnode's `applied_reported`: batch order makes
    /// regression unreachable today, but the invariant consumers rely on ("a
    /// barrier waiter never observes a going-backward position") is pinned
    /// HERE rather than inherited from delivery order, so a future delivery
    /// change cannot silently hand out a regressing watermark. The refusal is
    /// verified by its own test — the phenomenon (a lower candidate must not
    /// win), not a fabricated delivery reorder.
    fn publish_driver_applied(&self, candidate: DriverAppliedPosition) {
        let mut wm = self.driver_applied.lock().expect("driver_applied poisoned");
        if wm.is_none_or(|cur| candidate.index > cur.index) {
            *wm = Some(candidate);
        }
    }

    /// Record a fatal apply-path failure and stop the pump (the poison path:
    /// skipping a committed entry would silently diverge this replica).
    fn poison(&self, term: u64, index: u64, cause: &Error) -> Error {
        let msg = format!("fatal at committed entry (term {term}, index {index}): {cause}");
        *self.fatal.lock().expect("fatal poisoned") = Some(msg.clone());
        self.stop.store(true, Ordering::Relaxed);
        self.proposals.close();
        self.peer.work_signal.stop();
        let _ = self.completion.publish();
        Error::Raft(msg)
    }

    /// Preserve the failure without unwinding through peer/driver mutexes.
    /// The runtime can still read status and exit through its normal fatal path.
    fn poison_persistence(&self, cause: &Error) -> Error {
        let mut fatal = self.fatal.lock().expect("fatal poisoned");
        let msg = fatal.get_or_insert_with(|| cause.to_string()).clone();
        drop(fatal);
        self.stop.store(true, Ordering::Relaxed);
        self.proposals.close();
        self.peer.work_signal.stop();
        let _ = self.completion.publish();
        Error::Raft(msg)
    }

    /// Propose adding `node` as a raft **learner** (replicates, never votes or
    /// campaigns). Returns the conf entry's exact `(term, index)`; confirm with
    /// [`Self::wait_conf_applied`]. Admission policy (cluster id, address,
    /// ticket) lives ABOVE this call — this is the raft mechanism only.
    pub fn add_learner(&self, node: NodeId) -> Result<ProposedAt> {
        self.peer
            .propose_conf_change_traced(single_change(node, ConfChangeType::AddLearnerNode))
    }

    /// Propose promoting `node` (a caught-up learner) to **voter**. One change
    /// at a time; raft refuses a new conf change until the previous one is
    /// applied — which is why apply progress must be reported truthfully.
    pub fn promote_voter(&self, node: NodeId) -> Result<ProposedAt> {
        self.peer
            .propose_conf_change_traced(single_change(node, ConfChangeType::AddNode))
    }

    /// Wait until the conf change proposed at `at` is applied HERE, verified by
    /// exact `(term, index)`; returns the post-change membership actually
    /// produced by `apply_conf_change` (never the proposal-time expectation).
    pub fn wait_conf_applied(
        &self,
        at: ProposedAt,
        deadline: Duration,
    ) -> Result<ConfChangeReceipt> {
        let start = Instant::now();
        loop {
            let observed = self.completion.observe()?;
            if let Some(f) = self.fatal.lock().expect("fatal poisoned").as_ref() {
                return Err(Error::Raft(format!("driver is poisoned: {f}")));
            }
            {
                // The receipt captured AT APPLY TIME for this exact position —
                // never the live membership, which may already reflect a later
                // change by the time the waiter wakes (Tess's review).
                let receipts = self.conf_receipts.lock().expect("receipts poisoned");
                if let Some(r) = receipts.iter().find(|r| r.index == at.index.0) {
                    if r.term != at.term {
                        return Err(Error::Raft(format!(
                            "conf change at term {} index {} was overwritten by another leader",
                            at.term, at.index.0
                        )));
                    }
                    return Ok(ConfChangeReceipt {
                        applied: at,
                        conf_index: r.index,
                        voters: r.voters.clone(),
                        learners: r.learners.clone(),
                    });
                }
            }
            // Position passed without a conf receipt: the slot went to a
            // different (non-conf or other-leader) entry.
            if self.peer.status_snapshot().conf_applied > at.index.0
                || self.sm.lock().expect("sm poisoned").applied_index().0 >= at.index.0
            {
                return Err(Error::Raft(format!(
                    "conf change at term {} index {} was overwritten by another leader",
                    at.term, at.index.0
                )));
            }
            if start.elapsed() > deadline {
                return Err(Error::Raft(format!(
                    "wait_conf_applied deadline: index {} not reached",
                    at.index.0
                )));
            }
            self.completion
                .wait(observed, deadline.saturating_sub(start.elapsed()))?;
        }
    }

    /// Tick + step (the driver loop body).
    pub fn tick_and_step(&self) -> Result<()> {
        let _owner = self.pump_gate.lock().expect("pump gate poisoned");
        self.peer.tick_once();
        self.step_observed()
    }

    /// Propose a command on this node (must currently be leader).
    pub fn propose(&self, cmd: &Command) -> Result<ProposedAt> {
        self.metrics
            .proposal_submission
            .observe(|| self.peer.propose_traced(cmd.encode()), classify_proposal)
    }

    /// Catalog plans are valid only in the term whose ordered barrier they read.
    pub fn propose_in_term(&self, cmd: &Command, term: u64) -> Result<ProposedAt> {
        self.metrics.proposal_submission.observe(
            || self.peer.propose_in_term(cmd.encode(), Some(term)),
            classify_proposal,
        )
    }

    /// Queue one command without acquiring the peer/persistence mutex, then
    /// await its exact submission result within the original absolute deadline.
    /// The owner must be running. Queue acceptance is not commitment or apply.
    pub fn propose_queued(
        &self,
        cmd: &Command,
        expected_term: Option<u64>,
        deadline: Instant,
    ) -> Result<ProposedAt> {
        self.metrics.proposal_submission.observe(
            || {
                self.proposals
                    .enqueue(cmd.encode(), expected_term, deadline)?
                    .wait()
            },
            classify_proposal,
        )
    }

    pub fn proposal_queue_snapshot(&self) -> crate::ProposalQueueSnapshot {
        self.proposals.snapshot()
    }

    /// One region's authoritative manifest pair, read through this driver's
    /// state machine (task #9). ONE key, one get — precondition P1 is
    /// structural; see `MemStateMachine::manifest_pair`. Lock note: takes
    /// `sm` alone, never while holding `applied` (leaf read).
    pub fn manifest_pair(&self, region: u64) -> Result<crate::ManifestPair> {
        self.sm.lock().expect("sm poisoned").manifest_pair(region)
    }

    pub fn manifest_at(&self, region: u64, generation: u64) -> Result<Option<(u64, u64, Vec<u8>)>> {
        self.sm
            .lock()
            .expect("sm poisoned")
            .manifest_at(region, generation)
    }

    /// Wait until this node has applied `at` — verified by **term + index**,
    /// never position alone. Pure condition-poll: the pump must be running
    /// (via [`Self::spawn`] or a caller-driven loop).
    ///
    /// The typed receipt (task #30, forced by the 2026-08-31 master red): a
    /// proposal whose position is consumed by an election BARRIER hit neither
    /// of the old detection arms — a barrier never enters the command ring and
    /// never advances the state-machine watermark — so with no later command
    /// traffic the only possible answer was a full deadline burn reported as
    /// unavailability. The unified driver watermark is the discriminator that
    /// closes the gap: it advances through every committed entry, barriers
    /// included, so "driver watermark passed the position, ring never saw it"
    /// IS the replacement verdict, delivered in milliseconds.
    pub fn wait_applied(
        &self,
        at: ProposedAt,
        deadline: Duration,
    ) -> std::result::Result<ApplyWaitOutcome, ApplyWaitError> {
        self.metrics.application_wait.observe(
            || self.wait_applied_inner(at, deadline),
            classify_apply_wait,
        )
    }

    fn wait_applied_inner(
        &self,
        at: ProposedAt,
        deadline: Duration,
    ) -> std::result::Result<ApplyWaitOutcome, ApplyWaitError> {
        let start = Instant::now();
        loop {
            let observed = self.completion.observe().map_err(ApplyWaitError::Failed)?;
            if let Some(f) = self.fatal.lock().expect("fatal poisoned").as_ref() {
                return Err(ApplyWaitError::Failed(Error::Raft(format!(
                    "driver is poisoned: {f}"
                ))));
            }
            // Snapshot the unified watermark BEFORE the ring lock (never
            // nested inside it — the pump publishes under these locks in its
            // own order). A stale-low read only delays a Replaced verdict by
            // one recheck; monotonicity makes a stale read safe, never
            // wrong.
            let wm_passed = self
                .driver_applied()
                .is_some_and(|wm| wm.index >= at.index.0);
            {
                let applied = self.applied.lock().expect("applied poisoned");
                if let Some(entry) = applied.iter().find(|e| e.index == at.index.0) {
                    return if entry.term == at.term {
                        // The receipt is the RING's recorded values — position
                        // AND verdict as the apply loop stored them, never the
                        // proposal echoed back. A fence-rejected entry applied
                        // successfully (watermark advanced, nothing written)
                        // and its verdict must reach the proposer — dropping
                        // it here reported a rejected write as a success (the
                        // silent-lost-write blocker).
                        let at_pos = kv9_common::AppliedPosition {
                            term: entry.term,
                            index: entry.index,
                        };
                        // Exhaustive over the exclusive outcome — no
                        // wildcard, no impossible state to trust away.
                        match entry.outcome {
                            crate::ApplyOutcome::Manifest(verdict) => {
                                Ok(ApplyWaitOutcome::Manifest {
                                    at: at_pos,
                                    verdict,
                                })
                            }
                            crate::ApplyOutcome::FenceRejected(region) => {
                                Ok(ApplyWaitOutcome::FenceRejected { at: at_pos, region })
                            }
                            crate::ApplyOutcome::Plain => Ok(ApplyWaitOutcome::Applied(at_pos)),
                        }
                    } else {
                        // The position applied here, but as ANOTHER leader's
                        // command.
                        Ok(ApplyWaitOutcome::Replaced)
                    };
                }
                // Ring-eviction honesty (review round, Cindy): the ring is
                // bounded, so an index below its oldest retained entry — with
                // the ring at capacity — may have applied and been evicted:
                // indistinguishable from never-applied. When we cannot
                // distinguish, say so. A fabricated Replaced invites the
                // caller to retry a possibly-SUCCEEDED non-idempotent write;
                // Unconfirmed keeps the unknown unknown.
                if applied.len() == APPLIED_RING
                    && applied.first().is_some_and(|e| at.index.0 < e.index)
                {
                    return Err(ApplyWaitError::Unconfirmed {
                        index: at.index.0,
                        waited: start.elapsed(),
                    });
                }
                // The position was passed without this index entering the
                // command ring. Two watermarks can prove that:
                //  - the state-machine watermark (some LATER command applied);
                //  - the unified driver watermark (ANY later entry applied —
                //    the only arm that fires when the replacing entry is the
                //    new leader's barrier and no further command traffic
                //    arrives; exactly the CI scene: driver=3, sm=2, wait=3).
                if self.sm.lock().expect("sm poisoned").applied_index() >= at.index || wm_passed {
                    return Ok(ApplyWaitOutcome::Replaced);
                }
            }
            if start.elapsed() > deadline {
                return Err(ApplyWaitError::Unconfirmed {
                    index: at.index.0,
                    waited: deadline,
                });
            }
            self.completion
                .wait(observed, deadline.saturating_sub(start.elapsed()))
                .map_err(ApplyWaitError::Failed)?;
        }
    }

    /// Establish a quorum-confirmed read barrier (task #28 step 3): the
    /// linearizable-read primitive raw reads and the txn Get both consume.
    ///
    /// Sequence, each half with its own typed failure:
    /// 1. leader check — `NotLeader { hint }` re-routes the caller;
    /// 2. mint `rctx = incarnation ++ seq` and ask raft for a read index;
    ///    ReadOnlyOption::Safe means the returned index is valid only after a
    ///    LIVE quorum acknowledges this leader — an isolated stale leader
    ///    never gets one (`Unconfirmed { QuorumConfirmation }`);
    /// 3. wait until the UNIFIED driver watermark reaches the confirmed index
    ///    (`Unconfirmed { ApplyCatchUp }` on deadline). The command-scoped
    ///    watermark is wrong here by definition: the confirmed index may be a
    ///    barrier entry that never touches the state machine — the exact
    ///    wrong-watermark wiring that burned the 2026-08-31 master red.
    ///
    /// The caller MUST take its engine snapshot AFTER this returns (seam
    /// contract): a snapshot taken before the barrier can miss entries the
    /// barrier proves applied. Notification-driven condition recheck; the pump must be running.
    pub fn read_barrier(
        &self,
        deadline: Duration,
    ) -> std::result::Result<ReadBarrier, ReadIndexError> {
        self.metrics
            .read_establishment
            .observe(|| self.read_barrier_inner(deadline), classify_read)
    }

    fn read_barrier_inner(
        &self,
        deadline: Duration,
    ) -> std::result::Result<ReadBarrier, ReadIndexError> {
        use std::sync::atomic::Ordering;
        let start = Instant::now();
        // 1. Leadership: fail fast and typed.
        let status = self.peer.status_snapshot();
        if status.raw_role != Role::Leader {
            return Err(ReadIndexError::NotLeader {
                hint: status.leader_hint,
            });
        }
        // 2. Mint the context and request the read index. The incarnation
        //    prefix makes receipts from a previous process life unmatchable.
        // The mint counter doubles as the observable "how many barriers did
        // this node request" diagnostic (see `read_barriers_minted`).
        let seq = self.read_seq.fetch_add(1, Ordering::Relaxed);
        let mut rctx = Vec::with_capacity(24);
        rctx.extend_from_slice(&self.read_incarnation);
        rctx.extend_from_slice(&seq.to_be_bytes());
        // A new leader can be observable before its election no-op commits.
        // Retain this invocation's context until raft-rs can admit it. Never
        // reset the request deadline or treat readiness as quorum confirmation.
        loop {
            let observed = self.completion.observe().map_err(ReadIndexError::Failed)?;
            let submitted = self.peer.read_index(rctx.clone()).map_err(|e| match e {
                Error::NotLeader { leader } => ReadIndexError::NotLeader { hint: leader },
                other => ReadIndexError::Failed(other),
            })?;
            #[cfg(test)]
            if let Some(observer) = self.read_attempt_observer.lock().unwrap().as_ref() {
                let _ = observer.send(());
            }
            if submitted {
                break;
            }
            if start.elapsed() > deadline {
                return Err(ReadIndexError::Unconfirmed {
                    phase: BarrierPhase::QuorumConfirmation,
                    waited: start.elapsed(),
                });
            }
            self.completion
                .wait(observed, deadline.saturating_sub(start.elapsed()))
                .map_err(ReadIndexError::Failed)?;
        }
        // 3. Wait for the quorum confirmation correlated by EXACT context.
        let confirmed = loop {
            let observed = self.completion.observe().map_err(ReadIndexError::Failed)?;
            if let Some(f) = self.fatal.lock().expect("fatal poisoned").as_ref() {
                return Err(ReadIndexError::Failed(Error::Raft(format!(
                    "driver is poisoned: {f}"
                ))));
            }
            let hit = {
                let receipts = self.read_receipts.lock().expect("read receipts poisoned");
                receipts
                    .iter()
                    .find(|(ctx, _)| ctx == &rctx)
                    .map(|&(_, index)| index)
            };
            if let Some(index) = hit {
                break index;
            }
            if start.elapsed() > deadline {
                return Err(ReadIndexError::Unconfirmed {
                    phase: BarrierPhase::QuorumConfirmation,
                    waited: start.elapsed(),
                });
            }
            self.completion
                .wait(observed, deadline.saturating_sub(start.elapsed()))
                .map_err(ReadIndexError::Failed)?;
        };
        // 4. Wait for the unified watermark to pass the confirmed index.
        loop {
            let observed = self.completion.observe().map_err(ReadIndexError::Failed)?;
            if let Some(f) = self.fatal.lock().expect("fatal poisoned").as_ref() {
                return Err(ReadIndexError::Failed(Error::Raft(format!(
                    "driver is poisoned: {f}"
                ))));
            }
            if self
                .driver_applied()
                .is_some_and(|wm| wm.index >= confirmed)
            {
                return Ok(ReadBarrier { index: confirmed });
            }
            if start.elapsed() > deadline {
                return Err(ReadIndexError::Unconfirmed {
                    phase: BarrierPhase::ApplyCatchUp,
                    waited: start.elapsed(),
                });
            }
            self.completion
                .wait(observed, deadline.saturating_sub(start.elapsed()))
                .map_err(ReadIndexError::Failed)?;
        }
    }

    /// How many read barriers this node has REQUESTED (minted a read
    /// context for) since boot — successful or not. Diagnostic counter; the
    /// delete-range evidence cell pins "one request establishes exactly one
    /// quorum barrier" on it (a per-chunk regression multiplies it).
    pub fn read_barriers_minted(&self) -> u64 {
        self.read_seq.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn metrics(&self) -> &DriverMetrics {
        &self.metrics
    }

    pub fn apply_lag_observation(&self) -> ApplyLagObservation {
        let before = self.peer.status_snapshot();
        let driver_applied = self.driver_applied();
        let after = self.peer.status_snapshot();
        ApplyLagObservation {
            term_before: before.term,
            term_after: after.term,
            committed_before: before.committed,
            committed_after: after.committed,
            driver_applied,
        }
    }

    /// The queryable status surface.
    pub fn status(&self) -> NodeStatus {
        // ONE peer lock acquisition for everything peer-side: piecemeal reads
        // can tear during a conf apply (role from before, membership from
        // after) and misreport a healthy node for an instant. Peer never
        // nests with driver locks in either direction, so the snapshot is
        // taken before any driver lock.
        let p = self.peer.status_snapshot();
        // Membership-first role derivation (三分, Ren's rule): a learner is a
        // follower whose id is in the learner set; a node in NEITHER set is a
        // config-identity fault and must never masquerade as a healthy
        // follower (removed node, wrong config, stale ConfState).
        let me = p.node_id.0;
        let role = if p.learners.contains(&me) {
            Role::Learner
        } else if !p.promotable && !p.voters.contains(&me) {
            Role::Unconfigured
        } else {
            p.raw_role
        };
        let applied = self.applied.lock().expect("applied poisoned");
        NodeStatus {
            node_id: p.node_id,
            leader_id: p.leader_hint,
            role,
            term: p.term,
            raft_committed: p.committed,
            applied_index: self.sm.lock().expect("sm poisoned").applied_index().0,
            applied_term: applied.last().map_or(0, |e| e.term),
            fatal: self.fatal.lock().expect("fatal poisoned").clone(),
            step_errors: p.step_errors,
            conf_index: p.conf_applied,
            voters: p.voters,
            learners: p.learners,
            driver_applied: self.driver_applied(),
        }
    }

    /// Read a key from this node's applied state machine (harness verification).
    pub fn get(&self, cf: kv9_engine::ColumnFamily, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.sm.lock().expect("sm poisoned").get(cf, key)
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.proposals.close();
        self.peer.work_signal.stop();
        let _ = self.completion.publish();
    }

    /// One event-driven owner. Tick deadlines are independent of work arrival;
    /// repeated spawn or an invalid interval is refused before creating a thread.
    pub fn spawn(self: &Arc<Self>, tick_every: Duration) -> Result<std::thread::JoinHandle<()>> {
        if tick_every.is_zero() || tick_every > Duration::from_secs(60) {
            return Err(Error::Config(
                "Raft tick interval must be within (0, 60s]".into(),
            ));
        }
        if self
            .pump_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(Error::Raft(
                "background pump already owns this driver".into(),
            ));
        }
        let driver = Arc::clone(self);
        Ok(std::thread::spawn(move || {
            let mut previous_iteration = None;
            let mut ticks = crate::work::TickDeadline::new(Instant::now(), tick_every);
            while !driver.stop.load(Ordering::Relaxed) && driver.peer.work_signal.begin_turn() {
                let iteration = Instant::now();
                if let Some(previous) = previous_iteration {
                    driver
                        .metrics
                        .pump_iteration_spacing
                        .record(iteration.duration_since(previous), Outcome::Success);
                }
                previous_iteration = Some(iteration);
                let result = {
                    let _owner = driver.pump_gate.lock().expect("pump gate poisoned");
                    if ticks.due(Instant::now()) {
                        driver.peer.tick_once();
                    }
                    driver.step_observed()
                };
                if result.is_err() {
                    break;
                }
                // Applying a bounded Ready can expose its successor without
                // another producer notification. Drain it on the next turn.
                if driver.peer.has_pending_ready() {
                    driver.peer.work_signal.notify();
                }
                let idle = driver.metrics.pump_idle_wait.start();
                driver.peer.work_signal.wait_until(ticks.next());
                idle.finish(Outcome::Success);
            }
        }))
    }
}

fn classify_proposal(result: &Result<ProposedAt>) -> Outcome {
    match result {
        Ok(_) => Outcome::Success,
        Err(Error::NotLeader { .. } | Error::ProposalRefused { .. }) => Outcome::Rejected,
        Err(Error::ProposalUnconfirmed) => Outcome::Unconfirmed,
        Err(_) => Outcome::Error,
    }
}

fn classify_apply_wait(result: &std::result::Result<ApplyWaitOutcome, ApplyWaitError>) -> Outcome {
    match result {
        Ok(ApplyWaitOutcome::Applied(_)) => Outcome::Success,
        Ok(ApplyWaitOutcome::Manifest {
            verdict: crate::ManifestVerdict::Applied { .. },
            ..
        }) => Outcome::Success,
        Ok(ApplyWaitOutcome::Replaced) => Outcome::Replaced,
        // AlreadyApplied is not a newly accepted manifest transition.
        Ok(ApplyWaitOutcome::FenceRejected { .. } | ApplyWaitOutcome::Manifest { .. }) => {
            Outcome::Rejected
        }
        Err(ApplyWaitError::Unconfirmed { .. }) => Outcome::Unconfirmed,
        Err(ApplyWaitError::Failed(_)) => Outcome::Error,
    }
}

fn classify_read(result: &std::result::Result<ReadBarrier, ReadIndexError>) -> Outcome {
    match result {
        Ok(_) => Outcome::Success,
        Err(ReadIndexError::NotLeader { .. }) => Outcome::Rejected,
        Err(ReadIndexError::Unconfirmed { .. }) => Outcome::Unconfirmed,
        Err(ReadIndexError::Failed(_)) => Outcome::Error,
    }
}

/// The typed outcome of [`NodeDriver::wait_applied`] (task #30): what became
/// of the proposal's position, judged on applied state, never on elapsed time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyWaitOutcome {
    /// The exact position APPLIED on this node — built by the driver from the
    /// ring's recorded `(term, index)` at hit time, never by a caller renaming
    /// a `ProposedAt`. That renaming is the disease this card was opened to
    /// kill: a proposal-time pair dressed as an applied receipt survives every
    /// review that doesn't ask where the value came from (review round: the
    /// first draft of THIS fix reintroduced it).
    Applied(kv9_common::AppliedPosition),
    /// The position was consumed by a DIFFERENT entry — another leader's
    /// command, or an election barrier (the entry raft appends on winning).
    /// The proposal will never apply; the correct reaction is to re-propose
    /// on the current leader, not to keep waiting.
    Replaced,
    /// The exact proposal applied — as a fenced write whose fence FAILED
    /// adjudication: the watermark advanced, nothing was written, and the
    /// verdict names the rejected region. Mutually exclusive with `Applied`
    /// by contract: folding it into `Applied` is precisely the silent lost
    /// write this variant exists to prevent, and the caller maps it to the
    /// typed `StaleEpoch {{ region }}` — NEVER a retry (the epoch will not
    /// come back; the client must re-route/re-validate).
    FenceRejected {
        at: kv9_common::AppliedPosition,
        region: kv9_common::RegionId,
    },
    /// The exact proposal applied — as a manifest change; the discriminator's
    /// verdict rides the receipt (task #9). A manifest entry NEVER surfaces
    /// as bare `Applied`: `ManifestVerdict::AlreadyApplied` folded into a
    /// generic success would be the crash-point-3 false acceptance, the same
    /// disease `FenceRejected` guards against for fenced writes.
    Manifest {
        at: kv9_common::AppliedPosition,
        verdict: crate::ManifestVerdict,
    },
}

/// The typed error of [`NodeDriver::wait_applied`] (task #30). `Unconfirmed`
/// is a first-class state, never string-detected: the deadline passed with the
/// position still ahead of every watermark — not applied, not provably
/// replaced. The caller must treat it as UNKNOWN (the entry may still commit
/// later), which is different in kind from `Replaced` (provably never).
#[derive(Debug)]
pub enum ApplyWaitError {
    Unconfirmed {
        index: u64,
        waited: Duration,
    },
    /// The driver itself failed (poisoned apply, machinery error).
    Failed(Error),
}

impl std::fmt::Display for ApplyWaitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApplyWaitError::Unconfirmed { index, waited } => write!(
                f,
                "proposal at index {index} unconfirmed after {waited:?}: \
                 not applied, not provably replaced"
            ),
            ApplyWaitError::Failed(e) => write!(f, "apply wait failed: {e}"),
        }
    }
}

impl From<ApplyWaitError> for Error {
    fn from(e: ApplyWaitError) -> Error {
        match e {
            ApplyWaitError::Failed(inner) => inner,
            unconfirmed => Error::Raft(unconfirmed.to_string()),
        }
    }
}

/// A quorum-confirmed read barrier (task #28): every entry committed before
/// the read was issued is applied on THIS node at or below `index`. An engine
/// snapshot taken AFTER receiving this value observes all of them — take the
/// snapshot after, never before (the seam contract).
///
/// Deliberately neither `Clone` nor `Copy` (interface ruling on the
/// establishing-read seam): this value is a CREDENTIAL — one barrier is
/// exchanged, by value, for exactly one established view. A loop that mints
/// one barrier and reuses it per iteration is thereby unrepresentable
/// (E0507), and a snapshot taken before the barrier has no credential to
/// present. `NOT_CLONE_OR_COPY` below is the compile-time guard: adding
/// either derive back turns it into a build error, not a silently weaker
/// contract.
#[derive(Debug, PartialEq, Eq)]
pub struct ReadBarrier {
    /// The quorum-confirmed read index; the unified driver watermark has
    /// passed it at the moment this value is returned. PRIVATE: the only
    /// constructor is `NodeDriver::read_barrier` — a value another crate
    /// could forge with a struct literal would not be a credential at all
    /// (external construction is E0451; observation goes through
    /// [`Self::index`]).
    index: u64,
}

impl ReadBarrier {
    /// The confirmed index — read-only observation; there is deliberately
    /// no way to build a `ReadBarrier` from one.
    pub fn index(&self) -> u64 {
        self.index
    }

    /// Compile-time probe: const-evaluated even though never read. The
    /// inherent associated const shadows the trait fallback exactly when
    /// `ReadBarrier: Clone` holds (and `Copy: Clone` covers both), turning
    /// the panic into an E0080 build error. Verified in both directions
    /// when introduced: without the derives this compiles clean; adding
    /// `Clone` reds precisely here.
    #[allow(dead_code)]
    const NOT_CLONE_OR_COPY: () = {
        struct Probe<T>(core::marker::PhantomData<T>);
        trait Fallback {
            const CHECK: () = ();
        }
        impl<T> Fallback for Probe<T> {}
        impl<T: Clone> Probe<T> {
            const CHECK: () = panic!("ReadBarrier must be neither Clone nor Copy");
        }
        Probe::<ReadBarrier>::CHECK
    };
}

/// The one seam authority over a node (task #9 round 4): round 3's
/// free-floating token could be minted from driver A and paired with any
/// node B — the capability counted mints but bound nothing. This handle
/// BINDS the authority to the exact node it was minted from: the node
/// reference lives INSIDE (private field), `ManifestSeam::mint` takes only
/// the handle, and there is no second parameter to cross-pair. Production
/// construction is `NodeDriver::mint_seam_handle` alone (once-CAS on that
/// driver); the trait-implementer route to seam authority is gone —
/// a `Liar: ManifestNode` can still exist, but it can never inhabit a
/// production handle, so its `RefusedPreAppend` answers never reach a
/// production slot.
///
/// Deliberately neither `Clone` nor `Copy` (guard below): a duplicable
/// seam authority is two slot tables again.
pub struct SeamHandle {
    node: Arc<dyn ManifestNode>,
}

impl SeamHandle {
    /// Harness-only construction over an arbitrary `ManifestNode` (models a
    /// fresh process incarnation in restart tests, and lets fault-injection
    /// fakes exist WITHOUT being production-eligible).
    #[cfg(any(test, feature = "testing"))]
    pub fn for_harness(node: Arc<dyn ManifestNode>) -> SeamHandle {
        SeamHandle { node }
    }

    pub fn propose_command(&self, cmd: &Command) -> std::result::Result<ProposeOutcome, Error> {
        self.node.propose_command(cmd)
    }

    pub fn wait_manifest(
        &self,
        at: ProposedAt,
        deadline: Duration,
    ) -> std::result::Result<ApplyWaitOutcome, ApplyWaitError> {
        self.node.wait_manifest(at, deadline)
    }

    pub fn manifest_pair(&self, region: u64) -> Result<crate::ManifestPair> {
        self.node.manifest_pair(region)
    }

    pub fn manifest_effect(&self, region: u64, intended: &[u8]) -> Result<bool> {
        self.node.manifest_effect(region, intended)
    }

    pub fn manifest_transition(
        &self,
        region: u64,
        generation: u64,
    ) -> Result<Option<crate::ManifestPair>> {
        self.node.manifest_transition(region, generation)
    }

    /// Compile-time guard (E0080 pattern shared with `ReadBarrier` and
    /// `DrainToken`): adding `Clone`/`Copy` back is a build error.
    #[allow(dead_code)]
    const NOT_CLONE_OR_COPY: () = {
        struct Probe<T>(core::marker::PhantomData<T>);
        trait Fallback {
            const CHECK: () = ();
        }
        impl<T> Fallback for Probe<T> {}
        impl<T: Clone> Probe<T> {
            const CHECK: () = panic!("SeamHandle must be neither Clone nor Copy");
        }
        Probe::<SeamHandle>::CHECK
    };
}

/// The typed outcome of submitting a command through the propose face
/// (task #9 round 3). The distinction is load-bearing for slot settlement:
/// only `RefusedPreAppend` proves NOTHING entered the log from this send —
/// a generic error proves nothing either way and must hold the slot.
#[derive(Debug)]
pub enum ProposeOutcome {
    /// The command was accepted at this position (a claim, not a commit —
    /// correlate by term+index as always).
    Accepted(ProposedAt),
    /// Refused strictly BEFORE any append: leadership is verified inside
    /// the peer's lock and raft-rs `propose` returns error without
    /// appending, so this send left no trace in the log. On a FIRST send
    /// this settles the attempt; on a re-send it says nothing about
    /// earlier sends.
    RefusedPreAppend(Error),
}

/// The seam-facing face of a driven node (task #9): propose + typed manifest
/// receipt + the P1 pair read. The region runtime's `ManifestSeam` does NOT
/// hold this trait directly — it holds a [`SeamHandle`], which wraps the
/// node reference privately and is minted at most once per driver
/// ([`NodeDriver::mint_seam_handle`]). The trait exists as the internal
/// abstraction the handle delegates to, and as the surface harness fakes
/// implement; implementers relay observations, they do not issue
/// capabilities — only a handle carries seam authority, and only the real
/// driver (or a testing-gated harness constructor) can produce one.
pub trait ManifestNode: Send + Sync {
    /// Submit; `Err` = AMBIGUOUS failure (the send may or may not have
    /// entered the log) — implementations must only return
    /// [`ProposeOutcome::RefusedPreAppend`] when refusal provably preceded
    /// any append.
    fn propose_command(&self, cmd: &Command) -> std::result::Result<ProposeOutcome, Error>;
    fn wait_manifest(
        &self,
        at: ProposedAt,
        deadline: Duration,
    ) -> std::result::Result<ApplyWaitOutcome, ApplyWaitError>;
    fn manifest_pair(&self, region: u64) -> Result<crate::ManifestPair>;
    fn manifest_effect(&self, _region: u64, _intended: &[u8]) -> Result<bool> {
        Ok(false)
    }
    fn manifest_transition(
        &self,
        _region: u64,
        _generation: u64,
    ) -> Result<Option<crate::ManifestPair>> {
        Ok(None)
    }
}

impl<S: PersistentRaftStorage, E: crate::ApplyStore + 'static> ManifestNode for NodeDriver<S, E> {
    fn propose_command(&self, cmd: &Command) -> std::result::Result<ProposeOutcome, Error> {
        // propose_traced verifies leadership and appends under ONE peer
        // lock, and raft-rs `propose` returns error without appending — an
        // Err here is a certain pre-append refusal, never ambiguous.
        match self.propose(cmd) {
            Ok(at) => Ok(ProposeOutcome::Accepted(at)),
            Err(e) => Ok(ProposeOutcome::RefusedPreAppend(e)),
        }
    }

    fn manifest_effect(&self, region: u64, intended: &[u8]) -> Result<bool> {
        self.sm
            .lock()
            .expect("sm poisoned")
            .manifest_effect(region, intended)
    }

    fn manifest_transition(
        &self,
        region: u64,
        generation: u64,
    ) -> Result<Option<crate::ManifestPair>> {
        self.sm
            .lock()
            .expect("sm poisoned")
            .manifest_transition(region, generation)
    }

    fn wait_manifest(
        &self,
        at: ProposedAt,
        deadline: Duration,
    ) -> std::result::Result<ApplyWaitOutcome, ApplyWaitError> {
        self.wait_applied(at, deadline)
    }

    fn manifest_pair(&self, region: u64) -> Result<crate::ManifestPair> {
        NodeDriver::manifest_pair(self, region)
    }
}

/// Typed failure of [`NodeDriver::read_barrier`] (task #28). Independent from
/// [`ApplyWaitError`] by seam contract, and its variants deliberately make
/// "quorum unreachable" DISTINGUISHABLE from a connection-level failure: the
/// partition E2E's typed-exclusivity assertions cannot be written otherwise
/// (an isolated leader times out here — it never surfaces a transport error,
/// because the read never touches the transport on this node).
#[derive(Debug)]
pub enum ReadIndexError {
    /// This node is not the leader; the establishing read type surfaces this
    /// with the hint so the caller can re-route.
    NotLeader { hint: Option<NodeId> },
    /// The deadline passed without the barrier establishing. `phase` says
    /// which half never arrived — the quorum confirmation (the isolated-
    /// leader case: check_quorum has not yet deposed us, but no quorum will
    /// acknowledge the read), or the local apply catching up to the confirmed
    /// index. Unknown outcome: the caller must not serve the read.
    Unconfirmed {
        phase: BarrierPhase,
        waited: Duration,
    },
    /// The driver itself is poisoned.
    Failed(Error),
}

/// Which half of the read barrier did not complete (see
/// [`ReadIndexError::Unconfirmed`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarrierPhase {
    /// No quorum acknowledgment for the read index arrived.
    QuorumConfirmation,
    /// The quorum confirmed an index, but this node's unified driver
    /// watermark did not reach it in time.
    ApplyCatchUp,
}

impl std::fmt::Display for ReadIndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadIndexError::NotLeader { hint } => match hint {
                Some(id) => write!(f, "not the leader; current leader hint: node {}", id.0),
                None => write!(f, "not the leader; no current leader known"),
            },
            ReadIndexError::Unconfirmed { phase, waited } => write!(
                f,
                "read barrier unconfirmed after {waited:?} ({})",
                match phase {
                    BarrierPhase::QuorumConfirmation => "no quorum confirmation for the read index",
                    BarrierPhase::ApplyCatchUp => "apply did not catch up to the confirmed index",
                }
            ),
            ReadIndexError::Failed(e) => write!(f, "read barrier failed: {e}"),
        }
    }
}

impl From<ReadIndexError> for Error {
    fn from(e: ReadIndexError) -> Error {
        match e {
            ReadIndexError::Failed(inner) => inner,
            ReadIndexError::NotLeader { hint } => Error::NotLeader { leader: hint },
            // The typed mapping is the point (partition acceptance): each
            // barrier phase keeps its identity all the way to the public
            // error, so "quorum unreachable" stays distinguishable from
            // both local lag AND any transport failure without string
            // parsing.
            ReadIndexError::Unconfirmed { phase, .. } => Error::ReadUnconfirmed {
                phase: match phase {
                    BarrierPhase::QuorumConfirmation => {
                        kv9_common::ReadBarrierPhase::QuorumConfirmation
                    }
                    BarrierPhase::ApplyCatchUp => kv9_common::ReadBarrierPhase::ApplyCatchUp,
                },
            },
        }
    }
}

/// Post-conf-change receipt: the exact applied `(term, index)` and the
/// membership `apply_conf_change` actually produced (never the proposal-time
/// expectation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfChangeReceipt {
    pub applied: ProposedAt,
    /// Log index at which this configuration took effect.
    pub conf_index: u64,
    pub voters: Vec<u64>,
    pub learners: Vec<u64>,
}

fn push_ring(applied: &mut Vec<RingEntry>, index: u64, term: u64, outcome: crate::ApplyOutcome) {
    applied.push(RingEntry {
        index,
        term,
        outcome,
    });
    let len = applied.len();
    if len > APPLIED_RING {
        applied.drain(..len - APPLIED_RING);
    }
}

fn single_change(node: NodeId, kind: ConfChangeType) -> ConfChangeV2 {
    let mut step = ConfChangeSingle::default();
    step.set_change_type(kind);
    step.node_id = node.0;
    let mut cc = ConfChangeV2::default();
    cc.set_changes(vec![step].into());
    cc
}

#[cfg(test)]
mod tests {

    #[test]
    fn apply_lag_requires_stable_commit_and_term_and_a_compatible_watermark() {
        let mut sample = super::ApplyLagObservation {
            term_before: 3,
            term_after: 3,
            committed_before: 10,
            committed_after: 10,
            driver_applied: Some(super::DriverAppliedPosition { term: 2, index: 7 }),
        };
        assert_eq!(sample.lag(), Some(3));
        sample.committed_after = 11;
        assert_eq!(sample.lag(), None);
        sample.committed_after = 10;
        sample.term_after = 4;
        assert_eq!(sample.lag(), None);
        sample.term_after = 3;
        sample.driver_applied = Some(super::DriverAppliedPosition { term: 3, index: 11 });
        assert_eq!(
            sample.lag(),
            None,
            "incompatible observation is not zero lag"
        );
        sample.driver_applied = Some(super::DriverAppliedPosition { term: 4, index: 7 });
        assert_eq!(sample.lag(), None);
        sample.driver_applied = None;
        assert_eq!(sample.lag(), None);
    }
    use super::*;
    use crate::rawnode::RaftPeer;
    use crate::transport::{InProcHub, RaftTransport};
    use crate::{KvOp, RaftGroup};
    use kv9_common::{NodeId, RegionId};
    use kv9_engine::{ColumnFamily, Mutation, ReadView, ScanEntry, WriteBatch};

    #[test]
    fn pump_service_finishes_only_after_the_actual_transport_drain_returns() {
        struct GatedTransport {
            entered: std::sync::mpsc::Sender<Instant>,
            release: Mutex<std::sync::mpsc::Receiver<()>>,
        }
        impl RaftTransport for GatedTransport {
            fn send(&self, _: NodeId, _: raft::prelude::Message) {}
            fn drain(&self) -> Vec<raft::prelude::Message> {
                self.entered.send(Instant::now()).unwrap();
                self.release.lock().unwrap().recv().unwrap();
                Vec::new()
            }
        }
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let peer = Arc::new(RaftPeer::new(NodeId(1), RegionId(1), &[NodeId(1)]).unwrap());
        let driver = NodeDriver::new(
            peer,
            Arc::new(GatedTransport {
                entered: entered_tx,
                release: Mutex::new(release_rx),
            }),
            MemStateMachine::new(),
        )
        .unwrap();
        let worker = driver.clone();
        let task = std::thread::spawn(move || worker.step());
        let entered = entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let during = driver.metrics().pump_service.snapshot();
        let released = Instant::now();
        release_tx.send(()).unwrap();
        task.join().unwrap().unwrap();
        assert!(
            during.outcomes.iter().all(|h| h.count == 0),
            "unfinished pump was reported as completed"
        );
        let after = driver.metrics().pump_service.snapshot();
        let service = &after.outcomes[Outcome::Success as usize];
        assert_eq!(service.count, 1);
        assert!(
            service.sum_ns >= released.duration_since(entered).as_nanos() as u64,
            "pump observation omitted time inside the actual drain"
        );
        assert_eq!(
            driver.metrics().pump_idle_wait.snapshot().outcomes[Outcome::Success as usize].count,
            0
        );
        assert_eq!(
            driver.metrics().pump_iteration_spacing.snapshot().outcomes[Outcome::Success as usize]
                .count,
            0
        );
    }

    #[test]
    fn background_pump_records_returned_waits_and_no_first_spacing_sample() {
        let driver = single_node_driver();
        let initial =
            driver.metrics().pump_service.snapshot().outcomes[Outcome::Success as usize].count;
        let interval = Duration::from_millis(2);
        let started = Instant::now();
        let task = driver.spawn(interval).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while driver.metrics().pump_idle_wait.snapshot().outcomes[Outcome::Success as usize].count
            < 3
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(1));
        }
        driver.stop();
        task.join().unwrap();
        let service = driver.metrics().pump_service.snapshot();
        let idle = driver.metrics().pump_idle_wait.snapshot();
        let spacing = driver.metrics().pump_iteration_spacing.snapshot();
        let iterations = service.outcomes[Outcome::Success as usize].count - initial;
        let sleeps = &idle.outcomes[Outcome::Success as usize];
        assert!(
            iterations >= 3,
            "background pump did not make observed progress"
        );
        assert_eq!(sleeps.count, iterations);
        assert!(sleeps.sum_ns > 0, "actual waits were not observed");
        assert!(
            sleeps.sum_ns <= started.elapsed().as_nanos() as u64,
            "idle observation fabricated more wait time than elapsed"
        );
        assert_eq!(
            spacing.outcomes[Outcome::Success as usize].count,
            iterations - 1
        );
        assert_eq!(service.outcomes[Outcome::Error as usize].count, 0);
    }

    #[test]
    fn failed_background_pump_records_error_without_another_idle_wait() {
        let driver = single_node_driver();
        let initial =
            driver.metrics().pump_service.snapshot().outcomes[Outcome::Success as usize].count;
        driver
            .peer()
            .propose_traced(vec![0xff, 0xee, 0xdd])
            .unwrap();
        let task = driver.spawn(Duration::from_millis(2)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while driver.status().fatal.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        driver.stop();
        task.join().unwrap();
        assert!(
            driver.status().fatal.is_some(),
            "committed decode failure did not stop the pump"
        );
        let service = driver.metrics().pump_service.snapshot();
        let successes = service.outcomes[Outcome::Success as usize].count - initial;
        assert_eq!(
            service.outcomes[Outcome::Error as usize].count,
            1,
            "failed pump was reported as successful"
        );
        assert_eq!(
            driver.metrics().pump_idle_wait.snapshot().outcomes[Outcome::Success as usize].count,
            successes,
            "failed pump recorded an extra idle sleep"
        );
        assert_eq!(
            driver.metrics().pump_iteration_spacing.snapshot().outcomes[Outcome::Success as usize]
                .count,
            successes
        );
    }

    fn single_node_driver() -> Arc<NodeDriver> {
        let hub = InProcHub::new();
        let peer = Arc::new(RaftPeer::new(NodeId(1), RegionId(1), &[NodeId(1)]).unwrap());
        let endpoint = hub.endpoint(NodeId(1));
        let driver = NodeDriver::new(
            peer,
            Arc::new(endpoint) as Arc<dyn RaftTransport>,
            MemStateMachine::new(),
        )
        .expect("drain token minted once per peer");
        driver.peer().campaign().unwrap();
        for _ in 0..50 {
            driver.tick_and_step().unwrap();
            if driver.status().role == Role::Leader {
                break;
            }
        }
        assert_eq!(driver.status().role, Role::Leader);
        driver
    }

    #[test]
    fn queued_proposals_keep_exact_positions_and_wait_for_real_apply() {
        let driver = single_node_driver();
        let before = driver.driver_applied().unwrap();
        driver.pause_apply(true);
        let tickets: Vec<_> = (0..4u8)
            .map(|value| {
                driver
                    .proposals
                    .enqueue(
                        Command::Put {
                            cf: 0,
                            key: vec![value],
                            value: vec![value + 1],
                        }
                        .encode(),
                        Some(before.term),
                        Instant::now() + Duration::from_secs(5),
                    )
                    .unwrap()
            })
            .collect();
        assert_eq!(driver.proposal_queue_snapshot().queued, 4);
        driver.step().unwrap();
        let positions: Vec<_> = tickets
            .into_iter()
            .map(|ticket| ticket.wait().unwrap())
            .collect();
        for (offset, at) in positions.iter().copied().enumerate() {
            assert_eq!(at.term, before.term);
            assert_eq!(at.index.0, before.index + offset as u64 + 1);
            assert!(
                matches!(
                    driver.wait_applied(at, Duration::ZERO),
                    Err(ApplyWaitError::Unconfirmed { .. })
                ),
                "a submission receipt was promoted to apply success"
            );
            assert_eq!(
                driver.get(ColumnFamily::Default, &[offset as u8]).unwrap(),
                None
            );
        }
        driver.pause_apply(false);
        driver.step().unwrap();
        for (offset, at) in positions.into_iter().enumerate() {
            assert!(
                matches!(driver.wait_applied(at, Duration::ZERO), Ok(ApplyWaitOutcome::Applied(actual)) if actual.term == at.term && actual.index == at.index.0)
            );
            assert_eq!(
                driver.get(ColumnFamily::Default, &[offset as u8]).unwrap(),
                Some(vec![offset as u8 + 1])
            );
        }
        let queue = driver.proposal_queue_snapshot();
        assert_eq!(
            (queue.queued, queue.in_flight, queue.encoded_bytes),
            (0, 0, 0)
        );
    }

    #[test]
    fn queued_term_check_occurs_at_submission_and_inbound_leadership_wins() {
        let driver = single_node_driver();
        let before = driver.driver_applied().unwrap();
        let command = Command::Put {
            cf: 0,
            key: b"stale".to_vec(),
            value: b"v".to_vec(),
        };
        let stale = driver
            .proposals
            .enqueue(
                command.encode(),
                Some(before.term + 1),
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap();
        driver.step().unwrap();
        assert!(matches!(stale.wait(), Err(Error::WriteConflict(_))));
        assert_eq!(driver.driver_applied().unwrap(), before);

        let hub = InProcHub::new();
        let peer = Arc::new(RaftPeer::new(NodeId(1), RegionId(1), &[NodeId(1)]).unwrap());
        let inbound = Arc::new(hub.endpoint(NodeId(1)));
        let sender = hub.endpoint(NodeId(2));
        let driver = NodeDriver::new(peer, inbound, MemStateMachine::new()).unwrap();
        driver.peer().campaign().unwrap();
        driver.step().unwrap();
        assert_eq!(driver.status().role, Role::Leader);
        let term = driver.status().term;
        let queued = driver
            .proposals
            .enqueue(
                command.encode(),
                Some(term),
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap();
        let mut heartbeat = raft::prelude::Message::default();
        heartbeat.set_msg_type(raft::prelude::MessageType::MsgHeartbeat);
        heartbeat.from = 2;
        heartbeat.to = 1;
        heartbeat.term = term + 1;
        sender.send(NodeId(1), heartbeat);
        driver.step().unwrap();
        assert!(
            matches!(
                queued.wait(),
                Err(Error::NotLeader {
                    leader: Some(NodeId(2))
                })
            ),
            "queued proposal used leadership from before the inbound term change"
        );
        assert_eq!(driver.get(ColumnFamily::Default, b"stale").unwrap(), None);
    }

    #[test]
    fn queued_suffix_is_refused_when_a_real_committed_decode_failure_stops_the_owner() {
        let driver = single_node_driver();
        let bad = driver
            .peer()
            .propose_traced(vec![0xff, 0xee, 0xdd])
            .unwrap();
        let mut tickets: Vec<_> = (0..65u8)
            .map(|value| {
                driver
                    .proposals
                    .enqueue(
                        Command::Put {
                            cf: 0,
                            key: vec![value],
                            value: vec![1],
                        }
                        .encode(),
                        None,
                        Instant::now() + Duration::from_secs(5),
                    )
                    .unwrap()
            })
            .collect();
        let suffix = tickets.pop().unwrap();
        assert!(
            driver.step().is_err(),
            "malformed committed command did not fence the owner"
        );
        assert!(
            matches!(
                suffix.wait(),
                Err(Error::ProposalRefused {
                    reason: kv9_common::ProposalRefusal::Stopped
                })
            ),
            "fatal pump retained an unclaimed request"
        );
        for ticket in tickets {
            let at = ticket.wait().unwrap();
            assert!(
                driver.wait_applied(at, Duration::ZERO).is_err(),
                "failed Ready produced false apply success"
            );
        }
        assert!(driver.status().applied_index < bad.index.0);
        let queue = driver.proposal_queue_snapshot();
        assert!(queue.stopped);
        assert_eq!(
            (queue.queued, queue.in_flight, queue.encoded_bytes),
            (0, 0, 0)
        );
    }

    #[test]
    fn parked_owner_serves_a_read_before_its_next_tick_and_refuses_a_second_owner() {
        let driver = single_node_driver();
        let parked = driver.peer.work_signal.observe_next_park();
        let task = driver.spawn(Duration::from_secs(60)).unwrap();
        assert!(driver.spawn(Duration::from_millis(1)).is_err());
        parked.recv_timeout(Duration::from_secs(5)).unwrap();
        // The observer fires under the predicate lock immediately before
        // Condvar::wait releases it. This submission must survive either side
        // of the check/park boundary; a timer-driven owner misses the deadline.
        let result = driver.read_barrier(Duration::from_secs(2));
        driver.stop();
        task.join().unwrap();
        assert!(
            result.is_ok(),
            "parked owner failed to process local work: {result:?}"
        );
    }

    #[test]
    fn completion_notifications_require_the_exact_applied_receipt() {
        let driver = single_node_driver();
        let at = driver
            .propose(&Command::Write {
                ops: vec![KvOp::Put {
                    cf: 0,
                    key: b"completion".to_vec(),
                    value: b"confirmed".to_vec(),
                }],
            })
            .unwrap();
        let parked = driver.completion.observe_next_park();
        let waiter = driver.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let task = std::thread::spawn(move || {
            tx.send(waiter.wait_applied(at, Duration::from_secs(5)))
                .unwrap();
        });
        parked.recv_timeout(Duration::from_secs(1)).unwrap();
        let parked_again = driver.completion.observe_next_park();
        driver.completion.publish().unwrap(); // a hint without a receipt
        parked_again.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(
            matches!(rx.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)),
            "notification was mistaken for a committed write receipt"
        );
        driver.step().unwrap();
        let result = rx.recv_timeout(Duration::from_secs(1)).unwrap().unwrap();
        task.join().unwrap();
        assert_eq!(
            result,
            ApplyWaitOutcome::Applied(kv9_common::AppliedPosition {
                term: at.term,
                index: at.index.0,
            })
        );
    }

    #[test]
    fn fatal_application_wakes_a_parked_completion_waiter() {
        let driver = single_node_driver();
        let at = driver
            .peer()
            .propose_traced(vec![0xff, 0xee, 0xdd])
            .unwrap();
        let parked = driver.completion.observe_next_park();
        let waiter = driver.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let task = std::thread::spawn(move || {
            tx.send(waiter.wait_applied(at, Duration::from_secs(5)))
                .unwrap();
        });
        parked.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(driver.step().is_err());
        let result = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        task.join().unwrap();
        assert!(
            matches!(result, Err(ApplyWaitError::Failed(_))),
            "fatal notification did not preserve the failed outcome: {result:?}"
        );
    }

    #[test]
    fn transport_and_campaign_wake_every_voter_without_election_tick_polling() {
        let hub = InProcHub::new();
        let voters = [NodeId(1), NodeId(2), NodeId(3)];
        let drivers: Vec<_> = voters
            .iter()
            .map(|id| {
                NodeDriver::new(
                    Arc::new(RaftPeer::new(*id, RegionId(1), &voters).unwrap()),
                    Arc::new(hub.endpoint(*id)),
                    MemStateMachine::new(),
                )
                .unwrap()
            })
            .collect();
        let parked: Vec<_> = drivers
            .iter()
            .map(|d| d.peer.work_signal.observe_next_park())
            .collect();
        let tasks: Vec<_> = drivers
            .iter()
            .map(|d| d.spawn(Duration::from_secs(60)).unwrap())
            .collect();
        for observer in parked {
            observer.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        drivers[0].peer().campaign().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while drivers[0].status().role != Role::Leader && Instant::now() < deadline {
            std::thread::yield_now();
        }
        let elected = drivers[0].status().role == Role::Leader;
        let read = drivers[0].read_barrier(Duration::from_secs(2));
        for driver in &drivers {
            driver.stop();
        }
        for task in tasks {
            task.join().unwrap();
        }
        assert!(
            elected,
            "campaign or inbound work remained asleep until a tick"
        );
        assert!(
            read.is_ok(),
            "quorum read failed without timer polling: {read:?}"
        );
    }

    /// Negative 1 (Tess's review): an UNDECODABLE committed entry is fatal —
    /// it must never enter the success ring, wait_applied must error (not
    /// report success), the pump stops, and status carries the poison.
    #[test]
    fn undecodable_committed_entry_poisons_the_driver() {
        let driver = single_node_driver();
        // Propose raw garbage bytes directly (below the Command layer).
        let at = driver
            .peer()
            .propose_traced(vec![0xFF, 0xEE, 0xDD])
            .unwrap();
        let mut poisoned = false;
        for _ in 0..50 {
            if driver.tick_and_step().is_err() {
                poisoned = true;
                break;
            }
        }
        assert!(poisoned, "a bad committed entry must fail the pump");
        let status = driver.status();
        assert!(status.fatal.is_some(), "status must surface the poison");
        // The failed entry is NOT reported as success.
        assert!(driver.wait_applied(at, Duration::from_millis(50)).is_err());
        // The watermark did not advance over the hole.
        assert!(status.applied_index < at.index.0);
    }

    /// An engine whose writes always fail (apply-path fault injection).
    struct FailingEngine;
    impl kv9_engine::Engine for FailingEngine {
        fn get(&self, _: ColumnFamily, _: &[u8]) -> kv9_common::Result<Option<Vec<u8>>> {
            Ok(None) // watermark recovery read: pretend empty
        }
        fn write(&self, _: WriteBatch) -> kv9_common::Result<()> {
            Err(Error::Engine("injected write failure".into()))
        }
        fn scan(
            &self,
            _: ColumnFamily,
            _: &[u8],
            _: &[u8],
            _: usize,
        ) -> kv9_common::Result<Vec<ScanEntry>> {
            Ok(Vec::new())
        }
        fn delete_range(&self, _: ColumnFamily, _: &[u8], _: &[u8]) -> kv9_common::Result<()> {
            Err(Error::Engine("injected".into()))
        }
        fn checksum(&self, _: ColumnFamily, _: &[u8], _: &[u8]) -> kv9_common::Result<u64> {
            Ok(0)
        }
        fn snapshot(&self) -> kv9_common::Result<Box<dyn ReadView + '_>> {
            Err(Error::Engine("injected".into()))
        }
        fn durability(&self) -> kv9_engine::Durability {
            kv9_engine::Durability::Volatile
        }
    }
    impl kv9_engine::ReplicatedEngine for FailingEngine {
        fn write_applied(
            &self,
            _: WriteBatch,
            _: kv9_common::AppliedPosition,
        ) -> kv9_common::Result<()> {
            Err(Error::Engine("injected write failure".into()))
        }
        fn applied_position(&self) -> kv9_common::Result<kv9_engine::DurableAppliedPosition> {
            Ok(kv9_engine::DurableAppliedPosition::Volatile)
        }
    }
    // Silence unused-variant lint on Mutation import in some cfgs.
    const _: Option<Mutation> = None;

    /// Negative 2 (Tess's review): a VALID command whose engine apply fails is
    /// equally fatal — decode success must not mask apply failure.
    #[test]
    fn failed_apply_poisons_the_driver() {
        let hub = InProcHub::new();
        let peer = Arc::new(RaftPeer::new(NodeId(1), RegionId(1), &[NodeId(1)]).unwrap());
        let endpoint = hub.endpoint(NodeId(1));
        let driver = NodeDriver::new(
            peer,
            Arc::new(endpoint) as Arc<dyn RaftTransport>,
            MemStateMachine::with_engine(Arc::new(FailingEngine)).unwrap(),
        )
        .expect("drain token minted once per peer");
        driver.peer().campaign().unwrap();
        for _ in 0..50 {
            if driver.tick_and_step().is_err() {
                break;
            }
            if driver.status().role == Role::Leader {
                break;
            }
        }
        let at = driver
            .propose(&Command::Put {
                cf: 0,
                key: b"k".to_vec(),
                value: b"v".to_vec(),
            })
            .unwrap();
        let mut poisoned = false;
        for _ in 0..50 {
            if driver.tick_and_step().is_err() {
                poisoned = true;
                break;
            }
        }
        assert!(poisoned, "an apply failure must fail the pump");
        assert!(driver.status().fatal.is_some());
        assert!(driver.wait_applied(at, Duration::from_millis(50)).is_err());
    }

    /// Sensitivity control: on a HEALTHY driver the same shapes succeed — the
    /// poison path is reachable only through real failures, not always-on.
    /// Common-piece regression 1 (watermark contract): all three entry kinds
    /// advance the unified watermark, and the pair members always come from
    /// the same entry. The FIRST assertion is also the liveness sensitivity
    /// Tess pinned: with NO application proposal, a fresh leader's watermark
    /// term must reach the leader term purely via the election no-op — if
    /// someone suppresses Noop processing, this reds.
    #[test]
    fn driver_applied_advances_through_all_three_entry_kinds() {
        let driver = single_node_driver();
        // Election no-op only (nothing proposed yet): term must equal the
        // leader term — the current-term barrier's liveness anchor.
        let noop_wm = driver.driver_applied().expect("noop must set watermark");
        assert_eq!(noop_wm.term, driver.status().term);

        // Command advances it further, same term.
        let at = driver
            .propose(&Command::Put {
                cf: 0,
                key: b"wm".to_vec(),
                value: b"1".to_vec(),
            })
            .unwrap();
        for _ in 0..50 {
            driver.tick_and_step().unwrap();
            if driver.status().applied_index >= at.index.0 {
                break;
            }
        }
        let cmd_wm = driver.driver_applied().unwrap();
        assert!(cmd_wm.index >= at.index.0 && cmd_wm.index > noop_wm.index);
        assert_eq!(cmd_wm.term, at.term);

        // ConfChange advances it too (the command-scoped watermark would NOT
        // move here — that asymmetry is why this quantity exists).
        let conf_at = driver.add_learner(NodeId(9)).unwrap();
        let mut conf_ok = false;
        for _ in 0..100 {
            driver.tick_and_step().unwrap();
            if driver
                .wait_conf_applied(conf_at, Duration::from_millis(1))
                .is_ok()
            {
                conf_ok = true;
                break;
            }
        }
        assert!(conf_ok, "conf change never applied");
        let receipt = driver
            .wait_conf_applied(conf_at, Duration::from_millis(100))
            .unwrap();
        let conf_wm = driver.driver_applied().unwrap();
        // Exact same-entry pair: the watermark must BE the conf entry's
        // position, not merely lie beyond the command's — a fabricated pair
        // (right index, wrong term or vice versa) must not satisfy this.
        assert_eq!(
            (conf_wm.term, conf_wm.index),
            (receipt.applied.term, receipt.conf_index),
            "watermark must be the conf entry's own (term,index) pair"
        );
        assert!(conf_wm.index > cmd_wm.index);
    }

    /// Common-piece regression 2 — the LOAD-BEARING negative: a failed item
    /// must not advance the watermark. The bootstrap current-term barrier's
    /// safety proof stands on this contiguity. TWO assertions, each lit by
    /// its own verified mutant (review finding — the first mutant redded on
    /// the wrong one and the credit had to be split):
    ///   - "must poison" (arming proof): a poison-to-skip mutant reds HERE —
    ///     it proves the mutation landed on the tested path, not that the
    ///     watermark held;
    ///   - "contiguity broken" (the property itself): a publish-despite-
    ///     failure mutant — poison still fires, watermark published anyway —
    ///     passes the arming assertion and reds HERE. This line, not the
    ///     first, is what the barrier proof depends on.
    #[test]
    fn driver_applied_never_advances_past_a_failed_item() {
        let driver = single_node_driver();
        let before = driver.driver_applied().expect("noop watermark");
        let at = driver
            .peer()
            .propose_traced(vec![0xFF, 0xEE, 0xDD]) // undecodable → poison
            .unwrap();
        for _ in 0..50 {
            let _ = driver.tick_and_step();
            if driver.status().fatal.is_some() {
                break;
            }
        }
        assert!(driver.status().fatal.is_some(), "garbage entry must poison");
        let after = driver.driver_applied().unwrap();
        assert_eq!(
            after, before,
            "watermark advanced past a failed item: contiguity broken"
        );
        assert!(after.index < at.index.0);
    }

    /// Common-piece regression 3: restart replays the log from zero and the
    /// watermark catches back up by REPROVING, never by guessing an initial
    /// value from another quantity.
    #[test]
    fn driver_applied_catches_up_across_restart_by_replay() {
        use crate::storage::DiskRaftStorage;
        let dir = std::env::temp_dir().join(format!(
            "kv9-wm-restart-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let make = || {
            let hub = InProcHub::new();
            let (storage, _) = DiskRaftStorage::open(&dir, &[1]).unwrap();
            let peer = Arc::new(RaftPeer::with_storage(NodeId(1), RegionId(1), storage).unwrap());
            NodeDriver::new(
                peer,
                Arc::new(hub.endpoint(NodeId(1))) as Arc<dyn RaftTransport>,
                MemStateMachine::new(),
            )
            .expect("drain token minted once per peer")
        };

        // First incarnation: commit a real command, remember the watermark.
        let d1 = make();
        d1.peer().campaign().unwrap();
        let mut at = None;
        for _ in 0..100 {
            d1.tick_and_step().unwrap();
            if d1.status().role == Role::Leader && at.is_none() {
                at = Some(
                    d1.propose(&Command::Put {
                        cf: 0,
                        key: b"persist".to_vec(),
                        value: b"me".to_vec(),
                    })
                    .unwrap(),
                );
            }
            if let Some(a) = at {
                if d1.status().applied_index >= a.index.0 {
                    break;
                }
            }
        }
        let wm1 = d1.driver_applied().expect("first run watermark");
        assert!(wm1.index >= at.unwrap().index.0);
        drop(d1);

        // Restart: fresh driver over the same storage. Watermark starts None
        // (nothing proven THIS run), then replay re-proves it.
        let d2 = make();
        assert_eq!(d2.driver_applied(), None, "restart must not inherit");
        d2.peer().campaign().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            d2.tick_and_step().unwrap();
            if let Some(wm2) = d2.driver_applied() {
                if wm2.index >= wm1.index {
                    break;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "replay never caught the watermark up to the first run"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        // The watermark alone could be satisfied by the new election's no-op;
        // the state machine carrying the first run's write proves the old
        // Command genuinely REPLAYED rather than the position merely being
        // re-published past it.
        assert_eq!(
            d2.get(kv9_engine::ColumnFamily::Default, b"persist")
                .unwrap()
                .as_deref(),
            Some(b"me".as_ref()),
            "replay must re-apply the first run's command, not just re-publish a position"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The monotonic guard's own teeth (review blocker: with the guard
    /// deleted, everything else stays green — this test is the one that
    /// reds). Phenomenon, not cause: a lower candidate handed to the
    /// publication point must not win, regardless of how delivery might some
    /// day produce one.
    #[test]
    fn watermark_publication_refuses_a_regressing_candidate() {
        let driver = single_node_driver();
        driver.publish_driver_applied(DriverAppliedPosition { term: 2, index: 5 });
        let high = driver.driver_applied().unwrap();
        assert_eq!((high.term, high.index), (2, 5));

        driver.publish_driver_applied(DriverAppliedPosition { term: 3, index: 3 });
        assert_eq!(
            driver.driver_applied().unwrap(),
            high,
            "a regressing candidate must not overwrite the watermark"
        );

        // Control: a genuinely higher candidate still advances.
        driver.publish_driver_applied(DriverAppliedPosition { term: 3, index: 6 });
        assert_eq!(
            driver.driver_applied().unwrap(),
            DriverAppliedPosition { term: 3, index: 6 }
        );
    }

    #[test]
    fn control_healthy_driver_applies_and_reports_success() {
        let driver = single_node_driver();
        let at = driver
            .propose(&Command::Put {
                cf: 0,
                key: b"ok".to_vec(),
                value: b"yes".to_vec(),
            })
            .unwrap();
        for _ in 0..50 {
            driver.tick_and_step().unwrap();
            if driver.status().applied_index >= at.index.0 {
                break;
            }
        }
        assert!(matches!(
            driver.wait_applied(at, Duration::from_millis(100)).unwrap(),
            ApplyWaitOutcome::Applied(_)
        ));
        assert!(driver.status().fatal.is_none());
    }

    /// The 2026-08-31 master red, reproduced deterministically (task #30): a
    /// proposal accepted by the old leader, never replicated, its position
    /// consumed by the new leader's election barrier. The barrier enters
    /// neither the command ring nor the state-machine watermark, so before
    /// this fix the wait could only burn its whole deadline and answer with
    /// unavailability; the unified driver watermark is the discriminator.
    ///
    /// Mutant contract: deleting the driver-watermark arm turns this test's
    /// verdict into a deadline burn — it must red at the named Replaced
    /// assertion (and the elapsed-time assertion pins that the verdict comes
    /// from state, not from waiting).
    #[test]
    fn a_proposal_replaced_by_an_election_barrier_reports_replaced_not_deadline() {
        let hub = InProcHub::new();
        let ids = [NodeId(1), NodeId(2), NodeId(3)];
        let mk = |id: NodeId| {
            NodeDriver::new(
                Arc::new(RaftPeer::new(id, RegionId(1), &ids).unwrap()),
                Arc::new(hub.endpoint(id)) as Arc<dyn RaftTransport>,
                MemStateMachine::new(),
            )
            .expect("drain token minted once per peer")
        };
        let d1 = mk(NodeId(1));
        let d2 = mk(NodeId(2));
        let d3 = mk(NodeId(3));
        let pump_all = |n: usize| {
            for _ in 0..n {
                d1.tick_and_step().unwrap();
                d2.tick_and_step().unwrap();
                d3.tick_and_step().unwrap();
            }
        };

        // n1 leads; a seed command applies everywhere (healthy baseline).
        d1.peer().campaign().unwrap();
        for _ in 0..200 {
            pump_all(1);
            if d1.status().role == Role::Leader {
                break;
            }
        }
        assert_eq!(d1.status().role, Role::Leader);
        let seed = d1
            .propose(&Command::Put {
                cf: 0,
                key: b"seed".to_vec(),
                value: b"x".to_vec(),
            })
            .unwrap();
        for _ in 0..200 {
            pump_all(1);
            if [&d2, &d3].iter().all(|d| {
                d.driver_applied()
                    .is_some_and(|wm| wm.index >= seed.index.0)
            }) {
                break;
            }
        }

        // The doomed proposal: accepted by n1, never replicated — everything
        // n1 emits from here until its deposition is eaten by the "network"
        // (the hub inboxes are discarded before n2/n3 next step; a real
        // partition drops packets identically).
        let doomed = d1
            .propose(&Command::Put {
                cf: 0,
                key: b"doomed".to_vec(),
                value: b"never".to_vec(),
            })
            .unwrap();
        // Depose n1: tick it alone past the election timeout with no quorum
        // contact — check_quorum's own discipline steps it down.
        for _ in 0..40 {
            d1.tick_and_step().unwrap();
        }
        assert_ne!(
            d1.status().role,
            Role::Leader,
            "check_quorum must depose a leader with no quorum contact (precondition)"
        );
        hub.endpoint(NodeId(2)).drain();
        hub.endpoint(NodeId(3)).drain();

        // n2/n3 tick together until one wins the new term (randomized
        // election timeouts break the tie; WHICH one wins is irrelevant —
        // only that a new-term leader emerges among the nodes that never saw
        // the proposal). n1 answers but never ticks: a deposed peer whose
        // timer also fires becomes a competing pre-candidate. n1 refuses its
        // vote anyway (longer log) — n2+n3 are the majority, the CI scene.
        for _ in 0..2000 {
            d1.step().unwrap();
            d2.tick_and_step().unwrap();
            d3.tick_and_step().unwrap();
            if [&d2, &d3].iter().any(|d| {
                let s = d.status();
                s.role == Role::Leader && s.term > doomed.term
            }) {
                break;
            }
        }
        assert!(
            [&d2, &d3].iter().any(|d| {
                let s = d.status();
                s.role == Role::Leader && s.term > doomed.term
            }),
            "precondition: a new-term leader among the nodes that never saw the proposal"
        );
        // Let the new leader's barrier replicate to n1, overwriting the
        // doomed entry at its own position.
        for _ in 0..300 {
            d1.step().unwrap();
            d2.tick_and_step().unwrap();
            d3.tick_and_step().unwrap();
            if d1
                .driver_applied()
                .is_some_and(|wm| wm.index >= doomed.index.0 && wm.term > doomed.term)
            {
                break;
            }
        }

        // The CI-scene preconditions, by name: unified watermark past the
        // position at a NEWER term; command watermark still below it (no
        // later command traffic — the arm this test exists for).
        let wm = d1.driver_applied().expect("barrier applied on n1");
        assert!(
            wm.index >= doomed.index.0 && wm.term > doomed.term,
            "precondition: the barrier must have consumed the position"
        );
        assert!(
            d1.status().applied_index < doomed.index.0,
            "precondition: the command watermark must still be below the position"
        );

        let asked = Instant::now();
        let outcome = d1
            .wait_applied(doomed, Duration::from_secs(5))
            .expect("a consumed position is a verdict, not an error");
        assert_eq!(
            outcome,
            ApplyWaitOutcome::Replaced,
            "a barrier-consumed position must report Replaced: the proposal              provably never applied (master-red scene: driver watermark passed              it, command ring never saw it)"
        );
        assert!(
            asked.elapsed() < Duration::from_millis(500),
            "the verdict must come from applied state, not from burning the deadline"
        );
    }

    /// Ring-eviction honesty (task #30 review round): an index below the
    /// ring's oldest retained entry, with the ring at capacity, may have
    /// applied and been evicted — indistinguishable from never-applied. The
    /// answer must be Unconfirmed, never a fabricated Replaced (which would
    /// invite retrying a possibly-succeeded non-idempotent write).
    #[test]
    fn an_index_below_the_ring_floor_is_unconfirmed_not_replaced() {
        let driver = single_node_driver();
        let first = driver
            .propose(&Command::Put {
                cf: 0,
                key: b"k0".to_vec(),
                value: b"v".to_vec(),
            })
            .unwrap();
        // Push the ring to capacity so the first command is evicted.
        let mut last = first;
        for i in 0..APPLIED_RING {
            last = driver
                .propose(&Command::Put {
                    cf: 0,
                    key: format!("k{i}").into_bytes(),
                    value: b"v".to_vec(),
                })
                .unwrap();
            for _ in 0..50 {
                driver.tick_and_step().unwrap();
                if driver.status().applied_index >= last.index.0 {
                    break;
                }
            }
        }
        assert!(driver.status().applied_index >= last.index.0);

        // The first command DID apply — but the ring no longer remembers it.
        let err = driver
            .wait_applied(first, Duration::from_secs(5))
            .expect_err("below the ring floor the outcome is unknowable");
        assert!(
            matches!(err, ApplyWaitError::Unconfirmed { .. }),
            "an evicted position must be Unconfirmed, never Replaced: {err}"
        );
        // Control: the LAST command is still in the ring and reports Applied.
        assert!(matches!(
            driver
                .wait_applied(last, Duration::from_millis(100))
                .unwrap(),
            ApplyWaitOutcome::Applied(_)
        ));
        assert!(
            driver.metrics().application_wait.snapshot().outcomes[Outcome::Unconfirmed as usize]
                .count
                >= 1,
            "real protocol outcome missing from observation"
        );
    }
    /// The SECOND replacement shape (task #30 review round; construction by
    /// Cindy's verification probe): the position is consumed not by a barrier
    /// but by another leader's COMMAND — a ring HIT at a different term. The
    /// `term == at.term` comparison is the only line separating this from
    /// Applied, and reporting Applied here tells the caller its write
    /// succeeded while the slot holds someone else's write. Two doomed
    /// proposals make both shapes at once: the new leader's barrier eats the
    /// first slot (watermark arm), its own command eats the second (this arm).
    ///
    /// Mutant contract: `term == at.term` → `true` must red at the named
    /// assertion below.
    #[test]
    fn a_position_taken_by_another_leaders_command_is_replaced_not_applied() {
        let hub = InProcHub::new();
        let ids = [NodeId(1), NodeId(2), NodeId(3)];
        let mk = |id: NodeId| {
            NodeDriver::new(
                Arc::new(RaftPeer::new(id, RegionId(1), &ids).unwrap()),
                Arc::new(hub.endpoint(id)) as Arc<dyn RaftTransport>,
                MemStateMachine::new(),
            )
            .expect("drain token minted once per peer")
        };
        let d1 = mk(NodeId(1));
        let d2 = mk(NodeId(2));
        let d3 = mk(NodeId(3));
        let pump_all = |n: usize| {
            for _ in 0..n {
                d1.tick_and_step().unwrap();
                d2.tick_and_step().unwrap();
                d3.tick_and_step().unwrap();
            }
        };
        d1.peer().campaign().unwrap();
        for _ in 0..200 {
            pump_all(1);
            if d1.status().role == Role::Leader {
                break;
            }
        }
        assert_eq!(d1.status().role, Role::Leader);

        // Two doomed proposals, never replicated: the network eats everything
        // n1 emits while it ticks itself into check_quorum deposition.
        let _doomed1 = d1
            .propose(&Command::Put {
                cf: 0,
                key: b"d1".to_vec(),
                value: b"x".to_vec(),
            })
            .unwrap();
        let doomed2 = d1
            .propose(&Command::Put {
                cf: 0,
                key: b"d2".to_vec(),
                value: b"x".to_vec(),
            })
            .unwrap();
        for _ in 0..40 {
            d1.tick_and_step().unwrap();
            hub.endpoint(NodeId(2)).drain();
            hub.endpoint(NodeId(3)).drain();
        }
        assert_ne!(d1.status().role, Role::Leader, "precondition: n1 deposed");

        // A new leader emerges among the nodes that never saw the proposals.
        for _ in 0..2000 {
            d1.step().unwrap();
            d2.tick_and_step().unwrap();
            d3.tick_and_step().unwrap();
            if [&d2, &d3].iter().any(|d| d.status().role == Role::Leader) {
                break;
            }
        }
        let leader = if d2.status().role == Role::Leader {
            &d2
        } else {
            &d3
        };
        assert_eq!(
            leader.status().role,
            Role::Leader,
            "precondition: a new leader"
        );
        // Its barrier takes doomed1's slot; this COMMAND takes doomed2's —
        // and that placement is ASSERTED, not hoped for (Cindy's probe
        // hardening): the watermark precondition below cannot distinguish
        // "our index holds a command" from "our index holds a barrier and a
        // LATER command moved the watermark", and only the former exercises
        // the ring-hit arm this test exists to guard. Branch selection must
        // be pinned, or a passing mutant check is a sample, not a guarantee.
        let real = leader
            .propose(&Command::Put {
                cf: 0,
                key: b"real".to_vec(),
                value: b"y".to_vec(),
            })
            .unwrap();
        assert_eq!(
            real.index, doomed2.index,
            "precondition: the replacing entry at our position must be a \
             COMMAND — that is what puts the index in the ring and selects \
             the arm under test"
        );
        assert!(
            real.term > doomed2.term,
            "precondition: the replacing command is at a newer term than ours"
        );
        for _ in 0..400 {
            d1.step().unwrap();
            d2.tick_and_step().unwrap();
            d3.tick_and_step().unwrap();
            if d1.status().applied_index >= doomed2.index.0 {
                break;
            }
        }
        assert!(
            d1.status().applied_index >= doomed2.index.0,
            "precondition: n1's COMMAND watermark reaches the position — this is \
             a ring HIT, not a watermark-only verdict"
        );

        let outcome = d1
            .wait_applied(doomed2, Duration::from_millis(200))
            .expect("a ring-recorded position is a verdict");
        assert_eq!(
            outcome,
            ApplyWaitOutcome::Replaced,
            "a position consumed by ANOTHER leader's command must be Replaced: \
             reporting Applied would claim this caller's write succeeded while \
             the slot holds someone else's"
        );
        assert!(
            d1.metrics().application_wait.snapshot().outcomes[Outcome::Replaced as usize].count
                >= 1,
            "real protocol outcome missing from observation"
        );
    }

    /// A poisoned driver answers Failed, never Unconfirmed (task #30 review
    /// round: this distinction existed in code but no test bound it — crushing
    /// Failed into Unconfirmed left 89 tests green). Unconfirmed invites the
    /// caller to keep polling or stay pending; Failed says this node will
    /// never answer. Conflating them turns a dead node into a silent spinner.
    #[test]
    fn a_poisoned_driver_answers_failed_not_unconfirmed() {
        let hub = InProcHub::new();
        let peer = Arc::new(RaftPeer::new(NodeId(1), RegionId(1), &[NodeId(1)]).unwrap());
        let endpoint = hub.endpoint(NodeId(1));
        let driver = NodeDriver::new(
            peer,
            Arc::new(endpoint) as Arc<dyn RaftTransport>,
            MemStateMachine::with_engine(Arc::new(FailingEngine)).unwrap(),
        )
        .expect("drain token minted once per peer");
        driver.peer().campaign().unwrap();
        for _ in 0..50 {
            if driver.tick_and_step().is_err() || driver.status().role == Role::Leader {
                break;
            }
        }
        let at = driver
            .propose(&Command::Put {
                cf: 0,
                key: b"k".to_vec(),
                value: b"v".to_vec(),
            })
            .unwrap();
        for _ in 0..50 {
            if driver.tick_and_step().is_err() {
                break;
            }
        }
        assert!(driver.status().fatal.is_some(), "precondition: poisoned");

        let err = driver
            .wait_applied(at, Duration::from_millis(1))
            .expect_err("a poisoned driver cannot produce a verdict");
        assert!(
            matches!(err, ApplyWaitError::Failed(_)),
            "poison must answer Failed, never Unconfirmed — a caller told \
             Unconfirmed keeps waiting on a node that will never answer: {err}"
        );
        assert!(
            err.to_string().contains("poisoned"),
            "the poison cause must survive recognizably: {err}"
        );
        assert!(
            driver.metrics().application_wait.snapshot().outcomes[Outcome::Error as usize].count
                >= 1,
            "real protocol outcome missing from observation"
        );
    }

    /// The fence-rejection receipt reaches the proposer (the silent-lost-write
    /// blocker from Ren's layer-3 firing test): a fenced write rejected in
    /// ordered apply must come back as the EXCLUSIVE FenceRejected verdict —
    /// exact position AND rejected region from the ring's apply-time record —
    /// while the watermark advances and nothing lands in the engine.
    ///
    /// Mutant contract: pushing the ring without the verdict (None) collapses
    /// this into Applied — reds at the named exclusivity assertion.
    #[test]
    fn a_fence_rejected_write_reports_its_verdict_not_success() {
        struct AlwaysStale;
        impl crate::state_machine::FenceAdjudicator for AlwaysStale {
            fn is_fresh(&self, _f: &crate::RegionFence) -> kv9_common::Result<bool> {
                Ok(false)
            }
        }
        let hub = InProcHub::new();
        let peer = Arc::new(RaftPeer::new(NodeId(1), RegionId(1), &[NodeId(1)]).unwrap());
        let endpoint = hub.endpoint(NodeId(1));
        let mut sm = MemStateMachine::new();
        sm.set_fence_adjudicator(Arc::new(AlwaysStale));
        let driver = NodeDriver::new(peer, Arc::new(endpoint) as Arc<dyn RaftTransport>, sm)
            .expect("drain token minted once per peer");
        driver.peer().campaign().unwrap();
        for _ in 0..50 {
            driver.tick_and_step().unwrap();
            if driver.status().role == Role::Leader {
                break;
            }
        }
        let fenced = Command::Fenced {
            fence: crate::RegionFence {
                region_id: 42,
                conf_ver: 1,
                version: 1,
            },
            inner: crate::FencedInner::Write {
                ops: vec![crate::KvOp::Put {
                    cf: 0,
                    key: b"fr".to_vec(),
                    value: b"v".to_vec(),
                }],
            },
        };
        let at = driver.propose(&fenced).unwrap();
        for _ in 0..100 {
            driver.tick_and_step().unwrap();
            if driver
                .driver_applied()
                .is_some_and(|wm| wm.index >= at.index.0)
            {
                break;
            }
        }
        let outcome = driver
            .wait_applied(at, Duration::from_millis(200))
            .expect("a rejected fence is a verdict, not an error");
        assert_eq!(
            outcome,
            ApplyWaitOutcome::FenceRejected {
                at: kv9_common::AppliedPosition {
                    term: at.term,
                    index: at.index.0,
                },
                region: kv9_common::RegionId(42),
            },
            "a rejected fenced write must surface the EXCLUSIVE verdict with the \
             exact position and rejected region — reporting Applied here is the \
             silent lost write"
        );
        assert!(
            driver
                .driver_applied()
                .is_some_and(|wm| wm.index >= at.index.0),
            "the rejected entry still advances the unified watermark"
        );
        assert_eq!(
            driver.get(ColumnFamily::Default, b"fr").unwrap(),
            None,
            "a rejected fence writes nothing"
        );
        assert!(
            driver.metrics().application_wait.snapshot().outcomes[Outcome::Rejected as usize].count
                >= 1,
            "real protocol outcome missing from observation"
        );
    }
    // ---- task #28 step 3: the read barrier ----

    /// Healthy path: a quorum-confirmed barrier covers every prior committed
    /// write, and the engine read AFTER the barrier observes it (the snapshot-
    /// after-barrier seam contract, exercised in test form).
    #[test]
    fn read_barrier_on_a_healthy_leader_covers_committed_writes() {
        let driver = single_node_driver();
        let _pump = driver.spawn(Duration::from_millis(2)).unwrap();
        let at = driver
            .propose(&Command::Put {
                cf: 0,
                key: b"rb".to_vec(),
                value: b"v".to_vec(),
            })
            .unwrap();
        assert!(matches!(
            driver.wait_applied(at, Duration::from_secs(10)).unwrap(),
            ApplyWaitOutcome::Applied(_)
        ));
        let barrier = driver
            .read_barrier(Duration::from_secs(10))
            .expect("a healthy single-node leader must establish a barrier");
        assert!(
            barrier.index >= at.index.0,
            "the barrier must cover the committed write: barrier {} < write {}",
            barrier.index,
            at.index.0
        );
        // Read AFTER the barrier: the value must be there.
        assert_eq!(
            driver.get(ColumnFamily::Default, b"rb").unwrap(),
            Some(b"v".to_vec()),
            "a post-barrier read must observe the pre-barrier write"
        );
        driver.stop();
        assert!(
            driver.metrics().read_establishment.snapshot().outcomes[Outcome::Success as usize]
                .count
                >= 1,
            "real protocol outcome missing from observation"
        );
    }

    /// Freeze a real election immediately after the winning vote, before
    /// followers can acknowledge its no-op. Observe the completed request
    /// attempt before permitting any further Raft delivery.
    fn drivers_before_current_term_commit() -> Vec<Arc<NodeDriver>> {
        let hub = InProcHub::new();
        let ids = [NodeId(1), NodeId(2), NodeId(3)];
        let drivers: Vec<_> = ids
            .iter()
            .map(|&id| {
                NodeDriver::new(
                    Arc::new(RaftPeer::new(id, RegionId(1), &ids).unwrap()),
                    Arc::new(hub.endpoint(id)) as Arc<dyn RaftTransport>,
                    MemStateMachine::new(),
                )
                .unwrap()
            })
            .collect();
        drivers[0].peer().campaign().unwrap();
        for _ in 0..16 {
            drivers[0].step().unwrap();
            if drivers[0].status().role == Role::Leader {
                break;
            }
            drivers[1].step().unwrap();
            drivers[2].step().unwrap();
        }
        assert_eq!(drivers[0].status().role, Role::Leader);
        assert_eq!(drivers[0].status().raft_committed, 0);
        assert_eq!(drivers[0].driver_applied(), None);
        drivers
    }

    #[test]
    fn read_barrier_survives_submission_before_current_term_commit() {
        let drivers = drivers_before_current_term_commit();
        let (attempt_tx, attempt_rx) = std::sync::mpsc::channel();
        *drivers[0].read_attempt_observer.lock().unwrap() = Some(attempt_tx);
        let reader = Arc::clone(&drivers[0]);
        let read = std::thread::spawn(move || reader.read_barrier(Duration::from_secs(2)));
        attempt_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("the early read must attempt submission before quorum delivery resumes");
        assert_eq!(drivers[0].status().raft_committed, 0);

        // No new client request is issued. The original invocation must
        // survive establishment of the term's commit and confirm its context.
        while !read.is_finished() {
            for driver in &drivers {
                driver.step().unwrap();
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let status = drivers[0].status();
        assert_eq!(status.raft_committed, 1);
        assert_eq!(status.driver_applied.unwrap().term, status.term);
        let barrier = read
            .join()
            .unwrap()
            .expect("read submitted before election barrier was lost after quorum recovered");
        assert_eq!(barrier.index(), 1);
        assert_eq!(drivers[0].read_barriers_minted(), 1);
    }

    #[test]
    fn deferred_read_admission_respects_the_original_deadline() {
        let drivers = drivers_before_current_term_commit();
        let reader = Arc::clone(&drivers[0]);
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let read = std::thread::spawn(move || {
            let result = reader.read_barrier(Duration::from_millis(30));
            let _ = done_tx.send(result);
        });
        let result = done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("deferred admission exceeded the original request budget");
        read.join().unwrap();
        assert!(
            matches!(
                result,
                Err(ReadIndexError::Unconfirmed {
                    phase: BarrierPhase::QuorumConfirmation,
                    ..
                })
            ),
            "uncommitted term must refuse the read with a quorum deadline: {result:?}"
        );
        assert_eq!(drivers[0].status().raft_committed, 0);
        assert_eq!(drivers[0].read_barriers_minted(), 1);
    }

    #[test]
    fn deferred_read_admission_refuses_a_deposed_leader() {
        let drivers = drivers_before_current_term_commit();
        let (attempt_tx, attempt_rx) = std::sync::mpsc::channel();
        *drivers[0].read_attempt_observer.lock().unwrap() = Some(attempt_tx);
        let reader = Arc::clone(&drivers[0]);
        let read = std::thread::spawn(move || reader.read_barrier(Duration::from_millis(200)));
        attempt_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let mut heartbeat = raft::prelude::Message::default();
        heartbeat.set_msg_type(raft::prelude::MessageType::MsgHeartbeat);
        heartbeat.from = 2;
        heartbeat.to = 1;
        heartbeat.term = drivers[0].status().term + 1;
        drivers[0].peer().step_message(heartbeat);
        assert_eq!(drivers[0].status().role, Role::Follower);
        let result = read.join().unwrap();
        assert!(
            matches!(
                result,
                Err(ReadIndexError::NotLeader {
                    hint: Some(NodeId(2))
                })
            ),
            "deferred admission ignored loss of leadership: {result:?}"
        );
    }

    /// A follower answers with the TYPED NotLeader + hint — never a barrier,
    /// never a bare error (the establishing read type re-routes on this).
    #[test]
    fn read_barrier_on_a_follower_is_typed_not_leader() {
        let hub = InProcHub::new();
        let ids = [NodeId(1), NodeId(2), NodeId(3)];
        let mk = |id: NodeId| {
            NodeDriver::new(
                Arc::new(RaftPeer::new(id, RegionId(1), &ids).unwrap()),
                Arc::new(hub.endpoint(id)) as Arc<dyn RaftTransport>,
                MemStateMachine::new(),
            )
            .expect("drain token minted once per peer")
        };
        let d1 = mk(NodeId(1));
        let d2 = mk(NodeId(2));
        let d3 = mk(NodeId(3));
        d1.peer().campaign().unwrap();
        for _ in 0..200 {
            d1.tick_and_step().unwrap();
            d2.tick_and_step().unwrap();
            d3.tick_and_step().unwrap();
            if d1.status().role == Role::Leader && d2.status().leader_id == Some(NodeId(1)) {
                break;
            }
        }
        let err = d2
            .read_barrier(Duration::from_millis(100))
            .expect_err("a follower must refuse to establish a barrier");
        assert!(
            matches!(
                err,
                ReadIndexError::NotLeader {
                    hint: Some(NodeId(1))
                }
            ),
            "the refusal must be typed NotLeader with the leader hint: {err}"
        );
        assert!(
            d2.metrics().read_establishment.snapshot().outcomes[Outcome::Rejected as usize].count
                >= 1,
            "real protocol outcome missing from observation"
        );
    }

    /// The isolated-leader case the linearizable promise exists for: a leader
    /// whose peers never answer must NOT establish a barrier — the failure is
    /// the TYPED quorum-confirmation timeout, not a transport error (no
    /// transport is involved on this node's read path at all; this is what
    /// makes the partition E2E's typed-exclusivity assertions writable).
    #[test]
    fn an_isolated_leader_cannot_confirm_a_read_barrier() {
        let hub = InProcHub::new();
        let ids = [NodeId(1), NodeId(2), NodeId(3)];
        let mk = |id: NodeId| {
            NodeDriver::new(
                Arc::new(RaftPeer::new(id, RegionId(1), &ids).unwrap()),
                Arc::new(hub.endpoint(id)) as Arc<dyn RaftTransport>,
                MemStateMachine::new(),
            )
            .expect("drain token minted once per peer")
        };
        let d1 = mk(NodeId(1));
        let d2 = mk(NodeId(2));
        let d3 = mk(NodeId(3));
        d1.peer().campaign().unwrap();
        for _ in 0..200 {
            d1.tick_and_step().unwrap();
            d2.tick_and_step().unwrap();
            d3.tick_and_step().unwrap();
            if d1.status().role == Role::Leader {
                break;
            }
        }
        assert_eq!(d1.status().role, Role::Leader);
        // Precondition, pinned by name (Tess's review): keep pumping all
        // three until the leader's CURRENT-TERM barrier has applied - live
        // quorum contact has demonstrably happened. Without this, cutting
        // immediately after role=Leader leaves the term's barrier/lease
        // unestablished, and a LeaseBased mutant also times out - the test
        // would pass without Safe being load-bearing. With it, the
        // Safe-to-LeaseBased single-defect mutant reds precisely: an isolated
        // leader under LeaseBased hands out a barrier from its clock-trusted
        // lease instead of the typed quorum timeout.
        for _ in 0..500 {
            d1.tick_and_step().unwrap();
            d2.tick_and_step().unwrap();
            d3.tick_and_step().unwrap();
            let s1 = d1.status();
            if d1.driver_applied().is_some_and(|wm| wm.term == s1.term) {
                break;
            }
        }
        {
            let s1 = d1.status();
            assert!(
                d1.driver_applied().is_some_and(|wm| wm.term == s1.term),
                "precondition: the leader's current-term barrier must be applied \
                 (live quorum contact established) BEFORE the cut - otherwise \
                 LeaseBased also times out and the test cannot distinguish Safe"
            );
        }
        // From here the peers go silent: d2/d3 never step again - every
        // Safe-read heartbeat d1 sends dies unacknowledged.
        let _pump = d1.spawn(Duration::from_millis(2)).unwrap();
        let err = d1
            .read_barrier(Duration::from_millis(400))
            .expect_err("an isolated leader must not confirm a barrier");
        assert!(
            matches!(
                err,
                ReadIndexError::Unconfirmed {
                    phase: BarrierPhase::QuorumConfirmation,
                    ..
                }
            ),
            "isolation must surface as the typed quorum-confirmation timeout: {err}"
        );
        d1.stop();
        assert!(
            d1.metrics().read_establishment.snapshot().outcomes[Outcome::Unconfirmed as usize]
                .count
                >= 1,
            "real protocol outcome missing from observation"
        );
    }

    /// The committed-but-unapplied window — the arm the UNIFIED watermark
    /// exists for. Quorum confirms the index fast, but the local apply is
    /// frozen below it: the barrier must NOT establish (typed ApplyCatchUp),
    /// and after unfreezing the same barrier establishes and the read sees
    /// the write.
    ///
    /// Mutant contract: returning Ok right after quorum confirmation (deleting
    /// the watermark wait) must red at the named frozen assertion below.
    #[test]
    fn a_read_barrier_waits_for_apply_not_just_confirmation() {
        let driver = single_node_driver();
        let _pump = driver.spawn(Duration::from_millis(2)).unwrap();
        driver.pause_apply(true);
        let at = driver
            .propose(&Command::Put {
                cf: 0,
                key: b"frozen".to_vec(),
                value: b"v".to_vec(),
            })
            .unwrap();
        // The entry commits (raft runs; freeze stops APPLY only) — pin that
        // precondition before asking for the barrier, or the read index can
        // legitimately land below the put (a read need not cover a write that
        // was never acknowledged) and the test exercises the wrong arm.
        for _ in 0..500 {
            if driver.status().raft_committed >= at.index.0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(
            driver.status().raft_committed >= at.index.0,
            "precondition: the frozen write must COMMIT (freeze stops apply, not commits)"
        );
        let err = driver.read_barrier(Duration::from_millis(400)).expect_err(
            "a barrier must not establish while apply lags the confirmed index \
                 (returning here would let a read miss a committed write)",
        );
        assert!(
            matches!(
                err,
                ReadIndexError::Unconfirmed {
                    phase: BarrierPhase::ApplyCatchUp,
                    ..
                }
            ),
            "the frozen window must surface as the typed apply-catch-up timeout: {err}"
        );
        driver.pause_apply(false);
        let barrier = driver
            .read_barrier(Duration::from_secs(10))
            .expect("after unfreezing the barrier must establish");
        assert!(barrier.index >= at.index.0);
        assert_eq!(
            driver.get(ColumnFamily::Default, b"frozen").unwrap(),
            Some(b"v".to_vec())
        );
        driver.stop();
    }
}
