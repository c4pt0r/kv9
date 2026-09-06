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
pub mod rawnode;
pub mod state_machine;
pub mod storage;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
pub mod transport;

pub use command::{cf_code, cf_from_code, Command, FencedInner, KvOp, RegionFence};
pub use rawnode::{InProcessCluster, ProposedAt, RaftPeer};
pub use state_machine::{
    drive_apply, ApplyResult, FenceAdjudicator, MemStateMachine, StateMachine,
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

    /// Propose an opaque command for replication. Returns the assigned log index once
    /// accepted by the leader (DESIGN §6.1).
    fn propose(&self, data: Vec<u8>) -> Result<LogIndex>;

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
/// Production wiring gives this face to exactly one holder: `NodeDriver`'s
/// pump. Single-node test harnesses hold `SingleNodeRaft` concretely and
/// pump through the same trait.
///
/// # Resident guards — measured, not assumed, in both directions
///
/// A holder of the propose face cannot drain. This probe must fail to
/// compile, and stays here so re-adding `take_ready` to `RaftGroup` turns
/// the doc test red:
///
/// ```compile_fail,E0599
/// fn probe(g: &dyn kv9_raft::RaftGroup) {
///     let _ = g.take_ready();
/// }
/// ```
///
/// Green twin — the identical call against this face compiles, pinning the
/// red probe to "capability absent from `RaftGroup`" rather than a typo,
/// missing import, or wrong receiver:
///
/// ```
/// fn probe(c: &dyn kv9_raft::ReadyConsume) {
///     let _ = c.take_ready();
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

    fn propose(&self, data: Vec<u8>) -> Result<LogIndex> {
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

impl ReadyConsume for SingleNodeRaft {
    fn take_ready(&self) -> Result<Vec<CommittedEntry>> {
        let mut log = self.log.lock().expect("raft log poisoned");
        Ok(std::mem::take(&mut log.ready))
    }
}
