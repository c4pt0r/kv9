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

/// An `sst_files` row — the catalog's **GC/billing view only** (agreed design ruling).
///
/// Which files a region holds is answered by that region's raft-replicated manifest,
/// the single authority for LSM structure; this row exists for object-storage GC
/// (`refcount` as a conservative upper bound: +ref commits *before* the manifest
/// change referencing a file, −ref only *after* the change dropping it) and for
/// per-tenant billing (`keyspace_id` is unique and stable even when a split shares a
/// file across regions, because regions never span keyspaces; `bytes` is written once
/// at creation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SstFile {
    pub file_id: u64,
    pub keyspace_id: KeyspaceId,
    pub bytes: u64,
    pub refcount: u32,
    pub state: u32,
    pub created: u64,
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
        let txn = self.store.begin()?;
        let row = txn.get(&schema::KEYSPACES_DESC, &[memcmp_uint(id.0 as u64)])?;
        Ok(row.map(|r| decode_keyspace(id, &r.value)))
    }

    /// Join — *keyspaces of tenant T* → `index_scan(keyspaces, by_tenant, T)`
    /// (METADATA-CATALOG §4).
    pub fn keyspaces_of_tenant(&self, tenant: TenantId) -> Result<Vec<KeyspaceId>> {
        let txn = self.store.begin()?;
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
    pub fn regions_on_node(&self, node: NodeId) -> Result<Vec<Region>> {
        let txn = self.store.begin()?;
        let peer_pks = txn.index_scan(
            &schema::REGION_PEERS_DESC,
            IndexId(1), // by_node
            &[memcmp_uint(node.0)],
            usize::MAX,
        )?;
        let mut out = Vec::with_capacity(peer_pks.len());
        for pk in peer_pks {
            // region_peers pk = (region_id, node_id); the region id is the first comp.
            let Some(region_comp) = pk.first() else {
                continue;
            };
            let region_id = crate::codec::decode_uint_component(region_comp)?;
            if let Some(row) = txn.get(&schema::REGIONS_DESC, &[memcmp_uint(region_id)])? {
                out.push(decode_region(RegionId(region_id), &row.value));
            }
        }
        Ok(out)
    }

    /// Join — *which region owns key K in keyspace KS* → last `by_range` entry with
    /// `start_key ≤ K`, then the `end_key` check (METADATA-CATALOG §4).
    ///
    /// The `by_range` index key is `(keyspace_id, start_key)` prefix-encoded and the
    /// search is bounded below by the keyspace prefix, so it structurally cannot
    /// resolve into another keyspace. The `end_key` check still guards the gap case
    /// (K past the keyspace's last region): that returns `None`, never a neighbor —
    /// principle 4's tenant-isolation line.
    ///
    /// Phase-1 note: forward-scans the keyspace's index entries and keeps the last
    /// `≤ K` — O(regions-in-keyspace). Switches to the engine ReadView's `seek_le`
    /// (one consistent view for both steps) once that lands on this path.
    pub fn region_for_key(&self, keyspace: KeyspaceId, key: &[u8]) -> Result<Option<Region>> {
        let txn = self.store.begin()?;
        let entries = txn.index_entries(
            &schema::REGIONS_DESC,
            IndexId(1), // by_range
            &[memcmp_uint(keyspace.0 as u64)],
            usize::MAX,
        )?;
        // Entry key suffix = memcmp(keyspace_id) ++ memcmp(start_key) ++ memcmp(region_id).
        // Encoded order == logical order, so compare encoded start_key directly.
        let ks_comp = memcmp_uint(keyspace.0 as u64);
        let target = crate::codec::memcmp_bytes(key);
        let mut candidate: Option<(Vec<u8>, u64)> = None; // (encoded start_key, region_id)
        for (suffix, _) in entries {
            let rest = &suffix[ks_comp.len()..];
            let comps = crate::codec::split_components(
                &[crate::schema::ColumnType::Bytes, crate::schema::ColumnType::Uint],
                rest,
                true,
            )?;
            let start_enc = &comps[0];
            if start_enc.as_slice() <= target.as_slice() {
                let region_id = crate::codec::decode_uint_component(&comps[1])?;
                candidate = Some((start_enc.clone(), region_id));
            } else {
                break; // entries are in ascending start_key order
            }
        }
        let Some((_, region_id)) = candidate else {
            return Ok(None);
        };
        let Some(row) = txn.get(&schema::REGIONS_DESC, &[memcmp_uint(region_id)])? else {
            return Ok(None);
        };
        let region = decode_region(RegionId(region_id), &row.value);
        // end_key check: empty end_key = unbounded (to the keyspace's end).
        if region.end_key.is_empty() || key < region.end_key.as_slice() {
            Ok(Some(region))
        } else {
            Ok(None)
        }
    }

    /// Join — *txn group for key K in keyspace KS* → `index_scan(txn_groups,
    /// by_keyspace, KS)`, pick the sub-range ∋ K (METADATA-CATALOG §4).
    ///
    /// A `raw` keyspace has no groups (returns `None`); the default single group has
    /// empty `sub_start`/`sub_end` = the whole keyspace.
    pub fn txn_group_for_key(&self, keyspace: KeyspaceId, key: &[u8]) -> Result<Option<TxnGroupId>> {
        let txn = self.store.begin()?;
        let group_pks = txn.index_scan(
            &schema::TXN_GROUPS_DESC,
            IndexId(1), // by_keyspace
            &[memcmp_uint(keyspace.0 as u64)],
            usize::MAX,
        )?;
        for pk in group_pks {
            let Some(id_comp) = pk.first() else { continue };
            let group_id = crate::codec::decode_uint_component(id_comp)?;
            let Some(row) = txn.get(&schema::TXN_GROUPS_DESC, &[memcmp_uint(group_id)])? else {
                continue;
            };
            let sub_start = bytes_or(&row.value, ColumnId(4));
            let sub_end = bytes_or(&row.value, ColumnId(5));
            let ge_start = sub_start.is_empty() || key >= sub_start.as_slice();
            let lt_end = sub_end.is_empty() || key < sub_end.as_slice();
            if ge_start && lt_end {
                return Ok(Some(TxnGroupId(group_id)));
            }
        }
        Ok(None)
    }
}

/// Decode a `regions` [`RowValue`] into the typed [`Region`] (METADATA-CATALOG §3).
fn decode_region(id: RegionId, v: &RowValue) -> Region {
    Region {
        id,
        keyspace_id: KeyspaceId(uint_or(v, ColumnId(2)) as u32),
        start_key: bytes_or(v, ColumnId(3)),
        end_key: bytes_or(v, ColumnId(4)),
        epoch_conf: uint_or(v, ColumnId(5)),
        epoch_ver: uint_or(v, ColumnId(6)),
        leader_node: NodeId(uint_or(v, ColumnId(7))),
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
