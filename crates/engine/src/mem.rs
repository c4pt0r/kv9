//! In-memory `BTreeMap`-backed engine for the skeleton and tests (DESIGN §6.2).

use std::collections::BTreeMap;
use std::sync::{RwLock, RwLockReadGuard};

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
#[derive(Debug, Default)]
pub struct MemEngine {
    state: RwLock<State>,
}

impl MemEngine {
    pub fn new() -> Self {
        MemEngine::default()
    }

    fn read(&self) -> RwLockReadGuard<'_, State> {
        self.state.read().expect("mem engine lock poisoned")
    }
}

impl Engine for MemEngine {
    fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Value>> {
        Ok(self.read().get(cf, key))
    }

    fn write(&self, batch: WriteBatch) -> Result<()> {
        // One lock acquisition for the whole batch: readers observe either none of these
        // mutations or all of them, never a prefix.
        let mut state = self.state.write().expect("mem engine lock poisoned");
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
        let mut state = self.state.write().expect("mem engine lock poisoned");
        let map = state.cf_mut(cf);
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
        // v0: clone the whole state under one read lock. That is O(size) and fine for the
        // skeleton and tests; the point here is the *semantics*. A real engine hands back
        // a cheap handle (immutable SSTs + a pinned memtable), and swapping this for a
        // persistent map later keeps the same signature.
        let snapshot = self.read().clone();
        Ok(Box::new(MemSnapshot { state: snapshot }))
    }
}

/// A point-in-time view of a [`MemEngine`], produced by [`Engine::snapshot`].
#[derive(Debug)]
struct MemSnapshot {
    state: State,
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
