use super::*;
use crate::retention::checkpoint_intent::tests::fixture_intent;
use crate::retention::tests::{all, apply, plan};
use kv9_engine::{Engine, MemEngine, WalEngine};

fn source(root: &RootDescriptor) -> NodeEndpoint {
    let v = root.voter(NodeId(1)).unwrap();
    NodeEndpoint {
        node: v.node_id,
        incarnation: v.store_incarnation,
        address: v.addr,
        generation: 0,
        previous_address: None,
        active: true,
    }
}
fn publish(e: &MemEngine, root: &RootDescriptor, i: &CheckpointIntent) {
    apply(e, root, LedgerRequest::BeginCheckpoint(i.clone()));
    let v = i.binding(OwnerKind::Version).unwrap();
    apply(
        e,
        root,
        LedgerRequest::Share {
            from: i.binding(OwnerKind::Pending).unwrap().token(),
            to: v.clone(),
        },
    );
    apply(e, root, LedgerRequest::Publish(v.token()));
}
fn floor(root: &RootDescriptor, i: &CheckpointIntent) -> ReadFloor {
    let s = source(root);
    expected(
        root,
        i,
        s.node,
        s.incarnation,
        RootDigest::from_bytes([7; 32]),
    )
    .unwrap()
}
fn verified(e: &impl Engine, root: &RootDescriptor, i: &CheckpointIntent) -> Result<LedgerPlan> {
    plan_verified(
        e.snapshot()?.as_ref(),
        root,
        i.binding(OwnerKind::Pending)?.token(),
        source(root),
        floor(root, i),
    )
}
#[test]
fn data_floor_request_is_canonical_and_plain_planner_cannot_forge_observation() {
    let (e, r, i) = fixture_intent();
    publish(&e, &r, &i);
    let f = floor(&r, &i);
    let request = LedgerRequest::RecordDataReadFence {
        pending: i.binding(OwnerKind::Pending).unwrap().token(),
        source: f.identity.node_id,
        store: f.identity.store_incarnation,
        creation: f.creation,
    };
    let bytes = encode_request(r.digest(), &request).unwrap();
    assert_eq!(decode_request(&bytes, r.digest()).unwrap(), request);
    for cut in 0..bytes.len() {
        assert!(decode_request(&bytes[..cut], r.digest()).is_err());
    }
    for range in [65..73, 73..89, 89..121] {
        let mut raw = bytes[..bytes.len() - 32].to_vec();
        raw[range].fill(0);
        assert!(decode_request(&seal(raw), r.digest()).is_err());
    }
    let before = all(&e);
    assert!(plan(&e, &r, &request).is_err());
    assert!(plan_retry(
        e.snapshot().unwrap().as_ref(),
        &r,
        i.binding(OwnerKind::Pending).unwrap().token(),
        f.identity.node_id,
        f.identity.store_incarnation,
        f.creation
    )
    .is_err());
    assert_eq!(all(&e), before);
}
#[test]
fn data_floor_is_independent_from_selection_monotone_and_preserves_pins() {
    let (e, r, first) = fixture_intent();
    publish(&e, &r, &first);
    let s = source(&r);
    let token = |i: &CheckpointIntent| i.binding(OwnerKind::Pending).unwrap().token();
    e.write(
        checkpoint_selection::plan_verified(e.snapshot().unwrap().as_ref(), &r, token(&first), s)
            .unwrap()
            .into_batch(),
    )
    .unwrap();
    assert!(observation(
        e.snapshot().unwrap().as_ref(),
        &r,
        s.node,
        s.incarnation,
        RegionId(100)
    )
    .unwrap()
    .is_none());
    e.write(verified(&e, &r, &first).unwrap().into_batch())
        .unwrap();
    let mut manifest = first.manifest().unwrap();
    manifest.index += 1;
    manifest.scope.version += 1;
    let next = CheckpointIntent::new(&r, &manifest, first.predecessor() + 1).unwrap();
    publish(&e, &r, &next);
    e.write(verified(&e, &r, &next).unwrap().into_batch())
        .unwrap();
    let before = all(&e);
    assert!(!verified(&e, &r, &first).unwrap().changed());
    assert!(!plan_retry(
        e.snapshot().unwrap().as_ref(),
        &r,
        token(&first),
        s.node,
        s.incarnation,
        floor(&r, &first).creation
    )
    .unwrap()
    .changed());
    let mut wrong = floor(&r, &next);
    wrong.creation = RootDigest::from_bytes([9; 32]);
    assert!(plan_verified(e.snapshot().unwrap().as_ref(), &r, token(&next), s, wrong).is_err());
    wrong = floor(&r, &next);
    wrong.image.digest = RootDigest::from_bytes([5; 32]);
    assert!(plan_verified(e.snapshot().unwrap().as_ref(), &r, token(&next), s, wrong).is_err());
    assert_eq!(all(&e), before);
    for i in [&first, &next] {
        let v = retention_owner(
            e.snapshot().unwrap().as_ref(),
            &r,
            i.binding(OwnerKind::Version).unwrap().descriptor.id,
        )
        .unwrap()
        .unwrap();
        assert_eq!(v.phase, PinPhase::Published);
    }
    let actual = observation(
        e.snapshot().unwrap().as_ref(),
        &r,
        s.node,
        s.incarnation,
        RegionId(100),
    )
    .unwrap()
    .unwrap();
    assert_eq!(actual.floor, floor(&r, &next));
    assert_eq!(actual.encode().unwrap().len(), BYTES);
    assert_eq!(
        e.snapshot()
            .unwrap()
            .iter(ColumnFamily::Default, &key(48, &[]), &key(49, &[]))
            .unwrap()
            .count(),
        1
    );
}
#[test]
fn data_floor_upgrade_rejects_orphans_and_fact_survives_wal_reopen() {
    let (e, r, i) = fixture_intent();
    publish(&e, &r, &i);
    let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
    h[..8].copy_from_slice(IMAGE_PUBLISHER_HEADER_MAGIC);
    let mut batch = WriteBatch::new();
    batch.put(
        ColumnFamily::Default,
        key(1, &[]),
        seal(h[..h.len() - 32].to_vec()),
    );
    e.write(batch).unwrap();
    e.write(verified(&e, &r, &i).unwrap().into_batch()).unwrap();
    assert!(e
        .get(ColumnFamily::Default, &key(1, &[]))
        .unwrap()
        .unwrap()
        .starts_with(HEADER_MAGIC));
    let path = std::env::temp_dir().join(format!(
        "kv9-data-fence-catalog-{}",
        StoreIncarnation::mint().unwrap()
    ));
    let disk = WalEngine::open(&path).unwrap().0;
    let (_, mut batch) = crate::retention::tests::seed();
    for (k, v) in all(&e) {
        batch.put(ColumnFamily::Default, k, v);
    }
    disk.write(batch).unwrap();
    drop(disk);
    let reopened = WalEngine::open(&path).unwrap().0;
    let s = source(&r);
    assert_eq!(
        observation(
            reopened.snapshot().unwrap().as_ref(),
            &r,
            s.node,
            s.incarnation,
            RegionId(100)
        )
        .unwrap()
        .unwrap()
        .floor,
        floor(&r, &i)
    );
    let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
    h[..8].copy_from_slice(IMAGE_PUBLISHER_HEADER_MAGIC);
    let mut batch = WriteBatch::new();
    batch.put(
        ColumnFamily::Default,
        key(1, &[]),
        seal(h[..h.len() - 32].to_vec()),
    );
    e.write(batch).unwrap();
    assert!(plan(&e, &r, &LedgerRequest::Initialize).is_err());
    assert!(observation(
        e.snapshot().unwrap().as_ref(),
        &r,
        s.node,
        s.incarnation,
        RegionId(100)
    )
    .is_err());
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}
