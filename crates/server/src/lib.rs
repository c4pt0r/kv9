//! # kv9-server
//!
//! Node assembly and the request-serving surface (DESIGN §4, §11): the API traits
//! (`TxnApi`/`RawApi`/`AdminApi`/`RouterApi`), request routing, and the `Node` that
//! assembles the store, metadata plane, router, and executors into one process.

pub mod admission;
pub mod api;
mod bootstrap_retention;
mod catalog_lock;
pub mod client;
pub mod data_groups;
pub mod endpoints;
pub mod fence;
pub mod grpc;
pub mod installation_retention;
pub mod node;
mod point_stream;
#[cfg(test)]
mod point_test_support;
mod point_wire;
mod recovery_retention;
mod region_manager;
pub mod retention;
pub use region_manager::{GroupPreparation, GroupRecoveryStatus, RecoverySelectionRelation};
pub mod raw_ttl;
pub mod routing;
pub mod runtime;
pub mod ttl_clock;
pub mod workload;

pub use api::{
    AdminApi, AppliedPosition, ClusterInfo, CreateKeyspaceResult, RawApi, RegionLocation,
    RequestContext, RequestOrigin, RouterApi,
};
pub use grpc::proto;
pub use grpc::{
    admit_node_blocking, create_keyspace_blocking, list_keyspaces_blocking, promote_node_blocking, remove_node_blocking,
    revoke_admission_blocking, AuthContext, AuthKind, Authenticator, Kv9Grpc, PublicApiBackend,
    RawClient, RawClientOutcome, RawScanPage, TokenAuthenticator, LEADER_HINT_KEY, NOT_LEADER_KEY,
};
pub use node::{MetaPlane, MetaRaft, Node, Store};
pub use routing::{route_request, Routed};
pub use runtime::{NodeRuntime, RuntimeAuth};

mod checkpoint_retention;
mod log_budget;
mod maintenance;
mod migration_retention;
mod observability;
mod remote_storage;
mod write_pressure;
