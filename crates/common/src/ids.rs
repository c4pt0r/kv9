//! Cluster identifier newtypes (DESIGN §3).
//!
//! These are deliberately thin newtypes over integers so the type system prevents
//! mixing, e.g., a `RegionId` where a `KeyspaceId` is expected.

use serde::{Deserialize, Serialize};

/// Identifies one `kv9` process / store in the cluster (DESIGN §3.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NodeId(pub u64);

/// Identifies a region (range shard = Raft group) (DESIGN §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RegionId(pub u64);

/// The well-known, fixed region id of the L0 bootstrap meta group `META_REGION_0`
/// (DESIGN §5.1.1, §5.2). It covers the system key range and never grows.
pub const META_REGION_0: RegionId = RegionId(1);

/// Identifies a keyspace (DESIGN §3.2). Physically 3 bytes on the wire / in keys
/// (DESIGN §3.4), so the valid range is `0..=0x00FF_FFFF` (2^24 keyspaces).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct KeyspaceId(pub u32);

impl KeyspaceId {
    /// The reserved system keyspace (`keyspace_id = 0`, mode `'s'`) — DESIGN §5.
    pub const SYSTEM: KeyspaceId = KeyspaceId(0);

    /// Maximum encodable keyspace id given the 3-byte on-disk width (DESIGN §3.4).
    pub const MAX: u32 = 0x00FF_FFFF;
}

/// Identifies a tenant: the isolation and accounting boundary (DESIGN §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TenantId(pub u64);

impl TenantId {
    /// The default tenant created at bootstrap (DESIGN §5.2).
    pub const DEFAULT: TenantId = TenantId(0);
}

/// Identifies a transaction/consistency domain = timestamp shard (DESIGN §3.6, §8.1).
///
/// Every `txn` keyspace belongs to exactly one txn group; a transaction never crosses
/// a group boundary (the confinement invariant), which is what lets each group own an
/// independent, sharded TSO timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TxnGroupId(pub u64);

impl TxnGroupId {
    /// The `default` txn group — one timeline, behaves like a single classic TSO
    /// (DESIGN §3.6, §8.1).
    pub const DEFAULT: TxnGroupId = TxnGroupId(0);
}

/// Identifies one TSO timeline (1:1 with a txn group) — DESIGN §8.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TimelineId(pub u64);

/// Identifies a TSO provider (pool member) hosting one or more timelines — DESIGN §8.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TsoProviderId(pub u64);
