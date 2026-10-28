use super::*;
use kv9_raft::storage::membership_history::InitialConfiguration;

pub(super) fn native(c: &CreationIntent) -> OriginPage {
    let mut p = page(&[], None);
    p.initial = Some(InitialConfiguration::Voters(
        c.replicas().iter().map(|r| r.node).collect(),
    ));
    p
}
fn downgrade(e: &impl Engine) {
    let mut header = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
    header.truncate(header.len() - 32);
    header[..8].copy_from_slice(DATA_ORIGINS_HEADER_MAGIC);
    let mut b = WriteBatch::new();
    b.put(ColumnFamily::Default, key(1, &[]), seal(header));
    e.write(b).unwrap();
}

#[test]
fn initial_origin_v24_upgrade_preserves_old_pages_until_native_observation() {
    let (s, r) = fixture();
    let c = group(&s, 1);
    let e = s.engine().as_ref();
    let revision = complete(e, &r);
    let first = q(&c, revision, 0);
    record(e, &r, first, None, &page(&[], None));
    let old = e
        .get(ColumnFamily::Default, &first.source.key(54).unwrap())
        .unwrap()
        .unwrap();
    downgrade(e);
    assert!(observation(e.snapshot().unwrap().as_ref(), &r, first.source).is_err());
    apply(e, &r, LedgerRequest::Initialize);
    assert_eq!(
        e.get(ColumnFamily::Default, &first.source.key(54).unwrap())
            .unwrap()
            .unwrap(),
        old
    );
    assert!(
        observation(e.snapshot().unwrap().as_ref(), &r, first.source)
            .unwrap()
            .unwrap()
            .has_known_gaps()
    );
    assert!(
        super::super::initial::observation(e.snapshot().unwrap().as_ref(), &r, first.source)
            .unwrap()
            .is_none()
    );
    // Even equal descriptive fingerprints cannot make legacy progress skip
    // the first initial mapping and its atomic progress update.
    record(e, &r, q(&c, revision, 1), None, &native(&c));
    let view = e.snapshot().unwrap();
    let mapped = super::super::initial::observation(view.as_ref(), &r, first.source)
        .unwrap()
        .unwrap();
    assert_eq!(mapped.creation_task, c.task());
    assert_eq!(mapped.replicas, c.replicas());
    let progress = observation(view.as_ref(), &r, first.source)
        .unwrap()
        .unwrap();
    assert_eq!(progress.sequence, 2);
    assert_eq!(progress.recorded_at, mapped.recorded_at);
    assert!(!progress.has_known_gaps());
    drop(view);
    downgrade(e);
    assert!(
        plan_retention(
            e.snapshot().unwrap().as_ref(),
            &r,
            &LedgerRequest::Initialize
        )
        .is_err(),
        "new mapping under v24 must refuse upgrade"
    );
}

#[test]
fn initial_origin_wrong_native_voters_never_become_a_store_mapping() {
    let (s, r) = fixture();
    let c = group(&s, 1);
    let e = s.engine().as_ref();
    let revision = complete(e, &r);
    let first = q(&c, revision, 0);
    let mut wrong = native(&c);
    wrong.initial = Some(InitialConfiguration::Voters(vec![
        NodeId(1),
        NodeId(2),
        NodeId(5),
    ]));
    let before = crate::retention::tests::all(e);
    assert!(
        plan_verified(
            e.snapshot().unwrap().as_ref(),
            &r,
            first,
            endpoint(&r, first.source),
            None,
            &wrong
        )
        .is_err(),
        "different observed voters must refuse before mutation"
    );
    assert_eq!(crate::retention::tests::all(e), before);
    assert!(
        super::super::initial::observation(e.snapshot().unwrap().as_ref(), &r, first.source)
            .unwrap()
            .is_none()
    );
}

#[test]
fn initial_origin_missing_mapping_refuses_observation_receipt_and_repair() {
    let (s, r) = fixture();
    let c = group(&s, 1);
    let e = s.engine().as_ref();
    let revision = complete(e, &r);
    let first = q(&c, revision, 0);
    let p = native(&c);
    record(e, &r, first, None, &p);
    assert!(!plan_retry(e.snapshot().unwrap().as_ref(), &r, first)
        .unwrap()
        .changed());
    let mut b = WriteBatch::new();
    b.delete(ColumnFamily::Default, first.source.key(56).unwrap());
    e.write(b).unwrap();
    let view = e.snapshot().unwrap();
    assert!(super::super::initial::observation(view.as_ref(), &r, first.source).is_err());
    assert!(observation(view.as_ref(), &r, first.source).is_err());
    assert!(plan_retry(view.as_ref(), &r, first).is_err());
    assert!(plan_verified(
        view.as_ref(),
        &r,
        q(&c, revision, 1),
        endpoint(&r, first.source),
        None,
        &p
    )
    .is_err());
}

#[test]
fn initial_origin_snapshot_unknown_is_sticky_even_after_known_native_page() {
    for unknown in [
        InitialConfiguration::Missing,
        InitialConfiguration::Ambiguous,
        InitialConfiguration::Unsupported,
    ] {
        let (s, r) = fixture();
        let c = group(&s, 1);
        let e = s.engine().as_ref();
        let revision = complete(e, &r);
        let first = q(&c, revision, 0);
        let mut p = native(&c);
        p.initial = Some(unknown);
        record(e, &r, first, None, &p);
        assert!(super::super::initial::observation(
            e.snapshot().unwrap().as_ref(),
            &r,
            first.source
        )
        .unwrap()
        .is_none());
        record(e, &r, q(&c, revision, 1), None, &native(&c));
        assert!(
            observation(e.snapshot().unwrap().as_ref(), &r, first.source)
                .unwrap()
                .unwrap()
                .has_known_gaps()
        );
    }
}

#[test]
fn initial_origin_imported_store_cannot_claim_native_creation() {
    let (s, r) = fixture();
    let c = group(&s, 1);
    let h = history(&s, &c, &r);
    let e = s.engine().as_ref();
    let revision = complete(e, &r);
    let first = Request {
        source: Source {
            region: c.region(),
            creation: c.digest(),
            node: NodeId(4),
            store: h[0].origin.store,
        },
        task: h[0].origin.task,
        revision,
        expected: 0,
    };
    let endpoint = NodeEndpoint {
        node: NodeId(4),
        incarnation: first.source.store,
        address: "127.0.0.1:18004".parse().unwrap(),
        generation: 0,
        previous_address: None,
        active: true,
    };
    assert!(
        plan_verified(
            e.snapshot().unwrap().as_ref(),
            &r,
            first,
            endpoint,
            None,
            &native(&c)
        )
        .is_err(),
        "matching node set is insufficient on an imported store"
    );
}

#[test]
fn initial_origin_uses_creation_stores_after_current_registration_changes() {
    let (s, r) = fixture();
    let mut t = s.begin().unwrap();
    let c =
        data_groups::plan_empty_group(&mut t, [1; 16], &[NodeId(1), NodeId(2), NodeId(4)]).unwrap();
    t.commit().unwrap();
    let mut t = s.begin().unwrap();
    t.update(
        &NODES_DESC,
        &[memcmp_uint(4)],
        vec![(ColumnId(5), ColumnValue::Bytes(vec![44; 16]))],
    )
    .unwrap();
    t.commit().unwrap();
    let e = s.engine().as_ref();
    let revision = complete(e, &r);
    let first = q(&c, revision, 0);
    record(e, &r, first, None, &native(&c));
    let mapped =
        super::super::initial::observation(e.snapshot().unwrap().as_ref(), &r, first.source)
            .unwrap()
            .unwrap();
    assert_eq!(mapped.replicas, c.replicas());
    assert_eq!(
        mapped.replicas[2].incarnation,
        StoreIncarnation::from_bytes([4; 16])
    );
}

#[test]
fn initial_origin_seven_voters_canonical_row_and_orphan_progress_refusal() {
    let (s, r) = fixture();
    let mut t = s.begin().unwrap();
    for id in 6..=7 {
        let mut row = RowValue::new();
        row.set(
            ColumnId(2),
            ColumnValue::Text(format!("127.0.0.1:{}", 18000 + id)),
        );
        row.set(ColumnId(3), ColumnValue::Uint(2));
        row.set(ColumnId(4), ColumnValue::Uint(0));
        row.set(ColumnId(5), ColumnValue::Bytes(vec![id as u8; 16]));
        t.insert(&NODES_DESC, &[memcmp_uint(id)], row).unwrap();
    }
    let c =
        data_groups::plan_empty_group(&mut t, [1; 16], &(1..=7).map(NodeId).collect::<Vec<_>>())
            .unwrap();
    t.commit().unwrap();
    let e = s.engine().as_ref();
    let revision = complete(e, &r);
    let first = q(&c, revision, 0);
    record(e, &r, first, None, &native(&c));
    let view = e.snapshot().unwrap();
    let stored = super::super::initial::observation(view.as_ref(), &r, first.source)
        .unwrap()
        .unwrap();
    let original = data_store_history::exact_origin(
        view.as_ref(),
        r.digest(),
        c.region(),
        c.digest(),
        first.source.node,
        first.source.store,
        c.task(),
    )
    .unwrap();
    assert_eq!(stored.source_digest, original.source_digest);
    assert_eq!(stored.replicas, c.replicas());
    let bytes = e
        .get(ColumnFamily::Default, &first.source.key(56).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(bytes.len(), 353);
    drop(view);
    // A checksummed but noncanonical row cannot borrow the first store twice.
    let mut bad = bytes[..bytes.len() - 32].to_vec();
    let first_store = bad[161..177].to_vec();
    bad[185..201].copy_from_slice(&first_store);
    let mut b = WriteBatch::new();
    b.put(
        ColumnFamily::Default,
        first.source.key(56).unwrap(),
        seal(bad),
    );
    e.write(b).unwrap();
    assert!(
        super::super::initial::observation(e.snapshot().unwrap().as_ref(), &r, first.source)
            .is_err()
    );
    let mut b = WriteBatch::new();
    b.put(ColumnFamily::Default, first.source.key(56).unwrap(), bytes);
    b.delete(ColumnFamily::Default, first.source.key(54).unwrap());
    e.write(b).unwrap();
    assert!(
        super::super::initial::observation(e.snapshot().unwrap().as_ref(), &r, first.source)
            .is_err()
    );
    assert!(plan_verified(
        e.snapshot().unwrap().as_ref(),
        &r,
        first,
        endpoint(&r, first.source),
        None,
        &native(&c)
    )
    .is_err());
}
