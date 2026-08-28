//! Gate 2 (cluster identity) + gate 3 (node admission) catalog semantics
//! (task #24). Every rule the three-gate contract states is pinned by a
//! negative here; the positives are the controls that keep the negatives
//! honest.

use std::str::FromStr;
use std::sync::Arc;

use kv9_common::{ClusterId, NodeId};
use kv9_engine::MemEngine;
use kv9_meta::admission::{
    admission, admit_node, cluster_id, consume_admission, initialize_cluster,
    pending_admissions, AdmissionState, AdmittedRole,
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
    assert!(admit_node(&mut txn, NodeId(4), "127.0.0.1:9", AdmittedRole::Learner, 10).is_err());
}

#[test]
fn admission_lifecycle_pending_consume_once() {
    let store = store();
    let mut txn = store.begin().unwrap();
    initialize_cluster(&mut txn, cid("c"), 1).unwrap();
    admit_node(&mut txn, NodeId(4), "127.0.0.1:9004", AdmittedRole::Learner, 100).unwrap();
    txn.commit().unwrap();

    let txn = store.begin().unwrap();
    let pending = pending_admissions(&txn).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].node_id, NodeId(4));
    assert_eq!(pending[0].cluster_id, cid("c"));
    assert_eq!(pending[0].state, AdmissionState::Pending);
    drop(txn);

    // Duplicate admission refused (revoke first — one approval, one use).
    let mut txn = store.begin().unwrap();
    assert!(admit_node(&mut txn, NodeId(4), "elsewhere:1", AdmittedRole::Learner, 100).is_err());
    drop(txn);

    // Consume: exactly once.
    let mut txn = store.begin().unwrap();
    let adm = consume_admission(&mut txn, NodeId(4), cid("c"), 50).unwrap();
    assert_eq!(adm.state, AdmissionState::Consumed);
    txn.commit().unwrap();

    let mut txn = store.begin().unwrap();
    assert_eq!(
        admission(&txn, NodeId(4)).unwrap().unwrap().state,
        AdmissionState::Consumed
    );
    // Second consume refused — the record is terminal for this call.
    assert!(consume_admission(&mut txn, NodeId(4), cid("c"), 60).is_err());
    // And the pending view no longer lists it (control for the lifecycle).
    assert!(pending_admissions(&txn).unwrap().is_empty());
}

#[test]
fn consume_rejects_wrong_cluster_and_expiry() {
    let store = store();
    let mut txn = store.begin().unwrap();
    initialize_cluster(&mut txn, cid("d"), 1).unwrap();
    admit_node(&mut txn, NodeId(5), "127.0.0.1:9005", AdmittedRole::Learner, 100).unwrap();
    txn.commit().unwrap();

    // Wrong cluster: a record replayed into another environment admits nobody.
    let mut txn = store.begin().unwrap();
    assert!(consume_admission(&mut txn, NodeId(5), cid("e"), 50).is_err());
    // Expired: now > expires_unix.
    assert!(consume_admission(&mut txn, NodeId(5), cid("d"), 101).is_err());
    // Control: the right cluster within the window still works — otherwise the
    // two negatives above would also pass against a consume that rejects
    // everything.
    assert!(consume_admission(&mut txn, NodeId(5), cid("d"), 100).is_ok());
    // Unknown node.
    assert!(consume_admission(&mut txn, NodeId(99), cid("d"), 50).is_err());
}
