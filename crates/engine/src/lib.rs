//! # kv9-engine
//!
//! Storage engine abstraction and the v0 in-memory implementation (DESIGN §6.2, §8.1).
//!
//! `Engine` is a trait; a region owns a logical LSM keyed within its range. v0 ships
//! [`MemEngine`]; a real `LsmEngine` plugs in behind the same trait later (DESIGN §12).

pub mod cf;
pub mod mem;
pub mod persist;
pub mod wal;
pub mod write_batch;

pub use cf::ColumnFamily;
pub use mem::MemEngine;
pub use persist::WalEngine;
pub use wal::{Replay, Wal};
pub use write_batch::{Mutation, WriteBatch};

/// Whether an engine's accepted writes survive a restart (DESIGN §13 principle 14,
/// "flushed/persisted-index that gates truncation").
///
/// This exists so a volatile engine cannot be *mistaken* for a durable one. The raft log
/// may only be truncated to the extent the state machine has actually landed its data;
/// asking an engine that keeps everything in memory to answer that question with a number
/// invites the answer "0", which reads as a legitimate watermark. Making the volatile case
/// a distinct variant means the caller has to handle it, and the illegal state — a
/// volatile engine authorising truncation — cannot be written down (principle 16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Durability {
    /// Nothing survives a restart; the log must **never** be truncated on this engine's
    /// account.
    Volatile,
    /// Every write accepted before the call is on stable storage.
    ///
    /// Deliberately not a raft index: `write` is not told one, so an engine reporting an
    /// index would be reporting a number it inferred rather than one it knows. A caller
    /// that tracks which entries it applied can combine that with this answer. Carrying
    /// the index explicitly is a Phase-2 refinement, alongside flush-to-SST.
    DurableThroughLastWrite,
}

use kv9_common::{Result, UserKey, Value};

/// One entry produced by a range scan: `(key, value)`.
pub type ScanEntry = (UserKey, Value);

/// A consistent read view over the engine.
///
/// Every read taken through one `ReadView` observes the same version of the data. Two
/// separate [`Engine::get`] calls do *not* give that: an entire [`WriteBatch`] may commit
/// in between them, so the two reads can straddle a commit and disagree.
///
/// Both consumers need this for correctness, for different reasons:
///
/// * `meta` — the catalog's `index_scan` → `get(pk)` is two steps (METADATA-CATALOG §4).
///   If state changes between them, the scan chases an index entry whose row is gone.
///   Routing's `region_for_key` has the same shape: [`ReadView::seek_le`] for the
///   candidate region, then an `end_key` bound check, which must agree with each other.
/// * `txn` — a Percolator `Get` must check the `lock` CF and read `write`/`default` from
///   one view, or a racing prewrite's lock is missed and snapshot isolation breaks
///   (DESIGN §9.1: *"snapshot reads see versions ≤ start_ts"*). That lands with the txn
///   path; the view itself is timestamp-agnostic, and MVCC version selection sits above.
pub trait ReadView: Send + Sync {
    /// Point read from a column family, within this view.
    fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Value>>;

    /// Forward range scan `[start, end)` over a column family, bounded by `limit`.
    fn scan(
        &self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<ScanEntry>>;

    /// Reverse seek: the entry with the greatest key `≤ target`, or `None` if there is
    /// none.
    ///
    /// This is the "which region contains key K" lookup — routing resolves a key to the
    /// last region whose start key does not exceed it (DESIGN §5.1, the routing table's
    /// `key → region` range index).
    ///
    /// **This answers a byte-order question only.** It knows nothing about region or
    /// keyspace boundaries, so on its own it can return the last region of the *previous*
    /// keyspace when `target` falls in a gap. Callers must still bound-check the
    /// candidate's `end_key`; that check is what upholds the DESIGN §13 principle 4
    /// invariant that cross-tenant misrouting is impossible. Correctness also assumes the
    /// caller's key encoding is order-preserving (memcomparable).
    ///
    /// **Not sufficient on its own for a caller that buffers writes.** A single hit cannot
    /// be stepped past, so if the caller's overlay has *deleted* the entry this returns,
    /// there is no way to ask for the next one down. Such callers must use
    /// [`ReadView::iter_rev`] and walk backwards, skipping their own tombstones.
    fn seek_le(&self, cf: ColumnFamily, target: &[u8]) -> Result<Option<ScanEntry>>;

    /// Streaming ascending iterator over `[start, end)`.
    ///
    /// Unlike [`ReadView::scan`], nothing is materialized up front: the caller pulls
    /// entries and stops when it has enough. This is what lets a caller that merges an
    /// overlay honour a `limit` without first loading the whole range — DESIGN §13
    /// principle 13 (*"no unquota'd in-memory path… no 'no-size-hint' bypass that can OOM
    /// a node"*) rules out the load-everything-then-truncate shape.
    ///
    /// Items are `Result` because a real disaggregated engine fetches blocks from object
    /// storage mid-iteration and can fail there (DESIGN §6.5); an in-memory engine simply
    /// never yields `Err`.
    fn iter<'a>(
        &'a self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
    ) -> Result<Box<dyn Iterator<Item = Result<ScanEntry>> + 'a>>;

    /// Streaming **descending** iterator over `[start, end)`, yielding the greatest key
    /// first.
    ///
    /// This is the primitive behind "which region contains key K" for any caller that
    /// buffers writes: seek to the top of the range and walk down, skipping entries the
    /// caller's own overlay has deleted, until a live candidate appears. [`seek_le`] alone
    /// cannot do this — it yields one entry with no way to continue past it.
    ///
    /// [`seek_le`]: ReadView::seek_le
    fn iter_rev<'a>(
        &'a self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
    ) -> Result<Box<dyn Iterator<Item = Result<ScanEntry>> + 'a>>;
}

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
    ///
    /// Single-key convenience. When two or more reads must agree with each other, take a
    /// [`Engine::snapshot`] instead — consecutive `get` calls can straddle a commit.
    fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Value>>;

    /// Apply a batch of mutations **atomically** (DESIGN §6.2).
    ///
    /// Atomic means no reader — via [`Engine::get`], [`Engine::scan`], or a [`ReadView`] —
    /// ever observes a state where some of `batch`'s mutations are applied and others are
    /// not. This holds *across* column families, which is what makes a catalog
    /// transaction (a row plus its secondary index entries) and a Percolator commit
    /// (`lock` → `write` plus `default`) safe to build on.
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

    /// Whether this engine's accepted writes survive a restart.
    ///
    /// Callers deciding raft-log truncation must consult this: truncating past what the
    /// state machine has durably applied is silent data loss on the next restart.
    fn durability(&self) -> Durability;

    /// Take a consistent [`ReadView`]; all reads through it observe one version of the
    /// data.
    fn snapshot(&self) -> Result<Box<dyn ReadView + '_>>;
}
