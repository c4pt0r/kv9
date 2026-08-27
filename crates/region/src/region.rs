//! Region and epoch (DESIGN §3.3, §6.1).

use kv9_common::{KeyspaceId, NodeId, RegionId};

/// Region epoch `(conf_ver, version)` (DESIGN §6.1). Every request is epoch-checked so
/// stale-routed requests are rejected and retried after a routing refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RegionEpoch {
    /// Bumped on membership (conf) changes.
    pub conf_ver: u64,
    /// Bumped on range changes (split/merge).
    pub version: u64,
}

impl RegionEpoch {
    /// Whether `self` is at least as fresh as `other` in both dimensions.
    pub fn is_fresh_as(&self, other: &RegionEpoch) -> bool {
        self.conf_ver >= other.conf_ver && self.version >= other.version
    }
}

/// A peer (replica) of a region on some node (DESIGN §3.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub node: NodeId,
    /// Whether this peer is a voting member or a learner.
    pub is_learner: bool,
}

/// A region: a half-open key range `[start, end)` replicated as a Raft group
/// (DESIGN §3.3). Regions never span keyspace boundaries (DESIGN §3.3, §13 principle 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub id: RegionId,
    /// The keyspace this region belongs to. A region belongs to exactly one keyspace
    /// (DESIGN §13 principle 3), so this is authoritative for GC/backup/encryption attribution.
    pub keyspace: KeyspaceId,
    /// Inclusive start of the physical (prefix-encoded) key range.
    pub start_key: Vec<u8>,
    /// Exclusive end of the physical key range.
    pub end_key: Vec<u8>,
    pub epoch: RegionEpoch,
    pub peers: Vec<Peer>,
    /// Best-known leader hint (DESIGN §5.1).
    pub leader_hint: Option<NodeId>,
}

impl Region {
    /// Whether a physical key falls inside this region's range.
    pub fn contains(&self, key: &[u8]) -> bool {
        key >= self.start_key.as_slice()
            && (self.end_key.is_empty() || key < self.end_key.as_slice())
    }
}
