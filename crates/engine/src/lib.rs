//! # kv9-engine
//!
//! Storage engine abstraction and the v0 in-memory implementation (DESIGN §6.2, §8.1).
//!
//! `Engine` is a trait; a region owns a logical LSM keyed within its range. v0 ships
//! [`MemEngine`]; a real `LsmEngine` plugs in behind the same trait later (DESIGN §12).

pub mod cf;
pub mod mem;
pub mod write_batch;

pub use cf::ColumnFamily;
pub use mem::MemEngine;
pub use write_batch::{Mutation, WriteBatch};

use kv9_common::{Result, UserKey, Value};

/// One entry produced by a range scan: `(key, value)`.
pub type ScanEntry = (UserKey, Value);

/// The storage engine trait (DESIGN §6.2).
///
/// Keys and values here are the *physical* (prefix-encoded) keys within a region's
/// range. Method bodies in the skeleton may return typed errors or be unimplemented;
/// the trait shape is real and reflects the design.
///
/// Note on the write path (DESIGN §6.2): the raft log *is* the WAL for the memtable —
/// a committed raft entry is applied here via [`Engine::write`]. Flush semantics,
/// watermarks, and backpressure signalling live in the region/raft layers.
pub trait Engine: Send + Sync {
    /// Point read from a column family.
    fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Value>>;

    /// Apply a batch of mutations atomically (DESIGN §6.2).
    fn write(&self, batch: WriteBatch) -> Result<()>;

    /// Forward range scan `[start, end)` over a column family, bounded by `limit`.
    fn scan(
        &self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<ScanEntry>>;

    /// Delete a physical range `[start, end)` from a column family (DESIGN §8.2
    /// `RawDeleteRange`; also GC).
    fn delete_range(&self, cf: ColumnFamily, start: &[u8], end: &[u8]) -> Result<()>;

    /// A cheap checksum over a physical key range, used by the replica scrubber
    /// (DESIGN §6.3, continuous verification).
    fn checksum(&self, cf: ColumnFamily, start: &[u8], end: &[u8]) -> Result<u64>;
}
