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
use kv9_meta::codec::{memcmp_uint, ColumnValue, RowValue};
use kv9_meta::schema::{
    ColumnId, KEYSPACES_DESC, NODES_DESC, REGIONS_DESC, REGION_PEERS_DESC, SCHEMA_VERSION,
    SCHEMA_VERSION_DESC, TENANTS_DESC, TSO_TIMELINES_DESC, TXN_GROUPS_DESC,
};
use kv9_meta::store::SequenceKind;
use kv9_meta::tables::{Keyspace as KeyspaceRow, TxnGroup};
use kv9_meta::{Bootstrap, MetaStore};
use kv9_raft::{drive_apply, Command, MemStateMachine, RaftGroup, SingleNodeRaft};
use kv9_region::RegionRouter;
use kv9_txn::{PercolatorExecutor, RawExecutor};

/// The metadata-plane lifecycle state co-located in this node (DESIGN §5). Catalog,
/// membership, and routing rows have a single authority in [`MetaRaft::store`].
pub struct MetaPlane {
    pub bootstrap: Bootstrap,
}

impl MetaPlane {
    pub fn new(node: NodeId) -> Self {
        MetaPlane {
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
/// KV. `// TODO(phase1): back by tikv/raft-rs`.
pub struct MetaRaft {
    pub raft: SingleNodeRaft,
    pub sm: Mutex<MemStateMachine<MemEngine>>,
    pub store: MetaStore<MemEngine>,
    /// Serializes catalog transaction construction through committed apply. Raft orders
    /// commands, but it cannot repair two overlays that both read the same sequence value
    /// before either proposal is submitted.
    catalog_txn: Mutex<()>,
}

impl MetaRaft {
    /// Wire the meta-region raft group + state machine + catalog store over `engine`.
    pub fn new(node: NodeId, engine: Arc<MemEngine>) -> Self {
        MetaRaft {
            raft: SingleNodeRaft::new(node, META_REGION_0),
            sm: Mutex::new(MemStateMachine::with_engine(engine.clone())),
            store: MetaStore::new(engine),
            catalog_txn: Mutex::new(()),
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
    /// Allocation of the concrete [`KeyspaceId`]/[`TxnGroupId`] uses system sequence
    /// rows in the same transaction, so the bumps and rows are one replicated batch.
    pub fn create_keyspace(
        &self,
        name: &str,
        tenant: TenantId,
        api_type: ApiType,
    ) -> Result<KeyspaceId> {
        let _txn_guard = self
            .meta_raft
            .catalog_txn
            .lock()
            .expect("catalog transaction lock poisoned");
        let mut txn = self.meta_raft.store.begin();
        let raw_ks_id = txn.allocate_id(SequenceKind::Keyspace)?;
        let encoded_ks_id = u32::try_from(raw_ks_id).map_err(|_| {
            Error::MetaNotReady("keyspace id sequence exhausted its u32 representation".into())
        })?;
        if encoded_ks_id > KeyspaceId::MAX {
            return Err(Error::KeyspaceIdOutOfRange(encoded_ks_id));
        }
        let ks_id = KeyspaceId(encoded_ks_id);

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
                id: TxnGroupId(txn.allocate_id(SequenceKind::TxnGroup)?),
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
            self.initialize_metadata()?;
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
    fn initialize_metadata(&self) -> Result<()> {
        let _txn_guard = self
            .meta_raft
            .catalog_txn
            .lock()
            .expect("catalog transaction lock poisoned");
        let mut txn = self.meta_raft.store.begin();

        txn.insert(
            &TENANTS_DESC,
            &[memcmp_uint(TenantId::DEFAULT.0)],
            tenant_row(TenantId::DEFAULT, "default"),
        )?;

        let system = KeyspaceRow {
            id: KeyspaceId::SYSTEM,
            name: "system".into(),
            tenant_id: TenantId::DEFAULT,
            api_type: ApiType::Txn,
            start_key: Vec::new(),
            end_key: Vec::new(),
            state: 0,
            config: Vec::new(),
        };
        txn.insert(&KEYSPACES_DESC, &system.pk(), system.to_row_value())?;

        let default_group = TxnGroup {
            id: TxnGroupId::DEFAULT,
            keyspace_id: KeyspaceId::SYSTEM,
            name: "default".into(),
            sub_start: Vec::new(),
            sub_end: Vec::new(),
        };
        txn.insert(
            &TXN_GROUPS_DESC,
            &default_group.pk(),
            default_group.to_row_value(),
        )?;

        txn.insert(&NODES_DESC, &[memcmp_uint(self.id.0)], node_row(self.id))?;
        txn.insert(
            &TSO_TIMELINES_DESC,
            &[memcmp_uint(0)],
            timeline_row(self.id),
        )?;
        txn.insert(
            &REGIONS_DESC,
            &[memcmp_uint(META_REGION_0.0)],
            meta_region_row(self.id),
        )?;
        txn.insert(
            &REGION_PEERS_DESC,
            &[memcmp_uint(META_REGION_0.0), memcmp_uint(self.id.0)],
            region_peer_row(self.id),
        )?;
        txn.insert(
            &SCHEMA_VERSION_DESC,
            &[memcmp_uint(0)],
            schema_version_row(),
        )?;

        self.meta_raft
            .propose_apply(Command::from_batch(&txn.into_batch()))?;
        Ok(())
    }
}

fn tenant_row(id: TenantId, name: &str) -> RowValue {
    let mut row = RowValue::new();
    row.set(ColumnId(1), ColumnValue::Uint(id.0));
    row.set(ColumnId(2), ColumnValue::Text(name.into()));
    row.set(ColumnId(3), ColumnValue::Uint(0));
    row.set(ColumnId(4), ColumnValue::Uint(0));
    row
}

fn node_row(id: NodeId) -> RowValue {
    let mut row = RowValue::new();
    row.set(ColumnId(1), ColumnValue::Uint(id.0));
    row.set(ColumnId(2), ColumnValue::Text(String::new()));
    row.set(ColumnId(3), ColumnValue::Uint(0));
    row.set(ColumnId(4), ColumnValue::Uint(0));
    row.set(ColumnId(5), ColumnValue::Bytes(Vec::new()));
    row
}

fn timeline_row(provider: NodeId) -> RowValue {
    let mut row = RowValue::new();
    row.set(ColumnId(1), ColumnValue::Uint(0));
    row.set(ColumnId(2), ColumnValue::Uint(TxnGroupId::DEFAULT.0));
    row.set(ColumnId(3), ColumnValue::Uint(provider.0));
    row.set(ColumnId(4), ColumnValue::Uint(0));
    row
}

fn meta_region_row(leader: NodeId) -> RowValue {
    let mut row = RowValue::new();
    row.set(ColumnId(1), ColumnValue::Uint(META_REGION_0.0));
    row.set(ColumnId(2), ColumnValue::Uint(KeyspaceId::SYSTEM.0 as u64));
    row.set(ColumnId(3), ColumnValue::Bytes(Vec::new()));
    row.set(ColumnId(4), ColumnValue::Bytes(Vec::new()));
    row.set(ColumnId(5), ColumnValue::Uint(1));
    row.set(ColumnId(6), ColumnValue::Uint(1));
    row.set(ColumnId(7), ColumnValue::Uint(leader.0));
    row
}

fn region_peer_row(node: NodeId) -> RowValue {
    let mut row = RowValue::new();
    row.set(ColumnId(1), ColumnValue::Uint(META_REGION_0.0));
    row.set(ColumnId(2), ColumnValue::Uint(node.0));
    row.set(ColumnId(3), ColumnValue::Uint(0));
    row
}

fn schema_version_row() -> RowValue {
    let mut row = RowValue::new();
    row.set(ColumnId(1), ColumnValue::Uint(0));
    row.set(ColumnId(2), ColumnValue::Uint(SCHEMA_VERSION as u64));
    row
}

/// The admin / meta API over a node (DESIGN §11; METADATA-CATALOG §4). Authenticated
/// from day one; Phase-1 wires bootstrap, create/list/get, and cluster-info reads through
/// the catalog engine + raft. Region splitting remains a typed later-phase stub.
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
        use std::collections::BTreeMap;

        let txn = self.meta_raft.store.begin();
        let mut groups = BTreeMap::new();
        for row in txn.scan(&TXN_GROUPS_DESC, usize::MAX)? {
            groups.insert(
                uint_column(&row.value, ColumnId(2))?,
                TxnGroupId(uint_column(&row.value, ColumnId(1))?),
            );
        }

        txn.scan(&KEYSPACES_DESC, usize::MAX)?
            .into_iter()
            .map(|row| {
                let raw_id = uint_column(&row.value, ColumnId(1))?;
                let id = u32::try_from(raw_id).map_err(|_| {
                    Error::MalformedKey(format!("catalog keyspace id {raw_id} exceeds u32"))
                })?;
                let api_type =
                    kv9_meta::tables::api_type_from_code(uint_column(&row.value, ColumnId(4))?);
                let txn_group = match api_type {
                    ApiType::Raw => TxnGroupId::DEFAULT,
                    ApiType::Txn => *groups.get(&raw_id).ok_or_else(|| {
                        Error::MetaNotReady(format!("txn keyspace {id} has no catalog txn group"))
                    })?,
                };
                Ok(kv9_common::Keyspace {
                    id: KeyspaceId(id),
                    name: text_column(&row.value, ColumnId(2))?,
                    tenant: TenantId(uint_column(&row.value, ColumnId(3))?),
                    api_type,
                    txn_group,
                })
            })
            .collect()
    }

    fn get_region(
        &self,
        _caller: &str,
        keyspace: KeyspaceId,
        key: &[u8],
    ) -> Result<crate::api::RegionLocation> {
        let tables = kv9_meta::tables::Tables::new(&self.meta_raft.store);
        let region = tables
            .region_for_key(keyspace, key)?
            .ok_or(Error::RegionNotFound)?;
        Ok(crate::api::RegionLocation {
            region: region.id,
            epoch: kv9_region::RegionEpoch {
                conf_ver: region.epoch_conf,
                version: region.epoch_ver,
            },
            leader: Some(region.leader_node),
        })
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
        let txn = self.meta_raft.store.begin();
        Ok(crate::api::ClusterInfo {
            node_count: txn.scan(&NODES_DESC, usize::MAX)?.len(),
            keyspace_count: txn.scan(&KEYSPACES_DESC, usize::MAX)?.len(),
            region_count: txn.scan(&REGIONS_DESC, usize::MAX)?.len(),
        })
    }
}

fn uint_column(row: &RowValue, column: ColumnId) -> Result<u64> {
    match row.get(column) {
        Some(ColumnValue::Uint(value)) => Ok(*value),
        _ => Err(Error::MalformedKey(format!(
            "catalog column {} is missing or not uint",
            column.0
        ))),
    }
}

fn text_column(row: &RowValue, column: ColumnId) -> Result<String> {
    match row.get(column) {
        Some(ColumnValue::Text(value)) => Ok(value.clone()),
        _ => Err(Error::MalformedKey(format!(
            "catalog column {} is missing or not text",
            column.0
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::AdminApi;

    /// Phase-1 milestone (ROADMAP): a node bootstraps election-first, then a
    /// `CreateKeyspace` flows through the catalog engine + meta-region raft.
    ///
    #[test]
    fn bootstrap_elect_then_create_keyspace() {
        let node = Node::new(NodeId(1), Config::default()).unwrap();
        node.bootstrap().unwrap();

        let ks = node
            .create_keyspace("app", TenantId::DEFAULT, ApiType::Txn)
            .expect("create_keyspace should succeed");

        // The keyspace row must be readable back through the catalog engine.
        let store = &node.meta_raft.store;
        let tables = kv9_meta::tables::Tables::new(store);
        let got = tables.keyspace(ks).unwrap();
        assert!(got.is_some(), "created keyspace should be queryable");

        let listed = AdminApi::list_keyspaces(&node, "test").unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(
            listed
                .iter()
                .find(|item| item.id == ks)
                .unwrap()
                .txn_group
                .0,
            100
        );

        let info = AdminApi::cluster_info(&node, "test").unwrap();
        assert_eq!(info.node_count, 1);
        assert_eq!(info.keyspace_count, 2);
        assert_eq!(info.region_count, 1);

        let location = AdminApi::get_region(&node, "test", KeyspaceId::SYSTEM, b"key").unwrap();
        assert_eq!(location.region, META_REGION_0);
        assert_eq!(location.leader, Some(NodeId(1)));
    }

    #[test]
    fn later_phase_split_returns_typed_error() {
        let node = Node::new(NodeId(1), Config::default()).unwrap();

        assert!(matches!(
            AdminApi::split_region(&node, "test", META_REGION_0, b"split".to_vec()),
            Err(Error::NotImplemented(_))
        ));
    }

    #[test]
    fn create_keyspace_constraints_and_raw_group_semantics() {
        let node = Node::new(NodeId(1), Config::default()).unwrap();
        node.bootstrap().unwrap();

        let txn_id = node
            .create_keyspace("app", TenantId::DEFAULT, ApiType::Txn)
            .unwrap();
        assert!(matches!(
            node.create_keyspace("app", TenantId::DEFAULT, ApiType::Raw),
            Err(Error::WriteConflict(_))
        ));

        // The failed duplicate transaction did not commit its sequence bump.
        let raw_id = node
            .create_keyspace("raw", TenantId::DEFAULT, ApiType::Raw)
            .unwrap();
        assert_eq!(txn_id.0, 100);
        assert_eq!(raw_id.0, 101);

        let tables = kv9_meta::tables::Tables::new(&node.meta_raft.store);
        assert_eq!(
            tables.txn_group_for_key(txn_id, b"key").unwrap(),
            Some(TxnGroupId(100))
        );
        assert_eq!(tables.txn_group_for_key(raw_id, b"key").unwrap(), None);
        assert!(matches!(
            AdminApi::get_region(&node, "test", raw_id, b"key"),
            Err(Error::RegionNotFound)
        ));
    }

    #[test]
    fn concurrent_creates_allocate_distinct_ids() {
        let node = Arc::new(Node::new(NodeId(1), Config::default()).unwrap());
        node.bootstrap().unwrap();

        let handles: Vec<_> = (0..8)
            .map(|i| {
                let node = Arc::clone(&node);
                std::thread::spawn(move || {
                    node.create_keyspace(&format!("raw-{i}"), TenantId::DEFAULT, ApiType::Raw)
                        .unwrap()
                })
            })
            .collect();
        let mut ids: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap().0)
            .collect();
        ids.sort_unstable();
        assert_eq!(ids, (100..108).collect::<Vec<_>>());
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
