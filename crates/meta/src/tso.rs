//! Sharded TSO — a pool of providers serving many keyspaces / txn groups (DESIGN §8.1–§8.2).
//!
//! ```text
//!    keyspace ──N:1──▶ txn group ──1:1──▶ TSO timeline ──N:1──▶ TSO provider (pool member)
//! ```
//!
//! Because a transaction never crosses a txn group (DESIGN §3.6), each group needs
//! timestamps ordered only within itself, so kv9 runs a *pool* of providers. A single
//! provider hosts one or more timelines and serves different keyspaces/groups at once.
//! No cross-group timestamp comparison ever happens.

use std::collections::HashMap;
use std::sync::Mutex;

use kv9_common::{Error, Result, TimeStamp, TimelineId, TsoProviderId, TxnGroupId};

/// A per-group timestamp oracle handle (DESIGN §8.2). Carries the [`TxnGroupId`] so a
/// caller can never accidentally mix timelines across groups.
///
/// Each group's oracle can be realized three ways (DESIGN §8.2): embedded (default),
/// HLC, or TrueTime-style commit-wait. v0 ships [`EmbeddedTso`].
pub trait TimestampOracle: Send + Sync {
    /// The txn group this oracle serves.
    fn txn_group(&self) -> TxnGroupId;

    /// The timeline id (1:1 with the txn group).
    fn timeline(&self) -> TimelineId;

    /// Allocate a fresh, strictly-increasing timestamp on this group's timeline
    /// (DESIGN §8.1). Never returns ≤ the persisted bound; refuses to serve until the
    /// provider lease is confirmed.
    fn now(&self) -> Result<TimeStamp>;
}

/// The persisted allocation window for one timeline (DESIGN §8.1).
///
/// Timestamps are allocated *ahead* into the system keyspace (`high`) and served from
/// memory (`served`), so a provider failover starts above the persisted window.
#[derive(Debug, Clone, Copy)]
pub struct TimelineWindow {
    /// Highest timestamp allocated (persisted) — the upper bound.
    pub high: TimeStamp,
    /// Highest timestamp actually handed out (in memory).
    pub served: TimeStamp,
}

impl TimelineWindow {
    pub fn new(high: TimeStamp) -> Self {
        TimelineWindow {
            high,
            served: TimeStamp::ZERO,
        }
    }
}

/// Embedded TSO: serve monotonic timestamps from a persisted window (DESIGN §8.2 opt 1).
///
/// This is the v0 stub. `lease_confirmed` models the "refuse to serve until the new
/// lease is confirmed" anti-regression rule (DESIGN §8.1).
pub struct EmbeddedTso {
    group: TxnGroupId,
    timeline: TimelineId,
    window: Mutex<TimelineWindow>,
    lease_confirmed: bool,
}

impl EmbeddedTso {
    /// Create an embedded oracle for a group/timeline starting above `persisted_high`.
    pub fn new(
        group: TxnGroupId,
        timeline: TimelineId,
        persisted_high: TimeStamp,
        lease_confirmed: bool,
    ) -> Self {
        EmbeddedTso {
            group,
            timeline,
            // Start serving at the persisted high so we never re-hand a bound (DESIGN §8.1).
            window: Mutex::new(TimelineWindow {
                high: persisted_high,
                served: persisted_high,
            }),
            lease_confirmed,
        }
    }
}

impl TimestampOracle for EmbeddedTso {
    fn txn_group(&self) -> TxnGroupId {
        self.group
    }

    fn timeline(&self) -> TimelineId {
        self.timeline
    }

    fn now(&self) -> Result<TimeStamp> {
        if !self.lease_confirmed {
            return Err(Error::TsoUnavailable("provider lease not yet confirmed".into()));
        }
        let mut w = self.window.lock().expect("tso window poisoned");
        let next = w.served.next();
        if next.0 > w.high.0 {
            // Skeleton: a real impl allocates a new window into the system keyspace here.
            return Err(Error::TsoUnavailable(
                "timeline window exhausted; allocate-ahead not implemented".into(),
            ));
        }
        w.served = next;
        Ok(next)
    }
}

/// One TSO provider (pool member) hosting one or more timelines (DESIGN §8.1).
///
/// A provider is elected and lease-held via the metadata plane; the timelines it owns
/// recover from the system keyspace on failover.
pub struct TsoProvider {
    id: TsoProviderId,
    /// Timelines this provider hosts, keyed by txn group.
    timelines: HashMap<TxnGroupId, Box<dyn TimestampOracle>>,
}

impl TsoProvider {
    pub fn new(id: TsoProviderId) -> Self {
        TsoProvider {
            id,
            timelines: HashMap::new(),
        }
    }

    pub fn id(&self) -> TsoProviderId {
        self.id
    }

    /// Assign a timeline (as a concrete oracle) to this provider (DESIGN §8.1).
    pub fn host(&mut self, oracle: Box<dyn TimestampOracle>) {
        self.timelines.insert(oracle.txn_group(), oracle);
    }

    /// Serve a timestamp for a txn group hosted here (DESIGN §8.1).
    pub fn now(&self, group: TxnGroupId) -> Result<TimeStamp> {
        self.timelines
            .get(&group)
            .ok_or_else(|| Error::TsoUnavailable(format!("group {group:?} not hosted here")))?
            .now()
    }

    pub fn hosts(&self, group: TxnGroupId) -> bool {
        self.timelines.contains_key(&group)
    }
}

/// The cluster's pool of TSO providers plus the `txn group → provider` assignment
/// (DESIGN §8.1). Assignment is data and rebalanceable: a hot group can get its own
/// provider; many cold groups can share one. The mapping has a single authoritative
/// source (DESIGN §8.1).
#[derive(Default)]
pub struct TsoPool {
    providers: HashMap<TsoProviderId, TsoProvider>,
    /// The authoritative `txn group → provider` assignment.
    assignment: HashMap<TxnGroupId, TsoProviderId>,
}

impl TsoPool {
    pub fn new() -> Self {
        TsoPool::default()
    }

    pub fn add_provider(&mut self, provider: TsoProvider) {
        self.providers.insert(provider.id(), provider);
    }

    /// Assign (or move) a txn group's timeline to a provider (DESIGN §8.1).
    pub fn assign(&mut self, group: TxnGroupId, provider: TsoProviderId) {
        self.assignment.insert(group, provider);
    }

    /// Which provider serves a txn group (DESIGN §8.1).
    pub fn provider_for(&self, group: TxnGroupId) -> Option<TsoProviderId> {
        self.assignment.get(&group).copied()
    }

    /// Resolve and serve a timestamp for a txn group via its assigned provider
    /// (DESIGN §8.1).
    pub fn now(&self, group: TxnGroupId) -> Result<TimeStamp> {
        let pid = self
            .provider_for(group)
            .ok_or_else(|| Error::TsoUnavailable(format!("no provider assigned to {group:?}")))?;
        self.providers
            .get(&pid)
            .ok_or_else(|| Error::TsoUnavailable(format!("provider {pid:?} missing")))?
            .now(group)
    }
}
