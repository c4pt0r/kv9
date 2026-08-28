//! # kv9-server
//!
//! Node assembly and the request-serving surface (DESIGN §4, §11): the API traits
//! (`TxnApi`/`RawApi`/`AdminApi`/`RouterApi`), request routing, and the `Node` that
//! assembles the store, metadata plane, router, and executors into one process.

pub mod api;
pub mod grpc;
pub mod node;
pub mod routing;
pub mod runtime;

pub use api::{
    AdminApi, AppliedPosition, ClusterInfo, CreateKeyspaceResult, RawApi, RegionLocation,
    RequestContext, RouterApi, TxnApi,
};
pub use grpc::{
    admit_node_blocking, create_keyspace_blocking, promote_node_blocking, AuthContext, AuthKind,
    Authenticator, Kv9Grpc, PublicApiBackend, RawClient, RawClientOutcome, TokenAuthenticator,
    LEADER_HINT_KEY, NOT_LEADER_KEY,
};
pub use node::{MetaPlane, MetaRaft, Node, Store};
pub use routing::{route_request, Routed};
pub use runtime::{NodeRuntime, RuntimeAuth};
