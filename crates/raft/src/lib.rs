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
pub mod storage;
pub mod transport;
pub mod state_machine;

pub use command::{cf_code, cf_from_code, Command, KvOp};
pub use rawnode::{InProcessCluster, ProposedAt, RaftPeer};
pub use state_machine::{drive_apply, ApplyResult, MemStateMachine, StateMachine};

use kv9_common::{NodeId, RegionId, Result};

/// The role of a peer within its Raft group (DESIGN §6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Leader,
    Follower,
    Candidate,
    Learner,
}

/// A committed log index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct LogIndex(pub u64);

/// A ready-to-apply committed entry handed to the region apply loop (DESIGN §6.1, §6.2).
#[derive(Debug, Clone)]
pub struct CommittedEntry {
    pub index: LogIndex,
    /// The term the entry was proposed in. Proposal correlation is by
    /// `(term, index)` — after a leader change the same index can carry a
    /// different leader's entry, so position alone never confirms a proposal
    /// (see [`rawnode::ProposedAt`]).
    pub term: u64,
    /// Opaque command bytes (a serialized region command / write batch).
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

    /// Drain entries that have been committed and are ready to apply (DESIGN §6.1).
    fn take_ready(&self) -> Result<Vec<CommittedEntry>>;

    /// The highest log index committed so far.
    fn committed_index(&self) -> LogIndex;

    /// Trigger / observe a leadership campaign (used by BootstrapElection over
    /// `META_REGION_0`, DESIGN §5.2, and MetaLeader election, DESIGN §5.3).
    fn campaign(&self) -> Result<()>;
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
            data,
        });
        Ok(idx)
    }

    fn take_ready(&self) -> Result<Vec<CommittedEntry>> {
        let mut log = self.log.lock().expect("raft log poisoned");
        Ok(std::mem::take(&mut log.ready))
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
