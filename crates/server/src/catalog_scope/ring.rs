//! The change ring a full node serves scoped nodes from (docs/META-REPLICA-MODES.md
//! §3b). It observes every metadata entry the state machine applies, in order, and
//! keeps a bounded tail of them: the catalog ops of each `CatalogTxn`, an empty
//! record for any other command, so positions stay contiguous. Its lock is held
//! across the apply write, which lets [`ChangeRing::frozen`] read the engine and the
//! ring at one applied position.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use kv9_common::{AppliedPosition, Result};
use kv9_raft::{ApplyObserver, ApplyResult, Command, KvOp};

/// Entries kept by default; `KV9_SCOPE_RING_ENTRIES` changes it.
pub const DEFAULT_RING_ENTRIES: usize = 8192;
/// Bytes of ops kept by default; `KV9_SCOPE_RING_BYTES` changes it.
pub const DEFAULT_RING_BYTES: usize = 64 << 20;

#[derive(Clone, Debug, PartialEq)]
pub struct RingEntry {
    pub at: AppliedPosition,
    pub ops: Arc<Vec<KvOp>>,
}

#[derive(Default)]
struct RingState {
    /// The position before the first entry; `None` until the engine's position is
    /// known (a fresh node that applied nothing yet starts at `(0, 0)`).
    base: Option<AppliedPosition>,
    last: Option<AppliedPosition>,
    entries: VecDeque<RingEntry>,
    bytes: usize,
}

/// Why a reader cannot continue from its position in the ring.
#[derive(Debug, PartialEq, Eq)]
pub enum Behind {
    /// The position is older than the ring (or not one this log applied): snapshot.
    Snapshot,
}

pub struct ChangeRing {
    state: Mutex<RingState>,
    changed: Condvar,
    max_entries: usize,
    max_bytes: usize,
}

fn op_bytes(op: &KvOp) -> usize {
    match op {
        KvOp::Put { key, value, .. } => key.len() + value.len() + 16,
        KvOp::Delete { key, .. } => key.len() + 16,
    }
}

impl ChangeRing {
    pub fn new(start: Option<AppliedPosition>, max_entries: usize, max_bytes: usize) -> Arc<Self> {
        Arc::new(ChangeRing {
            state: Mutex::new(RingState {
                base: start,
                last: start,
                ..RingState::default()
            }),
            changed: Condvar::new(),
            max_entries: max_entries.max(1),
            max_bytes: max_bytes.max(1),
        })
    }

    /// From the environment (`KV9_SCOPE_RING_ENTRIES`, `KV9_SCOPE_RING_BYTES`).
    pub fn from_env(start: Option<AppliedPosition>) -> Arc<Self> {
        let read = |name: &str, default: usize| {
            std::env::var(name)
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .filter(|v| *v > 0)
                .unwrap_or(default)
        };
        Self::new(
            start,
            read("KV9_SCOPE_RING_ENTRIES", DEFAULT_RING_ENTRIES),
            read("KV9_SCOPE_RING_BYTES", DEFAULT_RING_BYTES),
        )
    }

    /// Wait up to `timeout` for the ring to move past `from`; returns the last position.
    pub fn wait_past(&self, from: Option<AppliedPosition>, timeout: Duration) -> Option<AppliedPosition> {
        let state = self.state.lock().expect("change ring poisoned");
        let (state, _) = self
            .changed
            .wait_timeout_while(state, timeout, |s| s.last == from)
            .expect("change ring poisoned");
        state.last
    }

    pub fn last(&self) -> Option<AppliedPosition> {
        self.state.lock().expect("change ring poisoned").last
    }

    /// Run `read` with no apply in progress, handing it the position the engine is
    /// at. `None` when the ring does not know its position yet.
    pub fn frozen<R>(&self, read: impl FnOnce(AppliedPosition) -> R) -> Option<R> {
        let state = self.state.lock().expect("change ring poisoned");
        state.last.map(read)
    }

    /// The entries after `from`, at most `limit` of them.
    pub fn after(&self, from: AppliedPosition, limit: usize) -> std::result::Result<Vec<RingEntry>, Behind> {
        let state = self.state.lock().expect("change ring poisoned");
        let (Some(base), Some(last)) = (state.base, state.last) else {
            return Err(Behind::Snapshot);
        };
        if from == last {
            return Ok(Vec::new());
        }
        if from.index > last.index {
            return Err(Behind::Snapshot);
        }
        let start = if from == base {
            0
        } else {
            let Some(offset) = from.index.checked_sub(base.index + 1) else {
                return Err(Behind::Snapshot);
            };
            match state.entries.get(offset as usize) {
                Some(entry) if entry.at == from => offset as usize + 1,
                _ => return Err(Behind::Snapshot),
            }
        };
        Ok(state.entries.iter().skip(start).take(limit.max(1)).cloned().collect())
    }

    fn record(state: &mut RingState, at: AppliedPosition, cmd: &Command, max_entries: usize, max_bytes: usize) {
        if state.last.is_some_and(|last| at.index <= last.index) {
            return; // a replayed entry the engine already held
        }
        let contiguous = state.last.is_some_and(|last| at.index == last.index + 1);
        if !contiguous {
            // A gap (snapshot install, or the first entry of a ring that did not know
            // its start): nothing before `at` can be served from here.
            state.entries.clear();
            state.bytes = 0;
            state.base = Some(at);
            state.last = Some(at);
            return;
        }
        let ops = match cmd {
            Command::CatalogTxn { ops } => ops.clone(),
            _ => Vec::new(),
        };
        state.bytes += ops.iter().map(op_bytes).sum::<usize>() + 32;
        state.entries.push_back(RingEntry { at, ops: Arc::new(ops) });
        state.last = Some(at);
        while state.entries.len() > max_entries || (state.bytes > max_bytes && state.entries.len() > 1) {
            let Some(front) = state.entries.pop_front() else { break };
            state.bytes = state
                .bytes
                .saturating_sub(front.ops.iter().map(op_bytes).sum::<usize>() + 32);
            state.base = Some(front.at);
        }
    }
}

impl ApplyObserver for ChangeRing {
    fn apply(
        &self,
        at: AppliedPosition,
        cmd: &Command,
        write: &mut dyn FnMut() -> Result<ApplyResult>,
    ) -> Result<ApplyResult> {
        let result = {
            let mut state = self.state.lock().expect("change ring poisoned");
            let result = write()?;
            Self::record(&mut state, at, cmd, self.max_entries, self.max_bytes);
            result
        };
        self.changed.notify_all();
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(index: u64) -> AppliedPosition {
        AppliedPosition { term: 1, index }
    }
    fn put(key: &[u8]) -> Command {
        Command::CatalogTxn {
            ops: vec![KvOp::Put { cf: 0, key: key.to_vec(), value: vec![1] }],
        }
    }
    fn feed(ring: &ChangeRing, index: u64, cmd: Command) {
        ring.apply(at(index), &cmd, &mut || Ok(ApplyResult::write_ok(kv9_raft::LogIndex(index))))
            .unwrap();
    }

    #[test]
    fn readers_continue_from_any_position_in_the_ring_and_snapshot_otherwise() {
        let ring = ChangeRing::new(Some(at(10)), 3, 1 << 20);
        for i in 11..=14 {
            feed(&ring, i, put(&[i as u8]));
        }
        // Capacity 3: base moved to 11, entries 12..=14.
        assert_eq!(ring.after(at(10), 10), Err(Behind::Snapshot));
        assert_eq!(ring.after(at(11), 10).unwrap().len(), 3);
        assert_eq!(ring.after(at(13), 10).unwrap().iter().map(|e| e.at.index).collect::<Vec<_>>(), vec![14]);
        assert!(ring.after(at(14), 10).unwrap().is_empty());
        assert_eq!(ring.after(at(15), 10), Err(Behind::Snapshot));
        // A different term at a held index is not this log's position.
        assert_eq!(ring.after(AppliedPosition { term: 9, index: 12 }, 10), Err(Behind::Snapshot));
    }

    #[test]
    fn a_gap_resets_the_ring_and_a_replay_is_ignored() {
        let ring = ChangeRing::new(Some(at(5)), 100, 1 << 20);
        feed(&ring, 6, put(b"a"));
        feed(&ring, 6, put(b"b"));
        assert_eq!(ring.after(at(5), 10).unwrap().len(), 1);
        feed(&ring, 20, Command::Noop);
        assert_eq!(ring.after(at(6), 10), Err(Behind::Snapshot));
        assert!(ring.after(at(20), 10).unwrap().is_empty());
        feed(&ring, 21, Command::Noop);
        let tail = ring.after(at(20), 10).unwrap();
        assert_eq!(tail.len(), 1);
        assert!(tail[0].ops.is_empty());
    }

    #[test]
    fn a_failed_write_records_nothing() {
        let ring = ChangeRing::new(Some(at(1)), 100, 1 << 20);
        let err = ring.apply(at(2), &put(b"x"), &mut || Err(kv9_common::Error::Raft("disk".into())));
        assert!(err.is_err());
        assert_eq!(ring.last(), Some(at(1)));
    }
}
