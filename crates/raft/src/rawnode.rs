//! raft-rs adapter (ROADMAP §Dependency decisions): `RawNode`/`Ready` behind the
//! pull-model [`RaftGroup`] trait.
//!
//! Boundaries fixed by design review:
//! - raft-rs [`raft::Storage`] is the **raft-log** layer (Phase-1: `MemStorage`;
//!   later the LogStore/WalStream of DESIGN "6.4 Raft log vs. WAL stream"). It is
//!   NOT the state-machine engine — `kv9_engine::Engine` appears only downstream of
//!   apply (committed entry → `Command` → `WriteBatch`).
//! - The Ready loop **persists entries + hardstate before sending messages** — a
//!   raft safety requirement that is vacuous under `MemStorage` but structural here
//!   so the Phase-2 real log slots in without reshaping the loop.
//! - `propose` reports the **locally assigned** `(term, index)` ([`ProposedAt`]) —
//!   a position claim, not a commit promise. After a leader change the same index
//!   can be overwritten by another leader's entry, so callers correlate a committed
//!   entry by **term + index** (or payload/context), never by position alone.

use std::sync::{Arc, Mutex, MutexGuard};

use protobuf::Message as PbMessage;
use raft::eraftpb::EntryType;
use raft::prelude::{ConfChange, ConfChangeV2};
use raft::prelude::{ConfState, Entry, HardState, Message};
// Re-exported publicly: `RaftPeer::new` already returns `RaftPeer<MemStorage>`
// on the public surface, so consumers (e.g. kv9-server's in-proc harnesses)
// must be able to NAME the type they are already holding.
pub use raft::storage::MemStorage;
use raft::{Config, RawNode, ReadOnlyOption, ReadState, StateRole};
use slog::{o, Discard, Logger};

use kv9_common::{Error, NodeId, RegionId, Result};

use crate::{CommittedEntry, EntryKind, LogIndex, RaftGroup, Role};

/// A raft-rs [`raft::Storage`] that can also **persist** what the Ready loop
/// hands it: log entries and the HardState (term + vote + commit).
///
/// The raft safety contract lives here: **a vote must be durable before the
/// reply leaves the node** — the Ready loop calls `append`/`set_hardstate`
/// *before* messages are handed to the transport. `MemStorage` implements this
/// trait volatilely (in-process clusters, tests); `DiskRaftStorage` implements
/// it durably (real processes). A restarted node on volatile storage forgets
/// its vote and can vote twice in one term — two leaders — which is why the
/// cross-process path must use the durable impl.
pub trait PersistentRaftStorage: raft::Storage + Send + Sync + 'static {
    fn append(&self, entries: &[Entry]) -> Result<()>;
    fn set_hardstate(&self, hs: &HardState) -> Result<()>;
    /// Durably record a post-conf-change `ConfState` **paired with the log
    /// index it took effect at** (task #24). The pair must be one crash-safe
    /// record: ConfState without its index cannot gate replay, and a restart
    /// that re-applies old conf entries onto the final membership can demote
    /// or poison (Tess's P0 review of 107b161). Last write wins on replay.
    fn set_conf_state(&self, cs: &ConfState, at_index: u64) -> Result<()>;

    /// The conf-apply boundary recovered at open: the highest `at_index` ever
    /// recorded by [`Self::set_conf_state`] (0 for a fresh store / volatile
    /// impls). Conf entries at or below it are already reflected in the
    /// initial ConfState and must be skipped on replay.
    fn recovered_conf_index(&self) -> u64 {
        0
    }
}

impl PersistentRaftStorage for MemStorage {
    fn append(&self, entries: &[Entry]) -> Result<()> {
        self.wl().append(entries).map_err(raft_err)
    }

    fn set_conf_state(&self, cs: &ConfState, _at_index: u64) -> Result<()> {
        self.wl().set_conf_state(cs.clone());
        Ok(())
    }

    fn set_hardstate(&self, hs: &HardState) -> Result<()> {
        self.wl().set_hardstate(hs.clone());
        Ok(())
    }
}

/// The locally assigned position of an accepted proposal: a claim that entry
/// `(term, index)` is *this* proposal — committed only once a committed entry with
/// the **same term** appears at that index. If a later leader overwrites the index,
/// the term differs and the proposal must be reported failed, never silently
/// "reached" (observed in the selection spike: a failover's no-op barrier shifts
/// indexes; correlation by position alone returns someone else's command).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProposedAt {
    pub term: u64,
    pub index: LogIndex,
}

/// A single-lock snapshot of everything the status surface reads from the
/// peer (see [`RaftPeer::status_snapshot`]).
#[derive(Debug, Clone)]
pub struct PeerSnapshot {
    pub node_id: NodeId,
    pub leader_hint: Option<NodeId>,
    pub raw_role: Role,
    pub term: u64,
    pub committed: u64,
    pub step_errors: u64,
    pub promotable: bool,
    /// Highest conf-change index applied (0 = still on the seeded config).
    pub conf_applied: u64,
    pub voters: Vec<u64>,
    pub learners: Vec<u64>,
}

fn role_of(state: StateRole) -> Role {
    match state {
        StateRole::Leader => Role::Leader,
        StateRole::Follower => Role::Follower,
        StateRole::Candidate | StateRole::PreCandidate => Role::Candidate,
    }
}

/// One member of a raft-rs group on one node, driven by an external pump
/// (the `NodeDriver` in production; `InProcessCluster::round` in tests).
pub struct RaftPeer<S: PersistentRaftStorage = MemStorage> {
    node: NodeId,
    region: RegionId,
    inner: Mutex<PeerInner<S>>,
    /// Whether THE [`DrainToken`] for this peer has been minted (task #5).
    /// Set once, never cleared: the drain capability is issued at most once
    /// per peer for the life of the process.
    drain_minted: std::sync::atomic::AtomicBool,
}

struct PeerInner<S: PersistentRaftStorage> {
    raw: RawNode<S>,
    /// Committed, non-empty entries not yet drained via [`DrainToken`].
    ready: Vec<CommittedEntry>,
    /// Outgoing raft messages awaiting delivery by the cluster pump.
    outbox: Vec<Message>,
    /// Quorum-confirmed [`ReadState`]s not yet drained by
    /// [`RaftPeer::take_read_states`] (task #28). Captured in `process_ready`
    /// — the single Ready consumer — and correlated by exact `request_ctx`.
    read_states: Vec<ReadState>,
    /// Harness kill switch: a dead peer neither ticks nor receives messages.
    alive: bool,
    /// Highest apply progress reported to raft via [`RaftPeer::applied_to`]
    /// (monotonic guard; raft's one-at-a-time conf-change gate reads it).
    applied_reported: u64,
    /// Highest conf-change log index already applied (recovered from storage
    /// at open). The replay guard: a conf entry at or below this is already
    /// reflected in the current ConfState and is skipped — re-applying
    /// relative conf ops onto the final membership demotes or poisons.
    conf_applied: u64,
    /// Count of inbound messages `RawNode::step` rejected. Dropping one is
    /// protocol-sanctioned (indistinguishable from packet loss; the sender
    /// retransmits) — but a PERSISTENTLY growing count means a real problem
    /// (stale peer, version skew, corrupt messages) that would otherwise be
    /// invisible. Observability only; never fatal — a stale message from a
    /// removed peer must not be able to kill a healthy node.
    step_errors: u64,
    /// Persistence failure is terminal for this incarnation. Never resume a
    /// RawNode whose Ready may have been only partly written or advanced.
    fatal: Option<String>,
}

impl<S: PersistentRaftStorage> PeerInner<S> {
    fn check_fatal(&self) -> Result<()> {
        match &self.fatal {
            Some(cause) => Err(Error::Raft(cause.clone())),
            None => Ok(()),
        }
    }

    fn fail_storage(&mut self, operation: &str, cause: Error) -> Error {
        let cause = self
            .fatal
            .get_or_insert_with(|| {
                format!("fatal Raft persistence failure during {operation}: {cause}")
            })
            .clone();
        self.alive = false;
        self.outbox.clear();
        self.ready.clear();
        self.read_states.clear();
        Error::Raft(cause)
    }
}

impl RaftPeer<MemStorage> {
    /// Build a peer for `region` on `node`, with the (fixed, Phase-1) voter set,
    /// on volatile in-memory storage (tests / in-process clusters).
    pub fn new(node: NodeId, region: RegionId, voters: &[NodeId]) -> Result<RaftPeer> {
        let ids = validate_voter_declaration(voters)?;
        if !ids.contains(&node.0) {
            return Err(Error::Raft(format!(
                "initial-bootstrap mode requires self ({}) in the declared voter set; \
                 a node joining an existing cluster must use new_joining",
                node.0
            )));
        }
        let storage = MemStorage::new_with_conf_state(ConfState::from((ids, vec![])));
        RaftPeer::with_storage(node, region, storage)
    }

    /// Build a peer that **joins an existing cluster** (task #24): it is
    /// seeded with the cluster's **log-start (bootstrap) voter set** — which
    /// deliberately does NOT include itself — and becomes somebody only when
    /// a committed conf change admits it. It cannot campaign (`promotable()`
    /// is false) and never counts in any quorum denominator.
    ///
    /// Why the seed is REQUIRED (found empirically, not designed): the
    /// bootstrap voter set never exists as log entries — it lives in the
    /// initial ConfState. AppendEntries replays data and *subsequent* conf
    /// changes, but not that base: a truly-empty joiner applying the first
    /// AddLearner would produce a zero-voter config and be refused by raft
    /// ("removed all voters"). Without log truncation this seed is the only
    /// missing piece; once truncation exists, a snapshot's ConfState replaces
    /// it. The runtime passes the declared `--join` set (the joiner's own id
    /// absent from it — that asymmetry IS the join-existing mode marker).
    pub fn new_joining(
        node: NodeId,
        region: RegionId,
        bootstrap_voters: &[NodeId],
    ) -> Result<RaftPeer> {
        let ids = validate_voter_declaration(bootstrap_voters)?;
        // Typed error, not an assert: in release a self-including declaration
        // would silently seed the joiner AS A VOTER — the exact fail-open the
        // join-existing mode exists to prevent (Tess's review of 107b161).
        if ids.contains(&node.0) {
            return Err(Error::Raft(format!(
                "join-existing mode: node {} must NOT be in the declared \
                 bootstrap voter set (the asymmetry IS the mode marker)",
                node.0
            )));
        }
        let storage = MemStorage::new_with_conf_state(ConfState::from((ids, vec![])));
        RaftPeer::with_storage(node, region, storage)
    }
}

impl<S: PersistentRaftStorage> RaftPeer<S> {
    /// Build a peer over an explicit storage — durable
    /// ([`crate::storage::DiskRaftStorage`]) for real processes; the storage's
    /// initial state carries the voter set and any surviving HardState/log.
    pub fn with_storage(node: NodeId, region: RegionId, storage: S) -> Result<RaftPeer<S>> {
        let cfg = Config {
            id: node.0,
            election_tick: 10,
            heartbeat_tick: 3,
            // DESIGN "5.3 MetaLeader election": disturbance protection + the
            // gray-failure discipline (leader steps down without an active quorum).
            pre_vote: true,
            check_quorum: true,
            // PINNED, not defaulted (task #28 seam constraint): Safe makes a
            // ReadState index valid only after the leader confirms leadership
            // with a quorum round-trip. LeaseBased would trade that proof for
            // clock trust — the exact trade the linearizable-read promise
            // forbids. A raft-rs default change must not be able to change
            // our read semantics silently.
            read_only_option: ReadOnlyOption::Safe,
            ..Default::default()
        };
        cfg.validate().map_err(raft_err)?;
        let logger = Logger::root(Discard, o!());
        let raw = RawNode::new(&cfg, storage, &logger).map_err(raft_err)?;
        let conf_applied = raw.store().recovered_conf_index();
        Ok(RaftPeer {
            node,
            region,
            drain_minted: std::sync::atomic::AtomicBool::new(false),
            inner: Mutex::new(PeerInner {
                raw,
                ready: Vec::new(),
                outbox: Vec::new(),
                read_states: Vec::new(),
                alive: true,
                applied_reported: 0,
                conf_applied,
                step_errors: 0,
                fatal: None,
            }),
        })
    }

    pub fn node_id(&self) -> NodeId {
        self.node
    }

    fn lock(&self) -> MutexGuard<'_, PeerInner<S>> {
        self.inner.lock().expect("raft peer poisoned")
    }

    /// Request a quorum-confirmed read index (task #28). Leader-only by
    /// design: the establishing read type owns leadership discovery, and a
    /// follower answering reads is exactly what the linearizable promise
    /// forbids until it has proven leadership. `rctx` must be unique per
    /// request (the driver mints it from its boot incarnation + a counter);
    /// the confirmation returns through [`Self::take_read_states`] correlated
    /// by that exact context.
    pub fn read_index(&self, rctx: Vec<u8>) -> Result<()> {
        let mut g = self.lock();
        g.check_fatal()?;
        if g.raw.raft.state != StateRole::Leader {
            return Err(Error::NotLeader {
                leader: match g.raw.raft.leader_id {
                    0 => None,
                    id => Some(NodeId(id)),
                },
            });
        }
        g.raw.read_index(rctx);
        Ok(())
    }

    /// Drain quorum-confirmed read states captured by the Ready loop.
    pub fn take_read_states(&self) -> Vec<ReadState> {
        std::mem::take(&mut self.lock().read_states)
    }

    /// Propose on the leader, returning the locally assigned [`ProposedAt`].
    /// `propose` and the `last_index` read happen under one lock, so the pair is
    /// exact; it is still only a position claim (see [`ProposedAt`]).
    pub(crate) fn propose_traced(&self, data: Vec<u8>) -> Result<ProposedAt> {
        self.propose_in_term(data, None)
    }

    /// Compare leadership and the planning term under the same lock as append.
    pub(crate) fn propose_in_term(
        &self,
        data: Vec<u8>,
        expected: Option<u64>,
    ) -> Result<ProposedAt> {
        let mut g = self.lock();
        g.check_fatal()?;
        if g.raw.raft.state != StateRole::Leader {
            let leader = g.raw.raft.leader_id;
            return Err(Error::NotLeader {
                leader: (leader != raft::INVALID_ID).then_some(NodeId(leader)),
            });
        }
        if expected.is_some_and(|term| term != g.raw.raft.term) {
            return Err(Error::WriteConflict(
                "catalog planning term changed; rebuild the transaction".into(),
            ));
        }
        let term = g.raw.raft.term;
        g.raw.propose(Vec::new(), data).map_err(raft_err)?;
        let index = g.raw.raft.raft_log.last_index();
        Ok(ProposedAt {
            term,
            index: LogIndex(index),
        })
    }

    /// This node's current view of the group leader, if any.
    pub fn leader_hint(&self) -> Option<NodeId> {
        let g = self.lock();
        let id = g.raw.raft.leader_id;
        (id != raft::INVALID_ID).then_some(NodeId(id))
    }

    /// The current raft term at this peer.
    pub fn term(&self) -> u64 {
        self.lock().raw.raft.term
    }

    /// The highest applied-position handed out via `take_ready` so far comes from
    /// the drained entries themselves; the raft-committed watermark is this.
    pub fn raft_committed(&self) -> LogIndex {
        LogIndex(self.lock().raw.raft.raft_log.committed)
    }

    fn tick(&self) {
        let mut g = self.lock();
        if g.alive {
            g.raw.tick();
        }
    }

    /// Inbound raft message. A rejected message is DROPPED, not fatal: raft is
    /// built for lossy transport and the sender retransmits — the opposite
    /// contract from committed-entry apply, where nobody re-delivers and
    /// skipping means divergence (droppable iff someone resends it).
    fn step(&self, msg: Message) {
        let mut g = self.lock();
        if g.alive && g.raw.step(msg).is_err() {
            g.step_errors += 1;
        }
    }

    /// How many inbound messages this peer's `step` has rejected (diagnostic;
    /// see the field doc — growth signals misconfiguration, not data loss).
    pub fn step_errors(&self) -> u64 {
        self.lock().step_errors
    }

    /// Drain this peer's `Ready`: **persist entries + hardstate first**, then queue
    /// messages, then stash committed entries for `take_ready`, then advance.
    fn process_ready(&self) -> Result<()> {
        let mut g = self.lock();
        g.check_fatal()?;
        if !g.alive || !g.raw.has_ready() {
            return Ok(());
        }
        let mut ready = g.raw.ready();
        // Quorum-confirmed read states (task #28): drained HERE because this
        // is the single Ready consumer — a second consumer would steal them
        // exactly like it would steal committed entries.
        let read_states = ready.take_read_states();
        let mut msgs = ready.take_messages();
        // 1. Persist raft-log entries and hardstate (the safety point: durable
        //    BEFORE any message leaves this node — a vote must never outrun its
        //    own persistence).
        if !ready.entries().is_empty() {
            if let Err(cause) = g.raw.store().append(ready.entries()) {
                return Err(g.fail_storage("append", cause));
            }
        }
        if let Some(hs) = ready.hs() {
            if let Err(cause) = g.raw.store().set_hardstate(hs) {
                return Err(g.fail_storage("hardstate", cause));
            }
        }
        // 2. Only now hand messages to the transport.
        msgs.extend(ready.take_persisted_messages());
        // 3. Committed entries → the take_ready queue, typed by raft entry kind
        //    so the apply loop can route them (conf changes must reach
        //    `apply_conf_change`, never `Command::decode`). No-op barriers are
        //    queued too: the driver needs their indexes to advance raft's
        //    applied progress, even though they never touch the state machine.
        let mut committed: Vec<CommittedEntry> = Vec::new();
        for e in ready.take_committed_entries() {
            committed.push(classify_entry(e));
        }
        // `advance_append`, NOT `advance`: `advance()` internally marks apply
        // progress as caught up, but our apply happens later, when the driver
        // drains `take_ready`. raft-rs gates one-at-a-time conf-change safety
        // on the applied index, so marking early would let a second conf
        // change in before the first is truly applied. The driver reports real
        // progress via [`RaftPeer::applied_to`].
        let mut light = g.raw.advance_append(ready);
        msgs.extend(light.take_messages());
        for e in light.take_committed_entries() {
            committed.push(classify_entry(e));
        }
        g.ready.extend(committed);
        g.outbox.extend(msgs);
        g.read_states.extend(read_states);
        Ok(())
    }

    /// Report real apply progress to raft (task #24). Call ONLY after the
    /// entries up to `idx` have actually been applied (state machine writes
    /// done, conf changes applied) — raft uses this to gate the next
    /// one-at-a-time configuration change. Never called with a lower index
    /// than a previous call (monotonic; regressions are skipped defensively).
    ///
    /// Lock discipline: takes only `peer.inner`. Callers must NOT hold the
    /// driver's `applied`/`sm` locks (peer never nests with driver locks).
    pub fn applied_to(&self, idx: u64) {
        let mut g = self.lock();
        if g.fatal.is_none() && idx > g.applied_reported {
            g.applied_reported = idx;
            g.raw.advance_apply_to(idx);
        }
    }

    /// Propose a raft configuration change (AddLearnerNode / AddNode / …),
    /// correlated by `(term, index)` exactly like [`Self::propose_traced`].
    /// Harness-only raw-byte proposal (task #9 round 4): production callers
    /// propose COMMANDS; the byte surface exists solely so tests can inject
    /// undecodable garbage and prove the apply loop poisons on it.
    #[cfg(any(test, feature = "testing"))]
    pub fn propose_raw_for_harness(&self, data: Vec<u8>) -> Result<ProposedAt> {
        self.propose_traced(data)
    }

    pub fn propose_conf_change_traced(&self, cc: ConfChangeV2) -> Result<ProposedAt> {
        let mut g = self.lock();
        g.check_fatal()?;
        let term = g.raw.raft.term;
        g.raw
            .propose_conf_change(Vec::new(), cc)
            .map_err(raft_err)?;
        let index = g.raw.raft.raft_log.last_index();
        Ok(ProposedAt {
            term,
            index: LogIndex(index),
        })
    }

    /// Apply a committed configuration-change entry (called by the driver's
    /// apply loop, in log order) and durably persist the resulting
    /// `ConfState`. Returns the post-change `(voters, learners)`.
    ///
    /// The persistence is NOT optional: raft-rs only mutates the in-memory
    /// tracker; without an on-disk ConfState record a restart would resurrect
    /// the pre-change membership (Tess's #24 finding).
    pub fn apply_conf_change_bytes(
        &self,
        kind: EntryKind,
        data: &[u8],
        index: u64,
    ) -> Result<(Vec<u64>, Vec<u64>)> {
        let mut g = self.lock();
        g.check_fatal()?;
        // Replay guard: at or below the recovered boundary this change is
        // already reflected in the ConfState we opened with. Single-step conf
        // ops are RELATIVE (AddLearner on a voter is a demotion), so
        // re-application onto the final membership is not idempotent.
        if index <= g.conf_applied {
            let cs = g.raw.raft.prs().conf().to_conf_state();
            let (mut v, mut l) = (cs.voters.to_vec(), cs.learners.to_vec());
            v.sort_unstable();
            l.sort_unstable();
            return Ok((v, l));
        }
        let cs = match kind {
            EntryKind::ConfChangeV1 => {
                let cc = ConfChange::parse_from_bytes(data)
                    .map_err(|e| Error::Raft(format!("undecodable ConfChange: {e}")))?;
                g.raw.apply_conf_change(&cc).map_err(raft_err)?
            }
            EntryKind::ConfChangeV2 => {
                let cc = ConfChangeV2::parse_from_bytes(data)
                    .map_err(|e| Error::Raft(format!("undecodable ConfChangeV2: {e}")))?;
                g.raw.apply_conf_change(&cc).map_err(raft_err)?
            }
            other => {
                return Err(Error::Raft(format!(
                    "apply_conf_change_bytes called with non-conf kind {other:?}"
                )))
            }
        };
        if let Err(cause) = g.raw.store().set_conf_state(&cs, index) {
            return Err(g.fail_storage("confstate", cause));
        }
        g.conf_applied = index;
        let (mut v, mut l) = (cs.voters.to_vec(), cs.learners.to_vec());
        v.sort_unstable();
        l.sort_unstable();
        Ok((v, l))
    }

    /// The current membership as raft sees it: `(voters, learners)`, sorted.
    pub fn membership(&self) -> (Vec<u64>, Vec<u64>) {
        let g = self.lock();
        let cs = g.raw.raft.prs().conf().to_conf_state();
        let (mut v, mut l) = (cs.voters.to_vec(), cs.learners.to_vec());
        v.sort_unstable();
        l.sort_unstable();
        (v, l)
    }

    /// Everything the status surface needs from the peer, read under ONE lock
    /// acquisition — during a conf apply, piecemeal reads can tear (role from
    /// before the change, membership from after) and misreport a healthy node
    /// as follower/unconfigured for an instant (Tess's review of 107b161).
    pub fn status_snapshot(&self) -> PeerSnapshot {
        let g = self.lock();
        let cs = g.raw.raft.prs().conf().to_conf_state();
        let (mut voters, mut learners) = (cs.voters.to_vec(), cs.learners.to_vec());
        voters.sort_unstable();
        learners.sort_unstable();
        let leader = g.raw.raft.leader_id;
        PeerSnapshot {
            node_id: self.node,
            leader_hint: if leader == 0 {
                None
            } else {
                Some(NodeId(leader))
            },
            raw_role: role_of(g.raw.raft.state),
            term: g.raw.raft.term,
            committed: g.raw.raft.raft_log.committed,
            step_errors: g.step_errors,
            promotable: g.raw.raft.promotable(),
            conf_applied: g.conf_applied,
            voters,
            learners,
        }
    }

    /// Whether this node may campaign (it is a voter in the current config and
    /// present in the progress list). `false` for learners AND for a node that
    /// is in neither set — callers must distinguish those two via
    /// [`Self::membership`]: "not in the config at all" must never be reported
    /// as an ordinary follower (Ren's #23 finding).
    pub fn promotable(&self) -> bool {
        self.lock().raw.raft.promotable()
    }

    /// Feed one raft message from the transport into this peer.
    pub fn step_message(&self, msg: Message) {
        self.step(msg);
    }

    /// Advance one tick (called by the driver on its real-time cadence).
    pub fn tick_once(&self) {
        self.tick();
    }

    /// Process pending Ready state and take the outgoing messages for the
    /// transport. Persistence happens inside, before messages are returned.
    /// An error emits nothing and permanently stops this peer until recovery.
    pub fn pump(&self) -> Result<Vec<Message>> {
        self.process_ready()?;
        Ok(std::mem::take(&mut self.lock().outbox))
    }

    /// Testing-only election seam: ask THIS peer (it must currently be the
    /// leader) to hand leadership to `transferee` (raft-rs MsgTransferLeader
    /// → MsgTimeoutNow; the transferee campaigns immediately, exempt from
    /// the pre_vote/check_quorum leader-stickiness that would reject an
    /// ordinary campaign against a live leader). This is the deterministic
    /// way to construct "a specific node is leader" in tests; it is NOT a
    /// product API — deliberately absent from `RawApi` and every server
    /// surface, so no production path can call it.
    #[cfg(any(test, feature = "testing"))]
    pub fn transfer_leader_for_tests(&self, transferee: NodeId) {
        self.lock().raw.transfer_leader(transferee.0);
    }
}

impl<S: PersistentRaftStorage> RaftGroup for RaftPeer<S> {
    fn region_id(&self) -> RegionId {
        self.region
    }

    fn role(&self) -> Role {
        match self.lock().raw.raft.state {
            StateRole::Leader => Role::Leader,
            StateRole::Follower => Role::Follower,
            StateRole::Candidate | StateRole::PreCandidate => Role::Candidate,
        }
    }

    fn propose(&self, cmd: &crate::Command) -> Result<LogIndex> {
        let data = cmd.encode();
        Ok(self.propose_traced(data)?.index)
    }

    fn committed_index(&self) -> LogIndex {
        self.raft_committed()
    }

    fn campaign(&self) -> Result<()> {
        let mut g = self.lock();
        g.check_fatal()?;
        g.raw.campaign().map_err(raft_err)
    }
}

impl<S: PersistentRaftStorage> RaftPeer<S> {
    /// Module-private drain. `RaftPeer` deliberately does NOT implement
    /// [`crate::ReadyConsume`]: the peer is public and `Arc`-shared
    /// (`NodeDriver::peer()`, harness accessors), so a public impl here would
    /// hand the destructive drain to every holder — the exact second-consumer
    /// hole task #5 closes. Private to `rawnode` (not `pub(crate)`): a
    /// sibling module bypassing the token would be a second in-crate drain,
    /// the same hole one layer in. The consume faces living in this module
    /// ([`DrainToken`], the gated `HarnessPump`) are its only callers.
    fn drain_ready(&self) -> Result<Vec<CommittedEntry>> {
        let mut g = self.lock();
        g.check_fatal()?;
        Ok(std::mem::take(&mut g.ready))
    }
}

/// THE drain capability over one [`RaftPeer`] (task #5).
///
/// Minted at most once per peer for the life of the process — a second mint
/// is a typed refusal, so a second production Ready consumer cannot be wired
/// even on purpose. Deliberately neither `Clone` nor `Copy` (measured below):
/// duplicating the token would be duplicating the drain.
///
/// Production wiring: `NodeDriver::new` mints this token and never exposes
/// it; `NodeDriver::peer()` keeps handing out the peer, which can propose
/// and observe but not drain.
///
/// The wholesale-peer-accessor route (`InProcessCluster::peers()`) is gated
/// out of production builds with the other harness types; see the guard
/// boundary note on [`crate::ReadyConsume`] for why that gate has no
/// doc-test probe (feature unification makes one unfireable).
pub struct DrainToken<S: PersistentRaftStorage = MemStorage> {
    peer: Arc<RaftPeer<S>>,
}

impl<S: PersistentRaftStorage> DrainToken<S> {
    /// Mint the single drain token for `peer`. Crate-internal: an external
    /// `RaftPeer` holder must not be able to mint first — that would make it
    /// the production consumer and turn the real `NodeDriver::new` into a
    /// typed failure. `NodeDriver::new` is the only production mint site;
    /// the atomic once-mint below keeps even in-crate callers from coexisting
    /// with it.
    pub(crate) fn mint(peer: &Arc<RaftPeer<S>>) -> Result<DrainToken<S>> {
        use std::sync::atomic::Ordering;
        if peer
            .drain_minted
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(Error::Raft(format!(
                "drain token already minted for peer {} region {} — one Ready \
                 consumer per group (task #5); pump through the existing driver",
                peer.node.0, peer.region.0
            )));
        }
        Ok(DrainToken { peer: peer.clone() })
    }
}

impl<S: PersistentRaftStorage> crate::ReadyConsume for DrainToken<S> {
    fn take_ready(&self) -> Result<Vec<CommittedEntry>> {
        self.peer.drain_ready()
    }
}

/// Harness-only pump wrapper (task #5): deterministic test drivers hold many
/// peers and pump each in lockstep, which the once-minted [`DrainToken`]
/// deliberately does not allow twice over one peer across harness rebuilds.
/// Gated so production code never regains "any peer holder can drain".
#[cfg(any(test, feature = "testing"))]
pub struct HarnessPump<'a, S: PersistentRaftStorage = MemStorage>(pub &'a RaftPeer<S>);

#[cfg(any(test, feature = "testing"))]
impl<S: PersistentRaftStorage> crate::ReadyConsume for HarnessPump<'_, S> {
    fn take_ready(&self) -> Result<Vec<CommittedEntry>> {
        self.0.drain_ready()
    }
}

impl DrainToken<MemStorage> {
    /// Compile-time guard (E0080 pattern shared with `ReadBarrier`): adding
    /// `Clone` or `Copy` back turns this into a build error, not a silently
    /// duplicable drain. Verified in both directions when introduced.
    #[allow(dead_code)]
    const NOT_CLONE_OR_COPY: () = {
        struct Probe<T>(core::marker::PhantomData<T>);
        trait Fallback {
            const CHECK: () = ();
        }
        impl<T> Fallback for Probe<T> {}
        impl<T: Clone> Probe<T> {
            const CHECK: () = panic!("DrainToken must be neither Clone nor Copy");
        }
        Probe::<DrainToken>::CHECK
    };
}

/// A deterministic in-process cluster of [`RaftPeer`]s for one region: explicit
/// `round()` pumping (tick → process readies → deliver messages), no threads, no
/// timers, no sleeps — the shape the Phase-1 acceptance harness drives.
///
/// Gated out of production builds (task #5): its `peers()`/`peer()` accessors
/// hand out `Arc<RaftPeer>`s wholesale, which is harness ergonomics — not a
/// capability production code should be able to reach.
#[cfg(any(test, feature = "testing"))]
pub struct InProcessCluster {
    region: RegionId,
    peers: Vec<Arc<RaftPeer<MemStorage>>>,
}

#[cfg(any(test, feature = "testing"))]
impl InProcessCluster {
    pub fn new(region: RegionId, voters: &[NodeId]) -> Result<InProcessCluster> {
        let peers = voters
            .iter()
            .map(|&n| RaftPeer::new(n, region, voters).map(Arc::new))
            .collect::<Result<Vec<_>>>()?;
        Ok(InProcessCluster { region, peers })
    }

    pub fn region_id(&self) -> RegionId {
        self.region
    }

    pub fn peers(&self) -> &[Arc<RaftPeer<MemStorage>>] {
        &self.peers
    }

    pub fn peer(&self, node: NodeId) -> Option<&Arc<RaftPeer<MemStorage>>> {
        self.peers.iter().find(|p| p.node == node)
    }

    /// Harness kill switch: a dead peer stops ticking and drops traffic both ways.
    pub fn set_alive(&self, node: NodeId, alive: bool) {
        if let Some(p) = self.peer(node) {
            p.lock().alive = alive;
        }
    }

    /// The current leader, if exactly one *alive* peer believes it is leader.
    pub fn leader(&self) -> Option<NodeId> {
        let mut leaders = self
            .peers
            .iter()
            .filter(|p| {
                let g = p.lock();
                g.alive && g.raw.raft.state == StateRole::Leader
            })
            .map(|p| p.node);
        match (leaders.next(), leaders.next()) {
            (Some(l), None) => Some(l),
            _ => None,
        }
    }

    /// One deterministic pump round: tick every alive peer, then settle multi-hop
    /// message exchanges (process readies → deliver → repeat a bounded number of
    /// passes). No wall-clock anywhere.
    pub fn round(&self) {
        for p in &self.peers {
            p.tick();
        }
        for _ in 0..10 {
            for p in &self.peers {
                p.process_ready().expect("in-process persistence");
            }
            let mut in_flight: Vec<Message> = Vec::new();
            for p in &self.peers {
                let mut g = p.lock();
                if g.alive {
                    in_flight.append(&mut g.outbox);
                } else {
                    g.outbox.clear();
                }
            }
            if in_flight.is_empty() {
                continue;
            }
            for msg in in_flight {
                if let Some(target) = self.peers.iter().find(|p| p.node.0 == msg.to) {
                    target.step(msg);
                }
            }
        }
    }

    /// Pump until `cond` holds, at most `max_rounds` rounds. Deterministic: fails
    /// with a typed error instead of hanging or sleeping.
    pub fn run_until<F: Fn(&InProcessCluster) -> bool>(
        &self,
        max_rounds: usize,
        what: &str,
        cond: F,
    ) -> Result<()> {
        for _ in 0..max_rounds {
            if cond(self) {
                return Ok(());
            }
            self.round();
        }
        Err(Error::Raft(format!(
            "condition not reached within {max_rounds} rounds: {what}"
        )))
    }
}

fn raft_err(e: raft::Error) -> Error {
    Error::Raft(e.to_string())
}

/// Validate a declared voter set at a peer-construction entry point: non-empty
/// and duplicate-free, regardless of what the CLI did upstream (fail closed at
/// the library boundary, not only at the flag parser).
fn validate_voter_declaration(voters: &[NodeId]) -> Result<Vec<u64>> {
    if voters.is_empty() {
        return Err(Error::Raft("declared voter set must not be empty".into()));
    }
    let mut ids: Vec<u64> = voters.iter().map(|n| n.0).collect();
    ids.sort_unstable();
    if ids.windows(2).any(|w| w[0] == w[1]) {
        return Err(Error::Raft(
            "declared voter set contains duplicate node ids".into(),
        ));
    }
    Ok(ids)
}

/// Map a raft entry to the typed committed item the apply loop consumes.
/// Runs outside any lock; pure classification.
fn classify_entry(e: Entry) -> CommittedEntry {
    let kind = match e.get_entry_type() {
        EntryType::EntryNormal if e.data.is_empty() => EntryKind::Noop,
        EntryType::EntryNormal => EntryKind::Command,
        EntryType::EntryConfChange => EntryKind::ConfChangeV1,
        EntryType::EntryConfChangeV2 => EntryKind::ConfChangeV2,
    };
    CommittedEntry {
        index: LogIndex(e.index),
        term: e.term,
        kind,
        data: e.data.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{drive_apply, Command, MemStateMachine, RaftGroup, StateMachine};
    use kv9_engine::ColumnFamily;

    const R: RegionId = RegionId(1);
    const N1: NodeId = NodeId(1);
    const N2: NodeId = NodeId(2);
    const N3: NodeId = NodeId(3);

    /// Task #5: the drain capability is minted at most once per peer — the
    /// second mint is a typed refusal, not a second consumer.
    #[test]
    fn drain_token_mints_exactly_once_per_peer() {
        let peer = Arc::new(RaftPeer::new(N1, R, &[N1]).unwrap());
        let _token = DrainToken::mint(&peer).expect("first mint is THE drain");
        let second = DrainToken::mint(&peer);
        match second {
            Err(Error::Raft(msg)) => assert!(
                msg.contains("already minted"),
                "refusal must name the cause, got: {msg}"
            ),
            Ok(_) => panic!("second mint must refuse, got a second drain token"),
            Err(other) => panic!("refusal must be Error::Raft, got: {other:?}"),
        }
    }

    #[test]
    fn catalog_proposal_checks_planning_term_before_append() {
        let peer = RaftPeer::new(N1, R, &[N1]).unwrap();
        let empty = peer.lock().raw.raft.raft_log.last_index();
        assert!(matches!(
            peer.propose_in_term(put(b"not-leader", b"v"), Some(1)),
            Err(Error::NotLeader { leader: None })
        ));
        assert_eq!(peer.lock().raw.raft.raft_log.last_index(), empty);
        peer.campaign().unwrap();
        peer.pump().unwrap();
        assert_eq!(peer.role(), Role::Leader);
        let term = peer.term();
        let before = peer.lock().raw.raft.raft_log.last_index();
        assert!(matches!(
            peer.propose_in_term(put(b"stale", b"v"), Some(term + 1)),
            Err(Error::WriteConflict(_))
        ));
        assert_eq!(
            peer.lock().raw.raft.raft_log.last_index(),
            before,
            "stale plan never enters log"
        );
        assert_eq!(
            peer.propose_in_term(put(b"fresh", b"v"), Some(term))
                .unwrap()
                .index
                .0,
            before + 1
        );
    }

    fn put(key: &[u8], value: &[u8]) -> Vec<u8> {
        Command::Put {
            cf: 0,
            key: key.to_vec(),
            value: value.to_vec(),
        }
        .encode()
    }

    /// Pump + apply every alive peer's drained entries into its own state machine.
    ///
    /// Apply failures crash the test — skipping a committed entry is as wrong
    /// in a test as in production (a silently diverged replica). The production
    /// apply loop is `NodeDriver::step()`, which poisons the driver instead of
    /// panicking.
    fn drive(cluster: &InProcessCluster, sms: &mut [MemStateMachine]) {
        cluster.round();
        for (p, sm) in cluster.peers().iter().zip(sms.iter_mut()) {
            drive_apply(&HarnessPump(p.as_ref()), sm).expect("apply failed in test drive");
        }
    }

    fn run<F: Fn(&InProcessCluster, &[MemStateMachine]) -> bool>(
        cluster: &InProcessCluster,
        sms: &mut [MemStateMachine],
        what: &str,
        cond: F,
    ) {
        for _ in 0..500 {
            if cond(cluster, sms) {
                return;
            }
            drive(cluster, sms);
        }
        panic!("not reached in 500 rounds: {what}");
    }

    /// Real consensus single-node: campaign → propose → committed entry drained via
    /// take_ready → applied → readable. The full pull-model path over raft-rs.
    #[test]
    fn single_node_propose_apply_get() {
        let cluster = InProcessCluster::new(R, &[N1]).unwrap();
        let mut sms = vec![MemStateMachine::new()];
        cluster.peers()[0].campaign().unwrap();
        run(&cluster, &mut sms, "self-elect", |c, _| {
            c.leader().is_some()
        });

        let at = cluster.peers()[0].propose_traced(put(b"k", b"v")).unwrap();
        run(&cluster, &mut sms, "applied", |_, sms| {
            sms[0].applied_index() >= at.index
        });
        assert_eq!(
            sms[0].get(ColumnFamily::Default, b"k").unwrap(),
            Some(b"v".to_vec())
        );
    }

    /// 3 nodes: elect, replicate, every replica applies the entry with the
    /// proposer's (term, index) — the acceptance sync criterion, no sleeps.
    #[test]
    fn three_node_replication_and_failover() {
        let cluster = InProcessCluster::new(R, &[N1, N2, N3]).unwrap();
        let mut sms = vec![
            MemStateMachine::new(),
            MemStateMachine::new(),
            MemStateMachine::new(),
        ];
        cluster.peers()[0].campaign().unwrap();
        run(&cluster, &mut sms, "elect", |c, _| c.leader().is_some());
        let leader = cluster.leader().unwrap();

        let at = cluster
            .peer(leader)
            .unwrap()
            .propose_traced(put(b"a", b"1"))
            .unwrap();
        run(&cluster, &mut sms, "all applied", |_, sms| {
            sms.iter().all(|sm| sm.applied_index() >= at.index)
        });
        for sm in &sms {
            assert_eq!(
                sm.get(ColumnFamily::Default, b"a").unwrap(),
                Some(b"1".to_vec())
            );
        }

        // Live leader failover: kill the leader, survivors re-elect + commit.
        cluster.set_alive(leader, false);
        run(&cluster, &mut sms, "re-elect", |c, _| {
            c.leader().map(|l| l != leader).unwrap_or(false)
        });
        let leader2 = cluster.leader().unwrap();
        let at2 = cluster
            .peer(leader2)
            .unwrap()
            .propose_traced(put(b"b", b"2"))
            .unwrap();
        // The new leader's no-op barrier consumed at least one index.
        assert!(at2.index > at.index);
        let survivors: Vec<usize> = cluster
            .peers()
            .iter()
            .enumerate()
            .filter(|(_, p)| p.node_id() != leader)
            .map(|(i, _)| i)
            .collect();
        run(&cluster, &mut sms, "survivors applied", |_, sms| {
            survivors
                .iter()
                .all(|&i| sms[i].applied_index() >= at2.index)
        });
        for &i in &survivors {
            assert_eq!(
                sms[i].get(ColumnFamily::Default, b"b").unwrap(),
                Some(b"2".to_vec())
            );
        }
    }

    /// The overwrite hazard (contract item 13): a proposal accepted locally but
    /// never committed is overwritten after a failover. Its position is passed by
    /// the new leader's log, yet the proposal's payload never commits anywhere —
    /// so "position reached" MUST NOT be reported as success; only a matching
    /// (term, index) / payload may.
    #[test]
    fn uncommitted_proposal_is_overwritten_not_committed() {
        let cluster = InProcessCluster::new(R, &[N1, N2, N3]).unwrap();
        let mut sms = vec![
            MemStateMachine::new(),
            MemStateMachine::new(),
            MemStateMachine::new(),
        ];
        cluster.peers()[0].campaign().unwrap();
        run(&cluster, &mut sms, "elect", |c, _| c.leader().is_some());
        let old_leader = cluster.leader().unwrap();

        // Isolate the leader, then let it accept a proposal it can never commit.
        cluster.set_alive(old_leader, false);
        let orphan = cluster
            .peer(old_leader)
            .unwrap()
            .propose_traced(put(b"orphan", b"lost"))
            .unwrap();

        // Survivors elect a new leader (higher term) and write.
        run(&cluster, &mut sms, "re-elect", |c, _| {
            c.leader().map(|l| l != old_leader).unwrap_or(false)
        });
        let new_leader = cluster.leader().unwrap();
        let at2 = cluster
            .peer(new_leader)
            .unwrap()
            .propose_traced(put(b"live", b"yes"))
            .unwrap();
        assert!(at2.term > orphan.term, "new leader must have a higher term");
        run(
            &cluster,
            &mut sms,
            "survivors applied live write",
            |c, sms| {
                c.peers()
                    .iter()
                    .zip(sms.iter())
                    .filter(|(p, _)| p.node_id() != old_leader)
                    .all(|(_, sm)| sm.applied_index() >= at2.index)
            },
        );

        // Rejoin the old leader; its orphan entry is truncated away.
        cluster.set_alive(old_leader, true);
        run(&cluster, &mut sms, "old leader catches up", |c, sms| {
            c.peers()
                .iter()
                .zip(sms.iter())
                .all(|(_, sm)| sm.applied_index() >= at2.index)
        });

        // The orphan's position was passed on every replica…
        for p in cluster.peers() {
            assert!(p.raft_committed() >= orphan.index);
        }
        // …but its payload committed nowhere: position reached ≠ proposal reached.
        for sm in &sms {
            assert_eq!(sm.get(ColumnFamily::Default, b"orphan").unwrap(), None);
            assert_eq!(
                sm.get(ColumnFamily::Default, b"live").unwrap(),
                Some(b"yes".to_vec())
            );
        }
    }

    /// A follower must refuse proposals (routing sends clients to the leader).
    #[test]
    fn follower_rejects_propose() {
        let cluster = InProcessCluster::new(R, &[N1, N2, N3]).unwrap();
        let mut sms = vec![
            MemStateMachine::new(),
            MemStateMachine::new(),
            MemStateMachine::new(),
        ];
        cluster.peers()[0].campaign().unwrap();
        run(&cluster, &mut sms, "elect", |c, _| c.leader().is_some());
        let leader = cluster.leader().unwrap();
        let follower = cluster
            .peers()
            .iter()
            .find(|p| p.node_id() != leader)
            .unwrap();
        assert!(follower.propose_traced(put(b"x", b"y")).is_err());
    }
}
