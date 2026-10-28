use super::*;
use crate::retention::checkpoint_intent::tests::fixture_intent;
use crate::retention::tests::{all, apply, owner, plan};
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
fn token(i: &CheckpointIntent) -> OwnerToken {
    i.binding(OwnerKind::Pending).unwrap().token()
}
fn publish(e: &MemEngine, root: &RootDescriptor, i: &CheckpointIntent) {
    apply(e, root, LedgerRequest::BeginCheckpoint(i.clone()));
    let v = i.binding(OwnerKind::Version).unwrap();
    apply(
        e,
        root,
        LedgerRequest::Share {
            from: token(i),
            to: v.clone(),
        },
    );
    apply(e, root, LedgerRequest::Publish(v.token()));
    apply(
        e,
        root,
        LedgerRequest::QuiesceAfterTransfer {
            from: token(i),
            to: v.token(),
        },
    );
    apply(e, root, LedgerRequest::Release(token(i)));
}
fn observe(e: &impl Engine, root: &RootDescriptor) -> Option<Selection> {
    let s = source(root);
    selection(
        e.snapshot().unwrap().as_ref(),
        root,
        s.node,
        s.incarnation,
        RegionId(100),
    )
    .unwrap()
}
fn verified(e: &impl Engine, root: &RootDescriptor, i: &CheckpointIntent) -> Result<LedgerPlan> {
    plan_verified(e.snapshot()?.as_ref(), root, token(i), source(root))
}

#[test]
fn selection_request_is_bounded_canonical_and_cannot_mint_unverified_evidence() {
    let (e, r, i) = fixture_intent();
    publish(&e, &r, &i);
    let s = source(&r);
    let request = LedgerRequest::RecordCheckpointSelection {
        pending: token(&i),
        source: s.node,
        store: s.incarnation,
    };
    let bytes = encode_request(r.digest(), &request).unwrap();
    assert_eq!(decode_request(&bytes, r.digest()).unwrap(), request);
    for cut in 0..bytes.len() {
        assert!(decode_request(&bytes[..cut], r.digest()).is_err());
    }
    for range in [65..73, 73..89] {
        let mut raw = bytes[..bytes.len() - 32].to_vec();
        raw[range].fill(0);
        assert!(decode_request(&seal(raw), r.digest()).is_err());
    }
    let before = all(&e);
    assert!(plan(&e, &r, &request).is_err());
    assert!(plan_retry(
        e.snapshot().unwrap().as_ref(),
        &r,
        token(&i),
        s.node,
        s.incarnation
    )
    .is_err());
    assert_eq!(all(&e), before);
    assert!(observe(&e, &r).is_none());
}

#[test]
fn selection_replaces_one_row_monotonically_and_old_receipt_retry_cannot_regress_it() {
    let (e, r, first) = fixture_intent();
    publish(&e, &r, &first);
    e.write(verified(&e, &r, &first).unwrap().into_batch())
        .unwrap();
    let mut current = first.clone();
    for generation in 1..=16 {
        let mut manifest = current.manifest().unwrap();
        manifest.index += 1;
        manifest.scope.version += 1;
        current = CheckpointIntent::new(&r, &manifest, generation).unwrap();
        publish(&e, &r, &current);
        e.write(verified(&e, &r, &current).unwrap().into_batch())
            .unwrap();
    }
    let actual = observe(&e, &r).unwrap();
    assert_eq!(actual.intent(), &current);
    assert_eq!(actual.encode().unwrap().len(), 168);
    let rows: Vec<_> = e
        .snapshot()
        .unwrap()
        .iter(ColumnFamily::Default, &key(21, &[]), &key(22, &[]))
        .unwrap()
        .collect::<Result<_>>()
        .unwrap();
    assert_eq!(rows.len(), 1, "one latest record, no per-generation rows");
    let before = all(&e);
    for i in [&first, &current] {
        assert!(!verified(&e, &r, i).unwrap().changed());
        let s = source(&r);
        assert!(!plan_retry(
            e.snapshot().unwrap().as_ref(),
            &r,
            token(i),
            s.node,
            s.incarnation
        )
        .unwrap()
        .changed());
        assert_eq!(
            owner(&e, &r, i.binding(OwnerKind::Version).unwrap().descriptor.id).phase,
            PinPhase::Published
        );
    }
    assert_eq!(all(&e), before);
    let mut conflict = current.manifest().unwrap();
    conflict.scope.version += 1;
    let conflicting = CheckpointIntent::new(&r, &conflict, current.predecessor()).unwrap();
    publish(&e, &r, &conflicting);
    assert!(verified(&e, &r, &conflicting).is_err());
    assert_eq!(observe(&e, &r).unwrap(), actual);
}

#[test]
fn selection_requires_full_published_version_and_rejects_corrupt_or_cross_store_rows() {
    let (e, r, i) = fixture_intent();
    apply(&e, &r, LedgerRequest::BeginCheckpoint(i.clone()));
    assert!(verified(&e, &r, &i).is_err());
    publish(&e, &r, &i);
    let mut s = source(&r);
    s.active = false;
    assert!(plan_verified(e.snapshot().unwrap().as_ref(), &r, token(&i), s).is_err());
    e.write(verified(&e, &r, &i).unwrap().into_batch()).unwrap();
    let original = observe(&e, &r).unwrap();
    let key = selection_key(original.source, original.store, RegionId(100));
    let encoded = original.encode().unwrap();
    for byte in 0..encoded.len() {
        let mut bad = encoded.clone();
        bad[byte] ^= 1;
        let mut batch = WriteBatch::new();
        batch.put(ColumnFamily::Default, key.clone(), bad);
        e.write(batch).unwrap();
        assert!(selection(
            e.snapshot().unwrap().as_ref(),
            &r,
            original.source,
            original.store,
            RegionId(100)
        )
        .is_err());
    }
    let mut batch = WriteBatch::new();
    batch.put(ColumnFamily::Default, key, encoded.clone());
    let other = StoreIncarnation::from_bytes([9; 16]);
    batch.put(
        ColumnFamily::Default,
        selection_key(original.source, other, RegionId(100)),
        encoded,
    );
    e.write(batch).unwrap();
    assert!(selection(
        e.snapshot().unwrap().as_ref(),
        &r,
        original.source,
        other,
        RegionId(100)
    )
    .is_err());
    assert_eq!(observe(&e, &r).unwrap(), original);
    for range in [8..40, 40..48, 48..64, 64..72, 88..120, 128..136] {
        let mut raw = original.encode().unwrap();
        raw.truncate(raw.len() - 32);
        raw[range].fill(0);
        let mut batch = WriteBatch::new();
        batch.put(
            ColumnFamily::Default,
            selection_key(original.source, original.store, RegionId(100)),
            seal(raw),
        );
        e.write(batch).unwrap();
        assert!(selection(
            e.snapshot().unwrap().as_ref(),
            &r,
            original.source,
            original.store,
            RegionId(100)
        )
        .is_err());
    }
    let mut restored = WriteBatch::new();
    restored.put(
        ColumnFamily::Default,
        selection_key(original.source, original.store, RegionId(100)),
        original.encode().unwrap(),
    );
    e.write(restored).unwrap();
    let mut batch = WriteBatch::new();
    batch.delete(
        ColumnFamily::Default,
        owner_key(i.binding(OwnerKind::Version).unwrap().descriptor.id),
    );
    e.write(batch).unwrap();
    assert!(selection(
        e.snapshot().unwrap().as_ref(),
        &r,
        original.source,
        original.store,
        RegionId(100)
    )
    .is_err());
}

#[test]
fn selection_rows_under_older_ledger_header_cannot_be_silently_upgraded() {
    let (e, r, i) = fixture_intent();
    publish(&e, &r, &i);
    e.write(verified(&e, &r, &i).unwrap().into_batch()).unwrap();
    let mut header = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
    header.truncate(header.len() - 32);
    header[..8].copy_from_slice(ATOMIC_HEADER_MAGIC);
    let mut batch = WriteBatch::new();
    batch.put(ColumnFamily::Default, key(1, &[]), seal(header));
    // Isolate namespace 21: earlier V7 closure/refusal rows cannot explain refusal.
    for row in e
        .snapshot()
        .unwrap()
        .iter(ColumnFamily::Default, &key(18, &[]), &key(21, &[]))
        .unwrap()
    {
        batch.delete(ColumnFamily::Default, row.unwrap().0);
    }
    e.write(batch).unwrap();
    assert!(plan(&e, &r, &LedgerRequest::Initialize).is_err());
}

#[test]
fn wal_torn_selection_update_keeps_the_old_fact_and_all_version_pins() {
    let (seed, root, old) = fixture_intent();
    publish(&seed, &root, &old);
    seed.write(verified(&seed, &root, &old).unwrap().into_batch())
        .unwrap();
    let mut m = old.manifest().unwrap();
    m.index += 1;
    let next = CheckpointIntent::new(&root, &m, 1).unwrap();
    publish(&seed, &root, &next);
    let dir = std::env::temp_dir().join(format!(
        "kv9-selection-fact-{}",
        StoreIncarnation::mint().unwrap()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("metadata.wal");
    let e = WalEngine::open(&path).unwrap().0;
    let (_, mut batch) = crate::retention::tests::seed();
    for (k, v) in all(&seed) {
        batch.put(ColumnFamily::Default, k, v);
    }
    e.write(batch).unwrap();
    let before = std::fs::read(&path).unwrap().len();
    e.write(verified(&e, &root, &next).unwrap().into_batch())
        .unwrap();
    drop(e);
    let bytes = std::fs::read(&path).unwrap();
    for cut in [
        before,
        before + 1,
        before + (bytes.len() - before) / 2,
        bytes.len() - 1,
        bytes.len(),
    ] {
        let path = dir.join(format!("cut-{cut}"));
        std::fs::write(&path, &bytes[..cut]).unwrap();
        let e = WalEngine::open(&path).unwrap().0;
        assert_eq!(
            observe(&e, &root).unwrap().intent(),
            if cut == bytes.len() { &next } else { &old }
        );
        for i in [&old, &next] {
            assert_eq!(
                owner(
                    &e,
                    &root,
                    i.binding(OwnerKind::Version).unwrap().descriptor.id
                )
                .phase,
                PinPhase::Published
            );
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

mod metadata;
