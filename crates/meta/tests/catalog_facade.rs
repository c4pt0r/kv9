//! Identity preservation in the legacy in-memory catalog facade.

use kv9_common::{ApiType, Error, KeyspaceId, Tenant, TenantId, TxnGroupId};
use kv9_meta::Catalog;

#[test]
fn duplicate_id_cannot_replace_identity_or_leave_a_name_alias() {
    let mut catalog = Catalog::new();
    for id in [TenantId(1), TenantId(2)] {
        catalog.upsert_tenant(Tenant {
            id,
            name: format!("tenant-{}", id.0),
            read_capacity_units: 0,
            write_capacity_units: 0,
        });
    }
    let original = catalog
        .create_keyspace(
            KeyspaceId(7),
            "original",
            TenantId(1),
            ApiType::Raw,
            TxnGroupId::DEFAULT,
        )
        .unwrap()
        .clone();

    // Reusing a physical prefix must not change its name, owner or API type.
    let result = catalog.create_keyspace(
        original.id,
        "replacement",
        TenantId(2),
        ApiType::Txn,
        TxnGroupId(9),
    );
    assert!(matches!(result, Err(Error::Config(_))));
    assert_eq!(catalog.keyspace(original.id).unwrap(), &original);
    assert_eq!(catalog.keyspace_by_name("original"), Some(&original));
    assert!(catalog.keyspace_by_name("replacement").is_none());
    assert_eq!(
        catalog.list_keyspaces().collect::<Vec<_>>(),
        vec![&original]
    );

    // Rejection must not reserve the candidate name or redirect it on a retry.
    let replacement = catalog
        .create_keyspace(
            KeyspaceId(8),
            "replacement",
            TenantId(2),
            ApiType::Txn,
            TxnGroupId(9),
        )
        .unwrap()
        .clone();
    assert_eq!(catalog.keyspace_by_name("replacement"), Some(&replacement));
    assert_eq!(catalog.keyspace_by_name("original"), Some(&original));
    assert_eq!(catalog.keyspace(original.id).unwrap(), &original);
    assert_eq!(catalog.list_keyspaces().count(), 2);

    // An exact duplicate remains a rejected creation rather than an upsert.
    assert!(catalog
        .create_keyspace(
            original.id,
            original.name.clone(),
            original.tenant,
            original.api_type,
            original.txn_group,
        )
        .is_err());
    assert_eq!(catalog.keyspace(original.id).unwrap(), &original);
    assert_eq!(catalog.keyspace(replacement.id).unwrap(), &replacement);
}
