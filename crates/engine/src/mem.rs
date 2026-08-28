//! In-memory `BTreeMap`-backed engine for the skeleton and tests (DESIGN §6.2).

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use kv9_common::{Result, Value};

use crate::cf::ColumnFamily;
use crate::write_batch::{Mutation, WriteBatch};
use crate::{Engine, ReadView, ScanEntry};

/// A single column family's storage: an ordered `BTreeMap`.
type CfMap = BTreeMap<Vec<u8>, Vec<u8>>;

/// All column families as one value.
///
/// The three CFs share a single lock rather than holding one each. That is what makes
/// [`Engine::write`] atomic *across* column families: with a lock per CF there is no way
/// to apply a multi-CF batch without exposing an intermediate state, and a Percolator
/// commit (`lock` → `write` plus `default`) is exactly such a batch.
#[derive(Debug, Default, Clone)]
struct State {
    default: CfMap,
    lock: CfMap,
    write: CfMap,
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

/// In-memory engine. One `BTreeMap` per column family behind a single `RwLock`
/// (DESIGN §6.2). Suitable for the v0 skeleton and unit tests; **not durable**.
///
/// The state is held in an [`Arc`] so [`Engine::snapshot`] is O(1) — it clones the
/// pointer, not the data. Writes use copy-on-write ([`Arc::make_mut`]): they copy only
/// while a snapshot is still alive, and are in-place once the last one is dropped. The
/// read-heavy paths that motivated this (routing lookups, catalog queries — each opening
/// a view per transaction) therefore stop paying O(size) per view.
#[derive(Debug, Default)]
pub struct MemEngine {
    state: RwLock<Arc<State>>,
}

impl MemEngine {
    pub fn new() -> Self {
        MemEngine::default()
    }

    /// The current state, as a cheap handle.
    fn read(&self) -> Arc<State> {
        Arc::clone(&self.state.read().expect("mem engine lock poisoned"))
    }
}

impl Engine for MemEngine {
    fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Value>> {
        Ok(self.read().get(cf, key))
    }

    fn write(&self, batch: WriteBatch) -> Result<()> {
        // One lock acquisition for the whole batch: readers observe either none of these
        // mutations or all of them, never a prefix. `make_mut` gives copy-on-write — it
        // copies once if any snapshot still references this state, and mutates in place
        // otherwise, so live snapshots keep the version they were taken at.
        let mut guard = self.state.write().expect("mem engine lock poisoned");
        let state = Arc::make_mut(&mut *guard);
        for m in batch.mutations() {
            match m {
                Mutation::Put { cf, key, value } => {
                    state.cf_mut(*cf).insert(key.clone(), value.clone());
                }
                Mutation::Delete { cf, key } => {
                    state.cf_mut(*cf).remove(key);
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
        let mut guard = self.state.write().expect("mem engine lock poisoned");
        let map = Arc::make_mut(&mut *guard).cf_mut(cf);
        let doomed: Vec<Vec<u8>> = map
            .range(start.to_vec()..end.to_vec())
            .map(|(k, _)| k.clone())
            .collect();
        for k in doomed {
            map.remove(&k);
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

    fn snapshot(&self) -> Result<Box<dyn ReadView + '_>> {
        // O(1): clone the Arc, not the maps. Writes copy-on-write around us, so this view
        // keeps the version it was taken at. A real engine hands back an equally cheap
        // handle (immutable SSTs + a pinned memtable) behind this same signature.
        Ok(Box::new(MemSnapshot {
            state: self.read(),
        }))
    }
}

/// A point-in-time view of a [`MemEngine`], produced by [`Engine::snapshot`].
#[derive(Debug)]
struct MemSnapshot {
    state: Arc<State>,
}

impl ReadView for MemSnapshot {
    fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Value>> {
        Ok(self.state.get(cf, key))
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

    /// Snapshots are O(1) handles, and stay stable while the engine is rewritten many
    /// times over. If `snapshot()` ever went back to deep-copying, this still passes —
    /// it guards the semantics; the cost is covered by the Arc sharing below.
    #[test]
    fn snapshot_is_cheap_and_stable_across_many_writes() {
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

    /// Copy-on-write: with no snapshot alive the engine mutates its state in place
    /// (no reallocation), and taking a snapshot shares that same allocation.
    #[test]
    fn writes_are_in_place_when_no_snapshot_is_alive() {
        let engine = engine_with(&[(b"k", b"v")]);

        let addr = |e: &MemEngine| Arc::as_ptr(&e.read()) as usize;
        let before = addr(&engine);

        let mut b = WriteBatch::new();
        b.put(ColumnFamily::Default, b"k2".to_vec(), b"v".to_vec());
        engine.write(b).unwrap();
        assert_eq!(
            addr(&engine),
            before,
            "with no live snapshot, a write should not copy the state"
        );

        // Hold a snapshot: the next write must copy, leaving the view's version intact.
        let view = engine.snapshot().unwrap();
        let mut b = WriteBatch::new();
        b.put(ColumnFamily::Default, b"k3".to_vec(), b"v".to_vec());
        engine.write(b).unwrap();
        assert_ne!(
            addr(&engine),
            before,
            "with a live snapshot, a write must copy-on-write"
        );
        assert_eq!(view.get(ColumnFamily::Default, b"k3").unwrap(), None);
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
}
