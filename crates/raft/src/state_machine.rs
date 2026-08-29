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

/// Engine key holding the durably applied watermark. The `0x00` first byte
/// cannot collide with any `mode_byte`-encoded physical key (`'t'`/`'r'`/`'s'`),
/// so catalog scans never see it.
pub const APPLIED_INDEX_KEY: &[u8] = b"\x00kv9\x00applied_index";

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
        MemStateMachine::with_engine(Arc::new(MemEngine::new()))
            .expect("a fresh MemEngine has no watermark to corrupt")
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
    ///
    /// Recovers the durably applied watermark from the engine: data and
    /// watermark are written in ONE atomic batch (see [`Self::apply_command`]),
    /// so on a durable engine they are physically inseparable — a restarted
    /// node resumes from where its data actually is, instead of reporting 0
    /// over a full store (the "durable data, volatile watermark" mismatch).
    ///
    /// Construction is fallible: an engine read error or a malformed watermark
    /// value REFUSES to open — silently coercing either to "watermark 0" would
    /// re-apply the whole log over unknown state (guessing is worse than
    /// stopping). A missing key is genuinely fresh and starts at 0.
    pub fn with_engine(engine: Arc<E>) -> Result<Self> {
        let applied = match engine.get(ColumnFamily::Default, APPLIED_INDEX_KEY)? {
            None => 0,
            Some(v) => {
                let bytes: [u8; 8] = v.try_into().map_err(|v: Vec<u8>| {
                    kv9_common::Error::Engine(format!(
                        "corrupt applied watermark: {} bytes (want 8)",
                        v.len()
                    ))
                })?;
                u64::from_be_bytes(bytes)
            }
        };
        Ok(MemStateMachine {
            engine,
            applied: LogIndex(applied),
        })
    }

    /// The backing engine — the KV the `meta` catalog reads/writes (ROADMAP Phase 1).
    pub fn engine(&self) -> &Arc<E> {
        &self.engine
    }

    /// Apply an already-decoded command at `index` (used by the propose→apply path and
    /// by tests that construct commands directly, avoiding the entry codec stub).
    ///
    /// Idempotent under redelivery: an entry at or below the applied watermark
    /// is skipped — after a restart, log replay fast-forwards past everything
    /// the engine already holds, and a future NON-idempotent command stays
    /// correct instead of silently double-applying. The watermark rides in the
    /// SAME atomic batch as the data (cross-CF batch atomicity is the engine's
    /// contract), so the two cannot diverge on disk.
    pub fn apply_command(&mut self, index: LogIndex, cmd: &Command) -> Result<ApplyResult> {
        if index <= self.applied {
            return Ok(ApplyResult::write_ok(index));
        }
        // EVERY applied entry advances the durable watermark — including
        // commands with no data mutations (Noop/ConfChange). Advancing those
        // only in memory would regress the watermark on restart and re-deliver
        // entries the group considers applied; correctness would again rest on
        // the all-commands-are-idempotent coincidence this change removes.
        let mut batch = cmd.to_write_batch();
        batch.put(
            ColumnFamily::Default,
            APPLIED_INDEX_KEY.to_vec(),
            index.0.to_be_bytes().to_vec(),
        );
        self.engine.write(batch)?;
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
        match entry.kind {
            // Barriers and conf changes never reach the state machine; the
            // production driver routes them (conf → apply_conf_change) and
            // reports applied progress separately. This helper only feeds
            // command payloads.
            crate::EntryKind::Noop
            | crate::EntryKind::ConfChangeV1
            | crate::EntryKind::ConfChangeV2 => {}
            crate::EntryKind::Command => out.push(sm.apply(entry)?),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RaftGroup, SingleNodeRaft};
    use kv9_common::{NodeId, RegionId};

    /// The applied watermark rides in the same batch as the data: a state
    /// machine re-created over the SAME engine resumes at the durable
    /// watermark instead of 0 (the durable-data/volatile-watermark mismatch).
    #[test]
    fn applied_watermark_recovers_with_the_engine() {
        let engine = Arc::new(MemEngine::new());
        let mut sm = MemStateMachine::with_engine(Arc::clone(&engine)).unwrap();
        let cmd = Command::Put {
            cf: 0,
            key: b"k".to_vec(),
            value: b"v1".to_vec(),
        };
        sm.apply_command(LogIndex(3), &cmd).unwrap();
        drop(sm);

        let sm2 = MemStateMachine::with_engine(Arc::clone(&engine)).unwrap();
        assert_eq!(sm2.applied_index(), LogIndex(3));
        // Control (sensitivity): a fresh engine reports 0 — recovery reads
        // real state, not a constant.
        let fresh = MemStateMachine::with_engine(Arc::new(MemEngine::new())).unwrap();
        assert_eq!(fresh.applied_index(), LogIndex(0));
    }

    /// A corrupt watermark value refuses to open (typed error) — never
    /// silently coerces to 0 and replays the log over unknown state.
    #[test]
    fn corrupt_watermark_refuses_to_open() {
        let engine = Arc::new(MemEngine::new());
        let mut batch = kv9_engine::WriteBatch::new();
        batch.put(
            ColumnFamily::Default,
            APPLIED_INDEX_KEY.to_vec(),
            vec![1, 2, 3], // wrong width
        );
        engine.write(batch).unwrap();
        assert!(MemStateMachine::with_engine(Arc::clone(&engine)).is_err());
        // Control: a valid 8-byte watermark opens fine.
        let mut batch = kv9_engine::WriteBatch::new();
        batch.put(
            ColumnFamily::Default,
            APPLIED_INDEX_KEY.to_vec(),
            9u64.to_be_bytes().to_vec(),
        );
        engine.write(batch).unwrap();
        assert_eq!(
            MemStateMachine::with_engine(engine)
                .unwrap()
                .applied_index(),
            LogIndex(9)
        );
    }

    /// Commands with NO data mutations (Noop) must still advance the durable
    /// watermark — otherwise a restart regresses it and re-delivers entries
    /// the group considers applied.
    #[test]
    fn empty_batch_commands_persist_the_watermark() {
        let engine = Arc::new(MemEngine::new());
        let mut sm = MemStateMachine::with_engine(Arc::clone(&engine)).unwrap();
        sm.apply_command(
            LogIndex(1),
            &Command::Put {
                cf: 0,
                key: b"k".to_vec(),
                value: b"v".to_vec(),
            },
        )
        .unwrap();
        sm.apply_command(LogIndex(2), &Command::Noop).unwrap();
        drop(sm);
        // Restart: the watermark reflects the Noop, not just the last data write.
        let sm2 = MemStateMachine::with_engine(engine).unwrap();
        assert_eq!(sm2.applied_index(), LogIndex(2));
    }

    /// Redelivery at or below the watermark is skipped — replay after restart
    /// cannot double-apply. Sensitivity: the skipped command carries a
    /// DIFFERENT value; if it were re-applied the assertion would see it.
    #[test]
    fn replayed_entries_below_watermark_are_skipped() {
        let engine = Arc::new(MemEngine::new());
        let mut sm = MemStateMachine::with_engine(Arc::clone(&engine)).unwrap();
        sm.apply_command(
            LogIndex(5),
            &Command::Put {
                cf: 0,
                key: b"k".to_vec(),
                value: b"original".to_vec(),
            },
        )
        .unwrap();

        // Restarted state machine over the same engine replays the log; a
        // conflicting rewrite of index 5 must be ignored.
        let mut sm2 = MemStateMachine::with_engine(Arc::clone(&engine)).unwrap();
        sm2.apply_command(
            LogIndex(5),
            &Command::Put {
                cf: 0,
                key: b"k".to_vec(),
                value: b"DOUBLE-APPLIED".to_vec(),
            },
        )
        .unwrap();
        assert_eq!(
            sm2.get(ColumnFamily::Default, b"k").unwrap(),
            Some(b"original".to_vec()),
            "entries at/below the watermark must not re-apply"
        );
        // …while a NEW index applies normally (a watermark, not a wall).
        sm2.apply_command(
            LogIndex(6),
            &Command::Put {
                cf: 0,
                key: b"k".to_vec(),
                value: b"next".to_vec(),
            },
        )
        .unwrap();
        assert_eq!(
            sm2.get(ColumnFamily::Default, b"k").unwrap(),
            Some(b"next".to_vec())
        );
    }

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
