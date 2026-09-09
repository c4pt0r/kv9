use std::net::SocketAddr;
use std::sync::Arc;

use kv9_common::{ClusterId, NodeId, StoreIncarnation};
use kv9_engine::{ColumnFamily, Engine, MemEngine, WriteBatch};
use kv9_meta::codec::{encode_row_key, memcmp_uint, ColumnValue, RowValue};
use kv9_meta::endpoint::{
    change_endpoint, node_endpoint, refresh_registration_endpoint, EndpointChange,
    EndpointChangeOutcome as Outcome, EndpointRefusal as Refusal, ENDPOINT_GENERATION,
    ENDPOINT_PREVIOUS_ADDRESS,
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
fn endpoint_change_revokes_prior_admission_in_the_same_batch() {
    use kv9_meta::admission::{self, AdmissionState, AdmittedRole};
    for consumed in [false, true] {
        let store = store();
        let change = request(0, 1, 2);
        let mut txn = store.begin().unwrap();
        admission::admit_node(
            &mut txn,
            change.node,
            &address(1).to_string(),
            AdmittedRole::Learner,
            100,
        )
        .unwrap();
        if consumed {
            admission::consume_admission(
                &mut txn,
                change.node,
                change.cluster,
                &address(1).to_string(),
                1,
            )
            .unwrap();
        }
        txn.commit().unwrap();
        let original_state = if consumed {
            AdmissionState::Consumed
        } else {
            AdmissionState::Pending
        };
        let mut txn = store.begin().unwrap();
        assert!(matches!(
            change_endpoint(&mut txn, change).unwrap(),
            Outcome::Changed(_)
        ));
        assert_eq!(
            admission::admission(&txn, change.node)
                .unwrap()
                .unwrap()
                .state,
            AdmissionState::Superseded,
            "endpoint change retained obsolete registration authority"
        );
        let before = store.begin().unwrap();
        assert_eq!(
            admission::admission(&before, change.node)
                .unwrap()
                .unwrap()
                .state,
            original_state
        );
        assert_eq!(
            node_endpoint(&before, change.node)
                .unwrap()
                .unwrap()
                .address,
            address(1)
        );
        drop(before);
        txn.commit().unwrap();
        let after = store.begin().unwrap();
        assert_eq!(
            admission::admission(&after, change.node)
                .unwrap()
                .unwrap()
                .state,
            AdmissionState::Superseded
        );
        assert_eq!(
            node_endpoint(&after, change.node).unwrap().unwrap().address,
            address(2)
        );
    }
}

#[test]
fn endpoint_changes_preserve_explicit_decommission_and_cancel_old_tickets() {
    use kv9_meta::admission::{self, AdmissionState, AdmittedRole};
    let store = store();
    let mut txn = store.begin().unwrap();
    admission::admit_node(
        &mut txn,
        NodeId(4),
        &address(1).to_string(),
        AdmittedRole::Learner,
        100,
    )
    .unwrap();
    txn.commit().unwrap();
    assert!(matches!(
        apply(&store, request(0, 1, 2)),
        Outcome::Changed(_)
    ));
    let mut txn = store.begin().unwrap();
    assert!(
        admission::consume_admission(
            &mut txn,
            NodeId(4),
            request(0, 1, 2).cluster,
            &address(1).to_string(),
            1
        )
        .is_err(),
        "superseded ticket was consumed again"
    );
    admission::revoke_admission(&mut txn, NodeId(4)).unwrap();
    txn.commit().unwrap();
    assert!(matches!(
        apply(&store, request(1, 2, 3)),
        Outcome::Changed(_)
    ));
    let txn = store.begin().unwrap();
    assert_eq!(
        admission::admission(&txn, NodeId(4))
            .unwrap()
            .unwrap()
            .state,
        AdmissionState::Revoked,
        "endpoint change lifted an explicit decommission"
    );
}

#[test]
fn endpoint_confirmation_does_not_revoke_a_later_admission() {
    use kv9_meta::admission::{self, AdmissionState, AdmittedRole};
    let store = store();
    let change = request(0, 1, 2);
    assert!(matches!(apply(&store, change), Outcome::Changed(_)));
    let mut txn = store.begin().unwrap();
    admission::admit_node(
        &mut txn,
        change.node,
        &address(3).to_string(),
        AdmittedRole::Learner,
        100,
    )
    .unwrap();
    txn.commit().unwrap();
    assert!(matches!(apply(&store, change), Outcome::Confirmed(_)));
    assert_eq!(
        admission::admission(&store.begin().unwrap(), change.node)
            .unwrap()
            .unwrap()
            .state,
        AdmissionState::Pending
    );
    assert!(matches!(
        apply(&store, request(0, 1, 3)),
        Outcome::Refused(Refusal::Conflict)
    ));
    assert_eq!(
        admission::admission(&store.begin().unwrap(), change.node)
            .unwrap()
            .unwrap()
            .state,
        AdmissionState::Pending
    );
}

#[test]
fn registration_address_changes_participate_in_endpoint_aba_fencing() {
    let store = store();
    let change = request(0, 1, 3);
    for (old, new, generation) in [(1, 2, 1), (2, 1, 2)] {
        let mut txn = store.begin().unwrap();
        let next =
            refresh_registration_endpoint(&mut txn, change.node, change.incarnation, address(new))
                .unwrap();
        assert_eq!(
            next.generation, generation,
            "registration did not advance the endpoint generation"
        );
        assert_eq!(next.previous_address, Some(address(old)));
        txn.commit().unwrap();
    }
    assert_eq!(
        apply(&store, change),
        Outcome::Refused(Refusal::Conflict),
        "registration ABA admitted an obsolete operator CAS"
    );
    let mut txn = store.begin().unwrap();
    let unchanged =
        refresh_registration_endpoint(&mut txn, change.node, change.incarnation, address(1))
            .unwrap();
    assert_eq!(unchanged.generation, 2);
    assert!(txn.into_batch().mutations().is_empty());
}

#[test]
fn registration_endpoint_refuses_rebinding_and_generation_overflow() {
    let store = store();
    let change = request(0, 1, 2);
    let mut txn = store.begin().unwrap();
    assert!(refresh_registration_endpoint(
        &mut txn,
        change.node,
        StoreIncarnation::from_bytes([9; 16]),
        address(2)
    )
    .is_err());
    assert!(txn.into_batch().mutations().is_empty());
    let mut txn = store.begin().unwrap();
    txn.update(
        &NODES_DESC,
        &[memcmp_uint(4)],
        vec![
            (ENDPOINT_GENERATION, ColumnValue::Uint(u64::MAX)),
            (
                ENDPOINT_PREVIOUS_ADDRESS,
                ColumnValue::Text(address(3).to_string()),
            ),
        ],
    )
    .unwrap();
    txn.commit().unwrap();
    let mut txn = store.begin().unwrap();
    assert!(
        refresh_registration_endpoint(&mut txn, change.node, change.incarnation, address(2))
            .is_err(),
        "registration wrapped the endpoint generation"
    );
    assert!(txn.into_batch().mutations().is_empty());
    let mut txn = store.begin().unwrap();
    assert_eq!(
        refresh_registration_endpoint(&mut txn, change.node, change.incarnation, address(1))
            .unwrap()
            .generation,
        u64::MAX
    );
    assert!(txn.into_batch().mutations().is_empty());
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
