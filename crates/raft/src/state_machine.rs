//! Raft state-machine adapter (Phase-1 spine; ROADMAP Phase 1).
//!
//! The raft group replicates a log; the **state machine** deterministically applies each
//! committed entry. For the metadata plane the state machine is a KV backed by the
//! mocked [`kv9_engine::MemEngine`] (ROADMAP: "the raft state machine is the skeleton's
//! `MemEngine` (mock)"). The `meta` catalog engine ([`kv9_meta::MetaStore`]) runs *on
//! top of* this KV.
//!
//! Phase-1 path: `propose(cmd) → committed → apply → read` over the existing
//! [`crate::RaftGroup`] trait. Phase 1 provides both the immediate single-node group and
//! the deterministic raft-rs [`crate::RaftPeer`] adapter behind that same pull model.

use std::sync::Arc;

use kv9_engine::{ColumnFamily, Engine, MemEngine};

use kv9_common::Result;

use crate::command::Command;
use crate::{CommittedEntry, LogIndex};

/// The outcome of applying one committed entry to the state machine (ROADMAP Phase 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyResult {
    /// The log index that was applied (monotonic; the applied watermark advances to it).
    pub applied_index: LogIndex,
    /// Bytes returned to the proposer, if the command produces a read-back value
    /// (e.g. a conf-change ack). `None` for plain writes.
    pub response: Option<Vec<u8>>,
}

impl ApplyResult {
    pub fn write_ok(index: LogIndex) -> Self {
        ApplyResult {
            applied_index: index,
            response: None,
        }
    }
}

/// A deterministic raft state machine (ROADMAP Phase 1).
///
/// Every replica applies the *same* committed entries in the *same* order, reaching the
/// same state. `apply` must be deterministic and side-effect-free beyond its own state.
pub trait StateMachine: Send + Sync {
    /// Apply one committed entry, advancing the applied watermark (ROADMAP Phase 1).
    fn apply(&mut self, entry: &CommittedEntry) -> Result<ApplyResult>;

    /// The highest log index applied so far.
    fn applied_index(&self) -> LogIndex;
}

/// The Phase-1 metadata state machine: a KV over the mocked [`MemEngine`].
///
/// The catalog engine writes/read through the same engine, so a committed
/// `Command::CatalogTxn` lands atomically here and is then visible to
/// [`kv9_meta::MetaStore`] reads. Swapping `MemEngine` for the real disaggregated engine
/// is Phase-2 and does not change this type's shape (it is generic over [`Engine`]).
pub struct MemStateMachine<E: Engine = MemEngine> {
    engine: Arc<E>,
    applied: LogIndex,
}

impl MemStateMachine<MemEngine> {
    /// A fresh state machine over a new in-memory engine (ROADMAP Phase 1 first task).
    pub fn new() -> Self {
        MemStateMachine {
            engine: Arc::new(MemEngine::new()),
            applied: LogIndex(0),
        }
    }
}

impl Default for MemStateMachine<MemEngine> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: Engine> MemStateMachine<E> {
    /// Build a state machine over an existing shared engine (so the `meta` catalog and
    /// the raft apply loop observe the *same* KV).
    pub fn with_engine(engine: Arc<E>) -> Self {
        MemStateMachine {
            engine,
            applied: LogIndex(0),
        }
    }

    /// The backing engine — the KV the `meta` catalog reads/writes (ROADMAP Phase 1).
    pub fn engine(&self) -> &Arc<E> {
        &self.engine
    }

    /// Apply an already-decoded command at `index` (used by the propose→apply path and
    /// by tests that construct commands directly, avoiding the entry codec stub).
    pub fn apply_command(&mut self, index: LogIndex, cmd: &Command) -> Result<ApplyResult> {
        let batch = cmd.to_write_batch();
        if !batch.is_empty() {
            self.engine.write(batch)?;
        }
        self.applied = index;
        Ok(ApplyResult::write_ok(index))
    }

    /// Direct read-back from the state machine's KV (the `get` of the round-trip).
    pub fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.engine.get(cf, key)
    }
}

impl<E: Engine> StateMachine for MemStateMachine<E> {
    fn apply(&mut self, entry: &CommittedEntry) -> Result<ApplyResult> {
        // Phase-1: the committed entry carries opaque bytes; decode to a Command, then
        // apply its write batch.
        let cmd = Command::decode(&entry.data)?;
        self.apply_command(entry.index, &cmd)
    }

    fn applied_index(&self) -> LogIndex {
        self.applied
    }
}

/// Drive one `propose → commit → apply` cycle over a [`crate::RaftGroup`] into a
/// [`StateMachine`] (ROADMAP Phase 1 spine).
///
/// Drains all currently-ready committed entries and applies them in order, returning the
/// last [`ApplyResult`]. On the [`crate::SingleNodeRaft`] stub, a `propose` commits
/// immediately, so this is the whole path; real consensus commits asynchronously.
pub fn drive_apply<R, S>(raft: &R, sm: &mut S) -> Result<Vec<ApplyResult>>
where
    R: crate::RaftGroup + ?Sized,
    S: StateMachine,
{
    let ready = raft.take_ready()?;
    let mut out = Vec::with_capacity(ready.len());
    for entry in &ready {
        out.push(sm.apply(entry)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RaftGroup, SingleNodeRaft};
    use kv9_common::{NodeId, RegionId};

    /// Phase-1 milestone (ROADMAP): the first concrete task — a single-node raft with a
    /// `MemEngine` state machine and a `propose(put) → apply → get` round-trip, through
    /// the real encode → propose → take_ready → decode → apply path.
    #[test]
    fn propose_put_apply_get_roundtrip() {
        let raft = SingleNodeRaft::new(NodeId(1), RegionId(1));
        let mut sm = MemStateMachine::new();

        let cmd = Command::Put {
            cf: 0,
            key: b"k".to_vec(),
            value: b"v".to_vec(),
        };
        // The real path proposes encoded bytes and applies via take_ready → decode.
        raft.propose(cmd.encode()).unwrap();
        let results = drive_apply(&raft, &mut sm).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(
            sm.get(ColumnFamily::Default, b"k").unwrap(),
            Some(b"v".to_vec())
        );
    }

    /// The typed variant of the round-trip that does not depend on the entry codec: it
    /// applies the command directly at its committed index. This documents the Phase-1
    /// spine working end to end today.
    #[test]
    fn propose_put_apply_get_roundtrip_typed() {
        let raft = SingleNodeRaft::new(NodeId(1), RegionId(1));
        let mut sm = MemStateMachine::new();

        let index = raft.propose(Vec::new()).unwrap();
        let _ = raft.take_ready().unwrap();
        let cmd = Command::Put {
            cf: 0,
            key: b"k".to_vec(),
            value: b"v".to_vec(),
        };
        sm.apply_command(index, &cmd).unwrap();

        assert_eq!(sm.applied_index(), index);
        assert_eq!(
            sm.get(ColumnFamily::Default, b"k").unwrap(),
            Some(b"v".to_vec())
        );
    }
}
