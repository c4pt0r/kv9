//! # kv9-raft
//!
//! Consensus abstraction (DESIGN §6.1). Each region runs an independent Raft group
//! (multi-raft); metadata-plane groups (L0 bootstrap, L1 meta-regions) use the same
//! machinery (DESIGN §5). v0 ships a single-node stub; real consensus arrives in M2.
//!
//! Phase-1 spine (ROADMAP Phase 1): the [`state_machine`] module adds a [`StateMachine`]
//! trait and a [`MemStateMachine`] backed by [`kv9_engine::MemEngine`] (the mocked
//! storage), plus a `propose → commit → apply → read` path over the [`RaftGroup`] trait.
//! The replicated payloads are [`Command`]s (metadata mutations). The production
//! Phase-1 adapter is tikv/raft-rs (`RawNode`/`Ready`) behind the same pull interface.

pub mod command;
pub mod driver;
pub mod grpc;
mod proposal_queue;
pub mod rawnode;
pub mod state_machine;
pub mod storage;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
pub mod transport;
pub mod work;

pub use command::{
    cf_code, cf_from_code, Command, FencedInner, KvOp, ManifestChangePayload, RegionFence,
};
pub use proposal_queue::ProposalQueueSnapshot;
#[cfg(any(test, feature = "testing"))]
pub use rawnode::HarnessPump;
#[cfg(any(test, feature = "testing"))]
pub use rawnode::InProcessCluster;
pub use rawnode::{DrainToken, ProposedAt, RaftPeer};
pub use state_machine::{
    classify_reconciliation, drive_apply, ApplyOutcome, ApplyResult, ApplyStore, FenceAdjudicator,
    ManifestInvalidReason, ManifestPair, ManifestVerdict, MemStateMachine, ReconcileObservation,
    StateMachine,
};

use kv9_common::{NodeId, RegionId, Result};

/// The role of a peer within its Raft group (DESIGN §6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Leader,
    Follower,
    Candidate,
    Learner,
    /// This node is in NEITHER the voter nor the learner set of the live
    /// configuration (removed from membership, wrong config, or stale
    /// ConfState). Reported distinctly because a healthy follower and a node
    /// that isn't part of the cluster at all must never look the same
    /// (task #24; Ren's three-way rule).
    Unconfigured,
}

/// A committed log index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct LogIndex(pub u64);

/// What a committed entry carries — the raft-level entry type, preserved so the
/// apply loop can route it (task #24: a real `EntryConfChange` must reach
/// `apply_conf_change`, never `Command::decode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// A normal entry with an application command payload.
    Command,
    /// A leader-election no-op barrier (empty normal entry). Carried so raft's
    /// applied progress can advance through it; it never reaches the state
    /// machine and never advances the durable watermark.
    Noop,
    /// A raft `EntryConfChange` (protobuf `ConfChange` payload).
    ConfChangeV1,
    /// A raft `EntryConfChangeV2` (protobuf `ConfChangeV2` payload).
    ConfChangeV2,
}

/// A ready-to-apply committed entry handed to the region apply loop (DESIGN §6.1, §6.2).
#[derive(Debug, Clone)]
pub struct CommittedEntry {
    pub index: LogIndex,
    /// The term the entry was proposed in. Proposal correlation is by
    /// `(term, index)` — after a leader change the same index can carry a
    /// different leader's entry, so position alone never confirms a proposal
    /// (see [`rawnode::ProposedAt`]).
    pub term: u64,
    /// How to interpret `data` (command vs. configuration change vs. barrier).
    pub kind: EntryKind,
    /// Opaque payload bytes: a serialized [`Command`] for `Command`, a protobuf
    /// `ConfChange(V2)` for the conf kinds, empty for `Noop`.
    pub data: Vec<u8>,
}

/// One Raft group replicating a region's (or a meta-region's) log (DESIGN §6.1).
///
/// The `region` crate drives this: it proposes region commands, then applies the
/// committed entries into the engine (the raft log being the memtable WAL, DESIGN §6.2).
pub trait RaftGroup: Send + Sync {
    /// The region this group replicates.
    fn region_id(&self) -> RegionId;

    /// This node's role in the group.
    fn role(&self) -> Role;

    /// Whether this peer is the current leader.
    fn is_leader(&self) -> bool {
        self.role() == Role::Leader
    }

    /// Propose a COMMAND for replication (task #9 round 4: the byte-level
    /// surface is closed — a propose face that accepted opaque bytes let any
    /// holder hand-encode a `ManifestChange` wire image and bypass every
    /// constructor gate; review probe did exactly that from an external
    /// crate). Encoding happens inside the implementation; external callers
    /// can only propose commands they can CONSTRUCT, which is what makes
    /// payload privacy load-bearing. Returns the assigned log index once
    /// accepted by the leader (DESIGN §6.1).
    ///
    /// Resident guard — the decode-then-propose bypass must not compile
    /// (fires if `Command::decode` returns to the public surface):
    ///
    /// ```compile_fail
    /// fn probe(g: &dyn kv9_raft::RaftGroup, bytes: Vec<u8>) {
    ///     let _ = g.propose(&kv9_raft::Command::decode(&bytes).unwrap());
    /// }
    /// ```
    ///
    /// Green twin — proposing an externally-constructible command compiles:
    ///
    /// ```
    /// fn probe(g: &dyn kv9_raft::RaftGroup) {
    ///     let _ = g.propose(&kv9_raft::Command::Noop);
    /// }
    /// ```
    fn propose(&self, cmd: &Command) -> Result<LogIndex>;

    /// The highest log index committed so far.
    fn committed_index(&self) -> LogIndex;

    /// Trigger / observe a leadership campaign (used by BootstrapElection over
    /// `META_REGION_0`, DESIGN §5.2, and MetaLeader election, DESIGN §5.3).
    fn campaign(&self) -> Result<()>;
}

/// The DESTRUCTIVE drain face of a raft group, split from [`RaftGroup`]
/// (task #5). `take_ready` removes committed entries; whoever calls it owns
/// applying them. Two drains over one group hole the unified driver
/// watermark's contiguity — the foundation `wait_applied`, the bootstrap
/// barrier and ReadIndex all stand on — so the faces are separate TRAITS:
/// a component holding `Arc<dyn RaftGroup>` (Node, MetaRaft, the future
/// region runtime proposing manifest changes) can propose but can never
/// drain, no matter what impls it grows. The former near-miss — one
/// `impl RawApi for Node` away from a second production consumer — is now
/// unrepresentable rather than review-banned; landing this split retires
/// that temporary review constraint.
///
/// In production builds exactly one value implements this trait per group:
/// the [`DrainToken`] minted (at most once — typed refusal on the second
/// mint) inside `NodeDriver::new` and never exposed. The peer handed out by
/// `NodeDriver::peer()` does NOT implement this trait, and the harness types
/// that do (`SingleNodeRaft`, `HarnessPump`) carry the impl only under
/// `cfg(any(test, feature = "testing"))`.
///
/// # Resident guards — measured, not assumed, in both directions
///
/// A holder of the propose face cannot drain. Each probe must fail to
/// compile and stays here so re-opening the route turns the doc test red.
///
/// The trait-object route (re-adding `take_ready` to `RaftGroup` fires it):
///
/// ```compile_fail,E0599
/// fn probe(g: &dyn kv9_raft::RaftGroup) {
///     let _ = g.take_ready();
/// }
/// ```
///
/// The concrete-peer route — the near bypass: production code holds
/// `NodeDriver::peer()`'s `Arc<RaftPeer>` legitimately, so a public
/// `impl ReadyConsume for RaftPeer` (fires this probe) would hand every
/// such holder the drain:
///
/// ```compile_fail,E0599
/// fn probe(p: &std::sync::Arc<kv9_raft::RaftPeer>) {
///     use kv9_raft::ReadyConsume;
///     let _ = p.take_ready();
/// }
/// ```
///
/// The mint route — an external peer holder must not mint the token first
/// (that would make it the production consumer and turn the real
/// `NodeDriver::new` into a typed failure); `DrainToken::mint` is
/// crate-internal, so this fires exactly if minting goes public:
///
/// ```compile_fail,E0624
/// fn probe(p: &std::sync::Arc<kv9_raft::RaftPeer>) {
///     let _ = kv9_raft::DrainToken::mint(p);
/// }
/// ```
///
/// The harness-drain routes (`SingleNodeRaft`'s impl, `HarnessPump`,
/// `InProcessCluster`) are `cfg(any(test, feature = "testing"))`. They
/// cannot be guarded by a doc test in this workspace: workspace test runs
/// unify the `testing` feature on (kv9-server's dev-deps), so a
/// `compile_fail` probe against them is green in production builds but red
/// under `cargo test` — an unfireable guard, worse than none. They CAN be
/// guarded by an independent consumer probe built with
/// `default-features = false` outside the test feature unification
/// (measured: a gated import reds E0432 there, and un-gating turns it
/// green); until such a CI step exists, their enforcement is the
/// production build itself — any production caller of a gated item fails
/// `cargo check` — and the un-guarded cell is the GATE's presence with no
/// caller.
///
/// Green twin — the identical call against the minted drain token compiles,
/// pinning the red probes to "capability absent from that face" rather than
/// a typo, missing import, or wrong receiver:
///
/// ```
/// fn probe(t: &kv9_raft::DrainToken) {
///     use kv9_raft::ReadyConsume;
///     let _ = t.take_ready();
/// }
/// ```
pub trait ReadyConsume: Send + Sync {
    /// Drain entries that have been committed and are ready to apply
    /// (DESIGN §6.1).
    fn take_ready(&self) -> Result<Vec<CommittedEntry>>;
}

/// Trivial single-node Raft: one replica, entries commit immediately (DESIGN §6.1).
///
/// This is a skeleton stand-in so the workspace compiles and M1 runs single-node.
/// It is NOT real consensus.
pub struct SingleNodeRaft {
    node: NodeId,
    region: RegionId,
    log: std::sync::Mutex<SingleNodeLog>,
}

#[derive(Default)]
struct SingleNodeLog {
    next_index: u64,
    ready: Vec<CommittedEntry>,
}

impl SingleNodeRaft {
    pub fn new(node: NodeId, region: RegionId) -> Self {
        SingleNodeRaft {
            node,
            region,
            log: std::sync::Mutex::new(SingleNodeLog {
                next_index: 1,
                ready: Vec::new(),
            }),
        }
    }

    pub fn node_id(&self) -> NodeId {
        self.node
    }
}

impl RaftGroup for SingleNodeRaft {
    fn region_id(&self) -> RegionId {
        self.region
    }

    fn role(&self) -> Role {
        // A single node is always its own leader.
        Role::Leader
    }

    fn propose(&self, cmd: &Command) -> Result<LogIndex> {
        let data = cmd.encode();
        let mut log = self.log.lock().expect("raft log poisoned");
        let idx = LogIndex(log.next_index);
        log.next_index += 1;
        log.ready.push(CommittedEntry {
            index: idx,
            term: 1,
            kind: EntryKind::Command,
            data,
        });
        Ok(idx)
    }

    fn committed_index(&self) -> LogIndex {
        let log = self.log.lock().expect("raft log poisoned");
        LogIndex(log.next_index.saturating_sub(1))
    }

    fn campaign(&self) -> Result<()> {
        // Already leader; nothing to do for a single node.
        Ok(())
    }
}

/// Harness-only (task #5): production wiring hands `SingleNodeRaft` out as
/// `Arc<dyn RaftGroup>` — the propose face — and a bare `Node`/`MetaRaft`
/// must NOT be able to drain it. Test builds pump it through the same trait
/// the driver's `DrainToken` implements.
#[cfg(any(test, feature = "testing"))]
impl ReadyConsume for SingleNodeRaft {
    fn take_ready(&self) -> Result<Vec<CommittedEntry>> {
        let mut log = self.log.lock().expect("raft log poisoned");
        Ok(std::mem::take(&mut log.ready))
    }
}
