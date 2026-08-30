//! # kv9-meta
//!
//! The self-hosted metadata plane (DESIGN §5, §8, §10). Metadata is just data in a
//! reserved system keyspace, replicated by the same Raft as user data — there is no
//! external placement driver.
//!
//! Modules: membership, keyspace catalog, region routing, multi-level metadata (L0/L1),
//! election-first bootstrap, MetaLeader, placement/scheduler, and the sharded TSO
//! provider pool.
//!
//! The relational catalog engine (`docs/METADATA-CATALOG.md`) lives in
//! [`schema`]/[`codec`]/[`store`]/[`tables`]/[`migrate`]: `membership / catalog /
//! routing / placement / tso` become **one [`MetaStore`]** — a fixed, versioned schema
//! with auto-maintained indexes and transactions over the system keyspace KV. The
//! legacy in-memory [`Catalog`]/[`Membership`]/[`RoutingTable`] structs remain as the
//! Phase-0 typed façades and are migrated onto `MetaStore` incrementally.

// The relational catalog engine (docs/METADATA-CATALOG.md).
pub mod codec;
pub mod migrate;
pub mod schema;
pub mod store;
pub mod tables;

// Election-first FSM, MetaLeader, and the typed façades wired to MetaStore over time.
pub mod admission;
pub mod bootstrap;
pub mod catalog;
pub mod layered;
pub mod leader;
pub mod membership;
pub mod placement;
pub mod root;
pub mod routing;
pub mod tso;

pub use bootstrap::{Bootstrap, BootstrapEvent, BootstrapState};
pub use catalog::Catalog;
pub use layered::{HierarchyResolver, MetaLevel, RootRecord};
pub use leader::{should_force_failover, Lease, MetaLeader};
pub use membership::{Membership, NodeInfo, NodeState};
pub use placement::{store_score, ScheduleTask, Scheduler, StoreScoreInput, TokenBucket};
pub use routing::RoutingTable;
pub use tso::{EmbeddedTso, TimelineWindow, TimestampOracle, TsoPool, TsoProvider};

// Catalog engine exports (docs/METADATA-CATALOG.md).
pub use codec::{ColumnValue, RowValue};
pub use migrate::{migrate, MigrationStep};
pub use schema::{
    ColumnDesc, ColumnId, IndexDesc, IndexId, TableDesc, TableId, ALL_TABLES, SCHEMA_VERSION,
};
pub use store::{Changes, MetaStore, MetaTxn, Row};
pub use tables::{
    Keyspace as KeyspaceRow, Node as NodeRow, Region as RegionRow, RegionPeer, SstFile, Tables,
    Tenant as TenantRow, TsoTimeline, TxnGroup,
};
