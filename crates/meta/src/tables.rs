//! Typed row structs + typed accessors/queries over [`MetaStore`] (METADATA-CATALOG §2, §4).
//!
//! These wrap the generic [`crate::store`] op API in table-specific types and the
//! **known joins** the scheduler/router need (METADATA-CATALOG §4), hand-written as
//! index-driven nested lookups. Respecting the corrected hierarchy:
//! **tenant → keyspace → txn group → timeline**, where `txn_groups.keyspace_id`
//! points *up* to `keyspaces` (a keyspace CONTAINS its txn groups; a `raw` keyspace has
//! none) — there is no duplicated "which group" field to drift.

use kv9_common::{
    ApiType, KeyspaceId, NodeId, RegionId, Result, TenantId, TimelineId, TxnGroupId,
};
use kv9_engine::Engine;

use crate::codec::{memcmp_uint, ColumnValue, RowValue};
use crate::schema::{self, ColumnId, IndexId};
use crate::store::MetaStore;

// ---------------------------------------------------------------------------
// Typed row structs (METADATA-CATALOG §2).
// ---------------------------------------------------------------------------

/// A `tenants` row (METADATA-CATALOG §2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tenant {
    pub id: TenantId,
    pub name: String,
    pub quota_tokens: u64,
    pub state: u32,
}

/// A `keyspaces` row (METADATA-CATALOG §2). `api_type` selects `txn`/`raw`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keyspace {
    pub id: KeyspaceId,
    pub name: String,
    pub tenant_id: TenantId,
    pub api_type: ApiType,
    pub start_key: Vec<u8>,
    pub end_key: Vec<u8>,
    pub state: u32,
    pub config: Vec<u8>,
}

/// A `txn_groups` row (METADATA-CATALOG §2). `keyspace_id` points **up** to the owning
/// keyspace; groups merely subdivide a `txn` keyspace to shard its TSO. ONLY for
/// `api_type = txn`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxnGroup {
    pub id: TxnGroupId,
    pub keyspace_id: KeyspaceId,
    pub name: String,
    pub sub_start: Vec<u8>,
    pub sub_end: Vec<u8>,
}

/// A `tso_timelines` row (METADATA-CATALOG §2). 1:1 with a txn group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsoTimeline {
    pub id: TimelineId,
    pub txn_group_id: TxnGroupId,
    pub provider_node: NodeId,
    pub window_hi: u64,
}

/// A `nodes` (membership) row (METADATA-CATALOG §2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub id: NodeId,
    pub addr: String,
    pub state: u32,
    pub last_heartbeat: u64,
    pub capacity: Vec<u8>,
}

/// A `regions` row (METADATA-CATALOG §2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub id: RegionId,
    pub keyspace_id: KeyspaceId,
    pub start_key: Vec<u8>,
    pub end_key: Vec<u8>,
    pub epoch_conf: u64,
    pub epoch_ver: u64,
    pub leader_node: NodeId,
}

/// A `region_peers` row (METADATA-CATALOG §2). PK is `(region_id, node_id)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionPeer {
    pub region_id: RegionId,
    pub node_id: NodeId,
    pub role: u32,
}

/// An `sst_files` row (METADATA-CATALOG §2). Carries the refcount the GC keys off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SstFile {
    pub file_id: u64,
    pub region_id: RegionId,
    pub level: u8,
    pub refcount: u32,
    pub smallest: Vec<u8>,
    pub biggest: Vec<u8>,
    pub bytes: u64,
}

// ---------------------------------------------------------------------------
// Row <-> RowValue conversions (a real impl would derive these). Phase-1 wires
// the two used on the CreateKeyspace path; the rest are declared for the team.
// ---------------------------------------------------------------------------

impl Keyspace {
    /// Encode into a tag-length [`RowValue`] (METADATA-CATALOG §3).
    pub fn to_row_value(&self) -> RowValue {
        let mut r = RowValue::new();
        r.set(ColumnId(1), ColumnValue::Uint(self.id.0 as u64));
        r.set(ColumnId(2), ColumnValue::Text(self.name.clone()));
        r.set(ColumnId(3), ColumnValue::Uint(self.tenant_id.0));
        r.set(ColumnId(4), ColumnValue::Uint(api_type_code(self.api_type)));
        r.set(ColumnId(5), ColumnValue::Bytes(self.start_key.clone()));
        r.set(ColumnId(6), ColumnValue::Bytes(self.end_key.clone()));
        r.set(ColumnId(7), ColumnValue::Uint(self.state as u64));
        r.set(ColumnId(8), ColumnValue::Bytes(self.config.clone()));
        r
    }

    /// The primary-key components for this keyspace (id).
    pub fn pk(&self) -> Vec<Vec<u8>> {
        vec![memcmp_uint(self.id.0 as u64)]
    }
}

impl TxnGroup {
    /// Encode into a tag-length [`RowValue`] (METADATA-CATALOG §3).
    pub fn to_row_value(&self) -> RowValue {
        let mut r = RowValue::new();
        r.set(ColumnId(1), ColumnValue::Uint(self.id.0));
        r.set(ColumnId(2), ColumnValue::Uint(self.keyspace_id.0 as u64));
        r.set(ColumnId(3), ColumnValue::Text(self.name.clone()));
        r.set(ColumnId(4), ColumnValue::Bytes(self.sub_start.clone()));
        r.set(ColumnId(5), ColumnValue::Bytes(self.sub_end.clone()));
        r
    }

    /// The primary-key components for this txn group (id).
    pub fn pk(&self) -> Vec<Vec<u8>> {
        vec![memcmp_uint(self.id.0)]
    }
}

/// Stable on-disk code for an [`ApiType`] (0 = txn, 1 = raw).
pub fn api_type_code(api: ApiType) -> u64 {
    match api {
        ApiType::Txn => 0,
        ApiType::Raw => 1,
    }
}

/// Inverse of [`api_type_code`].
pub fn api_type_from_code(code: u64) -> ApiType {
    match code {
        1 => ApiType::Raw,
        _ => ApiType::Txn,
    }
}

// ---------------------------------------------------------------------------
// Typed queries + the known joins (METADATA-CATALOG §4). Signatures real;
// bodies are Phase-1 stubs pending the codec pk-decode helpers.
// ---------------------------------------------------------------------------

/// Typed catalog accessors bound to a [`MetaStore`] (METADATA-CATALOG §4, §8).
pub struct Tables<'a, E: Engine> {
    store: &'a MetaStore<E>,
}

impl<'a, E: Engine> Tables<'a, E> {
    pub fn new(store: &'a MetaStore<E>) -> Self {
        Tables { store }
    }

    /// Point get: a keyspace by id (METADATA-CATALOG §4).
    pub fn keyspace(&self, id: KeyspaceId) -> Result<Option<Keyspace>> {
        let txn = self.store.begin();
        let row = txn.get(&schema::KEYSPACES_DESC, &[memcmp_uint(id.0 as u64)])?;
        Ok(row.map(|r| decode_keyspace(id, &r.value)))
    }

    /// Join — *keyspaces of tenant T* → `index_scan(keyspaces, by_tenant, T)`
    /// (METADATA-CATALOG §4).
    pub fn keyspaces_of_tenant(&self, tenant: TenantId) -> Result<Vec<KeyspaceId>> {
        let txn = self.store.begin();
        let pks = txn.index_scan(
            &schema::KEYSPACES_DESC,
            IndexId(1), // by_tenant
            &[memcmp_uint(tenant.0)],
            usize::MAX,
        )?;
        Ok(pks
            .into_iter()
            .filter_map(|pk| pk.first().map(|c| KeyspaceId(be_u64(c) as u32)))
            .collect())
    }

    /// Join — *regions on node N* → `index_scan(region_peers, by_node, N)` →
    /// `get(regions, region_id)` (METADATA-CATALOG §4).
    pub fn regions_on_node(&self, _node: NodeId) -> Result<Vec<Region>> {
        // TODO(phase1): index_scan(region_peers, by_node, N) → region_ids → get regions.
        unimplemented!("regions_on_node join (METADATA-CATALOG §4)")
    }

    /// Join — *which region owns key K in keyspace KS* → `index_scan(regions, by_range,
    /// (KS, ≤K))` last ≤ K, check end_key (METADATA-CATALOG §4).
    pub fn region_for_key(&self, _keyspace: KeyspaceId, _key: &[u8]) -> Result<Option<Region>> {
        // TODO(phase1): reverse-bounded index_scan on by_range, then end_key check.
        unimplemented!("region_for_key join (METADATA-CATALOG §4)")
    }

    /// Join — *txn group for key K in keyspace KS* → `index_scan(txn_groups,
    /// by_keyspace, KS)`, pick the sub-range ∋ K (METADATA-CATALOG §4).
    ///
    /// Default: the keyspace's single group; a `raw` keyspace has none (returns `None`).
    pub fn txn_group_for_key(&self, _keyspace: KeyspaceId, _key: &[u8]) -> Result<Option<TxnGroupId>> {
        // TODO(phase1): index_scan(txn_groups, by_keyspace, KS) → pick sub-range ∋ K.
        unimplemented!("txn_group_for_key join (METADATA-CATALOG §4)")
    }
}

/// Decode a `keyspaces` [`RowValue`] into the typed [`Keyspace`] (METADATA-CATALOG §3).
fn decode_keyspace(id: KeyspaceId, v: &RowValue) -> Keyspace {
    Keyspace {
        id,
        name: text_or(v, ColumnId(2)),
        tenant_id: TenantId(uint_or(v, ColumnId(3))),
        api_type: api_type_from_code(uint_or(v, ColumnId(4))),
        start_key: bytes_or(v, ColumnId(5)),
        end_key: bytes_or(v, ColumnId(6)),
        state: uint_or(v, ColumnId(7)) as u32,
        config: bytes_or(v, ColumnId(8)),
    }
}

fn uint_or(v: &RowValue, c: ColumnId) -> u64 {
    match v.get(c) {
        Some(ColumnValue::Uint(u)) => *u,
        _ => 0,
    }
}
fn text_or(v: &RowValue, c: ColumnId) -> String {
    match v.get(c) {
        Some(ColumnValue::Text(s)) => s.clone(),
        _ => String::new(),
    }
}
fn bytes_or(v: &RowValue, c: ColumnId) -> Vec<u8> {
    match v.get(c) {
        Some(ColumnValue::Bytes(b)) => b.clone(),
        _ => Vec::new(),
    }
}
fn be_u64(b: &[u8]) -> u64 {
    let mut buf = [0u8; 8];
    let n = b.len().min(8);
    buf[8 - n..].copy_from_slice(&b[..n]);
    u64::from_be_bytes(buf)
}
