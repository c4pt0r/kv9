//! Region routing (DESIGN §5.1, §5.4, §6.1).

use std::collections::BTreeMap;

use kv9_common::{Error, RegionId, Result};

use crate::region::{Region, RegionEpoch};

/// Resolves `key → region` from a cached copy of the routing table, epoch-checking on
/// each request (DESIGN §6.1). The cache follows the MemDS discipline in DESIGN §5.4:
/// serve from cache, refresh on a steady background cadence, and refuse to serve until
/// a freshness watermark is reached (avoiding a synchronized cold-start miss-storm).
#[derive(Debug, Default)]
pub struct RegionRouter {
    /// Index by physical `start_key` for range lookup (mirrors the L1 range index).
    by_start: BTreeMap<Vec<u8>, Region>,
    /// Whether the cache has reached its freshness watermark and may serve (DESIGN §5.4).
    ready: bool,
}

impl RegionRouter {
    pub fn new() -> Self {
        RegionRouter {
            by_start: BTreeMap::new(),
            ready: false,
        }
    }

    /// Mark the cache warm enough to serve (DESIGN §5.4).
    pub fn mark_ready(&mut self) {
        self.ready = true;
    }

    pub fn is_ready(&self) -> bool {
        self.ready
    }

    /// Insert / update a region in the routing cache.
    pub fn upsert(&mut self, region: Region) {
        self.by_start.insert(region.start_key.clone(), region);
    }

    /// Resolve the region owning a physical key (DESIGN §6.1).
    ///
    /// Refuses to serve until warm (DESIGN §5.4). Returns [`Error::RegionNotFound`]
    /// when no cached region covers the key (caller should refresh and retry).
    pub fn route(&self, key: &[u8]) -> Result<&Region> {
        if !self.ready {
            return Err(Error::MetaNotReady(
                "router cache below freshness watermark".into(),
            ));
        }
        // Largest start_key <= key.
        self.by_start
            .range(..=key.to_vec())
            .next_back()
            .map(|(_, r)| r)
            .filter(|r| r.contains(key))
            .ok_or(Error::RegionNotFound)
    }

    /// Epoch check: reject a request whose epoch is staler than the cached region's
    /// (DESIGN §6.1). Returns [`Error::StaleEpoch`] on mismatch.
    pub fn check_epoch(&self, region: RegionId, req_epoch: &RegionEpoch) -> Result<()> {
        let cached = self
            .by_start
            .values()
            .find(|r| r.id == region)
            .ok_or(Error::RegionNotFound)?;
        if req_epoch.is_fresh_as(&cached.epoch) {
            Ok(())
        } else {
            Err(Error::StaleEpoch { region })
        }
    }
}
