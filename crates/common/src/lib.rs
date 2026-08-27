//! # kv9-common
//!
//! Foundational types shared by every kv9 crate (DESIGN §3, §7, §11):
//! ids, tenant/keyspace/api_type, the multi-tenant key codec, timestamp/HLC clocks,
//! the crate-wide error, and node config.
//!
//! This crate depends on no other kv9 crate.

pub mod codec;
pub mod config;
pub mod error;
pub mod ids;
pub mod keyspace;
pub mod time;

pub use config::Config;
pub use error::{Error, Result};
pub use ids::{
    KeyspaceId, NodeId, RegionId, TenantId, TimelineId, TsoProviderId, TxnGroupId, META_REGION_0,
};
pub use keyspace::{ApiType, Keyspace, Tenant};
pub use time::{Hlc, TimeSource, TimeStamp};

/// A raw user key (before physical prefix encoding). Alias for readability.
pub type UserKey = Vec<u8>;
/// A value payload.
pub type Value = Vec<u8>;
