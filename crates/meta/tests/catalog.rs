//! Catalog-engine integration tests over the public `MetaStore` API
//! (METADATA-CATALOG §4/§5; Phase-1 acceptance contract items 2, 4, 8, 11).

use std::sync::Arc;

use kv9_engine::MemEngine;
use kv9_meta::codec::{memcmp_text, memcmp_uint, ColumnValue, RowValue};
use kv9_meta::schema::{self, ColumnId, IndexId};
use kv9_meta::store::{MetaStore, SequenceKind, FIRST_DYNAMIC_ID};
use kv9_meta::tables::{Keyspace, Region, RegionPeer, Tables, Tenant, TxnGroup};
use kv9_common::{ApiType, KeyspaceId, NodeId, RegionId, TenantId};

fn store() -> MetaStore<MemEngine> {
    MetaStore::new(Arc::new(MemEngine::new()))
}

fn tenant_row(id: u64, name: &str) -> RowValue {
    let mut r = RowValue::new();
    r.set(ColumnId(1), ColumnValue::Uint(id));
    r.set(ColumnId(2), ColumnValue::Text(name.into()));
    r.set(ColumnId(3), ColumnValue::Uint(0));
    r.set(ColumnId(4), ColumnValue::Uint(0));
    r
}

fn keyspace_row(id: u32, name: &str, tenant: u64, api: ApiType) -> (Vec<Vec<u8>>, RowValue) {
    let ks = Keyspace {
        id: KeyspaceId(id),
        name: name.into(),
        tenant_id: TenantId(tenant),
        api_type: api,
        start_key: Vec::new(),
        end_key: Vec::new(),
        state: 0,
        config: Vec::new(),
    };
    (ks.pk(), ks.to_row_value())
}

fn region_row(id: u64, ks: u32, start: &[u8], end: &[u8]) -> RowValue {
    let mut r = RowValue::new();
    r.set(ColumnId(1), ColumnValue::Uint(id));
    r.set(ColumnId(2), ColumnValue::Uint(ks as u64));
    r.set(ColumnId(3), ColumnValue::Bytes(start.to_vec()));
    r.set(ColumnId(4), ColumnValue::Bytes(end.to_vec()));
    r.set(ColumnId(5), ColumnValue::Uint(1));
    r.set(ColumnId(6), ColumnValue::Uint(1));
    r.set(ColumnId(7), ColumnValue::Uint(0));
    r
}

/// Seed a tenant so keyspace FKs resolve.
fn seed_tenant(s: &MetaStore<MemEngine>) {
    let mut txn = s.begin().unwrap();
    txn.insert(&schema::TENANTS_DESC, &[memcmp_uint(1)], tenant_row(1, "default"))
        .unwrap();
    txn.commit().unwrap();
}

#[test]
fn insert_get_scan_roundtrip_with_pk_reconstruction() {
    let s = store();
    seed_tenant(&s);
    let mut txn = s.begin().unwrap();
    let (pk, row) = keyspace_row(7, "ks-a", 1, ApiType::Txn);
    txn.insert(&schema::KEYSPACES_DESC, &pk, row).unwrap();
    // Read-your-writes: visible before commit.
    assert!(txn.get(&schema::KEYSPACES_DESC, &pk).unwrap().is_some());
    txn.commit().unwrap();

    let txn = s.begin().unwrap();
    let rows = txn.scan(&schema::KEYSPACES_DESC, usize::MAX).unwrap();
    assert_eq!(rows.len(), 1);
    // scan reconstructs the pk from the physical key.
    assert_eq!(rows[0].pk, pk);
}

#[test]
fn duplicate_pk_rejected() {
    let s = store();
    seed_tenant(&s);
    let mut txn = s.begin().unwrap();
    let (pk, row) = keyspace_row(7, "ks-a", 1, ApiType::Raw);
    txn.insert(&schema::KEYSPACES_DESC, &pk, row.clone()).unwrap();
    assert!(txn.insert(&schema::KEYSPACES_DESC, &pk, row).is_err());
}

#[test]
fn unique_name_index_rejects_duplicates_across_txns() {
    let s = store();
    seed_tenant(&s);
    let mut txn = s.begin().unwrap();
    let (pk, row) = keyspace_row(7, "same-name", 1, ApiType::Raw);
    txn.insert(&schema::KEYSPACES_DESC, &pk, row).unwrap();
    txn.commit().unwrap();

    // Same name, different id, later txn: the by_name UNIQUE index must reject it.
    let mut txn = s.begin().unwrap();
    let (pk2, row2) = keyspace_row(8, "same-name", 1, ApiType::Raw);
    assert!(txn.insert(&schema::KEYSPACES_DESC, &pk2, row2).is_err());
}

#[test]
fn fk_enforced_against_merged_view() {
    let s = store();
    // No tenant exists: keyspace insert must fail the FK check.
    let mut txn = s.begin().unwrap();
    let (pk, row) = keyspace_row(7, "ks-a", 99, ApiType::Raw);
    assert!(txn.insert(&schema::KEYSPACES_DESC, &pk, row.clone()).is_err());

    // Parent inserted earlier in the SAME txn satisfies the FK (bootstrap pattern).
    let mut txn = s.begin().unwrap();
    txn.insert(&schema::TENANTS_DESC, &[memcmp_uint(99)], tenant_row(99, "t99"))
        .unwrap();
    txn.insert(&schema::KEYSPACES_DESC, &pk, row).unwrap();
    txn.commit().unwrap();
}

#[test]
fn update_remaintains_indexes() {
    let s = store();
    seed_tenant(&s);
    let mut txn = s.begin().unwrap();
    txn.insert(&schema::TENANTS_DESC, &[memcmp_uint(2)], tenant_row(2, "other"))
        .unwrap();
    let (pk, row) = keyspace_row(7, "ks-a", 1, ApiType::Raw);
    txn.insert(&schema::KEYSPACES_DESC, &pk, row).unwrap();
    txn.commit().unwrap();

    // Move the keyspace to tenant 2; the by_tenant index must follow.
    let mut txn = s.begin().unwrap();
    txn.update(
        &schema::KEYSPACES_DESC,
        &pk,
        vec![(ColumnId(3), ColumnValue::Uint(2))],
    )
    .unwrap();
    txn.commit().unwrap();

    let t = Tables::new(&s);
    assert_eq!(t.keyspaces_of_tenant(TenantId(1)).unwrap(), vec![]);
    assert_eq!(
        t.keyspaces_of_tenant(TenantId(2)).unwrap(),
        vec![KeyspaceId(7)]
    );

    // Updating a missing row is an error.
    let mut txn = s.begin().unwrap();
    assert!(txn
        .update(
            &schema::KEYSPACES_DESC,
            &[memcmp_uint(999)],
            vec![(ColumnId(7), ColumnValue::Uint(1))],
        )
        .is_err());
}

#[test]
fn delete_removes_row_and_indexes_and_is_idempotent() {
    let s = store();
    seed_tenant(&s);
    let mut txn = s.begin().unwrap();
    let (pk, row) = keyspace_row(7, "ks-a", 1, ApiType::Raw);
    txn.insert(&schema::KEYSPACES_DESC, &pk, row).unwrap();
    txn.commit().unwrap();

    let mut txn = s.begin().unwrap();
    txn.delete(&schema::KEYSPACES_DESC, &pk).unwrap();
    // Idempotent: deleting again inside the same txn is a no-op.
    txn.delete(&schema::KEYSPACES_DESC, &pk).unwrap();
    txn.commit().unwrap();

    let txn = s.begin().unwrap();
    assert!(txn.get(&schema::KEYSPACES_DESC, &pk).unwrap().is_none());
    // Index side gone too: by_tenant scan finds nothing, and the name is reusable.
    let t = Tables::new(&s);
    assert_eq!(t.keyspaces_of_tenant(TenantId(1)).unwrap(), vec![]);
    let mut txn = s.begin().unwrap();
    let (pk2, row2) = keyspace_row(8, "ks-a", 1, ApiType::Raw);
    txn.insert(&schema::KEYSPACES_DESC, &pk2, row2).unwrap();
}

#[test]
fn allocate_id_sequences_are_independent_and_durable() {
    let s = store();
    let mut txn = s.begin().unwrap();
    assert_eq!(txn.allocate_id(SequenceKind::Keyspace).unwrap(), FIRST_DYNAMIC_ID);
    assert_eq!(
        txn.allocate_id(SequenceKind::Keyspace).unwrap(),
        FIRST_DYNAMIC_ID + 1
    );
    // A different kind has its own sequence.
    assert_eq!(txn.allocate_id(SequenceKind::TxnGroup).unwrap(), FIRST_DYNAMIC_ID);
    txn.commit().unwrap();

    // The bump survives the commit; a later txn continues, not restarts.
    let mut txn = s.begin().unwrap();
    assert_eq!(
        txn.allocate_id(SequenceKind::Keyspace).unwrap(),
        FIRST_DYNAMIC_ID + 2
    );
}

#[test]
fn uncommitted_txn_discards_its_overlay() {
    let s = store();
    {
        let mut txn = s.begin().unwrap();
        txn.insert(&schema::TENANTS_DESC, &[memcmp_uint(1)], tenant_row(1, "gone"))
            .unwrap();
        // Dropped without commit.
    }
    let txn = s.begin().unwrap();
    assert!(txn
        .get(&schema::TENANTS_DESC, &[memcmp_uint(1)])
        .unwrap()
        .is_none());
}

// ---------------------------------------------------------------------------
// Known joins (contract item 8) + the gap/boundary negative cases.
// ---------------------------------------------------------------------------

/// Seed: tenant 1; keyspace 7 (txn) with regions [a,b), [b,c); keyspace 8 (raw, empty);
/// keyspace 9 (txn) with the default whole-range group.
fn seed_routing(s: &MetaStore<MemEngine>) {
    let mut txn = s.begin().unwrap();
    txn.insert(&schema::TENANTS_DESC, &[memcmp_uint(1)], tenant_row(1, "default"))
        .unwrap();
    for (id, name, api) in [
        (7u32, "ks-txn", ApiType::Txn),
        (8, "ks-raw", ApiType::Raw),
        (9, "ks-other", ApiType::Txn),
    ] {
        let (pk, row) = keyspace_row(id, name, 1, api);
        txn.insert(&schema::KEYSPACES_DESC, &pk, row).unwrap();
    }
    for (rid, ks, start, end) in [
        (100u64, 7u32, b"a".as_slice(), b"b".as_slice()),
        (101, 7, b"b", b"c"),
    ] {
        txn.insert(
            &schema::REGIONS_DESC,
            &[memcmp_uint(rid)],
            region_row(rid, ks, start, end),
        )
        .unwrap();
    }
    // Node + peers for regions_on_node.
    let mut node = RowValue::new();
    node.set(ColumnId(1), ColumnValue::Uint(5));
    node.set(ColumnId(2), ColumnValue::Text("n5:20160".into()));
    node.set(ColumnId(3), ColumnValue::Uint(0));
    node.set(ColumnId(4), ColumnValue::Uint(0));
    node.set(ColumnId(5), ColumnValue::Bytes(Vec::new()));
    txn.insert(&schema::NODES_DESC, &[memcmp_uint(5)], node).unwrap();
    for rid in [100u64, 101] {
        let mut peer = RowValue::new();
        peer.set(ColumnId(1), ColumnValue::Uint(rid));
        peer.set(ColumnId(2), ColumnValue::Uint(5));
        peer.set(ColumnId(3), ColumnValue::Uint(0));
        txn.insert(
            &schema::REGION_PEERS_DESC,
            &[memcmp_uint(rid), memcmp_uint(5)],
            peer,
        )
        .unwrap();
    }
    // Default whole-range txn group for keyspace 7 only.
    let group = TxnGroup {
        id: kv9_common::TxnGroupId(200),
        keyspace_id: KeyspaceId(7),
        name: "default".into(),
        sub_start: Vec::new(),
        sub_end: Vec::new(),
    };
    txn.insert(&schema::TXN_GROUPS_DESC, &group.pk(), group.to_row_value())
        .unwrap();
    txn.commit().unwrap();
}

#[test]
fn region_for_key_hits_and_boundaries() {
    let s = store();
    seed_routing(&s);
    let t = Tables::new(&s);

    // In [a,b): first region. At the b boundary: second region (half-open ranges).
    assert_eq!(
        t.region_for_key(KeyspaceId(7), b"a").unwrap().map(|r| r.id),
        Some(RegionId(100))
    );
    assert_eq!(
        t.region_for_key(KeyspaceId(7), b"az").unwrap().map(|r| r.id),
        Some(RegionId(100))
    );
    assert_eq!(
        t.region_for_key(KeyspaceId(7), b"b").unwrap().map(|r| r.id),
        Some(RegionId(101))
    );

    // Negative cases (contract correction 2):
    // K before the first region's start.
    assert!(t.region_for_key(KeyspaceId(7), b"0").unwrap().is_none());
    // K past the keyspace's last end_key — the end_key check is the tenant-isolation
    // line: never resolve to a neighbor.
    assert!(t.region_for_key(KeyspaceId(7), b"c").unwrap().is_none());
    assert!(t.region_for_key(KeyspaceId(7), b"zzz").unwrap().is_none());
    // Empty keyspace (no regions at all).
    assert!(t.region_for_key(KeyspaceId(8), b"a").unwrap().is_none());
    // A keyspace that doesn't exist.
    assert!(t.region_for_key(KeyspaceId(6), b"a").unwrap().is_none());
}

#[test]
fn regions_on_node_join() {
    let s = store();
    seed_routing(&s);
    let t = Tables::new(&s);
    let mut ids: Vec<u64> = t
        .regions_on_node(NodeId(5))
        .unwrap()
        .into_iter()
        .map(|r: Region| r.id.0)
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![100, 101]);
    assert!(t.regions_on_node(NodeId(6)).unwrap().is_empty());
}

#[test]
fn txn_group_for_key_default_and_raw() {
    let s = store();
    seed_routing(&s);
    let t = Tables::new(&s);
    // Txn keyspace: default whole-range group hit for any key.
    assert_eq!(
        t.txn_group_for_key(KeyspaceId(7), b"anything").unwrap(),
        Some(kv9_common::TxnGroupId(200))
    );
    // Txn keyspace without a group row yet, and raw keyspace: None.
    assert!(t.txn_group_for_key(KeyspaceId(9), b"k").unwrap().is_none());
    assert!(t.txn_group_for_key(KeyspaceId(8), b"k").unwrap().is_none());
}

#[test]
fn region_peer_typed_struct_compiles_into_row() {
    // Silence "never constructed" drift between the typed structs and the schema:
    // the structs are the team-facing shape (METADATA-CATALOG §2).
    let p = RegionPeer {
        region_id: RegionId(1),
        node_id: NodeId(2),
        role: 0,
    };
    assert_eq!(p.node_id, NodeId(2));
    let t = Tenant {
        id: TenantId(1),
        name: "x".into(),
        quota_tokens: 0,
        state: 0,
    };
    assert_eq!(t.id, TenantId(1));
}

#[test]
fn index_scan_prefix_does_not_match_name_superstrings() {
    // "foo" must not match "foobar" via the unique by_name index — the codec
    // terminator guarantees it; this pins it at the store level.
    let s = store();
    seed_tenant(&s);
    let mut txn = s.begin().unwrap();
    let (pk, row) = keyspace_row(7, "foobar", 1, ApiType::Raw);
    txn.insert(&schema::KEYSPACES_DESC, &pk, row).unwrap();
    txn.commit().unwrap();

    let txn = s.begin().unwrap();
    let hits = txn
        .index_scan(
            &schema::KEYSPACES_DESC,
            IndexId(2), // by_name (unique)
            &[memcmp_text("foo")],
            usize::MAX,
        )
        .unwrap();
    assert!(hits.is_empty());
    let hits = txn
        .index_scan(
            &schema::KEYSPACES_DESC,
            IndexId(2),
            &[memcmp_text("foobar")],
            usize::MAX,
        )
        .unwrap();
    assert_eq!(hits, vec![vec![memcmp_uint(7)]]);
}
