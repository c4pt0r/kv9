//! Placement / scheduler state (DESIGN §5.1, §10).
//!
//! A MetaLeader responsibility. Scoring is **consumption-aware from day one**
//! (WCU/RCU/CPU/region-count), *not* local-disk-capacity-first (DESIGN §10, §13
//! principle 7). Global Admission Control uses per-tenant / per-keyspace token buckets
//! (DynamoDB GAC, DESIGN §10).

use std::collections::HashMap;

use kv9_common::{NodeId, TenantId};

/// A pending scheduler task in the operator log (DESIGN §5.1).
#[derive(Debug, Clone)]
pub enum ScheduleTask {
    /// Move a region replica off `from` onto `to` for rebalance.
    TransferReplica {
        region: kv9_common::RegionId,
        from: NodeId,
        to: NodeId,
    },
    /// Split a region at a physical key (throughput-driven, DESIGN §10).
    Split {
        region: kv9_common::RegionId,
        split_key: Vec<u8>,
    },
    /// Merge two adjacent low-traffic regions (DESIGN §10).
    Merge {
        left: kv9_common::RegionId,
        right: kv9_common::RegionId,
    },
}

/// Consumption-aware store score inputs (DESIGN §10, §13 principle 7).
#[derive(Debug, Clone, Copy, Default)]
pub struct StoreScoreInput {
    pub rcu_per_sec: f64,
    pub wcu_per_sec: f64,
    pub cpu_fraction: f64,
    pub region_count: u64,
}

/// Compute a consumption-first placement score for a store (DESIGN §10). Lower is more
/// eligible to receive load. Deliberately *not* disk-capacity-first.
pub fn store_score(input: &StoreScoreInput) -> f64 {
    // Weighted sum of consumption signals; region_count is a mild tiebreaker.
    input.rcu_per_sec * 1.0
        + input.wcu_per_sec * 1.5
        + input.cpu_fraction * 1000.0
        + (input.region_count as f64) * 0.01
}

/// A token bucket for Global Admission Control (DESIGN §10).
#[derive(Debug, Clone)]
pub struct TokenBucket {
    pub capacity: f64,
    pub tokens: f64,
    pub refill_per_sec: f64,
}

impl TokenBucket {
    pub fn new(capacity: f64, refill_per_sec: f64) -> Self {
        TokenBucket {
            capacity,
            tokens: capacity,
            refill_per_sec,
        }
    }

    /// Try to consume `n` tokens; returns whether admitted (DESIGN §10).
    pub fn try_consume(&mut self, n: f64) -> bool {
        if self.tokens >= n {
            self.tokens -= n;
            true
        } else {
            false
        }
    }

    /// Refill for `elapsed_secs`, capped at capacity.
    pub fn refill(&mut self, elapsed_secs: f64) {
        self.tokens = (self.tokens + self.refill_per_sec * elapsed_secs).min(self.capacity);
    }
}

/// The scheduler / placement state singleton owned by the MetaLeader (DESIGN §5.1, §10).
#[derive(Debug, Default)]
pub struct Scheduler {
    pub tasks: Vec<ScheduleTask>,
    /// Per-tenant admission buckets for GAC (DESIGN §10).
    pub tenant_buckets: HashMap<TenantId, TokenBucket>,
}

impl Scheduler {
    pub fn new() -> Self {
        Scheduler::default()
    }

    pub fn enqueue(&mut self, task: ScheduleTask) {
        self.tasks.push(task);
    }

    /// Admit a request for a tenant against its GAC bucket (DESIGN §10).
    pub fn admit(&mut self, tenant: TenantId, cost: f64) -> bool {
        match self.tenant_buckets.get_mut(&tenant) {
            Some(b) => b.try_consume(cost),
            // No bucket configured ⇒ unmetered (skeleton default).
            None => true,
        }
    }
}
