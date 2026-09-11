//! In-memory persistent-map engine for the skeleton and tests (DESIGN §6.2).

use std::sync::RwLock;

use kv9_common::{AppliedPosition, Error, Result, Value};
use rpds::RedBlackTreeMapSync;

use crate::cf::ColumnFamily;
use crate::replicated::{DurableAppliedPosition, ReplicatedEngine};
use crate::write_batch::{Mutation, WriteBatch};
use crate::{Durability, Engine, ReadView, ScanEntry};

/// A single column family's storage: a **persistent** ordered map.
///
/// Persistent (structurally shared) rather than a plain `BTreeMap` so that cloning is
/// O(1) and a clone is unaffected by later mutations. That is what makes
/// [`Engine::snapshot`] free and keeps every open [`ReadView`] pinned to its own version
/// without copying anything.
type CfMap = RedBlackTreeMapSync<Vec<u8>, Vec<u8>>;

/// All column families as one value.
///
/// The three CFs share a single lock rather than holding one each. That is what makes
/// [`Engine::write`] atomic *across* column families: with a lock per CF there is no way
/// to apply a multi-CF batch without exposing an intermediate state, and a Percolator
/// commit (`lock` → `write` plus `default`) is exactly such a batch.
///
/// Cloning a `State` is O(1) — it clones three persistent maps, each of which shares its
/// structure with the original.
#[derive(Debug, Default, Clone)]
struct State {
    default: CfMap,
    lock: CfMap,
    write: CfMap,
    /// The replicated position of the last [`ReplicatedEngine::write_applied`], if any.
    ///
    /// **In this struct, and therefore under this lock, on purpose.** The position and the
    /// data it describes must move together or not at all: a separate field behind a
    /// separate lock would let a reader observe the batch without the position, or the
    /// position without the batch, which is the same crash window that makes a two-call
    /// write-then-record API unusable.
    ///
    /// It never leaves this engine as a number. See
    /// [`MemEngine::applied_position`](ReplicatedEngine::applied_position).
    applied: Option<AppliedPosition>,
    data_revision: u64,
}

impl State {
    fn cf(&self, cf: ColumnFamily) -> &CfMap {
        match cf {
            ColumnFamily::Default => &self.default,
            ColumnFamily::Lock => &self.lock,
            ColumnFamily::Write => &self.write,
        }
    }

    fn cf_mut(&mut self, cf: ColumnFamily) -> &mut CfMap {
        match cf {
            ColumnFamily::Default => &mut self.default,
            ColumnFamily::Lock => &mut self.lock,
            ColumnFamily::Write => &mut self.write,
        }
    }

    fn get(&self, cf: ColumnFamily, key: &[u8]) -> Option<Value> {
        self.cf(cf).get(key).cloned()
    }

    fn scan(&self, cf: ColumnFamily, start: &[u8], end: &[u8], limit: usize) -> Vec<ScanEntry> {
        self.cf(cf)
            .range(start.to_vec()..end.to_vec())
            .take(limit)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    fn seek_le(&self, cf: ColumnFamily, target: &[u8]) -> Option<ScanEntry> {
        self.cf(cf)
            .range(..=target.to_vec())
            .next_back()
            .map(|(k, v)| (k.clone(), v.clone()))
    }
}

/// In-memory engine. One persistent ordered map per column family behind a single
/// `RwLock` (DESIGN §6.2). Suitable for the v0 skeleton and unit tests; **not durable**.
///
/// Persistent maps make [`Engine::snapshot`] O(1): each view retains its map roots.
/// Later writes preserve those versions through structural sharing and may copy shared
/// tree nodes along an updated path. Snapshot capture avoids copying the whole dataset;
/// it does not make subsequent writes free.
#[derive(Debug, Default)]
pub struct MemEngine {
    /// LOCK ORDER: this is the only lock in `crates/engine`, apart from
    /// [`WalEngine`](crate::WalEngine)'s log mutex, and the single legal nesting is
    /// `wal → state` (`persist.rs`: `write` holds the log across `index.write`). There is
    /// no reverse path, and there must never be one.
    ///
    /// What keeps that true is [`MemEngine::read`] returning an *owned* snapshot rather
    /// than a guard — see the note there before changing either.
    state: RwLock<State>,
}

impl MemEngine {
    pub fn new() -> Self {
        MemEngine::default()
    }

    /// Try to capture an owned, memory-only view without waiting for a writer.
    /// `None` means contention, never absence of data. No state guard escapes.
    /// Point lookups on the returned view perform no I/O or lock acquisition.
    pub fn try_resident_snapshot(&self) -> Option<Box<dyn ReadView>> {
        let state = match self.state.try_read() {
            Ok(state) => state.clone(),
            Err(std::sync::TryLockError::WouldBlock) => return None,
            Err(std::sync::TryLockError::Poisoned(_)) => panic!("mem engine lock poisoned"),
        };
        Some(Box::new(MemSnapshot { state }))
    }

    /// A snapshot of the current state. O(1): the maps share their structure.
    ///
    /// **Must return an owned `State`, never a `RwLockReadGuard`.** Returning a guard
    /// would hold this lock across the caller's nested calls, which is precisely the
    /// shape that deadlocked the raft driver (`status()` held one lock while acquiring
    /// another; see `driver.rs`'s lock-order note). Because the maps are persistent and
    /// structurally shared, the clone is O(1) — the guard would buy nothing and cost the
    /// immunity.
    ///
    /// This is a safety property wearing the costume of an avoidable clone, so it is
    /// pinned mechanically below rather than left to a reviewer's memory.
    fn read(&self) -> State {
        self.state.read().expect("mem engine lock poisoned").clone()
    }
}

/// Compile-time guard for the invariant documented on [`MemEngine::read`].
///
/// A `RwLockReadGuard<'_, State>` borrows `&self` and so cannot satisfy `T: 'static`;
/// an owned `State` can. If someone "optimises" `read` into returning a guard, this stops
/// compiling — and the error lands next to the comment explaining why. Zero runtime cost,
/// and it does not depend on anyone remembering to run a particular test.
///
/// Credit: suggested by Cindy while reviewing the driver lock-order fix.
#[allow(dead_code)]
fn _assert_read_returns_owned_snapshot(engine: &MemEngine) {
    fn requires_owned<T: 'static>(_: T) {}
    requires_owned(engine.read());
}

impl Engine for MemEngine {
    fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Value>> {
        Ok(self.read().get(cf, key))
    }

    fn write(&self, batch: WriteBatch) -> Result<()> {
        // One lock acquisition for the whole batch: readers observe either none of these
        // mutations or all of them, never a prefix. Open snapshots are unaffected — the
        // maps are persistent, so they still hold the version they were cloned at.
        let mut state = self.state.write().expect("mem engine lock poisoned");
        // The caller transfers the batch after any required WAL persistence. Move its
        // owned buffers into the index; cloning them here adds no snapshot protection.
        for m in batch.mutations {
            match m {
                Mutation::Put { cf, key, value } => {
                    state.cf_mut(cf).insert_mut(key, value);
                }
                Mutation::Delete { cf, key } => {
                    state.cf_mut(cf).remove_mut(&key);
                }
            }
        }
        Ok(())
    }

    fn scan(
        &self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<ScanEntry>> {
        Ok(self.read().scan(cf, start, end, limit))
    }

    fn delete_range(&self, cf: ColumnFamily, start: &[u8], end: &[u8]) -> Result<()> {
        let mut state = self.state.write().expect("mem engine lock poisoned");
        let map = state.cf_mut(cf);
        let doomed: Vec<Vec<u8>> = map
            .range(start.to_vec()..end.to_vec())
            .map(|(k, _)| k.clone())
            .collect();
        for k in doomed {
            map.remove_mut(&k);
        }
        Ok(())
    }

    fn checksum(&self, cf: ColumnFamily, start: &[u8], end: &[u8]) -> Result<u64> {
        // Simple FNV-1a style rolling hash over the range for the scrubber stub.
        let state = self.read();
        let mut h: u64 = 0xcbf29ce484222325;
        for (k, v) in state.cf(cf).range(start.to_vec()..end.to_vec()) {
            for b in k.iter().chain(v.iter()) {
                h ^= u64::from(*b);
                h = h.wrapping_mul(0x100000001b3);
            }
        }
        Ok(h)
    }

    fn durability(&self) -> Durability {
        // Nothing here is written anywhere; saying so is what stops a caller from
        // truncating a raft log against data that evaporates on restart.
        Durability::Volatile
    }

    fn snapshot(&self) -> Result<Box<dyn ReadView + '_>> {
        // O(1): share the map roots. Later updates may copy shared tree nodes while
        // preserving this view; capturing it does not copy all keys and values.
        Ok(Box::new(MemSnapshot { state: self.read() }))
    }
}

impl ReplicatedEngine for MemEngine {
    /// Apply the batch and record its position under **one** acquisition of the state lock.
    ///
    /// That single acquisition is the whole implementation of the atomicity this trait
    /// requires: there is no instant at which a reader, or a later `write_applied`, can
    /// observe the mutations without the position or the position without the mutations.
    fn write_applied(&self, batch: WriteBatch, at: AppliedPosition) -> Result<()> {
        let mut state = self.state.write().expect("mem engine lock poisoned");

        // Checked BEFORE anything is mutated, so a refused position leaves no partial batch
        // behind — a caller that retries after this error finds the state it had.
        //
        // Index only; no ordering is built on term. Ruled for task #13 (acceptance 2c):
        // term is not monotonic across an election the way index is, so ordering on it
        // would refuse legitimate sequences. A legal GAP is fine — only repeating or going
        // backwards is refused.
        if let Some(previous) = state.applied {
            if at.index <= previous.index {
                return Err(Error::Engine(format!(
                    "engine: applied position must advance; last applied index {}, \
                     refused index {}",
                    previous.index, at.index
                )));
            }
        }

        // Classify before consuming the buffers. Keep the existing short-circuit
        // predicate: any non-manifest mutation advances the revision exactly once,
        // including an absent delete or a batch whose net data change is empty.
        let changes_data = batch.mutations().iter().any(|m| {
            let key = match m {
                Mutation::Put { key, .. } | Mutation::Delete { key, .. } => key,
            };
            !key.starts_with(b"\x00kv9\x00manifest_")
        });
        for m in batch.mutations {
            match m {
                Mutation::Put { cf, key, value } => {
                    state.cf_mut(cf).insert_mut(key, value);
                }
                Mutation::Delete { cf, key } => {
                    state.cf_mut(cf).remove_mut(&key);
                }
            }
        }
        if changes_data {
            state.data_revision = state.data_revision.saturating_add(1);
        }
        state.applied = Some(at);
        Ok(())
    }

    /// Always [`DurableAppliedPosition::Volatile`] — never a number.
    ///
    /// This engine *has* a position, and within one process run it is perfectly good. That
    /// is exactly why this answers with a variant instead of that value: nothing here
    /// survives a restart, so a number returned through this method could be used to
    /// authorise truncating a raft log or reclaiming an object, and would then be wrong in
    /// the one direction that loses data.
    ///
    /// The in-memory value is reachable through
    /// [`MemEngine::volatile_applied_position`] — an *inherent* method, so a caller generic
    /// over `ReplicatedEngine` cannot reach it at all. Getting at it means naming this
    /// concrete type, which is a visible decision rather than a silent one.
    fn applied_position(&self) -> Result<DurableAppliedPosition> {
        Ok(DurableAppliedPosition::Volatile)
    }
}

impl MemEngine {
    pub(crate) fn freeze_parts(&self) -> (MemSnapshot, Option<AppliedPosition>) {
        let state = self.read();
        let position = state.applied;
        (MemSnapshot { state }, position)
    }
    pub fn data_revision(&self) -> u64 {
        self.state
            .read()
            .expect("mem engine lock poisoned")
            .data_revision
    }
}

impl MemEngine {
    /// The position last recorded by [`write_applied`](ReplicatedEngine::write_applied),
    /// for tests and diagnostics.
    ///
    /// Named `volatile_` because that is the whole caveat: true of this process, and
    /// meaningless after a restart. **Never a truncation or reclaim bound** — that question
    /// is [`applied_position`](ReplicatedEngine::applied_position)'s, and it refuses to
    /// answer with a number.
    pub fn volatile_applied_position(&self) -> Option<AppliedPosition> {
        self.state.read().expect("mem engine lock poisoned").applied
    }
}

/// A point-in-time view of a [`MemEngine`], produced by [`Engine::snapshot`].
#[derive(Debug)]
pub(crate) struct MemSnapshot {
    state: State,
}

impl MemSnapshot {
    pub(crate) fn iter_all(
        &self,
        cf: ColumnFamily,
    ) -> impl Iterator<Item = Result<ScanEntry>> + '_ {
        self.state
            .cf(cf)
            .iter()
            .map(|(k, v)| Ok((k.clone(), v.clone())))
    }
}

impl ReadView for MemSnapshot {
    fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Value>> {
        Ok(self.state.get(cf, key))
    }

    fn get_resident(&self, cf: ColumnFamily, key: &[u8]) -> Option<Option<&[u8]>> {
        Some(self.state.cf(cf).get(key).map(Vec::as_slice))
    }

    fn scan(
        &self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<ScanEntry>> {
        Ok(self.state.scan(cf, start, end, limit))
    }

    fn seek_le(&self, cf: ColumnFamily, target: &[u8]) -> Result<Option<ScanEntry>> {
        Ok(self.state.seek_le(cf, target))
    }

    fn iter<'a>(
        &'a self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
    ) -> Result<Box<dyn Iterator<Item = Result<ScanEntry>> + 'a>> {
        Ok(Box::new(
            self.state
                .cf(cf)
                .range(start.to_vec()..end.to_vec())
                .map(|(k, v)| Ok((k.clone(), v.clone()))),
        ))
    }

    fn iter_rev<'a>(
        &'a self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
    ) -> Result<Box<dyn Iterator<Item = Result<ScanEntry>> + 'a>> {
        Ok(Box::new(
            self.state
                .cf(cf)
                .range(start.to_vec()..end.to_vec())
                .rev()
                .map(|(k, v)| Ok((k.clone(), v.clone()))),
        ))
    }
}

#[cfg(test)]
mod resident_tests {
    use super::*;

    #[test]
    fn resident_snapshot_never_waits_for_a_writer_and_owns_its_version() {
        let engine = std::sync::Arc::new(MemEngine::new());
        let mut first = WriteBatch::new();
        first.put(ColumnFamily::Default, b"k".to_vec(), b"before".to_vec());
        first.put(ColumnFamily::Lock, b"k".to_vec(), b"before".to_vec());
        engine.write(first).unwrap();
        let held = engine.state.write().unwrap();
        let (sent, received) = std::sync::mpsc::channel();
        let reader = engine.clone();
        let worker = std::thread::spawn(move || {
            sent.send(reader.try_resident_snapshot().is_none()).unwrap();
        });
        let result = received.recv_timeout(std::time::Duration::from_secs(2));
        drop(held); // Always release before asserting, including faulty implementations.
        worker.join().unwrap();
        assert_eq!(
            result.ok(),
            Some(true),
            "resident snapshot waited for an index writer"
        );

        let view = engine
            .try_resident_snapshot()
            .expect("uncontended resident view");
        let mut second = WriteBatch::new();
        second.put(ColumnFamily::Default, b"k".to_vec(), b"after".to_vec());
        second.put(ColumnFamily::Lock, b"k".to_vec(), b"after".to_vec());
        engine.write(second).unwrap();
        for cf in [ColumnFamily::Default, ColumnFamily::Lock] {
            let borrowed: &dyn ReadView = view.as_ref();
            let wrapper: Box<dyn ReadView + '_> = Box::new(borrowed);
            assert_eq!(
                wrapper.get_resident(cf, b"k"),
                Some(Some(b"before".as_slice()))
            );
            assert_eq!(wrapper.get_resident(cf, b"missing"), Some(None));
            assert!(
                std::ptr::eq(
                    wrapper.get_resident(cf, b"k").unwrap().unwrap(),
                    view.get_resident(cf, b"k").unwrap().unwrap(),
                ),
                "borrowed wrapper copied the snapshot value"
            );
            assert_eq!(
                view.get(cf, b"k").unwrap().as_deref(),
                Some(b"before".as_slice()),
                "resident view changed after a later atomic batch"
            );
            assert_eq!(
                engine.get(cf, b"k").unwrap().as_deref(),
                Some(b"after".as_slice())
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine_with(entries: &[(&[u8], &[u8])]) -> MemEngine {
        let engine = MemEngine::new();
        let mut batch = WriteBatch::new();
        for (k, v) in entries {
            batch.put(ColumnFamily::Default, k.to_vec(), v.to_vec());
        }
        engine.write(batch).unwrap();
        engine
    }

    #[test]
    fn seek_le_finds_greatest_key_not_exceeding_target() {
        let engine = engine_with(&[(b"a", b"1"), (b"c", b"3"), (b"e", b"5")]);
        let view = engine.snapshot().unwrap();

        // Exact hit.
        assert_eq!(
            view.seek_le(ColumnFamily::Default, b"c").unwrap(),
            Some((b"c".to_vec(), b"3".to_vec()))
        );
        // Between keys: takes the predecessor, not the successor.
        assert_eq!(
            view.seek_le(ColumnFamily::Default, b"d").unwrap(),
            Some((b"c".to_vec(), b"3".to_vec()))
        );
        // Past the end: the last key.
        assert_eq!(
            view.seek_le(ColumnFamily::Default, b"z").unwrap(),
            Some((b"e".to_vec(), b"5".to_vec()))
        );
        // Before the first key: nothing.
        assert_eq!(view.seek_le(ColumnFamily::Default, b"A").unwrap(), None);
    }

    #[test]
    fn seek_le_is_per_column_family() {
        let engine = MemEngine::new();
        let mut batch = WriteBatch::new();
        batch.put(ColumnFamily::Default, b"a".to_vec(), b"d".to_vec());
        batch.put(ColumnFamily::Lock, b"b".to_vec(), b"l".to_vec());
        engine.write(batch).unwrap();
        let view = engine.snapshot().unwrap();

        assert_eq!(
            view.seek_le(ColumnFamily::Lock, b"z").unwrap(),
            Some((b"b".to_vec(), b"l".to_vec()))
        );
        // The `default` entry must not leak into the `lock` CF's answer.
        assert_eq!(
            view.seek_le(ColumnFamily::Default, b"z").unwrap(),
            Some((b"a".to_vec(), b"d".to_vec()))
        );
        assert_eq!(view.seek_le(ColumnFamily::Write, b"z").unwrap(), None);
    }

    #[test]
    fn snapshot_is_isolated_from_later_writes() {
        let engine = engine_with(&[(b"k", b"v1")]);
        let view = engine.snapshot().unwrap();

        let mut batch = WriteBatch::new();
        batch.put(ColumnFamily::Default, b"k".to_vec(), b"v2".to_vec());
        batch.put(ColumnFamily::Default, b"new".to_vec(), b"x".to_vec());
        engine.write(batch).unwrap();

        // The view still sees the state as of when it was taken.
        assert_eq!(
            view.get(ColumnFamily::Default, b"k").unwrap(),
            Some(b"v1".to_vec())
        );
        assert_eq!(view.get(ColumnFamily::Default, b"new").unwrap(), None);
        // ...while the engine itself has moved on.
        assert_eq!(
            engine.get(ColumnFamily::Default, b"k").unwrap(),
            Some(b"v2".to_vec())
        );
    }

    fn collect(it: Box<dyn Iterator<Item = Result<ScanEntry>> + '_>) -> Vec<ScanEntry> {
        it.map(|e| e.unwrap()).collect()
    }

    #[test]
    fn iter_is_ascending_and_half_open() {
        let engine = engine_with(&[(b"a", b"1"), (b"b", b"2"), (b"c", b"3")]);
        let view = engine.snapshot().unwrap();

        let got = collect(view.iter(ColumnFamily::Default, b"a", b"c").unwrap());
        assert_eq!(
            got,
            vec![
                (b"a".to_vec(), b"1".to_vec()),
                (b"b".to_vec(), b"2".to_vec())
            ]
        );
    }

    #[test]
    fn iter_rev_is_descending_and_half_open() {
        let engine = engine_with(&[(b"a", b"1"), (b"b", b"2"), (b"c", b"3")]);
        let view = engine.snapshot().unwrap();

        let got = collect(view.iter_rev(ColumnFamily::Default, b"a", b"c").unwrap());
        assert_eq!(
            got,
            vec![
                (b"b".to_vec(), b"2".to_vec()),
                (b"a".to_vec(), b"1".to_vec())
            ]
        );
    }

    /// The point of streaming: a caller may stop early, without the view having
    /// materialized the whole range first (DESIGN §13 principle 13).
    #[test]
    fn iter_can_stop_early_without_materializing_the_range() {
        let engine = MemEngine::new();
        let mut batch = WriteBatch::new();
        for i in 0..10_000u32 {
            batch.put(
                ColumnFamily::Default,
                format!("k{i:06}").into_bytes(),
                b"v".to_vec(),
            );
        }
        engine.write(batch).unwrap();
        let view = engine.snapshot().unwrap();

        let first_three: Vec<_> = view
            .iter(ColumnFamily::Default, b"k", b"l")
            .unwrap()
            .take(3)
            .map(|e| e.unwrap().0)
            .collect();
        assert_eq!(
            first_three,
            vec![
                b"k000000".to_vec(),
                b"k000001".to_vec(),
                b"k000002".to_vec()
            ]
        );
    }

    /// `end` is **exclusive in both directions**. Spelled out because getting it wrong in
    /// reverse is a routing bug, not a cosmetic one: "greatest key ≤ K" needs
    /// `end = successor(K)`, and passing `K` itself silently drops the exact-match case —
    /// a key landing precisely on a region's start key would route to the *previous*
    /// region.
    #[test]
    fn iter_rev_end_bound_is_exclusive() {
        let engine = engine_with(&[(b"r10", b"a"), (b"r20", b"b")]);
        let view = engine.snapshot().unwrap();

        // Exclusive: asking up to "r20" does NOT include "r20".
        let got = collect(view.iter_rev(ColumnFamily::Default, b"", b"r20").unwrap());
        assert_eq!(got, vec![(b"r10".to_vec(), b"a".to_vec())]);

        // To include an exact hit on the target, extend past it.
        let mut inclusive_end = b"r20".to_vec();
        inclusive_end.push(0);
        let got = collect(
            view.iter_rev(ColumnFamily::Default, b"", &inclusive_end)
                .unwrap(),
        );
        assert_eq!(got.first().unwrap().0, b"r20".to_vec());
    }

    /// The case `seek_le` alone cannot serve, and the reason `iter_rev` exists.
    ///
    /// A caller buffering its own writes asks for "greatest key ≤ target". The view's best
    /// candidate is one the caller has itself deleted, so it must be able to keep walking
    /// down to the next live one. `seek_le` yields a single entry with no way to continue.
    #[test]
    fn iter_rev_walks_past_a_callers_deleted_candidate() {
        let engine = engine_with(&[(b"r10", b"a"), (b"r20", b"b"), (b"r30", b"c")]);
        let view = engine.snapshot().unwrap();

        // The caller has buffered a delete of "r20" — it must not be routed to.
        let caller_deleted: &[&[u8]] = &[b"r20"];

        // seek_le alone hands back exactly the deleted row, and stops there.
        assert_eq!(
            view.seek_le(ColumnFamily::Default, b"r25").unwrap(),
            Some((b"r20".to_vec(), b"b".to_vec()))
        );

        // iter_rev lets the caller skip it and reach the real answer.
        let answer = view
            .iter_rev(ColumnFamily::Default, b"", b"r25")
            .unwrap()
            .map(|e| e.unwrap())
            .find(|(k, _)| !caller_deleted.contains(&k.as_slice()));
        assert_eq!(answer, Some((b"r10".to_vec(), b"a".to_vec())));
    }

    /// A snapshot stays pinned to its own version no matter how far the engine moves on.
    #[test]
    fn snapshot_is_stable_across_many_writes() {
        let engine = engine_with(&[(b"k", b"v0")]);
        let view = engine.snapshot().unwrap();

        for i in 1..1_000u32 {
            let mut b = WriteBatch::new();
            b.put(
                ColumnFamily::Default,
                b"k".to_vec(),
                format!("v{i}").into_bytes(),
            );
            engine.write(b).unwrap();
        }

        assert_eq!(
            view.get(ColumnFamily::Default, b"k").unwrap(),
            Some(b"v0".to_vec()),
            "the view must still show the version it was taken at"
        );
        assert_eq!(
            engine.get(ColumnFamily::Default, b"k").unwrap(),
            Some(b"v999".to_vec())
        );
    }

    /// Many snapshots may be alive at once, each pinned to a *different* version, and
    /// writes keep flowing regardless.
    ///
    /// This is the property a persistent map buys over copy-on-write: under COW every one
    /// of these live views would force the next write to copy the whole state, so the
    /// cost would grow with how many readers happen to be open. Here it does not.
    #[test]
    fn many_live_snapshots_each_pin_their_own_version() {
        let engine = MemEngine::new();
        let mut views = Vec::new();

        for i in 0..200u32 {
            let mut b = WriteBatch::new();
            b.put(
                ColumnFamily::Default,
                b"k".to_vec(),
                format!("v{i}").into_bytes(),
            );
            // Add a distinct key each round too, so the maps genuinely grow.
            b.put(
                ColumnFamily::Default,
                format!("extra{i}").into_bytes(),
                b"x".to_vec(),
            );
            engine.write(b).unwrap();
            views.push(engine.snapshot().unwrap());
        }

        // Every view still reports the value written in its own round, and sees exactly
        // the keys that existed then.
        for (i, view) in views.iter().enumerate() {
            assert_eq!(
                view.get(ColumnFamily::Default, b"k").unwrap(),
                Some(format!("v{i}").into_bytes()),
                "view {i} drifted off its version"
            );
            assert_eq!(
                view.get(ColumnFamily::Default, format!("extra{i}").as_bytes())
                    .unwrap(),
                Some(b"x".to_vec())
            );
            // A key added after this view was taken must not be visible to it.
            if i + 1 < views.len() {
                assert_eq!(
                    view.get(ColumnFamily::Default, format!("extra{}", i + 1).as_bytes())
                        .unwrap(),
                    None,
                    "view {i} saw a key written after it was taken"
                );
            }
        }
    }

    #[test]
    fn scan_is_bounded_and_half_open() {
        let engine = engine_with(&[(b"a", b"1"), (b"b", b"2"), (b"c", b"3")]);
        let view = engine.snapshot().unwrap();

        // `end` is exclusive.
        let got = view.scan(ColumnFamily::Default, b"a", b"c", 10).unwrap();
        assert_eq!(
            got,
            vec![
                (b"a".to_vec(), b"1".to_vec()),
                (b"b".to_vec(), b"2".to_vec())
            ]
        );

        // `limit` truncates.
        let got = view.scan(ColumnFamily::Default, b"a", b"z", 1).unwrap();
        assert_eq!(got, vec![(b"a".to_vec(), b"1".to_vec())]);
    }

    // -----------------------------------------------------------------------------------
    // ReplicatedEngine
    // -----------------------------------------------------------------------------------

    fn at(term: u64, index: u64) -> AppliedPosition {
        AppliedPosition { term, index }
    }

    fn one_put(key: &[u8], value: &[u8]) -> WriteBatch {
        let mut b = WriteBatch::new();
        b.put(ColumnFamily::Default, key.to_vec(), value.to_vec());
        b
    }

    #[test]
    fn write_applied_lands_the_data_and_the_position_together() {
        let engine = MemEngine::new();
        engine
            .write_applied(one_put(b"k", b"v"), at(7, 42))
            .unwrap();
        assert_eq!(
            engine.get(ColumnFamily::Default, b"k").unwrap().as_deref(),
            Some(&b"v"[..])
        );
        assert_eq!(engine.volatile_applied_position(), Some(at(7, 42)));
    }

    #[test]
    fn the_position_is_never_reported_as_a_number_through_the_trait() {
        // The load-bearing property. A MemEngine that has applied through index 42 still
        // answers `Volatile`, because that 42 does not survive a restart and would
        // otherwise be usable to authorise truncation.
        let engine = MemEngine::new();
        engine
            .write_applied(one_put(b"k", b"v"), at(7, 42))
            .unwrap();
        assert_eq!(
            engine.applied_position().unwrap(),
            DurableAppliedPosition::Volatile
        );
        // ...and the value IS there, so this is not passing merely because nothing applied.
        assert_eq!(engine.volatile_applied_position(), Some(at(7, 42)));
    }

    #[test]
    fn a_position_that_does_not_advance_is_refused_and_writes_nothing() {
        let engine = MemEngine::new();
        engine
            .write_applied(one_put(b"k", b"first"), at(1, 10))
            .unwrap();

        for backwards in [at(1, 10), at(2, 10), at(1, 9), at(9, 1)] {
            let err = engine
                .write_applied(one_put(b"k", b"second"), backwards)
                .expect_err("index must advance");
            assert!(
                format!("{err}").contains("must advance"),
                "unexpected error: {err}"
            );
        }

        // Refused before mutating: the value and the position are both untouched. Without
        // this half, an implementation that wrote the batch and *then* checked would pass
        // the assertions above while having corrupted the state.
        assert_eq!(
            engine.get(ColumnFamily::Default, b"k").unwrap().as_deref(),
            Some(&b"first"[..])
        );
        assert_eq!(engine.volatile_applied_position(), Some(at(1, 10)));
    }

    #[test]
    fn a_gap_in_indices_is_legal_and_term_does_not_order() {
        // Pin the generic engine's index-only ordering contract. This synthetic
        // sequence deliberately is not a valid Raft history; the runtime, rather
        // than the opaque engine, validates terms against committed Raft entries.
        let engine = MemEngine::new();
        engine
            .write_applied(one_put(b"a", b"1"), at(5, 10))
            .unwrap();
        engine
            .write_applied(one_put(b"b", b"2"), at(5, 40))
            .unwrap();
        engine
            .write_applied(one_put(b"c", b"3"), at(2, 41))
            .unwrap();
        assert_eq!(engine.volatile_applied_position(), Some(at(2, 41)));
    }

    #[test]
    fn plain_write_does_not_move_the_position() {
        // The named gap, pinned as behaviour rather than left to be discovered. `write` is
        // still reachable and deliberately does NOT touch the position; task #17 closes the
        // apply path with a capability trait exposing only `write_applied`.
        let engine = MemEngine::new();
        engine
            .write_applied(one_put(b"k", b"v"), at(1, 10))
            .unwrap();
        engine.write(one_put(b"k2", b"v2")).unwrap();
        assert_eq!(
            engine.get(ColumnFamily::Default, b"k2").unwrap().as_deref(),
            Some(&b"v2"[..]),
            "the data must land"
        );
        assert_eq!(
            engine.volatile_applied_position(),
            Some(at(1, 10)),
            "but the position must not move"
        );
    }

    #[test]
    fn a_fresh_engine_has_no_position_and_accepts_any_first_index() {
        let engine = MemEngine::new();
        assert_eq!(engine.volatile_applied_position(), None);
        assert_eq!(
            engine.applied_position().unwrap(),
            DurableAppliedPosition::Volatile
        );
        // No lower bound on the first index: a restarted volatile engine legitimately
        // starts wherever the log it is fed starts.
        engine
            .write_applied(one_put(b"k", b"v"), at(3, 900))
            .unwrap();
        assert_eq!(engine.volatile_applied_position(), Some(at(3, 900)));
    }

    #[test]
    fn an_empty_batch_still_advances_the_position() {
        // A fence-rejected command applies no mutations but must still move the watermark,
        // or the position falls behind the log and reclaim stalls (task #17 item 3).
        let engine = MemEngine::new();
        engine.write_applied(WriteBatch::new(), at(1, 10)).unwrap();
        assert_eq!(engine.volatile_applied_position(), Some(at(1, 10)));
    }
}
