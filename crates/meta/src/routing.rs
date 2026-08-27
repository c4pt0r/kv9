//! The region routing table snapshot (DESIGN §5.1).
//!
//! This is the authoritative `region_id → {range, epoch, peers, leader hint}` table plus
//! the `key → region` range index. Routers cache a copy in [`kv9_region::RegionRouter`]
//! and refresh it per the MemDS discipline (DESIGN §5.4).

use std::collections::HashMap;

use kv9_common::{Error, RegionId, Result};
use kv9_region::Region;

/// The routing table (DESIGN §5.1). Lives in L1 meta-regions once metadata splits
/// (DESIGN §5.1.1), so its throughput/memory scale horizontally.
#[derive(Debug, Default)]
pub struct RoutingTable {
    regions: HashMap<RegionId, Region>,
}

impl RoutingTable {
    pub fn new() -> Self {
        RoutingTable::default()
    }

    pub fn upsert(&mut self, region: Region) {
        self.regions.insert(region.id, region);
    }

    pub fn get(&self, id: RegionId) -> Result<&Region> {
        self.regions.get(&id).ok_or(Error::RegionNotFound)
    }

    /// Range lookup: which region owns a physical key (DESIGN §5.1).
    pub fn route(&self, key: &[u8]) -> Result<&Region> {
        self.regions
            .values()
            .find(|r| r.contains(key))
            .ok_or(Error::RegionNotFound)
    }

    pub fn regions(&self) -> impl Iterator<Item = &Region> {
        self.regions.values()
    }

    pub fn len(&self) -> usize {
        self.regions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }
}
