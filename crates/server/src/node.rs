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
use kv9_engine::{Engine, MemEngine};
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

/// The data-plane store on this node (DESIGN §3.5, §6). The engine is generic so the
/// deterministic harness can keep [`MemEngine`] while the real-process runtime uses
/// the Phase-1 persistent engine. A later real store owns per-region engines.
///
/// The **same** engine backs the raft state machine ([`MemStateMachine`]) *and* the
/// metadata catalog engine ([`MetaStore`]) so a committed `Command::CatalogTxn` applied
/// by raft is immediately visible to catalog reads (ROADMAP Phase 1).
pub struct Store<E: Engine = MemEngine> {
    pub engine: Arc<E>,
}

impl Store<MemEngine> {
    pub fn new() -> Self {
        Self::with_engine(Arc::new(MemEngine::new()))
    }
}

impl<E: Engine> Store<E> {
    pub fn with_engine(engine: Arc<E>) -> Self {
        Store { engine }
    }
}

impl Default for Store<MemEngine> {
    fn default() -> Self {
        Self::new()
    }
}

/// The Phase-1 raft spine for the system-keyspace group `META_REGION_0` (ROADMAP Phase 1):
/// a [`RaftGroup`] (single-node stub) whose committed [`Command`]s are applied into a
/// [`MemStateMachine`] sharing the store's engine, with a [`MetaStore`] reading that same
/// KV. `// TODO(phase1): back by tikv/raft-rs`.
pub struct MetaRaft<E: Engine = MemEngine> {
    pub raft: Arc<dyn RaftGroup>,
    pub sm: Mutex<MemStateMachine<E>>,
    pub store: MetaStore<E>,
    /// Serializes catalog transaction construction through committed apply. Raft orders
    /// commands, but it cannot repair two overlays that both read the same sequence value
    /// before either proposal is submitted.
    catalog_txn: Mutex<()>,
}

impl MetaRaft<MemEngine> {
    /// Wire the meta-region raft group + state machine + catalog store over `engine`.
    pub fn new(node: NodeId, engine: Arc<MemEngine>) -> Result<Self> {
        Self::with_raft(Arc::new(SingleNodeRaft::new(node, META_REGION_0)), engine)
    }
}

impl<E: Engine> MetaRaft<E> {
    pub(crate) fn lock_catalog_txn(&self) -> std::sync::MutexGuard<'_, ()> {
        self.catalog_txn
            .lock()
            .expect("catalog transaction lock poisoned")
    }

    /// Wire the state machine/catalog around an externally driven raft peer and a shared
    /// engine. The deterministic harness supplies [`MemEngine`]; the process runtime
    /// supplies the durable Phase-1 engine.
    pub fn with_raft(raft: Arc<dyn RaftGroup>, engine: Arc<E>) -> Result<Self> {
        Ok(MetaRaft {
            raft,
            sm: Mutex::new(MemStateMachine::with_engine(engine.clone())?),
            store: MetaStore::new(engine),
            catalog_txn: Mutex::new(()),
        })
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
        let applied = drive_apply(self.raft.as_ref(), &mut *sm)?;
        if applied.is_empty() {
            return Err(Error::Raft(
                "proposal produced no committed entry to apply".into(),
            ));
        }
        Ok(())
    }
}

/// One assembled `kv9` node (DESIGN §3.5, §4).
pub struct Node<E: Engine = MemEngine> {
    pub id: NodeId,
    pub config: Config,
    pub store: Store<E>,
    /// Metadata plane guarded for interior mutability during bootstrap/serving.
    pub meta: Mutex<MetaPlane>,
    /// The system-keyspace raft group + catalog engine (ROADMAP Phase 1).
    pub meta_raft: MetaRaft<E>,
    /// Client-side routing cache (DESIGN §5.4).
    pub router: Mutex<RegionRouter>,
    pub txn: PercolatorExecutor,
    pub raw: RawExecutor,
}

impl Node<MemEngine> {
    /// Assemble a node from config (DESIGN §4, §11). Does not yet run bootstrap; call
    /// [`Node::bootstrap`] to drive the election-first state machine.
    pub fn new(id: NodeId, config: Config) -> Result<Self> {
        Self::with_raft(id, config, Arc::new(SingleNodeRaft::new(id, META_REGION_0)))
    }

    /// Assemble a node around a supplied meta-region peer (used by the in-process
    /// raft-rs cluster and later by the real node bootstrap wiring).
    pub fn with_raft(id: NodeId, config: Config, raft: Arc<dyn RaftGroup>) -> Result<Self> {
        config.validate()?;
        let store = Store::new();
        Self::with_raft_and_engine(id, config, raft, store.engine)
    }
}

impl<E: Engine> Node<E> {
    /// Assemble a node around a supplied meta-region peer and engine. The same engine is
    /// shared by committed state-machine apply and MetaStore reads, so a restart cannot
    /// accidentally open a durable store while continuing to apply into a fresh
    /// in-memory store.
    pub fn with_raft_and_engine(
        id: NodeId,
        config: Config,
        raft: Arc<dyn RaftGroup>,
        engine: Arc<E>,
    ) -> Result<Self> {
        config.validate()?;
        let store = Store::with_engine(engine);
        let meta_raft = MetaRaft::with_raft(raft, store.engine.clone())?;
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
        let _txn_guard = self.meta_raft.lock_catalog_txn();
        let (ks_id, cmd) = self.build_create_keyspace_command(name, tenant, api_type)?;
        self.meta_raft.propose_apply(cmd)?;
        Ok(ks_id)
    }

    /// Build (but do not propose) the atomic catalog command for keyspace creation.
    /// The caller must hold `catalog_txn` until the command commits; the deterministic
    /// cluster harness uses this split to pump raft-rs explicitly.
    pub(crate) fn build_create_keyspace_command(
        &self,
        name: &str,
        tenant: TenantId,
        api_type: ApiType,
    ) -> Result<(KeyspaceId, Command)> {
        let mut txn = self.meta_raft.store.begin()?;
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
        Ok((ks_id, Command::from_batch(&txn.into_batch())))
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
    /// keyspace, and its fixed system transaction group and TSO timeline. User
    /// transaction groups are created with their owning keyspaces, not at node start.
    fn initialize_metadata(&self) -> Result<()> {
        let _txn_guard = self
            .meta_raft
            .catalog_txn
            .lock()
            .expect("catalog transaction lock poisoned");
        let cmd = self.build_initial_metadata_command()?;
        self.meta_raft.propose_apply(cmd)
    }

    /// Build the idempotent seed-row command. The elected bootstrap leader proposes
    /// this through raft; the 3-node harness pumps the resulting Ready entries.
    pub fn build_initial_metadata_command(&self) -> Result<Command> {
        self.build_initial_metadata_command_for(&[self.id])
    }

    /// Build the seed catalog command for the complete fixed Phase-1 voter set.
    /// Membership is declared before discovery; it must never be reconstructed
    /// from only the peers that happened to answer.
    pub fn build_initial_metadata_command_for(&self, voters: &[NodeId]) -> Result<Command> {
        let mut txn = self.meta_raft.store.begin()?;

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

        for voter in voters {
            txn.insert(&NODES_DESC, &[memcmp_uint(voter.0)], node_row(*voter))?;
        }
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
        for voter in voters {
            txn.insert(
                &REGION_PEERS_DESC,
                &[memcmp_uint(META_REGION_0.0), memcmp_uint(voter.0)],
                region_peer_row(*voter),
            )?;
        }
        txn.insert(
            &SCHEMA_VERSION_DESC,
            &[memcmp_uint(0)],
            schema_version_row(),
        )?;

        Ok(Command::from_batch(&txn.into_batch()))
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
impl<E: Engine> crate::api::AdminApi for Node<E> {
    fn create_keyspace(
        &self,
        _caller: &str,
        name: &str,
        tenant: TenantId,
        api_type: ApiType,
        _txn_group: TxnGroupId,
    ) -> Result<crate::api::CreateKeyspaceResult> {
        // The txn group is not a caller-supplied field: a `txn` keyspace's default group
        // is created for it (METADATA-CATALOG §2 corrected hierarchy).
        Ok(crate::api::CreateKeyspaceResult {
            keyspace: Node::create_keyspace(self, name, tenant, api_type)?,
            proposed: None,
        })
    }

    fn list_keyspaces(&self, _caller: &str) -> Result<Vec<kv9_common::Keyspace>> {
        use std::collections::BTreeMap;

        let txn = self.meta_raft.store.begin()?;
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
        let txn = self.meta_raft.store.begin()?;
        Ok(crate::api::ClusterInfo {
            node_count: txn.scan(&NODES_DESC, usize::MAX)?.len(),
            keyspace_count: txn.scan(&KEYSPACES_DESC, usize::MAX)?.len(),
            region_count: txn.scan(&REGIONS_DESC, usize::MAX)?.len(),
        })
    }
}

/// Data-plane adapters are intentionally thin: routing/epoch validation belongs
/// above the executors, while the executors own raw and Percolator semantics. The
/// Phase-1 executors still return typed `NotImplemented` errors; exposing them over
/// gRPC must preserve that error rather than panic or manufacture success.
impl<E: Engine> crate::api::RawApi for Node<E> {
    fn raw_get(&self, _ctx: &crate::api::RequestContext, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.raw.get(key)
    }

    fn raw_batch_get(
        &self,
        _ctx: &crate::api::RequestContext,
        keys: &[Vec<u8>],
    ) -> Result<Vec<Option<Vec<u8>>>> {
        keys.iter().map(|key| self.raw.get(key)).collect()
    }

    fn raw_put(
        &self,
        _ctx: &crate::api::RequestContext,
        key: Vec<u8>,
        value: Vec<u8>,
    ) -> Result<()> {
        self.raw
            .put(key, value, kv9_txn::RawWriteOptions::default())
    }

    fn raw_batch_put(
        &self,
        _ctx: &crate::api::RequestContext,
        kvs: &[(Vec<u8>, Vec<u8>)],
    ) -> Result<()> {
        for (key, value) in kvs {
            self.raw.put(
                key.clone(),
                value.clone(),
                kv9_txn::RawWriteOptions::default(),
            )?;
        }
        Ok(())
    }

    fn raw_delete(&self, _ctx: &crate::api::RequestContext, key: &[u8]) -> Result<()> {
        self.raw.delete(key)
    }

    fn raw_scan(
        &self,
        _ctx: &crate::api::RequestContext,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.raw.scan(start, end, limit)
    }

    fn raw_delete_range(
        &self,
        _ctx: &crate::api::RequestContext,
        start: &[u8],
        end: &[u8],
    ) -> Result<()> {
        self.raw.delete_range(start, end)
    }
}

impl<E: Engine> crate::api::TxnApi for Node<E> {
    fn kv_get(
        &self,
        ctx: &crate::api::RequestContext,
        key: &[u8],
        start_ts: kv9_common::TimeStamp,
    ) -> Result<Option<Vec<u8>>> {
        let txn_ctx = self.txn_context(ctx, key, start_ts)?;
        self.txn.get(&txn_ctx, key)
    }

    fn kv_batch_get(
        &self,
        ctx: &crate::api::RequestContext,
        keys: &[Vec<u8>],
        start_ts: kv9_common::TimeStamp,
    ) -> Result<Vec<Option<Vec<u8>>>> {
        let primary = keys
            .first()
            .ok_or_else(|| Error::WriteConflict("empty transaction key set".into()))?;
        let txn_ctx = self.txn_context(ctx, primary, start_ts)?;
        keys.iter().map(|key| self.txn.get(&txn_ctx, key)).collect()
    }

    fn kv_scan(
        &self,
        _ctx: &crate::api::RequestContext,
        _start: &[u8],
        _end: &[u8],
        _limit: usize,
        _start_ts: kv9_common::TimeStamp,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        Err(Error::NotImplemented("PercolatorExecutor::scan"))
    }

    fn kv_prewrite(
        &self,
        ctx: &crate::api::RequestContext,
        mutations: &[(Vec<u8>, Option<Vec<u8>>)],
        primary: &[u8],
        start_ts: kv9_common::TimeStamp,
    ) -> Result<()> {
        let txn_ctx = self.txn_context(ctx, primary, start_ts)?;
        let mutations = mutations
            .iter()
            .map(|(key, value)| match value {
                Some(value) => kv9_txn::TxnMutation::Put {
                    key: key.clone(),
                    value: value.clone(),
                },
                None => kv9_txn::TxnMutation::Delete { key: key.clone() },
            })
            .collect::<Vec<_>>();
        self.txn.prewrite(&txn_ctx, &mutations)
    }

    fn kv_commit(
        &self,
        ctx: &crate::api::RequestContext,
        keys: &[Vec<u8>],
        start_ts: kv9_common::TimeStamp,
        commit_ts: kv9_common::TimeStamp,
    ) -> Result<()> {
        let primary = keys
            .first()
            .ok_or_else(|| Error::WriteConflict("empty transaction key set".into()))?;
        let txn_ctx = self.txn_context(ctx, primary, start_ts)?;
        self.txn.commit(&txn_ctx, commit_ts, keys)
    }

    fn kv_pessimistic_lock(
        &self,
        _ctx: &crate::api::RequestContext,
        _keys: &[Vec<u8>],
        _start_ts: kv9_common::TimeStamp,
    ) -> Result<()> {
        Err(Error::NotImplemented(
            "PercolatorExecutor::pessimistic_lock",
        ))
    }

    fn kv_pessimistic_rollback(
        &self,
        _ctx: &crate::api::RequestContext,
        _keys: &[Vec<u8>],
        _start_ts: kv9_common::TimeStamp,
    ) -> Result<()> {
        Err(Error::NotImplemented(
            "PercolatorExecutor::pessimistic_rollback",
        ))
    }

    fn kv_resolve_lock(
        &self,
        _ctx: &crate::api::RequestContext,
        start_ts: kv9_common::TimeStamp,
        commit_ts: Option<kv9_common::TimeStamp>,
    ) -> Result<()> {
        self.txn.resolve_lock(start_ts, commit_ts)
    }

    fn kv_cleanup(
        &self,
        _ctx: &crate::api::RequestContext,
        _key: &[u8],
        _start_ts: kv9_common::TimeStamp,
    ) -> Result<()> {
        Err(Error::NotImplemented("PercolatorExecutor::cleanup"))
    }

    fn kv_check_txn_status(
        &self,
        _ctx: &crate::api::RequestContext,
        _primary: &[u8],
        _lock_ts: kv9_common::TimeStamp,
    ) -> Result<()> {
        Err(Error::NotImplemented(
            "PercolatorExecutor::check_txn_status",
        ))
    }
}

impl<E: Engine> Node<E> {
    fn txn_context(
        &self,
        ctx: &crate::api::RequestContext,
        primary: &[u8],
        start_ts: kv9_common::TimeStamp,
    ) -> Result<kv9_txn::TxnContext> {
        let tables = kv9_meta::tables::Tables::new(&self.meta_raft.store);
        let keyspace = tables
            .keyspace(ctx.keyspace)?
            .ok_or(Error::KeyspaceNotFound(ctx.keyspace))?;
        if keyspace.api_type != ApiType::Txn {
            return Err(Error::ApiTypeMismatch {
                keyspace: ctx.keyspace,
            });
        }
        let txn_group =
            tables
                .txn_group_for_key(ctx.keyspace, primary)?
                .ok_or(Error::ApiTypeMismatch {
                    keyspace: ctx.keyspace,
                })?;
        Ok(kv9_txn::TxnContext {
            start_ts,
            txn_group,
            primary: primary.to_vec(),
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
    use kv9_common::RegionId;
    use kv9_engine::WalEngine;
    use kv9_meta::{BootstrapEvent, BootstrapState};
    use kv9_raft::{CommittedEntry, InProcessCluster, ProposedAt, StateMachine};

    const N1: NodeId = NodeId(1);
    const N2: NodeId = NodeId(2);
    const N3: NodeId = NodeId(3);

    fn raft_nodes() -> (InProcessCluster, Vec<Node>) {
        let cluster = InProcessCluster::new(META_REGION_0, &[N1, N2, N3]).unwrap();
        let nodes = cluster
            .peers()
            .iter()
            .map(|peer| {
                let raft: Arc<dyn RaftGroup> = Arc::clone(peer) as Arc<dyn RaftGroup>;
                let node = Node::with_raft(peer.node_id(), Config::default(), raft).unwrap();
                node.meta.lock().unwrap().bootstrap =
                    Bootstrap::with_seeds(peer.node_id(), vec![N1, N2, N3]);
                node
            })
            .collect();
        (cluster, nodes)
    }

    fn node(nodes: &[Node], id: NodeId) -> &Node {
        nodes.iter().find(|node| node.id == id).unwrap()
    }

    /// Pump one raft round and apply every committed command to the corresponding
    /// node's real metadata state machine, returning the exact entries observed.
    fn pump_apply(cluster: &InProcessCluster, nodes: &[Node]) -> Vec<(NodeId, CommittedEntry)> {
        cluster.round();
        let mut observed = Vec::new();
        for peer in cluster.peers() {
            let entries = peer.take_ready().unwrap();
            let mut sm = node(nodes, peer.node_id())
                .meta_raft
                .sm
                .lock()
                .expect("meta sm poisoned");
            for entry in entries {
                // Barriers/conf entries never reach the state machine (the
                // production driver routes them); this harness feeds commands.
                if entry.kind != kv9_raft::EntryKind::Command {
                    continue;
                }
                sm.apply(&entry).unwrap();
                observed.push((peer.node_id(), entry));
            }
        }
        observed
    }

    /// Propose and deterministically wait until every target applies the exact
    /// `(term,index,payload)`. Reaching/passing the index with another entry never
    /// satisfies this helper (acceptance contract item 13).
    fn commit_exact(
        cluster: &InProcessCluster,
        nodes: &[Node],
        leader: NodeId,
        targets: &[NodeId],
        command: Command,
    ) -> ProposedAt {
        use std::collections::BTreeSet;

        let payload = command.encode();
        let at = cluster
            .peer(leader)
            .unwrap()
            .propose_traced(payload.clone())
            .unwrap();
        let mut matched = BTreeSet::new();
        for _ in 0..500 {
            for (node_id, entry) in pump_apply(cluster, nodes) {
                if entry.index == at.index {
                    assert_eq!(
                        entry.term, at.term,
                        "proposal position was overwritten by another term"
                    );
                    assert_eq!(entry.data, payload, "proposal payload mismatch");
                    matched.insert(node_id);
                }
            }
            if targets.iter().all(|target| matched.contains(target)) {
                return at;
            }
        }
        panic!("exact proposal was not applied on all targets");
    }

    fn test_region_row(id: RegionId, keyspace: KeyspaceId, start: &[u8], end: &[u8]) -> RowValue {
        let mut row = RowValue::new();
        row.set(ColumnId(1), ColumnValue::Uint(id.0));
        row.set(ColumnId(2), ColumnValue::Uint(keyspace.0 as u64));
        row.set(ColumnId(3), ColumnValue::Bytes(start.to_vec()));
        row.set(ColumnId(4), ColumnValue::Bytes(end.to_vec()));
        row.set(ColumnId(5), ColumnValue::Uint(1));
        row.set(ColumnId(6), ColumnValue::Uint(1));
        row.set(ColumnId(7), ColumnValue::Uint(N1.0));
        row
    }

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
    fn server_routing_preserves_overlay_and_exact_boundary_semantics() {
        let node = Node::new(N1, Config::default()).unwrap();
        node.bootstrap().unwrap();
        let keyspace = node
            .create_keyspace("routed", TenantId::DEFAULT, ApiType::Raw)
            .unwrap();

        // Seed adjacent regions through the same encoded raft apply path used by the
        // metadata server, then exercise the public Admin API at the exact boundary.
        let mut seed = node.meta_raft.store.begin().unwrap();
        seed.insert(
            &REGIONS_DESC,
            &[memcmp_uint(100)],
            test_region_row(RegionId(100), keyspace, b"a", b"b"),
        )
        .unwrap();
        seed.insert(
            &REGIONS_DESC,
            &[memcmp_uint(101)],
            test_region_row(RegionId(101), keyspace, b"b", b"c"),
        )
        .unwrap();
        node.meta_raft
            .propose_apply(Command::from_batch(&seed.into_batch()))
            .unwrap();
        let at_boundary = AdminApi::get_region(&node, "test", keyspace, b"b").unwrap();
        assert_eq!(at_boundary.region, RegionId(101));

        let tables = kv9_meta::tables::Tables::new(&node.meta_raft.store);
        let mut inserting = node.meta_raft.store.begin().unwrap();
        inserting
            .insert(
                &REGIONS_DESC,
                &[memcmp_uint(102)],
                test_region_row(RegionId(102), keyspace, b"c", b"d"),
            )
            .unwrap();
        assert_eq!(
            tables
                .region_for_key_in(&inserting, keyspace, b"cc")
                .unwrap()
                .map(|region| region.id),
            Some(RegionId(102))
        );
        assert!(tables.region_for_key(keyspace, b"cc").unwrap().is_none());

        // Deleting the view's best candidate must walk backward past the tombstone.
        // The previous region ends at "b", so "b" is a gap after the delete rather
        // than a false route to region 100.
        let mut deleting = node.meta_raft.store.begin().unwrap();
        deleting.delete(&REGIONS_DESC, &[memcmp_uint(101)]).unwrap();
        assert!(tables
            .region_for_key_in(&deleting, keyspace, b"b")
            .unwrap()
            .is_none());
        assert_eq!(
            tables
                .region_for_key_in(&deleting, keyspace, b"az")
                .unwrap()
                .map(|region| region.id),
            Some(RegionId(100))
        );
    }

    #[test]
    fn server_bootstrap_requires_seed_quorum_and_refuses_reinitialization() {
        let mut bootstrap = Bootstrap::with_seeds(N1, vec![N1, N2, N3]);
        assert!(bootstrap.discovered_uninitialized(&[N1]).is_err());
        assert!(bootstrap
            .discovered_uninitialized(&[N1, N1, NodeId(99)])
            .is_err());
        assert_eq!(bootstrap.state(), BootstrapState::Discovering);
        assert_eq!(
            bootstrap.discovered_uninitialized(&[N1, N2]).unwrap(),
            BootstrapState::BootstrapElection
        );
        bootstrap.on_event(BootstrapEvent::WonElection).unwrap();
        bootstrap
            .on_event(BootstrapEvent::MetadataInitialized)
            .unwrap();
        assert!(bootstrap.data_dir_initialized());

        // Model a restart that retained the initialized data-dir marker.
        let mut restarted = Bootstrap::with_seeds(N1, vec![N1, N2, N3]);
        restarted.mark_data_dir_initialized();
        assert!(restarted.discovered_uninitialized(&[N1, N2, N3]).is_err());
        assert_eq!(restarted.state(), BootstrapState::Discovering);
        assert_eq!(
            restarted
                .on_event(BootstrapEvent::FoundInitialized)
                .unwrap(),
            BootstrapState::Joining
        );
    }

    #[test]
    fn three_node_admin_e2e_failover_and_orphan_rejection() {
        let (cluster, nodes) = raft_nodes();

        // Discovery is fenced before any seed campaigns: silence is not evidence of
        // an empty cluster, while two positive answers form the declared 3-seed quorum.
        for replica in &nodes {
            let mut meta = replica.meta.lock().unwrap();
            assert!(meta
                .bootstrap
                .discovered_uninitialized(&[replica.id])
                .is_err());
            let corroborator = if replica.id == N1 { N2 } else { N1 };
            assert_eq!(
                meta.bootstrap
                    .discovered_uninitialized(&[replica.id, corroborator])
                    .unwrap(),
                BootstrapState::BootstrapElection
            );
        }
        cluster.peer(N1).unwrap().campaign().unwrap();
        cluster
            .run_until(500, "initial metadata leader", |cluster| {
                cluster.leader().is_some()
            })
            .unwrap();
        let leader1 = cluster.leader().unwrap();

        for replica in &nodes {
            let event = if replica.id == leader1 {
                BootstrapEvent::WonElection
            } else {
                BootstrapEvent::LostElection
            };
            replica
                .meta
                .lock()
                .unwrap()
                .bootstrap
                .on_event(event)
                .unwrap();
        }

        // Election-first bootstrap: only the elected leader constructs the seed batch;
        // all three replicas must apply that exact encoded command before any read.
        let seed = node(&nodes, leader1)
            .build_initial_metadata_command()
            .unwrap();
        commit_exact(&cluster, &nodes, leader1, &[N1, N2, N3], seed);

        for replica in &nodes {
            let mut meta = replica.meta.lock().unwrap();
            meta.bootstrap
                .on_event(BootstrapEvent::MetadataInitialized)
                .unwrap();
            if replica.id != leader1 {
                meta.bootstrap.on_event(BootstrapEvent::Registered).unwrap();
            }
            assert!(meta.bootstrap.is_serving());
            assert!(meta.bootstrap.data_dir_initialized());
        }

        let follower = [N1, N2, N3].into_iter().find(|id| *id != leader1).unwrap();
        let system = AdminApi::list_keyspaces(node(&nodes, follower), "test").unwrap();
        assert_eq!(system.len(), 1);
        assert_eq!(system[0].id, KeyspaceId::SYSTEM);

        let (txn_id, create_txn) = node(&nodes, leader1)
            .build_create_keyspace_command("app", TenantId::DEFAULT, ApiType::Txn)
            .unwrap();
        commit_exact(&cluster, &nodes, leader1, &[N1, N2, N3], create_txn);
        let follower_rows = AdminApi::list_keyspaces(node(&nodes, follower), "test").unwrap();
        assert!(follower_rows.iter().any(|row| row.id == txn_id));

        // Live leader failover: the surviving quorum elects and commits a raw keyspace.
        cluster.set_alive(leader1, false);
        cluster
            .run_until(500, "replacement metadata leader", |cluster| {
                cluster.leader().is_some_and(|leader| leader != leader1)
            })
            .unwrap();
        let leader2 = cluster.leader().unwrap();
        let survivors: Vec<_> = [N1, N2, N3]
            .into_iter()
            .filter(|id| *id != leader1)
            .collect();
        let (raw_id, create_raw) = node(&nodes, leader2)
            .build_create_keyspace_command("raw", TenantId::DEFAULT, ApiType::Raw)
            .unwrap();
        commit_exact(&cluster, &nodes, leader2, &survivors, create_raw);
        let survivor_follower = *survivors.iter().find(|id| **id != leader2).unwrap();
        let rows = AdminApi::list_keyspaces(node(&nodes, survivor_follower), "test").unwrap();
        assert!(rows
            .iter()
            .any(|row| row.id == raw_id && row.api_type == ApiType::Raw));

        // Strong item-13 scenario at the server/catalog layer. The isolated old leader
        // locally assigns a position to `orphan`, but cannot commit it. The live leader
        // consumes the same next catalog id for `durable`; after rejoin, position
        // progress must not make `orphan` appear successful.
        let (_, orphan) = node(&nodes, leader1)
            .build_create_keyspace_command("orphan", TenantId::DEFAULT, ApiType::Raw)
            .unwrap();
        let orphan_payload = orphan.encode();
        let orphan_at = cluster
            .peer(leader1)
            .unwrap()
            .propose_traced(orphan_payload.clone())
            .unwrap();

        let (_, durable) = node(&nodes, leader2)
            .build_create_keyspace_command("durable", TenantId::DEFAULT, ApiType::Raw)
            .unwrap();
        let durable_at = commit_exact(&cluster, &nodes, leader2, &survivors, durable);
        assert!(durable_at.term > orphan_at.term);

        cluster.set_alive(leader1, true);
        // A final exact write proves the rejoined peer caught up through the replacement
        // history, without treating the orphan's old position as success.
        let (_, after_rejoin) = node(&nodes, leader2)
            .build_create_keyspace_command("after-rejoin", TenantId::DEFAULT, ApiType::Raw)
            .unwrap();
        commit_exact(&cluster, &nodes, leader2, &[N1, N2, N3], after_rejoin);
        for replica in &nodes {
            let rows = AdminApi::list_keyspaces(replica, "test").unwrap();
            assert!(!rows.iter().any(|row| row.name == "orphan"));
            assert!(rows.iter().any(|row| row.name == "durable"));
        }
        assert!(cluster
            .peers()
            .iter()
            .all(|peer| peer.raft_committed() >= orphan_at.index));
    }

    #[test]
    fn meta_raft_applies_the_encoded_committed_command() {
        let engine = Arc::new(MemEngine::new());
        let meta = MetaRaft::new(NodeId(1), engine).unwrap();

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

    #[test]
    fn persistent_node_reopens_the_catalog_engine_used_by_apply() {
        let dir =
            std::env::temp_dir().join(format!("kv9-server-persistent-node-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let wal = dir.join("catalog.wal");

        {
            let (engine, _) = WalEngine::open(&wal).unwrap();
            let node = Node::with_raft_and_engine(
                N1,
                Config::default(),
                Arc::new(SingleNodeRaft::new(N1, META_REGION_0)),
                Arc::new(engine),
            )
            .unwrap();
            node.bootstrap().unwrap();
            assert_eq!(AdminApi::list_keyspaces(&node, "test").unwrap().len(), 1);
        }

        let (engine, replay) = WalEngine::open(&wal).unwrap();
        assert!(!replay.batches.is_empty());
        let reopened = Node::with_raft_and_engine(
            N1,
            Config::default(),
            Arc::new(SingleNodeRaft::new(N1, META_REGION_0)),
            Arc::new(engine),
        )
        .unwrap();
        let rows = AdminApi::list_keyspaces(&reopened, "test").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, KeyspaceId::SYSTEM);

        // Sensitivity control: a different WAL must not inherit the catalog.
        let (fresh_engine, _) = WalEngine::open(dir.join("fresh.wal")).unwrap();
        let fresh = Node::with_raft_and_engine(
            N1,
            Config::default(),
            Arc::new(SingleNodeRaft::new(N1, META_REGION_0)),
            Arc::new(fresh_engine),
        )
        .unwrap();
        assert!(AdminApi::list_keyspaces(&fresh, "test").unwrap().is_empty());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
