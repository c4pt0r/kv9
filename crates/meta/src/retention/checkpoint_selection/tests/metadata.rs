use super::*;
use crate::retention::bootstrap_reader;
use kv9_common::{StoreIdentity, META_REGION_0};

fn fixture() -> (MemEngine, RootDescriptor, CheckpointIntent, OwnerBinding) {
    let (e, r, v, b) = bootstrap_reader::tests::setup();
    let LedgerRequest::BeginBootstrapRead { pending, .. } =
        bootstrap_reader::tests::begin_request(&r, &v)
    else {
        panic!()
    };
    let i = checkpoint_intent::checkpoint_intent(e.snapshot().unwrap().as_ref(), &r, pending.owner)
        .unwrap()
        .unwrap();
    (e, r, i, b)
}
fn admit(e: &MemEngine, r: &RootDescriptor, i: &CheckpointIntent) -> OwnerBinding {
    let id = StoreIdentity::for_voter(r, NodeId(1)).unwrap();
    let b = bootstrap_reader::binding(r, &i.manifest().unwrap(), i.predecessor(), id).unwrap();
    apply(
        e,
        r,
        LedgerRequest::BeginBootstrapRead {
            pending: token(i),
            source: id.node_id,
            store: id.store_incarnation,
        },
    );
    apply(e, r, LedgerRequest::Publish(b.token()));
    b
}
fn fact(e: &impl Engine, r: &RootDescriptor) -> Option<Selection> {
    let s = source(r);
    selection(
        e.snapshot().unwrap().as_ref(),
        r,
        s.node,
        s.incarnation,
        META_REGION_0,
    )
    .unwrap()
}
fn retire(e: &MemEngine, r: &RootDescriptor, i: &CheckpointIntent) {
    let id = StoreIdentity::for_voter(r, NodeId(1)).unwrap();
    e.write(
        bootstrap_reader::plan_verified(e.snapshot().unwrap().as_ref(), r, token(i), id)
            .unwrap()
            .into_batch(),
    )
    .unwrap();
}

#[test]
fn metadata_fact_requires_exact_published_store_grant_and_independent_verification() {
    let (e, r, i, b) = fixture();
    assert!(verified(&e, &r, &i).is_err());
    let id = StoreIdentity::for_voter(&r, NodeId(1)).unwrap();
    apply(
        &e,
        &r,
        LedgerRequest::BeginBootstrapRead {
            pending: token(&i),
            source: id.node_id,
            store: id.store_incarnation,
        },
    );
    assert!(verified(&e, &r, &i).is_err());
    apply(&e, &r, LedgerRequest::Publish(b.token()));
    let request = LedgerRequest::RecordCheckpointSelection {
        pending: token(&i),
        source: id.node_id,
        store: id.store_incarnation,
    };
    let before = all(&e);
    assert!(plan(&e, &r, &request).is_err());
    assert_eq!(all(&e), before);
    for change in 0..2 {
        let mut s = source(&r);
        if change == 0 {
            s.incarnation = StoreIncarnation::from_bytes([8; 16]);
        } else {
            s.node = NodeId(9);
        }
        assert!(plan_verified(e.snapshot().unwrap().as_ref(), &r, token(&i), s).is_err());
    }
    e.write(verified(&e, &r, &i).unwrap().into_batch()).unwrap();
    let actual = fact(&e, &r).unwrap();
    assert_eq!(actual.bootstrap, Some(b.token()));
    assert_eq!(actual.encode().unwrap().len(), 192);
    assert!(selection(
        e.snapshot().unwrap().as_ref(),
        &r,
        id.node_id,
        id.store_incarnation,
        RegionId(100)
    )
    .unwrap()
    .is_none());
    assert!(!plan_retry(
        e.snapshot().unwrap().as_ref(),
        &r,
        token(&i),
        id.node_id,
        id.store_incarnation
    )
    .unwrap()
    .changed());
    assert_eq!(
        owner(&e, &r, i.binding(OwnerKind::Version).unwrap().descriptor.id).phase,
        PinPhase::Published
    );
}

#[test]
fn metadata_first_selection_upgrades_v13_without_rewriting_bootstrap_index_or_origin() {
    let (e, r, i, _) = fixture();
    let grant = admit(&e, &r, &i);
    let retained: Vec<_> = all(&e)
        .into_iter()
        .filter(|(k, _)| *k >= key(28, &[]) && *k < key(31, &[]))
        .collect();
    assert!(!retained.is_empty());
    let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
    h.truncate(h.len() - 32);
    h[..8].copy_from_slice(BOOTSTRAP_INDEX_HEADER_MAGIC);
    let mut downgrade = WriteBatch::new();
    // Explicit v13 fixture predates the image census and its mutation stamps.
    for (k, _) in all(&e) {
        if k >= key(33, &[]) && k < key(35, &[]) {
            downgrade.delete(ColumnFamily::Default, k);
        }
    }
    downgrade.put(ColumnFamily::Default, key(1, &[]), seal(h));
    e.write(downgrade).unwrap();
    let before = owner(&e, &r, grant.descriptor.id);
    assert_eq!(before.phase, PinPhase::Published);
    assert!(fact(&e, &r).is_none());
    e.write(verified(&e, &r, &i).unwrap().into_batch()).unwrap();
    assert!(e
        .get(ColumnFamily::Default, &key(1, &[]))
        .unwrap()
        .unwrap()
        .starts_with(HEADER_MAGIC));
    let after = owner(&e, &r, grant.descriptor.id);
    assert_eq!(after, before);
    for (k, v) in retained {
        assert_eq!(e.get(ColumnFamily::Default, &k).unwrap(), Some(v));
    }
    assert_eq!(fact(&e, &r).unwrap().intent(), &i);
}

#[test]
fn metadata_selection_advances_after_old_grant_retirement_and_old_retry_keeps_new_reference() {
    let (e, r, first, b) = fixture();
    admit(&e, &r, &first);
    e.write(verified(&e, &r, &first).unwrap().into_batch())
        .unwrap();
    retire(&e, &r, &first);
    assert_eq!(owner(&e, &r, b.descriptor.id).phase, PinPhase::Released);
    assert!(!verified(&e, &r, &first).unwrap().changed());
    let mut m = first.manifest().unwrap();
    m.index += 1;
    let next = CheckpointIntent::new(&r, &m, 1).unwrap();
    publish(&e, &r, &next);
    let new_grant = admit(&e, &r, &next);
    e.write(verified(&e, &r, &next).unwrap().into_batch())
        .unwrap();
    let current = fact(&e, &r).unwrap();
    assert_eq!(current.intent(), &next);
    assert_eq!(current.bootstrap, Some(new_grant.token()));
    let s = source(&r);
    assert!(!plan_retry(
        e.snapshot().unwrap().as_ref(),
        &r,
        token(&first),
        s.node,
        s.incarnation
    )
    .unwrap()
    .changed());
    assert_eq!(fact(&e, &r), Some(current));
    m.scope.version += 1;
    let conflict = CheckpointIntent::new(&r, &m, 2).unwrap();
    publish(&e, &r, &conflict);
    admit(&e, &r, &conflict);
    assert!(verified(&e, &r, &conflict).is_err());
    assert_eq!(
        e.snapshot()
            .unwrap()
            .iter(ColumnFamily::Default, &key(31, &[]), &key(32, &[]))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn metadata_selection_rejects_broken_grant_links_and_older_header_without_repair() {
    for damage in 0..4 {
        let (e, r, i, b) = fixture();
        admit(&e, &r, &i);
        e.write(verified(&e, &r, &i).unwrap().into_batch()).unwrap();
        let s = source(&r);
        let mut batch = WriteBatch::new();
        let k = selection_key(s.node, s.incarnation, META_REGION_0);
        match damage {
            0 | 1 => {
                let mut bytes = e.get(ColumnFamily::Default, &k).unwrap().unwrap();
                bytes.truncate(bytes.len() - 32);
                if damage == 0 {
                    bytes[136] ^= 1;
                } else {
                    bytes[152..160].copy_from_slice(&2u64.to_be_bytes());
                }
                batch.put(ColumnFamily::Default, k, seal(bytes));
            }
            2 => {
                let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
                h.truncate(h.len() - 32);
                h[..8].copy_from_slice(BOOTSTRAP_INDEX_HEADER_MAGIC);
                batch.put(ColumnFamily::Default, key(1, &[]), seal(h));
            }
            _ => {
                batch.delete(ColumnFamily::Default, owner_key(b.descriptor.id));
                batch.delete(
                    ColumnFamily::Default,
                    owner_registration_key(b.descriptor.id),
                );
            }
        }
        e.write(batch).unwrap();
        let before = all(&e);
        assert!(selection(
            e.snapshot().unwrap().as_ref(),
            &r,
            s.node,
            s.incarnation,
            META_REGION_0
        )
        .is_err());
        assert!(plan_retry(
            e.snapshot().unwrap().as_ref(),
            &r,
            token(&i),
            s.node,
            s.incarnation
        )
        .is_err());
        if damage == 2 {
            assert!(plan(&e, &r, &LedgerRequest::Initialize).is_err());
        }
        assert_eq!(all(&e), before);
    }
}

#[test]
fn wal_cuts_keep_metadata_selection_and_bootstrap_identity_atomic_without_releasing_versions() {
    let (memory, r, first, _) = fixture();
    admit(&memory, &r, &first);
    memory
        .write(verified(&memory, &r, &first).unwrap().into_batch())
        .unwrap();
    retire(&memory, &r, &first);
    let mut m = first.manifest().unwrap();
    m.index += 1;
    let next = CheckpointIntent::new(&r, &m, 1).unwrap();
    publish(&memory, &r, &next);
    admit(&memory, &r, &next);
    let dir = std::env::temp_dir().join(format!(
        "kv9-metadata-selection-{}",
        StoreIncarnation::mint().unwrap()
    ));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("meta.wal");
    let (e, _) = WalEngine::open(&path).unwrap();
    let (_, mut batch) = crate::retention::tests::seed();
    for (k, v) in all(&memory) {
        batch.put(ColumnFamily::Default, k, v);
    }
    e.write(batch).unwrap();
    let size = std::fs::read(&path).unwrap().len();
    e.write(verified(&e, &r, &next).unwrap().into_batch())
        .unwrap();
    drop(e);
    let bytes = std::fs::read(&path).unwrap();
    for cut in [
        size,
        size + 1,
        size + (bytes.len() - size) / 2,
        bytes.len() - 1,
        bytes.len(),
    ] {
        let path = dir.join(format!("cut-{cut}"));
        std::fs::write(&path, &bytes[..cut]).unwrap();
        let (e, _) = WalEngine::open(path).unwrap();
        assert_eq!(
            fact(&e, &r).unwrap().intent(),
            if cut == bytes.len() { &next } else { &first }
        );
        let p = verified(&e, &r, &next).unwrap();
        assert_eq!(p.changed(), cut != bytes.len());
        e.write(p.into_batch()).unwrap();
        for i in [&first, &next] {
            assert_eq!(
                owner(&e, &r, i.binding(OwnerKind::Version).unwrap().descriptor.id).phase,
                PinPhase::Published
            );
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}
