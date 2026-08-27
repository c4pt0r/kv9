//! Node assembly (DESIGN §3.5, §4).
//!
//! One `kv9` process = one `Node`, which simultaneously hosts a Store (engine + raft),
//! participates in the metadata plane (catalog, routing, bootstrap, TSO pool), serves
//! the router, and runs the txn/raw executors. One binary, one node type; roles are
//! behaviors, not deployables.

use std::sync::Mutex;

use kv9_common::{
    ApiType, Config, KeyspaceId, NodeId, Result, TenantId, TxnGroupId,
};
use kv9_engine::MemEngine;
use kv9_meta::{Bootstrap, Catalog, Membership, RoutingTable, Scheduler, TsoPool};
use kv9_region::RegionRouter;
use kv9_txn::{PercolatorExecutor, RawExecutor};

/// The metadata-plane state co-located in this node (DESIGN §5). In a running cluster
/// this is backed by the system keyspace's Raft groups; the skeleton holds it in memory.
pub struct MetaPlane {
    pub membership: Membership,
    pub catalog: Catalog,
    pub routing: RoutingTable,
    pub scheduler: Scheduler,
    pub tso: TsoPool,
    pub bootstrap: Bootstrap,
}

impl MetaPlane {
    pub fn new(node: NodeId) -> Self {
        MetaPlane {
            membership: Membership::new(),
            catalog: Catalog::new(),
            routing: RoutingTable::new(),
            scheduler: Scheduler::new(),
            tso: TsoPool::new(),
            bootstrap: Bootstrap::new(node),
        }
    }
}

/// The data-plane store on this node (DESIGN §3.5, §6). The skeleton uses a single
/// [`MemEngine`]; a real store owns per-region engines + raft state.
pub struct Store {
    pub engine: MemEngine,
}

impl Store {
    pub fn new() -> Self {
        Store {
            engine: MemEngine::new(),
        }
    }
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

/// One assembled `kv9` node (DESIGN §3.5, §4).
pub struct Node {
    pub id: NodeId,
    pub config: Config,
    pub store: Store,
    /// Metadata plane guarded for interior mutability during bootstrap/serving.
    pub meta: Mutex<MetaPlane>,
    /// Client-side routing cache (DESIGN §5.4).
    pub router: Mutex<RegionRouter>,
    pub txn: PercolatorExecutor,
    pub raw: RawExecutor,
}

impl Node {
    /// Assemble a node from config (DESIGN §4, §11). Does not yet run bootstrap; call
    /// [`Node::bootstrap`] to drive the election-first state machine.
    pub fn new(id: NodeId, config: Config) -> Result<Self> {
        config.validate()?;
        Ok(Node {
            id,
            config,
            store: Store::new(),
            meta: Mutex::new(MetaPlane::new(id)),
            router: Mutex::new(RegionRouter::new()),
            txn: PercolatorExecutor::new(),
            raw: RawExecutor::new(),
        })
    }

    /// Drive the election-first bootstrap to `Serving` (DESIGN §5.2). Skeleton: for a
    /// seedless single node, discovery finds the cluster uninitialized, this node wins
    /// the (trivial) election and initializes the default tenant + system keyspace.
    pub fn bootstrap(&self) -> Result<()> {
        use kv9_meta::BootstrapEvent::*;
        let mut meta = self.meta.lock().expect("meta poisoned");
        if self.config.join.is_empty() {
            // Uninitialized single node: elect self, initialize.
            meta.bootstrap.on_event(FoundUninitialized)?;
            meta.bootstrap.on_event(WonElection)?;
            self.initialize_metadata(&mut meta)?;
            meta.bootstrap.on_event(MetadataInitialized)?;
        } else {
            // A real join path contacts the seed set; skeleton treats it as initialized.
            meta.bootstrap.on_event(FoundInitialized)?;
            meta.bootstrap.on_event(Registered)?;
        }
        Ok(())
    }

    /// Write the initial metadata as the winner (DESIGN §5.2): default tenant, system
    /// keyspace, and the declared txn groups' default TSO windows.
    fn initialize_metadata(&self, meta: &mut MetaPlane) -> Result<()> {
        use kv9_common::Tenant;
        meta.catalog.upsert_tenant(Tenant {
            id: TenantId::DEFAULT,
            name: "default".into(),
            read_capacity_units: 0,
            write_capacity_units: 0,
        });
        // The reserved system keyspace (DESIGN §5).
        meta.catalog.create_keyspace(
            KeyspaceId::SYSTEM,
            "system",
            TenantId::DEFAULT,
            // The system keyspace is served like a txn keyspace in the default group.
            ApiType::Txn,
            TxnGroupId::DEFAULT,
        )?;
        Ok(())
    }
}
