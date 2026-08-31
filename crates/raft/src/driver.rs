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
    /// Recently applied (index, term) pairs, for (term, index) verification.
    /// ONLY successfully applied entries enter this ring — a failed decode or
    /// apply must never be reported as success by `wait_applied`.
    applied: Mutex<Vec<(u64, u64)>>,
    /// Conf-change receipts by exact (index, term) — the correlation store for
    /// [`Self::wait_conf_applied`]. Conf entries NEVER enter the command ring:
    /// `applied_index`/`applied_term` must remain a same-entry pair.
    /// Lock order: leaf — never held while acquiring `applied`/`sm`/peer.
    conf_receipts: Mutex<Vec<ConfReceiptEntry>>,
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
        #[cfg(any(test, feature = "testing"))]
        if self
            .apply_paused
            .load(std::sync::atomic::Ordering::Relaxed)
        {
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
                    if let Err(e) = sm.apply_command(entry.index, &cmd) {
                        drop(sm);
                        drop(applied);
                        return Err(self.poison(entry.term, entry.index.0, &e));
                    }
                    push_ring(&mut applied, entry.index.0, entry.term);
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
    /// (via [`Self::spawn`] or a caller-driven loop). Returns:
    /// - `Ok(true)`  — the exact proposal applied here;
    /// - `Ok(false)` — the position was passed by a DIFFERENT entry (overwritten
    ///   after a leader change): the proposal must be reported failed;
    /// - `Err(_)`    — deadline reached. Success is judged on applied state,
    ///   never on elapsed time.
    pub fn wait_applied(&self, at: ProposedAt, deadline: Duration) -> Result<bool> {
        let start = Instant::now();
        loop {
            if let Some(f) = self.fatal.lock().expect("fatal poisoned").as_ref() {
                return Err(Error::Raft(format!("driver is poisoned: {f}")));
            }
            {
                let applied = self.applied.lock().expect("applied poisoned");
                if let Some(&(_, term)) = applied.iter().find(|(i, _)| *i == at.index.0) {
                    return Ok(term == at.term);
                }
                // Position passed without this index ever being recorded: the
                // slot was consumed by an entry that never reached the command
                // path (e.g. a no-op barrier) — not ours.
                if self.sm.lock().expect("sm poisoned").applied_index() >= at.index {
                    return Ok(false);
                }
            }
            if start.elapsed() > deadline {
                return Err(Error::Raft(format!(
                    "wait_applied deadline: index {} not reached",
                    at.index.0
                )));
            }
            std::thread::sleep(Duration::from_millis(1));
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
            applied_term: applied.last().map_or(0, |(_, term)| *term),
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

fn push_ring(applied: &mut Vec<(u64, u64)>, index: u64, term: u64) {
    applied.push((index, term));
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
        assert!(driver.wait_applied(at, Duration::from_millis(100)).unwrap());
        assert!(driver.status().fatal.is_none());
    }
}
