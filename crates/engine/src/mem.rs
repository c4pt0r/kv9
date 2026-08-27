//! In-memory `BTreeMap`-backed engine for the skeleton and tests (DESIGN §6.2).

use std::collections::BTreeMap;
use std::sync::RwLock;

use kv9_common::{Result, Value};

use crate::cf::ColumnFamily;
use crate::write_batch::{Mutation, WriteBatch};
use crate::{Engine, ScanEntry};

/// A single column family's storage: an ordered `BTreeMap`.
type CfMap = BTreeMap<Vec<u8>, Vec<u8>>;

/// In-memory engine. One `BTreeMap` per column family, guarded by an `RwLock`
/// (DESIGN §6.2). Suitable for the v0 skeleton and unit tests; not durable.
#[derive(Debug, Default)]
pub struct MemEngine {
    default: RwLock<CfMap>,
    lock: RwLock<CfMap>,
    write: RwLock<CfMap>,
}

impl MemEngine {
    pub fn new() -> Self {
        MemEngine::default()
    }

    fn cf(&self, cf: ColumnFamily) -> &RwLock<CfMap> {
        match cf {
            ColumnFamily::Default => &self.default,
            ColumnFamily::Lock => &self.lock,
            ColumnFamily::Write => &self.write,
        }
    }
}

impl Engine for MemEngine {
    fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Value>> {
        let map = self.cf(cf).read().expect("mem engine lock poisoned");
        Ok(map.get(key).cloned())
    }

    fn write(&self, batch: WriteBatch) -> Result<()> {
        for m in batch.mutations() {
            match m {
                Mutation::Put { cf, key, value } => {
                    let mut map = self.cf(*cf).write().expect("mem engine lock poisoned");
                    map.insert(key.clone(), value.clone());
                }
                Mutation::Delete { cf, key } => {
                    let mut map = self.cf(*cf).write().expect("mem engine lock poisoned");
                    map.remove(key);
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
        let map = self.cf(cf).read().expect("mem engine lock poisoned");
        let out = map
            .range(start.to_vec()..end.to_vec())
            .take(limit)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        Ok(out)
    }

    fn delete_range(&self, cf: ColumnFamily, start: &[u8], end: &[u8]) -> Result<()> {
        let mut map = self.cf(cf).write().expect("mem engine lock poisoned");
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
        let map = self.cf(cf).read().expect("mem engine lock poisoned");
        let mut h: u64 = 0xcbf29ce484222325;
        for (k, v) in map.range(start.to_vec()..end.to_vec()) {
            for b in k.iter().chain(v.iter()) {
                h ^= u64::from(*b);
                h = h.wrapping_mul(0x100000001b3);
            }
        }
        Ok(h)
    }
}
