//! Throughput-aware split/merge hooks (DESIGN §10, §13 principles 3 & 7).
//!
//! kv9 splits on *consumed throughput/CPU*, not size, and chooses the split key from
//! the observed access distribution (DynamoDB 2022), never crossing a keyspace
//! boundary (DESIGN §3.3, §13 principle 3).

use kv9_common::codec::keyspace_of;
use kv9_common::{Error, Result};

use crate::region::Region;

/// Observed, consumption-first load signal for a region (DESIGN §10). This is the input
/// to split/merge/placement decisions — consumption-aware from day one, not disk-first.
#[derive(Debug, Clone, Copy, Default)]
pub struct RegionLoad {
    /// Read-capacity units consumed per second.
    pub rcu_per_sec: f64,
    /// Write-capacity units consumed per second.
    pub wcu_per_sec: f64,
    /// CPU fraction consumed by this region (0.0..=1.0).
    pub cpu_fraction: f64,
    /// Approximate on-disk / in-memory size in bytes (secondary signal).
    pub size_bytes: u64,
    /// True if load concentrates on a single key (splitting can't help — DESIGN §10).
    pub single_key_hot: bool,
}

/// The outcome of evaluating a region for splitting (DESIGN §10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplitDecision {
    /// Do not split.
    NoSplit,
    /// Split at this physical key (chosen from the access distribution).
    SplitAt(Vec<u8>),
    /// Region is hot on one key; splitting can't shed load — handle via capacity /
    /// adaptive routing instead (DESIGN §10).
    HotSingleKey,
}

/// Decide whether/where to split, throughput-first (DESIGN §10).
///
/// `access_hint` is a candidate split key derived from the observed access
/// distribution (not the midpoint). The chosen key is validated to stay within the
/// region's keyspace (DESIGN §3.3, §13 principle 3).
pub fn evaluate_split(
    region: &Region,
    load: &RegionLoad,
    hot_threshold_wcu: f64,
    access_hint: Option<&[u8]>,
) -> Result<SplitDecision> {
    let hot = load.wcu_per_sec >= hot_threshold_wcu
        || load.rcu_per_sec >= hot_threshold_wcu
        || load.cpu_fraction >= 0.75;
    if !hot {
        return Ok(SplitDecision::NoSplit);
    }
    if load.single_key_hot {
        return Ok(SplitDecision::HotSingleKey);
    }
    match access_hint {
        Some(key) => {
            validate_split_key(region, key)?;
            Ok(SplitDecision::SplitAt(key.to_vec()))
        }
        None => Ok(SplitDecision::NoSplit),
    }
}

/// Enforce the invariant that a split key never crosses a keyspace boundary
/// (DESIGN §3.3, §13 principle 3). Keeps keyspace-id derivable from a region's start
/// key and a tenant's blast radius contained.
pub fn validate_split_key(region: &Region, split_key: &[u8]) -> Result<()> {
    if !region.contains(split_key) {
        return Err(Error::RegionNotFound);
    }
    let ks = keyspace_of(split_key)?;
    if ks != region.keyspace {
        return Err(Error::SplitCrossesKeyspace);
    }
    Ok(())
}

/// Whether two adjacent, low-traffic regions in the same keyspace may be merged
/// (DESIGN §10). Merge reclaims raft overhead — the "cold region" idea, but merging
/// rather than only quiescing.
pub fn can_merge(
    left: &Region,
    right: &Region,
    left_load: &RegionLoad,
    right_load: &RegionLoad,
    cold_threshold_wcu: f64,
) -> bool {
    left.keyspace == right.keyspace
        && left.end_key == right.start_key
        && left_load.wcu_per_sec < cold_threshold_wcu
        && right_load.wcu_per_sec < cold_threshold_wcu
}
