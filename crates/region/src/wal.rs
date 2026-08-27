//! Sharded WAL — shared-but-sharded log ingress (DESIGN §6.4).
//!
//! The raft log is the WAL for a region's memtable. TiKV serializes a store's raft log
//! into a *single* shared WAL (great fsync amortization, but a single-writer ceiling on
//! per-node write throughput). kv9 keeps the amortization and removes the ceiling: a
//! node runs a **pool** of WAL streams, and each region is assigned to one stream;
//! multiple regions share a stream.
//!
//! - **Shared:** many regions → one stream, so one fsync amortizes across all of them.
//! - **Sharded:** N independent streams, each with its own writer/fsync/compaction, so
//!   per-node write throughput scales with stream count.
//! - **Assignment:** region → stream by a stable rule (hash of region id, or
//!   placement-driven to isolate a hot region), stable across restarts for unambiguous
//!   recovery.
//! - **Independent recovery & truncation:** each stream recovers on its own; the durable
//!   per-region log files stay per-region (the stream is only the ingress serialization
//!   point). A region's truncation is bounded by its own flush watermark.

use kv9_common::{RegionId, Result};

/// Identifies one WAL stream within a node's pool (DESIGN §6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WalStreamId(pub usize);

/// An append handed to a WAL stream: opaque raft-entry bytes for a specific region.
#[derive(Debug, Clone)]
pub struct WalAppend {
    pub region: RegionId,
    pub data: Vec<u8>,
}

/// One independent WAL stream: its own writer + fsync, shared by many regions
/// (DESIGN §6.4). This is the ingress serialization point; the durable per-region log
/// files are written per region.
pub trait WalStream: Send + Sync {
    fn id(&self) -> WalStreamId;

    /// Append a batch and group-commit-fsync once (amortized across regions on this
    /// stream) — DESIGN §6.4.
    fn append(&self, batch: Vec<WalAppend>) -> Result<()>;

    /// Force a durability barrier (fsync) for entries appended so far.
    fn sync(&self) -> Result<()>;
}

/// Strategy for mapping a region onto a stream (DESIGN §6.4). Stable across restarts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignmentStrategy {
    /// `region_id % stream_count` — the default deterministic rule.
    HashRegionId,
    /// Placement-driven: the mapping is recorded in region metadata (a hot region can
    /// be isolated onto its own stream; alignable with txn-group/keyspace for locality).
    PlacementDriven,
}

/// A node's pool of WAL streams plus the region→stream assignment (DESIGN §6.4).
pub struct WalPool {
    streams: Vec<Box<dyn WalStream>>,
    strategy: AssignmentStrategy,
    /// Explicit overrides (used by `PlacementDriven`), consulted before the hash rule.
    overrides: std::collections::HashMap<RegionId, WalStreamId>,
}

impl WalPool {
    /// Create a pool over `streams` with an assignment strategy (DESIGN §6.4).
    pub fn new(streams: Vec<Box<dyn WalStream>>, strategy: AssignmentStrategy) -> Self {
        WalPool {
            streams,
            strategy,
            overrides: std::collections::HashMap::new(),
        }
    }

    pub fn stream_count(&self) -> usize {
        self.streams.len()
    }

    /// Record a placement-driven override pinning a region to a specific stream
    /// (DESIGN §6.4). Stored in region metadata so recovery is unambiguous.
    pub fn pin(&mut self, region: RegionId, stream: WalStreamId) {
        self.overrides.insert(region, stream);
    }

    /// Resolve which stream a region is assigned to (DESIGN §6.4).
    pub fn stream_for(&self, region: RegionId) -> WalStreamId {
        if let Some(&s) = self.overrides.get(&region) {
            return s;
        }
        match self.strategy {
            AssignmentStrategy::HashRegionId | AssignmentStrategy::PlacementDriven => {
                let n = self.streams.len().max(1);
                WalStreamId((region.0 as usize) % n)
            }
        }
    }

    /// Borrow the stream a region is assigned to, for appending (DESIGN §6.4).
    pub fn stream(&self, region: RegionId) -> Option<&dyn WalStream> {
        let id = self.stream_for(region);
        self.streams.get(id.0).map(|s| s.as_ref())
    }

    /// Append a region's entry to its assigned stream.
    pub fn append(&self, region: RegionId, data: Vec<u8>) -> Result<()> {
        match self.stream(region) {
            Some(s) => s.append(vec![WalAppend { region, data }]),
            None => Err(kv9_common::Error::Engine("no WAL stream in pool".into())),
        }
    }
}
