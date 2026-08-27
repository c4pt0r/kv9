//! # kv9-server
//!
//! Node assembly and the request-serving surface (DESIGN §4, §11): the API traits
//! (`TxnApi`/`RawApi`/`AdminApi`/`RouterApi`), request routing, and the `Node` that
//! assembles the store, metadata plane, router, and executors into one process.

pub mod api;
pub mod node;
pub mod routing;

pub use api::{
    AdminApi, ClusterInfo, RawApi, RegionLocation, RequestContext, RouterApi, TxnApi,
};
pub use node::{MetaPlane, Node, Store};
pub use routing::{route_request, Routed};
