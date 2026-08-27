//! # kv9-meta
//!
//! The self-hosted metadata plane (DESIGN §5, §8, §10). Metadata is just data in a
//! reserved system keyspace, replicated by the same Raft as user data — there is no
//! external placement driver.
//!
//! Modules: membership, keyspace catalog, region routing, multi-level metadata (L0/L1),
//! election-first bootstrap, MetaLeader, placement/scheduler, and the sharded TSO
//! provider pool.

pub mod bootstrap;
pub mod catalog;
pub mod layered;
pub mod leader;
pub mod membership;
pub mod placement;
pub mod routing;
pub mod tso;

pub use bootstrap::{Bootstrap, BootstrapEvent, BootstrapState};
pub use catalog::Catalog;
pub use layered::{HierarchyResolver, MetaLevel, RootRecord};
pub use leader::{should_force_failover, Lease, MetaLeader};
pub use membership::{Membership, NodeInfo, NodeState};
pub use placement::{store_score, Scheduler, ScheduleTask, StoreScoreInput, TokenBucket};
pub use routing::RoutingTable;
pub use tso::{EmbeddedTso, TimelineWindow, TimestampOracle, TsoPool, TsoProvider};
