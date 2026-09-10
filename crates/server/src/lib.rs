//! # kv9-server
//!
//! Node assembly and the request-serving surface (DESIGN §4, §11): the API traits
//! (`TxnApi`/`RawApi`/`AdminApi`/`RouterApi`), request routing, and the `Node` that
//! assembles the store, metadata plane, router, and executors into one process.

pub mod admission;
pub mod api;
pub mod client;
pub mod endpoints;
pub mod fence;
pub mod grpc;
pub mod node;
mod point_stream;
#[cfg(test)]
mod point_test_support;
mod point_wire;
pub mod routing;
#[cfg(feature = "rpc-experiment")]
pub mod rpc_experiment;
pub mod runtime;
pub mod workload;

pub use api::{
    AdminApi, AppliedPosition, ClusterInfo, CreateKeyspaceResult, RawApi, RegionLocation,
    RequestContext, RequestOrigin, RouterApi, TxnApi,
};
pub use grpc::proto;
pub use grpc::{
    admit_node_blocking, create_keyspace_blocking, promote_node_blocking, AuthContext, AuthKind,
    Authenticator, Kv9Grpc, PublicApiBackend, RawClient, RawClientOutcome, TokenAuthenticator,
    LEADER_HINT_KEY, NOT_LEADER_KEY,
};
pub use node::{MetaPlane, MetaRaft, Node, Store};
pub use routing::{route_request, Routed};
pub use runtime::{NodeRuntime, RuntimeAuth};

mod observability;
mod remote_storage;
