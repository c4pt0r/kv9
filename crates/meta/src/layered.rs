//! Multi-level metadata (DESIGN §5.1.1) — the #1 design point that avoids PD's
//! monolithic in-memory model.
//!
//! ```text
//!   L0  Root (bootstrap group)  ── META_REGION_0: tiny, fixed, never grows.
//!        │                          Holds only: L1 meta-region locations, membership
//!        │                          root, TSO window, MetaLeader lease.
//!        ▼
//!   L1  Meta-regions            ── routing table + catalog + placement, stored as
//!        │                          ORDINARY KV in the system keyspace, SHARDED into
//!        ▼                          regions that split/merge like user data.
//!   L2  User regions
//! ```
//!
//! Lookup path (cached at every level):
//! `key → L0 root (which L1 meta-region routes this key) → L1 (which L2 user-region
//! owns the key) → L2 leader`.

use kv9_common::{RegionId, TimeStamp, META_REGION_0};

/// The three levels of the metadata hierarchy (DESIGN §5.1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaLevel {
    /// L0 — the fixed root bootstrap group (`META_REGION_0`).
    Root,
    /// L1 — sharded meta-regions holding routing/catalog/placement.
    MetaRegion,
    /// L2 — user regions.
    UserRegion,
}

/// The L0 root record (DESIGN §5.1.1). Tiny and near-static: it holds only pointers to
/// the L1 meta-regions plus the membership root, TSO window, and MetaLeader lease. It
/// **never grows** with cluster size.
#[derive(Debug, Clone, Default)]
pub struct RootRecord {
    /// The fixed region id of the root group.
    pub root_region: Option<RegionId>,
    /// Locations of L1 meta-regions ("meta of meta"): the region ids that route ranges
    /// of the system keyspace's routing/catalog data.
    pub meta_regions: Vec<RegionId>,
    /// The persisted upper bound of allocated timestamps — the TSO window (DESIGN §8).
    /// Kept at L0 so it survives even before L1 exists.
    pub tso_window_high: TimeStamp,
}

impl RootRecord {
    /// A freshly-initialized root pointing at `META_REGION_0` (DESIGN §5.2).
    pub fn bootstrap() -> Self {
        RootRecord {
            root_region: Some(META_REGION_0),
            meta_regions: Vec::new(),
            tso_window_high: TimeStamp::ZERO,
        }
    }

    /// Record a newly-created L1 meta-region (DESIGN §5.1.1: L0 records the first L1).
    pub fn add_meta_region(&mut self, region: RegionId) {
        if !self.meta_regions.contains(&region) {
            self.meta_regions.push(region);
        }
    }
}

/// A resolver that walks the hierarchy `L0 → L1 → L2`, caching at every level
/// (DESIGN §5.1.1). The skeleton models the *shape*; real caches follow the MemDS
/// freshness discipline (DESIGN §5.4).
#[derive(Debug, Default)]
pub struct HierarchyResolver {
    pub root: RootRecord,
}

impl HierarchyResolver {
    pub fn new(root: RootRecord) -> Self {
        HierarchyResolver { root }
    }

    /// L0 step: which L1 meta-region routes this physical key (DESIGN §5.1.1).
    ///
    /// The skeleton returns the first meta-region; a real impl range-partitions the
    /// system keyspace across L1 meta-regions.
    pub fn meta_region_for(&self, _key: &[u8]) -> Option<RegionId> {
        self.root.meta_regions.first().copied()
    }
}
