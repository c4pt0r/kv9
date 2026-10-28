//! # kv9-common
//!
//! Foundational types shared by every kv9 crate (DESIGN §3, §7, §11):
//! ids, tenant/keyspace/api_type, the multi-tenant key codec, the crate-wide error,
//! and node config.
//!
//! This crate depends on no other kv9 crate.

pub mod anchor;
pub mod codec;
pub mod config;
pub mod data_range;
pub mod data_read_fence;
pub mod error;
pub mod fs;
pub mod ids;
pub mod keyspace;
pub mod log_budget;
pub mod metrics;
pub mod raw_ttl;
pub mod retention;
pub mod root;
pub mod store_lifecycle;

pub use config::{Config, SeedPeer};
pub use error::{Error, ReadBarrierPhase, Result};
pub use ids::AppliedPosition;
pub use ids::{ClusterId, KeyspaceId, NodeId, RegionId, TenantId, META_REGION_0};
pub use keyspace::{ApiType, Keyspace, Tenant};
pub use root::{
    load_root_bundle, persist_root_bundle, write_new_root_descriptor, BootstrapGeneration,
    ProcessIncarnation, RootDescriptor, RootDigest, RootVoter, StoreIdentity, StoreIncarnation,
    ROOT_DESCRIPTOR_FILE, STORE_IDENTITY_FILE,
};

/// A raw user key (before physical prefix encoding). Alias for readability.
pub type UserKey = Vec<u8>;
/// A value payload.
pub type Value = Vec<u8>;
