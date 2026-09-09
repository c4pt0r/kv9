use std::net::SocketAddr;
use std::sync::Arc;

use kv9_common::{ClusterId, NodeId, StoreIncarnation};
use kv9_engine::{ColumnFamily, Engine, MemEngine, WriteBatch};
use kv9_meta::codec::{encode_row_key, memcmp_uint, ColumnValue, RowValue};
use kv9_meta::endpoint::{
    change_endpoint, node_endpoint, EndpointChange, EndpointChangeOutcome as Outcome,
    EndpointRefusal as Refusal, ENDPOINT_GENERATION, ENDPOINT_PREVIOUS_ADDRESS,
};
use kv9_meta::schema::{ColumnId, NODES_DESC};
use kv9_meta::MetaStore;

fn address(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
}

fn request(generation: u64, old: u16, new: u16) -> EndpointChange {
    EndpointChange {
        cluster: ClusterId::from_bytes([1; 16]),
        node: NodeId(4),
        incarnation: StoreIncarnation::from_bytes([4; 16]),
        expected_address: address(old),
        expected_generation: generation,
        new_address: address(new),
    }
}

fn store() -> MetaStore<MemEngine> {
    let store = MetaStore::new(Arc::new(MemEngine::new()));
    let mut txn = store.begin().unwrap();
    kv9_meta::admission::initialize_cluster(&mut txn, request(0, 1, 2).cluster, 1).unwrap();
    let mut row = RowValue::new();
    row.set(ColumnId(1), ColumnValue::Uint(4));
    row.set(ColumnId(2), ColumnValue::Text(address(1).to_string()));
    row.set(ColumnId(3), ColumnValue::Uint(2));
    row.set(ColumnId(4), ColumnValue::Uint(123));
    row.set(ColumnId(5), ColumnValue::Bytes(vec![4; 16]));
    row.set(
        ColumnId(99),
        ColumnValue::Bytes(b"retained extension".to_vec()),
    );
    txn.insert(&NODES_DESC, &[memcmp_uint(4)], row).unwrap();
    txn.commit().unwrap();
    store
}

fn apply(store: &MetaStore<MemEngine>, change: EndpointChange) -> Outcome {
    let mut txn = store.begin().unwrap();
    let result = change_endpoint(&mut txn, change).unwrap();
    if matches!(result, Outcome::Changed(_)) {
        txn.commit().unwrap();
    } else {
        assert!(
            txn.into_batch().mutations().is_empty(),
            "confirmation or refusal staged catalog mutations"
        );
    }
    result
}

#[test]
fn legacy_endpoint_cas_stages_address_and_generation_together() {
    let store = store();
    let before = node_endpoint(&store.begin().unwrap(), NodeId(4))
        .unwrap()
        .unwrap();
    assert_eq!(before.generation, 0);
    let mut txn = store.begin().unwrap();
    let Outcome::Changed(changed) = change_endpoint(&mut txn, request(0, 1, 2)).unwrap() else {
        panic!("valid endpoint transition was refused");
    };
    assert_eq!(changed.generation, 1);
    assert_eq!(changed.address, address(2));
    assert_eq!(changed.incarnation, before.incarnation);
    assert_eq!(node_endpoint(&txn, NodeId(4)).unwrap(), Some(changed));
    assert_eq!(
        node_endpoint(&store.begin().unwrap(), NodeId(4)).unwrap(),
        Some(before)
    );
    txn.commit().unwrap();
    let txn = store.begin().unwrap();
    assert_eq!(node_endpoint(&txn, NodeId(4)).unwrap(), Some(changed));
    let row = txn.get(&NODES_DESC, &[memcmp_uint(4)]).unwrap().unwrap();
    assert_eq!(row.value.get(ColumnId(4)), Some(&ColumnValue::Uint(123)));
    assert_eq!(
        row.value.get(ColumnId(99)),
        Some(&ColumnValue::Bytes(b"retained extension".to_vec()))
    );
}

#[test]
fn endpoint_retry_confirms_one_step_without_reapplying() {
    let store = store();
    let Outcome::Changed(changed) = apply(&store, request(0, 1, 2)) else {
        panic!()
    };
    assert_eq!(apply(&store, request(0, 1, 2)), Outcome::Confirmed(changed));
    assert_eq!(
        apply(&store, request(0, 1, 3)),
        Outcome::Refused(Refusal::Conflict)
    );
    assert_eq!(
        apply(&store, request(1, 3, 2)),
        Outcome::Refused(Refusal::Conflict)
    );
    assert_eq!(
        apply(&store, request(0, 3, 2)),
        Outcome::Refused(Refusal::Conflict),
        "confirmation ignored the original transition's old address"
    );
    assert_eq!(
        node_endpoint(&store.begin().unwrap(), NodeId(4)).unwrap(),
        Some(changed)
    );
}

#[test]
fn endpoint_cas_rejects_aba_and_late_duplicate_with_same_target() {
    let store = store();
    assert!(matches!(
        apply(&store, request(0, 1, 2)),
        Outcome::Changed(_)
    ));
    assert!(matches!(
        apply(&store, request(1, 2, 1)),
        Outcome::Changed(_)
    ));
    assert_eq!(
        apply(&store, request(0, 1, 2)),
        Outcome::Refused(Refusal::Conflict),
        "old address matched after ABA but its generation was obsolete"
    );
    assert!(matches!(
        apply(&store, request(2, 1, 2)),
        Outcome::Changed(_)
    ));
    assert_eq!(
        apply(&store, request(0, 1, 2)),
        Outcome::Refused(Refusal::Conflict),
        "matching target address acknowledged a superseded transition"
    );
}

#[test]
fn endpoint_binding_is_required_for_initial_update_and_confirmation() {
    let store = store();
    let wrong = EndpointChange {
        incarnation: StoreIncarnation::from_bytes([9; 16]),
        ..request(0, 1, 2)
    };
    assert_eq!(
        apply(&store, wrong),
        Outcome::Refused(Refusal::InvalidIncarnation),
        "wrong incarnation changed an existing endpoint"
    );
    assert!(matches!(
        apply(&store, request(0, 1, 2)),
        Outcome::Changed(_)
    ));
    assert_eq!(
        apply(&store, wrong),
        Outcome::Refused(Refusal::InvalidIncarnation)
    );
    assert_eq!(
        apply(
            &store,
            EndpointChange {
                cluster: ClusterId::from_bytes([9; 16]),
                ..request(0, 1, 2)
            }
        ),
        Outcome::Refused(Refusal::WrongCluster)
    );
    assert_eq!(
        apply(
            &store,
            EndpointChange {
                node: NodeId(9),
                ..request(0, 1, 2)
            }
        ),
        Outcome::Refused(Refusal::MissingNode)
    );
}

#[test]
fn endpoint_update_requires_active_membership() {
    for state in [0, 1, 3] {
        let store = store();
        let mut txn = store.begin().unwrap();
        txn.update(
            &NODES_DESC,
            &[memcmp_uint(4)],
            vec![(ColumnId(3), ColumnValue::Uint(state))],
        )
        .unwrap();
        txn.commit().unwrap();
        assert_eq!(
            apply(&store, request(0, 1, 2)),
            Outcome::Refused(Refusal::InactiveNode),
            "inactive member received endpoint authorization"
        );
    }
}

#[test]
fn endpoint_generation_exhaustion_never_wraps() {
    let store = store();
    let mut txn = store.begin().unwrap();
    txn.update(
        &NODES_DESC,
        &[memcmp_uint(4)],
        vec![
            (ENDPOINT_GENERATION, ColumnValue::Uint(u64::MAX - 1)),
            (
                ENDPOINT_PREVIOUS_ADDRESS,
                ColumnValue::Text(address(3).to_string()),
            ),
        ],
    )
    .unwrap();
    txn.commit().unwrap();
    let Outcome::Changed(last) = apply(&store, request(u64::MAX - 1, 1, 2)) else {
        panic!()
    };
    assert_eq!(last.generation, u64::MAX);
    assert_eq!(
        apply(&store, request(u64::MAX - 1, 1, 2)),
        Outcome::Confirmed(last)
    );
    assert_eq!(
        apply(&store, request(u64::MAX, 2, 1)),
        Outcome::Refused(Refusal::GenerationExhausted),
        "endpoint generation exhausted but a new transition was staged"
    );
    assert_eq!(
        node_endpoint(&store.begin().unwrap(), NodeId(4)).unwrap(),
        Some(last)
    );
}

#[test]
fn same_address_cas_advances_version_and_retry_is_read_only() {
    let store = store();
    let Outcome::Changed(changed) = apply(&store, request(0, 1, 1)) else {
        panic!()
    };
    assert_eq!(changed.generation, 1);
    assert_eq!(changed.address, address(1));
    assert_eq!(apply(&store, request(0, 1, 1)), Outcome::Confirmed(changed));
    assert_eq!(
        apply(&store, request(0, 1, 2)),
        Outcome::Refused(Refusal::Conflict)
    );
}

#[test]
fn corrupt_present_generation_is_not_treated_as_legacy_zero() {
    let store = store();
    let mut row = store
        .begin()
        .unwrap()
        .get(&NODES_DESC, &[memcmp_uint(4)])
        .unwrap()
        .unwrap()
        .value;
    row.set(
        ENDPOINT_GENERATION,
        ColumnValue::Text("bad generation".into()),
    );
    let mut batch = WriteBatch::new();
    batch.put(
        ColumnFamily::Default,
        encode_row_key(NODES_DESC.id, &[memcmp_uint(4)]).unwrap(),
        row.encode(),
    );
    store.engine().write(batch).unwrap();
    assert!(node_endpoint(&store.begin().unwrap(), NodeId(4)).is_err());
    assert!(change_endpoint(&mut store.begin().unwrap(), request(0, 1, 2)).is_err());
}
