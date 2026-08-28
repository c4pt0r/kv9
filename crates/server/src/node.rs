//! Node assembly (DESIGN §3.5, §4).
//!
//! One `kv9` process = one `Node`, which simultaneously hosts a Store (engine + raft),
//! participates in the metadata plane (catalog, routing, bootstrap, TSO pool), serves
//! the router, and runs the txn/raw executors. One binary, one node type; roles are
//! behaviors, not deployables.

use std::sync::{Arc, Mutex};

use kv9_common::{
    ApiType, Config, Error, KeyspaceId, NodeId, Result, TenantId, TxnGroupId, META_REGION_0,
};
use kv9_engine::MemEngine;
use kv9_meta::schema::{KEYSPACES_DESC, TXN_GROUPS_DESC};
use kv9_meta::tables::{Keyspace as KeyspaceRow, TxnGroup};
use kv9_meta::{Bootstrap, Catalog, Membership, MetaStore, RoutingTable, Scheduler, TsoPool};
use kv9_raft::{drive_apply, Command, MemStateMachine, RaftGroup, SingleNodeRaft};
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
/// shared [`MemEngine`]; a real store owns per-region engines + raft state.
///
/// The **same** engine backs the raft state machine ([`MemStateMachine`]) *and* the
/// metadata catalog engine ([`MetaStore`]) so a committed `Command::CatalogTxn` applied
/// by raft is immediately visible to catalog reads (ROADMAP Phase 1).
pub struct Store {
    pub engine: Arc<MemEngine>,
}

impl Store {
    pub fn new() -> Self {
        Store {
            engine: Arc::new(MemEngine::new()),
        }
    }
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

/// The Phase-1 raft spine for the system-keyspace group `META_REGION_0` (ROADMAP Phase 1):
/// a [`RaftGroup`] (single-node stub) whose committed [`Command`]s are applied into a
/// [`MemStateMachine`] sharing the store's engine, with a [`MetaStore`] reading that same
/// KV. `// TODO(phase1): back by openraft`.
pub struct MetaRaft {
    pub raft: SingleNodeRaft,
    pub sm: Mutex<MemStateMachine<MemEngine>>,
    pub store: MetaStore<MemEngine>,
}

impl MetaRaft {
    /// Wire the meta-region raft group + state machine + catalog store over `engine`.
    pub fn new(node: NodeId, engine: Arc<MemEngine>) -> Self {
        MetaRaft {
            raft: SingleNodeRaft::new(node, META_REGION_0),
            sm: Mutex::new(MemStateMachine::with_engine(engine.clone())),
            store: MetaStore::new(engine),
        }
    }

    /// Propose a command, then drain-and-apply the actual committed entries into the
    /// state machine (ROADMAP Phase 1: `encode → propose → commit → decode → apply`).
    ///
    /// The command applied here must be reconstructed from the committed log payload.
    /// Applying the caller's typed value directly would create state that a follower or
    /// restart could never replay from Raft.
    pub fn propose_apply(&self, cmd: Command) -> Result<()> {
        self.raft.propose(cmd.encode())?;
        let mut sm = self.sm.lock().expect("meta sm poisoned");
        let applied = drive_apply(&self.raft, &mut *sm)?;
        if applied.is_empty() {
            return Err(Error::Raft(
                "proposal produced no committed entry to apply".into(),
            ));
        }
        Ok(())
    }
}

/// One assembled `kv9` node (DESIGN §3.5, §4).
pub struct Node {
    pub id: NodeId,
    pub config: Config,
    pub store: Store,
    /// Metadata plane guarded for interior mutability during bootstrap/serving.
    pub meta: Mutex<MetaPlane>,
    /// The system-keyspace raft group + catalog engine (ROADMAP Phase 1).
    pub meta_raft: MetaRaft,
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
        let store = Store::new();
        let meta_raft = MetaRaft::new(id, store.engine.clone());
        Ok(Node {
            id,
            config,
            store,
            meta: Mutex::new(MetaPlane::new(id)),
            meta_raft,
            router: Mutex::new(RegionRouter::new()),
            txn: PercolatorExecutor::new(),
            raw: RawExecutor::new(),
        })
    }

    /// Create a keyspace end-to-end through the catalog engine (METADATA-CATALOG §4;
    /// ROADMAP Phase 1). Builds a [`kv9_meta::MetaTxn`] that inserts the `keyspaces` row
    /// (+ a default `txn_groups` row when `api_type = txn`), packages the buffered write
    /// batch as a `Command::CatalogTxn`, and commits it via the meta-region raft group so
    /// the mutation is replicated before it is applied.
    ///
    /// Allocation of the concrete [`KeyspaceId`]/[`TxnGroupId`] (via a system sequence)
    /// and UNIQUE/FK enforcement in the txn are Phase-1 `unimplemented!()`; the wiring
    /// and signatures are real.
    pub fn create_keyspace(
        &self,
        name: &str,
        tenant: TenantId,
        api_type: ApiType,
    ) -> Result<KeyspaceId> {
        // TODO(phase1): allocate ids from a system sequence table inside the same txn.
        let ks_id = self.allocate_keyspace_id(name, tenant, api_type)?;

        let mut txn = self.meta_raft.store.begin();

        let ks = KeyspaceRow {
            id: ks_id,
            name: name.to_string(),
            tenant_id: tenant,
            api_type,
            start_key: Vec::new(),
            end_key: Vec::new(),
            state: 0,
            config: Vec::new(),
        };
        txn.insert(&KEYSPACES_DESC, &ks.pk(), ks.to_row_value())?;

        // A `txn` keyspace gets its default txn group (the single-group default that
        // shards the TSO); a `raw` keyspace has none (METADATA-CATALOG §2).
        if api_type == ApiType::Txn {
            let group = TxnGroup {
                id: TxnGroupId::DEFAULT,
                keyspace_id: ks_id,
                name: "default".to_string(),
                sub_start: Vec::new(),
                sub_end: Vec::new(),
            };
            txn.insert(&TXN_GROUPS_DESC, &group.pk(), group.to_row_value())?;
        }

        // Package the atomic multi-table batch as a replicated catalog transaction and
        // commit it through raft (METADATA-CATALOG §5).
        let cmd = Command::from_batch(&txn.into_batch());
        self.meta_raft.propose_apply(cmd)?;
        Ok(ks_id)
    }

    /// Allocate a fresh keyspace id (Phase-1 stub — a real impl bumps a system sequence
    /// row inside the create txn, checking the name is UNIQUE first).
    fn allocate_keyspace_id(
        &self,
        _name: &str,
        _tenant: TenantId,
        _api_type: ApiType,
    ) -> Result<KeyspaceId> {
        Err(Error::NotImplemented(
            "Node::allocate_keyspace_id — system id sequence (METADATA-CATALOG §4)",
        ))
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

/// The admin / meta API over a node (DESIGN §11; METADATA-CATALOG §4). Authenticated
/// from day one; Phase-1 wires `create_keyspace` through the catalog engine + raft and
/// leaves the read/region ops as typed stubs.
impl crate::api::AdminApi for Node {
    fn create_keyspace(
        &self,
        _caller: &str,
        name: &str,
        tenant: TenantId,
        api_type: ApiType,
        _txn_group: TxnGroupId,
    ) -> Result<KeyspaceId> {
        // The txn group is not a caller-supplied field: a `txn` keyspace's default group
        // is created for it (METADATA-CATALOG §2 corrected hierarchy).
        Node::create_keyspace(self, name, tenant, api_type)
    }

    fn list_keyspaces(&self, _caller: &str) -> Result<Vec<kv9_common::Keyspace>> {
        // TODO(phase1): scan(keyspaces) via MetaStore and map rows to kv9_common::Keyspace.
        Err(Error::NotImplemented(
            "AdminApi::list_keyspaces — catalog scan (METADATA-CATALOG §4)",
        ))
    }

    fn get_region(
        &self,
        _caller: &str,
        _keyspace: KeyspaceId,
        _key: &[u8],
    ) -> Result<crate::api::RegionLocation> {
        // TODO(phase1): Tables::region_for_key join over MetaStore.
        Err(Error::NotImplemented(
            "AdminApi::get_region — region_for_key join (METADATA-CATALOG §4)",
        ))
    }

    fn split_region(
        &self,
        _caller: &str,
        _region: kv9_common::RegionId,
        _split_key: Vec<u8>,
    ) -> Result<()> {
        Err(Error::NotImplemented(
            "AdminApi::split_region (DESIGN §10, ROADMAP Phase 4)",
        ))
    }

    fn cluster_info(&self, _caller: &str) -> Result<crate::api::ClusterInfo> {
        let meta = self.meta.lock().expect("meta poisoned");
        Ok(crate::api::ClusterInfo {
            node_count: meta.membership.len(),
            keyspace_count: meta.catalog.list_keyspaces().count(),
            region_count: meta.routing.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::AdminApi;

    /// Phase-1 milestone (ROADMAP): a node bootstraps election-first, then a
    /// `CreateKeyspace` flows through the catalog engine + meta-region raft.
    ///
    /// Ignored until keyspace-id allocation (the system id sequence) lands; the wiring
    /// (`Node::create_keyspace` → `MetaTxn` → `Command::CatalogTxn` → raft apply) is real
    /// and compiles today.
    #[test]
    #[ignore = "phase1: pending keyspace-id allocation (system id sequence)"]
    fn bootstrap_elect_then_create_keyspace() {
        let node = Node::new(NodeId(1), Config::default()).unwrap();
        node.bootstrap().unwrap();

        let ks = node
            .create_keyspace("app", TenantId::DEFAULT, ApiType::Txn)
            .expect("create_keyspace should succeed once id allocation is implemented");

        // The keyspace row must be readable back through the catalog engine.
        let store = &node.meta_raft.store;
        let tables = kv9_meta::tables::Tables::new(store);
        let got = tables.keyspace(ks).unwrap();
        assert!(got.is_some(), "created keyspace should be queryable");
    }

    #[test]
    fn incomplete_admin_endpoints_return_typed_errors() {
        let node = Node::new(NodeId(1), Config::default()).unwrap();

        assert!(matches!(
            AdminApi::list_keyspaces(&node, "test"),
            Err(Error::NotImplemented(_))
        ));
        assert!(matches!(
            AdminApi::get_region(&node, "test", KeyspaceId::SYSTEM, b"key"),
            Err(Error::NotImplemented(_))
        ));
        assert!(matches!(
            AdminApi::split_region(&node, "test", META_REGION_0, b"split".to_vec()),
            Err(Error::NotImplemented(_))
        ));
    }

    #[test]
    fn meta_raft_applies_the_encoded_committed_command() {
        let engine = Arc::new(MemEngine::new());
        let meta = MetaRaft::new(NodeId(1), engine);

        meta.propose_apply(Command::Put {
            cf: 0,
            key: b"committed-key".to_vec(),
            value: b"committed-value".to_vec(),
        })
        .unwrap();

        let sm = meta.sm.lock().expect("meta sm poisoned");
        assert_eq!(
            sm.get(kv9_engine::ColumnFamily::Default, b"committed-key")
                .unwrap(),
            Some(b"committed-value".to_vec())
        );
    }
}
