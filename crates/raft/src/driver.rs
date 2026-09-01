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

use kv9_engine::{Engine, MemEngine};
use raft::storage::MemStorage;

use kv9_common::{Error, NodeId, Result};

use raft::eraftpb::{ConfChangeSingle, ConfChangeType, ConfChangeV2};

use crate::rawnode::{PersistentRaftStorage, ProposedAt, RaftPeer};
use crate::transport::RaftTransport;
use crate::{Command, EntryKind, MemStateMachine, RaftGroup, Role, StateMachine};

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
    /// A fatal apply-path failure (undecodable committed entry / engine apply
    /// error). Once set, the pump has stopped: continuing past a hole would
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
    /// `Some(region)` = the entry was a fenced write REJECTED for this region
    /// (logical outcome; watermark advanced). `None` = applied normally.
    fence_rejected: Option<NodeIdFreeRegionId>,
}

/// Local alias so the ring stays dependency-light in signatures.
type NodeIdFreeRegionId = kv9_common::RegionId;

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

pub struct NodeDriver<S: PersistentRaftStorage = MemStorage, E: Engine = MemEngine> {
    peer: Arc<RaftPeer<S>>,
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
    /// First fatal apply-path error; poisons the driver (pump stops).
    fatal: Mutex<Option<String>>,
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
}

impl<S: PersistentRaftStorage, E: Engine + 'static> NodeDriver<S, E> {
    pub fn new(
        peer: Arc<RaftPeer<S>>,
        transport: Arc<dyn RaftTransport>,
        sm: MemStateMachine<E>,
    ) -> Arc<NodeDriver<S, E>> {
        Arc::new(NodeDriver {
            peer,
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
            fatal: Mutex::new(None),
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
        })
    }

    pub fn peer(&self) -> &Arc<RaftPeer<S>> {
        &self.peer
    }

    /// One pump iteration WITHOUT a tick: deliver inbound, persist+send
    /// outbound, apply committed entries.
    ///
    /// A committed entry that fails to decode or apply is **fatal**: every
    /// replica must apply the same committed sequence, so skipping one and
    /// continuing would silently diverge this node from the group. On error
    /// the driver poisons itself (pump stops, `status().fatal` set) and the
    /// failed entry never enters the success-correlation ring.
    pub fn step(&self) -> Result<()> {
        if let Some(f) = self.fatal.lock().expect("fatal poisoned").as_ref() {
            return Err(Error::Raft(f.clone()));
        }
        for msg in self.transport.drain() {
            self.peer.step_message(msg);
        }
        for msg in self.peer.pump() {
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
        let entries = self.peer.take_ready().unwrap_or_default();
        if entries.is_empty() {
            return Ok(());
        }
        // Items are processed strictly in log order. Locks are taken per item
        // (always `applied` then `sm`, per the declared order) and NEVER held
        // across a peer call: conf changes go through `peer.inner`, and peer
        // must not nest with driver locks in either direction.
        let mut last_seen: u64 = 0;
        let mut last_term: u64 = 0;
        for entry in entries {
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
                    let result = match sm.apply_command(entry.index, &cmd) {
                        Ok(r) => r,
                        Err(e) => {
                            drop(sm);
                            drop(applied);
                            return Err(self.poison(entry.term, entry.index.0, &e));
                        }
                    };
                    push_ring(
                        &mut applied,
                        entry.index.0,
                        entry.term,
                        result.fence_rejected,
                    );
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
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Tick + step (the driver loop body).
    pub fn tick_and_step(&self) -> Result<()> {
        self.peer.tick_once();
        self.step()
    }

    /// Propose a command on this node (must currently be leader).
    pub fn propose(&self, cmd: &Command) -> Result<ProposedAt> {
        self.peer.propose_traced(cmd.encode())
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
        let start = Instant::now();
        loop {
            if let Some(f) = self.fatal.lock().expect("fatal poisoned").as_ref() {
                return Err(ApplyWaitError::Failed(Error::Raft(format!(
                    "driver is poisoned: {f}"
                ))));
            }
            // Snapshot the unified watermark BEFORE the ring lock (never
            // nested inside it — the pump publishes under these locks in its
            // own order). A stale-low read only delays a Replaced verdict by
            // one poll iteration; monotonicity makes a stale read safe, never
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
                        match entry.fence_rejected {
                            None => Ok(ApplyWaitOutcome::Applied(at_pos)),
                            Some(region) => {
                                Ok(ApplyWaitOutcome::FenceRejected { at: at_pos, region })
                            }
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
            std::thread::sleep(Duration::from_millis(1));
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
    /// barrier proves applied. Pure condition-poll; the pump must be running.
    pub fn read_barrier(
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
        if let Err(e) = self.peer.read_index(rctx.clone()) {
            return Err(match e {
                Error::NotLeader { leader } => ReadIndexError::NotLeader { hint: leader },
                other => ReadIndexError::Failed(other),
            });
        }
        // 3. Wait for the quorum confirmation correlated by EXACT context.
        let confirmed = loop {
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
            std::thread::sleep(Duration::from_millis(1));
        };
        // 4. Wait for the unified watermark to pass the confirmed index.
        loop {
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
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// How many read barriers this node has REQUESTED (minted a read
    /// context for) since boot — successful or not. Diagnostic counter; the
    /// delete-range evidence cell pins "one request establishes exactly one
    /// quorum barrier" on it (a per-chunk regression multiplies it).
    pub fn read_barriers_minted(&self) -> u64 {
        self.read_seq.load(std::sync::atomic::Ordering::Relaxed)
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
    }

    /// Background pump at `tick_every` cadence until [`Self::stop`].
    pub fn spawn(self: &Arc<Self>, tick_every: Duration) -> std::thread::JoinHandle<()> {
        let driver = Arc::clone(self);
        std::thread::spawn(move || {
            while !driver.stop.load(Ordering::Relaxed) {
                if driver.tick_and_step().is_err() {
                    break; // poisoned: fatal is recorded, status carries it
                }
                std::thread::sleep(tick_every);
            }
        })
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
    /// passed it at the moment this value is returned.
    pub index: u64,
}

impl ReadBarrier {
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
                quorum_confirmed: matches!(phase, BarrierPhase::ApplyCatchUp),
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

fn push_ring(
    applied: &mut Vec<RingEntry>,
    index: u64,
    term: u64,
    fence_rejected: Option<kv9_common::RegionId>,
) {
    applied.push(RingEntry {
        index,
        term,
        fence_rejected,
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
    use super::*;
    use crate::rawnode::RaftPeer;
    use crate::transport::{InProcHub, RaftTransport};
    use crate::RaftGroup;
    use kv9_common::{NodeId, RegionId};
    use kv9_engine::{ColumnFamily, Mutation, ReadView, ScanEntry, WriteBatch};

    fn single_node_driver() -> Arc<NodeDriver> {
        let hub = InProcHub::new();
        let peer = Arc::new(RaftPeer::new(NodeId(1), RegionId(1), &[NodeId(1)]).unwrap());
        let endpoint = hub.endpoint(NodeId(1));
        let driver = NodeDriver::new(
            peer,
            Arc::new(endpoint) as Arc<dyn RaftTransport>,
            MemStateMachine::new(),
        );
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
        );
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
        );
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
        let driver = NodeDriver::new(peer, Arc::new(endpoint) as Arc<dyn RaftTransport>, sm);
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
    }
    // ---- task #28 step 3: the read barrier ----

    /// Healthy path: a quorum-confirmed barrier covers every prior committed
    /// write, and the engine read AFTER the barrier observes it (the snapshot-
    /// after-barrier seam contract, exercised in test form).
    #[test]
    fn read_barrier_on_a_healthy_leader_covers_committed_writes() {
        let driver = single_node_driver();
        let _pump = driver.spawn(Duration::from_millis(2));
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
        let _pump = d1.spawn(Duration::from_millis(2));
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
        let _pump = driver.spawn(Duration::from_millis(2));
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
