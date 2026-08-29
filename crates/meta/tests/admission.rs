//! Gate 2 (cluster identity) + gate 3 (node admission) catalog semantics
//! (task #24). Every rule the three-gate contract states is pinned by a
//! negative here; the positives are the controls that keep the negatives
//! honest.

use std::str::FromStr;
use std::sync::Arc;

use kv9_common::{ClusterId, NodeId};
use kv9_engine::MemEngine;
use kv9_meta::admission::{
    admission, admit_node, cluster_id, consume_admission, initialize_cluster, pending_admissions,
    revoke_admission, AdmissionState, AdmittedRole,
};
use kv9_meta::store::MetaStore;

fn store() -> MetaStore<MemEngine> {
    MetaStore::new(Arc::new(MemEngine::new()))
}

fn cid(hex_byte: &str) -> ClusterId {
    ClusterId::from_str(&hex_byte.repeat(32)).unwrap()
}

#[test]
fn identity_is_minted_once_and_immutable() {
    let store = store();
    let mut txn = store.begin().unwrap();
    assert_eq!(cluster_id(&txn).unwrap(), None);
    initialize_cluster(&mut txn, cid("a"), 1_000).unwrap();
    txn.commit().unwrap();

    let txn = store.begin().unwrap();
    assert_eq!(cluster_id(&txn).unwrap(), Some(cid("a")));
    drop(txn);

    // Second mint refused — identity is immutable.
    let mut txn = store.begin().unwrap();
    assert!(initialize_cluster(&mut txn, cid("b"), 2_000).is_err());
    // And the original survives.
    assert_eq!(cluster_id(&txn).unwrap(), Some(cid("a")));
}

#[test]
fn admission_requires_identity_first() {
    let store = store();
    let mut txn = store.begin().unwrap();
    // Gate 3 cannot precede gate 2.
    assert!(admit_node(
        &mut txn,
        NodeId(4),
        "127.0.0.1:9",
        AdmittedRole::Learner,
        10
    )
    .is_err());
}

#[test]
fn admission_lifecycle_pending_consume_once() {
    let store = store();
    let mut txn = store.begin().unwrap();
    initialize_cluster(&mut txn, cid("c"), 1).unwrap();
    admit_node(
        &mut txn,
        NodeId(4),
        "127.0.0.1:9004",
        AdmittedRole::Learner,
        100,
    )
    .unwrap();
    txn.commit().unwrap();

    let txn = store.begin().unwrap();
    let pending = pending_admissions(&txn).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].node_id, NodeId(4));
    assert_eq!(pending[0].cluster_id, cid("c"));
    assert_eq!(pending[0].state, AdmissionState::Pending);
    drop(txn);

    // Duplicate admission refused (revoke first — one approval, one use).
    // A VALID address, so this errors for the duplicate reason, not parsing.
    let mut txn = store.begin().unwrap();
    assert!(admit_node(
        &mut txn,
        NodeId(4),
        "127.0.0.1:1",
        AdmittedRole::Learner,
        100
    )
    .is_err());
    drop(txn);

    // Consume: exactly once, from the admitted address.
    let mut txn = store.begin().unwrap();
    let adm = consume_admission(&mut txn, NodeId(4), cid("c"), "127.0.0.1:9004", 50).unwrap();
    assert_eq!(adm.state, AdmissionState::Consumed);
    txn.commit().unwrap();

    let mut txn = store.begin().unwrap();
    assert_eq!(
        admission(&txn, NodeId(4)).unwrap().unwrap().state,
        AdmissionState::Consumed
    );
    // Second consume refused — the record is terminal for this call.
    assert!(consume_admission(&mut txn, NodeId(4), cid("c"), "127.0.0.1:9004", 60).is_err());
    // And the pending view no longer lists it (control for the lifecycle).
    assert!(pending_admissions(&txn).unwrap().is_empty());
}

#[test]
fn consume_rejects_wrong_cluster_address_and_expiry() {
    let store = store();
    let mut txn = store.begin().unwrap();
    initialize_cluster(&mut txn, cid("d"), 1).unwrap();
    admit_node(
        &mut txn,
        NodeId(5),
        "127.0.0.1:9005",
        AdmittedRole::Learner,
        100,
    )
    .unwrap();
    txn.commit().unwrap();

    let mut txn = store.begin().unwrap();
    // Caller expectation != this catalog's identity.
    assert!(consume_admission(&mut txn, NodeId(5), cid("e"), "127.0.0.1:9005", 50).is_err());
    // Wrong ADDRESS: knowing the cluster id and squatting the node id is not
    // enough — the admission binds the exact endpoint.
    assert!(consume_admission(&mut txn, NodeId(5), cid("d"), "127.0.0.1:1666", 50).is_err());
    // Expired: now > expires_unix.
    assert!(consume_admission(&mut txn, NodeId(5), cid("d"), "127.0.0.1:9005", 101).is_err());
    // Control: right cluster + right address within the window works —
    // otherwise the negatives above would also pass against a consume that
    // rejects everything.
    assert!(consume_admission(&mut txn, NodeId(5), cid("d"), "127.0.0.1:9005", 100).is_ok());
    // Unknown node.
    assert!(consume_admission(&mut txn, NodeId(99), cid("d"), "127.0.0.1:9", 50).is_err());
}

/// Tess's replay scenario, constructed FOR REAL this time: an admission row
/// whose binding is cluster A physically present inside cluster B's catalog
/// (inserted directly through the public schema/RowValue API — the state a
/// copied/restored row produces). The caller presents B (which MATCHES the
/// local singleton), from the correct address — so the ONLY thing standing
/// between this row and consumption is the row-level binding comparison.
/// Deleting `adm.cluster_id != expected_cluster` turns this test red
/// (mechanical sensitivity, verified by running exactly that mutation).
#[test]
fn admission_row_replayed_into_another_cluster_admits_nobody() {
    use kv9_meta::codec::{memcmp_uint, ColumnValue, RowValue};
    use kv9_meta::schema::{ColumnId, NODE_ADMISSIONS_DESC};

    let store_b = store();
    let mut txn = store_b.begin().unwrap();
    initialize_cluster(&mut txn, cid("b"), 1).unwrap();
    // The replayed row: binds cluster A, in B's catalog.
    let mut row = RowValue::new();
    row.set(
        ColumnId(2),
        ColumnValue::Bytes(cid("a").as_bytes().to_vec()),
    );
    row.set(ColumnId(3), ColumnValue::Text("127.0.0.1:9006".into()));
    row.set(ColumnId(4), ColumnValue::Uint(1)); // Learner
    row.set(ColumnId(5), ColumnValue::Uint(1)); // Pending
    row.set(ColumnId(7), ColumnValue::Uint(100));
    txn.insert(&NODE_ADMISSIONS_DESC, &[memcmp_uint(6)], row)
        .unwrap();
    txn.commit().unwrap();

    let mut txn = store_b.begin().unwrap();
    // local = B, expected = B, row = A: only the row comparison can refuse.
    assert!(
        consume_admission(&mut txn, NodeId(6), cid("b"), "127.0.0.1:9006", 50).is_err(),
        "a row bound to another cluster was consumed against this catalog"
    );
    // Positive control: an identically shaped row bound to B consumes fine —
    // proving the refusal above is the binding, not something else about the
    // hand-built row.
    let mut row = RowValue::new();
    row.set(
        ColumnId(2),
        ColumnValue::Bytes(cid("b").as_bytes().to_vec()),
    );
    row.set(ColumnId(3), ColumnValue::Text("127.0.0.1:9007".into()));
    row.set(ColumnId(4), ColumnValue::Uint(1));
    row.set(ColumnId(5), ColumnValue::Uint(1));
    row.set(ColumnId(7), ColumnValue::Uint(100));
    txn.insert(&NODE_ADMISSIONS_DESC, &[memcmp_uint(16)], row)
        .unwrap();
    assert!(consume_admission(&mut txn, NodeId(16), cid("b"), "127.0.0.1:9007", 50).is_ok());
}

/// NodeId 0 means "no node" on the wire (leader_id=0 = none): the catalog
/// refuses to admit it regardless of what any CLI validated upstream.
#[test]
fn admit_rejects_node_id_zero() {
    let store = store();
    let mut txn = store.begin().unwrap();
    initialize_cluster(&mut txn, cid("0"), 1).unwrap();
    assert!(admit_node(
        &mut txn,
        NodeId(0),
        "127.0.0.1:9000",
        AdmittedRole::Learner,
        10
    )
    .is_err());
    // Control: id 1 with the same everything is admitted.
    assert!(admit_node(
        &mut txn,
        NodeId(1),
        "127.0.0.1:9000",
        AdmittedRole::Learner,
        10
    )
    .is_ok());
}

/// The operator retry path: revoke → re-admit replaces the record; pending and
/// consumed records are never silently replaced.
#[test]
fn revoke_then_readmit_is_the_only_replacement_path() {
    let store = store();
    let mut txn = store.begin().unwrap();
    initialize_cluster(&mut txn, cid("f"), 1).unwrap();
    admit_node(
        &mut txn,
        NodeId(7),
        "127.0.0.1:9007",
        AdmittedRole::Learner,
        100,
    )
    .unwrap();
    // Pending is not replaceable.
    assert!(admit_node(
        &mut txn,
        NodeId(7),
        "127.0.0.1:9008",
        AdmittedRole::Learner,
        200
    )
    .is_err());
    // Revoke, then re-admit with a new address/expiry — succeeds and is Pending.
    revoke_admission(&mut txn, NodeId(7)).unwrap();
    // Double revoke is a typed error.
    assert!(revoke_admission(&mut txn, NodeId(7)).is_err());
    admit_node(
        &mut txn,
        NodeId(7),
        "127.0.0.1:9008",
        AdmittedRole::Learner,
        200,
    )
    .unwrap();
    let adm = admission(&txn, NodeId(7)).unwrap().unwrap();
    assert_eq!(adm.state, AdmissionState::Pending);
    assert_eq!(adm.addr, "127.0.0.1:9008");
    // Consumed is not replaceable either (revoke first — decommission path).
    assert!(consume_admission(&mut txn, NodeId(7), cid("f"), "127.0.0.1:9008", 50).is_ok());
    assert!(admit_node(
        &mut txn,
        NodeId(7),
        "127.0.0.1:9009",
        AdmittedRole::Learner,
        300
    )
    .is_err());
    revoke_admission(&mut txn, NodeId(7)).unwrap();
    assert!(admit_node(
        &mut txn,
        NodeId(7),
        "127.0.0.1:9009",
        AdmittedRole::Learner,
        300
    )
    .is_ok());
}

/// Non-canonical address forms are refused at admit time — the binding is an
/// exact endpoint, not a string.
#[test]
fn admit_rejects_non_socket_addresses() {
    let store = store();
    let mut txn = store.begin().unwrap();
    initialize_cluster(&mut txn, cid("9"), 1).unwrap();
    for bad in ["localhost:9", "127.0.0.1", "not-an-addr", ""] {
        assert!(
            admit_node(&mut txn, NodeId(8), bad, AdmittedRole::Learner, 10).is_err(),
            "accepted {bad:?}"
        );
    }
    // Control.
    assert!(admit_node(
        &mut txn,
        NodeId(8),
        "127.0.0.1:9010",
        AdmittedRole::Learner,
        10
    )
    .is_ok());
}
