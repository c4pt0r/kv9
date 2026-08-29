//! MetaLeader election, lease, and gray-failure handling (DESIGN §5.3).
//!
//! The MetaLeader is the Raft leader of `META_REGION_0` (extended, once metadata
//! splits, to the coordinator owning the scheduler singleton via a lease held in the
//! meta group). Election is plain Raft leader election — no external lock service.

use kv9_common::{Error, NodeId, Result};

/// A lease held by the MetaLeader (DESIGN §5.3, DynamoDB leader-lease lesson).
///
/// A new MetaLeader only acts after the previous lease is known-expired (bounded by a
/// conservative clock estimate), preventing split-brain during failover.
#[derive(Debug, Clone, Copy)]
pub struct Lease {
    pub holder: NodeId,
    /// Lease expiry as nanos since epoch (conservative bound).
    pub expires_at_nanos: u64,
}

impl Lease {
    pub fn is_expired(&self, now_nanos: u64) -> bool {
        now_nanos >= self.expires_at_nanos
    }
}

/// The elected metadata coordinator (DESIGN §5.3). Runs placement, split/merge, and
/// hosts / assigns the timestamp oracle providers (DESIGN §5, §8, §10).
#[derive(Debug)]
pub struct MetaLeader {
    node: NodeId,
    lease: Option<Lease>,
}

impl MetaLeader {
    pub fn new(node: NodeId) -> Self {
        MetaLeader { node, lease: None }
    }

    pub fn node(&self) -> NodeId {
        self.node
    }

    /// Acquire / renew the lease. A new leader may only acquire after the previous
    /// lease is known-expired (DESIGN §5.3).
    pub fn acquire_lease(
        &mut self,
        now_nanos: u64,
        duration_nanos: u64,
        prev: Option<Lease>,
    ) -> Result<Lease> {
        if let Some(prev) = prev {
            if prev.holder != self.node && !prev.is_expired(now_nanos) {
                return Err(Error::MetaNotReady(
                    "previous MetaLeader lease not yet expired".into(),
                ));
            }
        }
        let lease = Lease {
            holder: self.node,
            expires_at_nanos: now_nanos + duration_nanos,
        };
        self.lease = Some(lease);
        Ok(lease)
    }

    /// Whether this leader currently holds a valid lease and may act (DESIGN §5.3).
    pub fn can_act(&self, now_nanos: u64) -> bool {
        self.lease
            .map(|l| !l.is_expired(now_nanos))
            .unwrap_or(false)
    }
}

/// Gray-failure "double-confirm down" decision (DESIGN §5.3, §13).
///
/// A follower that suspects the leader must ask a quorum before forcing an election, so
/// a one-way network glitch doesn't cause needless failover. Returns whether a failover
/// election should be forced.
pub fn should_force_failover(suspicions: usize, quorum: usize) -> bool {
    suspicions >= quorum
}
