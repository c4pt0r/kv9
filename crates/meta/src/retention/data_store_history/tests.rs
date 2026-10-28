use super::*;
use crate::codec::{memcmp_uint, ColumnValue, RowValue};
use crate::data_groups::{self, abort, activation, evidence, migration, removal, CreationIntent};
use crate::schema::{ColumnId, NODES_DESC, TASKS_DESC};
use crate::store::MetaStore;
use kv9_common::{AppliedPosition, RootVoter};
use kv9_engine::{Engine, MemEngine, WalEngine};
use std::sync::Arc;
mod configuration_origins;

fn fixture() -> (MetaStore<MemEngine>, RootDescriptor) {
    let (root, seed) = crate::retention::tests::seed_with_voters(
        (1..=3)
            .map(|id| RootVoter {
                node_id: NodeId(id),
                addr: format!("127.0.0.1:{}", 18000 + id).parse().unwrap(),
                store_incarnation: StoreIncarnation::from_bytes([id as u8; 16]),
            })
            .collect(),
    );
    let e = Arc::new(MemEngine::new());
    e.write(seed).unwrap();
    let store = MetaStore::new(e);
    let mut t = store.begin().unwrap();
    for id in 1..=5 {
        let mut r = RowValue::new();
        r.set(
            ColumnId(2),
            ColumnValue::Text(format!("127.0.0.1:{}", 18000 + id)),
        );
        r.set(ColumnId(3), ColumnValue::Uint(2));
        r.set(ColumnId(4), ColumnValue::Uint(0));
        r.set(ColumnId(5), ColumnValue::Bytes(vec![id as u8; 16]));
        t.insert(&NODES_DESC, &[memcmp_uint(id)], r).unwrap();
    }
    t.commit().unwrap();
    apply(store.engine().as_ref(), &root, LedgerRequest::Initialize);
    (store, root)
}
fn apply(e: &impl Engine, root: &RootDescriptor, request: LedgerRequest) {
    let plan = plan_retention(e.snapshot().unwrap().as_ref(), root, &request).unwrap();
    e.write(plan.into_batch()).unwrap();
}
fn group<E: Engine>(s: &MetaStore<E>, n: u8) -> CreationIntent {
    let mut t = s.begin().unwrap();
    let c =
        data_groups::plan_empty_group(&mut t, [n; 16], &[NodeId(1), NodeId(2), NodeId(3)]).unwrap();
    t.commit().unwrap();
    c
}
fn request(e: &impl Engine, r: &RootDescriptor) -> LedgerRequest {
    match observe(e.snapshot().unwrap().as_ref(), r).unwrap() {
        Work::Page {
            progress,
            revision,
            after,
            examined,
        } => {
            assert!(examined <= PAGE);
            progress.request(revision, after)
        }
        _ => panic!("expected pending census page"),
    }
}
fn complete(e: &impl Engine, r: &RootDescriptor) -> usize {
    for pages in 0..200 {
        if matches!(
            observe(e.snapshot().unwrap().as_ref(), r).unwrap(),
            Work::Complete { .. }
        ) {
            return pages;
        }
        apply(e, r, request(e, r));
    }
    panic!("census failed to finish")
}
fn collect(e: &impl Engine, r: &RootDescriptor, c: &CreationIntent) -> Vec<Origin> {
    let view = e.snapshot().unwrap();
    let mut after = None;
    let mut found = Vec::new();
    loop {
        let page = origins(view.as_ref(), r, c.region(), c.digest(), after).unwrap();
        assert!(page.origins.len() <= PAGE);
        found.extend(page.origins);
        match page.next {
            Some(cursor) => after = Some(cursor),
            None => return found,
        }
    }
}
#[test]
fn placement_census_keeps_aborted_destination_and_removed_initial_store() {
    let (s, r) = fixture();
    let c = group(&s, 1);
    let mut t = s.begin().unwrap();
    activation::plan_activation(&mut t, &c).unwrap();
    let (old, _) = migration::plan_migration(&mut t, [2; 16], c.task(), NodeId(4)).unwrap();
    abort::plan_abort(&mut t, old.operation(), [9; 32]).unwrap();
    t.commit().unwrap();
    let mut t = s.begin().unwrap();
    let (new, _) = migration::plan_migration(&mut t, [3; 16], c.task(), NodeId(5)).unwrap();
    let receipt = evidence::InstallEvidence::receipt(
        &new,
        StoreIncarnation::from_bytes([21; 16]),
        RootDigest::sha256(b"image"),
        [22; 32],
        AppliedPosition { term: 3, index: 12 },
    )
    .unwrap();
    evidence::plan_install_evidence(&mut t, &receipt).unwrap();
    removal::plan_removal(&mut t, new.operation(), c.replicas()[0]).unwrap();
    t.commit().unwrap();
    let e = s.engine().as_ref();
    assert_eq!(complete(e, &r), 2);
    let found = collect(e, &r, &c);
    assert_eq!(found.len(), 8);
    assert_eq!(
        found
            .iter()
            .map(|o| o.replica.node.0)
            .collect::<std::collections::BTreeSet<_>>(),
        (1..=5).collect()
    );
    assert!(found
        .iter()
        .any(|o| o.kind == 108 && o.replica.node == NodeId(4)));
    assert!(found
        .iter()
        .any(|o| o.kind == 106 && o.replica == c.replicas()[0]));
    for o in found {
        assert_eq!(o.creation, c.digest());
        assert_eq!(
            o.replica.incarnation,
            StoreIncarnation::from_bytes([o.replica.node.0 as u8; 16])
        );
    }
    assert!(origins(
        e.snapshot().unwrap().as_ref(),
        &r,
        c.region(),
        RootDigest::sha256(b"another creation"),
        None
    )
    .is_err());
}
#[test]
fn changed_catalog_restarts_behind_cursor_and_refuses_stale_or_skipped_page() {
    let (s, r) = fixture();
    let mut t = s.begin().unwrap();
    let reserved = t.allocate_id(crate::store::SequenceKind::Task).unwrap();
    t.commit().unwrap();
    let groups = (1..=13).map(|n| group(&s, n)).collect::<Vec<_>>();
    let e = s.engine().as_ref();
    let first = request(e, &r);
    apply(e, &r, first.clone());
    // Exact replay of the only committed page is an acknowledgement, not a second advance.
    let before = crate::retention::tests::all(e);
    apply(e, &r, first.clone());
    assert_eq!(crate::retention::tests::all(e), before);
    let stale = request(e, &r);
    let mut t = s.begin().unwrap();
    let mut row = RowValue::new();
    for (col, value) in [(1, reserved), (2, 101), (4, 0), (5, 0)] {
        row.set(ColumnId(col), ColumnValue::Uint(value));
    }
    row.set(ColumnId(3), ColumnValue::Bytes(groups[0].encode()));
    t.insert(&TASKS_DESC, &[memcmp_uint(reserved)], row)
        .unwrap();
    t.commit().unwrap();
    assert!(plan_retention(e.snapshot().unwrap().as_ref(), &r, &stale).is_err());
    let Work::Page {
        progress,
        revision,
        after,
        examined,
    } = observe(e.snapshot().unwrap().as_ref(), &r).unwrap()
    else {
        panic!()
    };
    assert_eq!(after, None);
    assert_eq!(examined, PAGE);
    assert!(plan_retention(
        e.snapshot().unwrap().as_ref(),
        &r,
        &progress.request(revision, Some(groups[8].task()))
    )
    .is_err());
    // An old successful receipt remains harmless even after source changes.
    apply(e, &r, first);
    assert!(matches!(
        observe(e.snapshot().unwrap().as_ref(), &r).unwrap(),
        Work::Page { after: None, .. }
    ));
    assert_eq!(complete(e, &r), 4);
    for c in groups {
        assert_eq!(collect(e, &r, &c).len(), 3);
    }
}
#[test]
fn erased_history_is_sticky_and_missing_or_regressed_revision_cannot_look_empty() {
    let (s, r) = fixture();
    let c = group(&s, 1);
    let e = s.engine().as_ref();
    let last = request(e, &r);
    apply(e, &r, last.clone());
    assert_eq!(complete(e, &r), 0);
    let original = e
        .get(ColumnFamily::Default, task_history::KEY)
        .unwrap()
        .unwrap();
    let mut b = WriteBatch::new();
    b.delete(ColumnFamily::Default, task_history::KEY.to_vec());
    e.write(b).unwrap();
    assert!(observe(e.snapshot().unwrap().as_ref(), &r).is_err());
    assert!(plan_retention(e.snapshot().unwrap().as_ref(), &r, &last).is_err());
    let mut b = WriteBatch::new();
    b.put(ColumnFamily::Default, task_history::KEY.to_vec(), original);
    e.write(b).unwrap();
    let mut t = s.begin().unwrap();
    t.delete(&TASKS_DESC, &[memcmp_uint(c.task())]).unwrap();
    t.commit().unwrap();
    assert!(matches!(
        observe(e.snapshot().unwrap().as_ref(), &r).unwrap(),
        Work::Uncertain(_)
    ));
    assert!(origins(
        e.snapshot().unwrap().as_ref(),
        &r,
        c.region(),
        c.digest(),
        None
    )
    .is_err());
    group(&s, 2);
    assert!(matches!(
        observe(e.snapshot().unwrap().as_ref(), &r).unwrap(),
        Work::Uncertain(_)
    ));
}
#[test]
fn unknown_or_corrupt_task_cannot_complete_and_origins_revalidate_source() {
    for defect in ["kind", "payload", "key"] {
        let (s, r) = fixture();
        let c = group(&s, 1);
        let e = s.engine().as_ref();
        let k = store_history::task_key(c.task()).unwrap();
        let mut row =
            RowValue::decode(&e.get(ColumnFamily::Default, &k).unwrap().unwrap()).unwrap();
        match defect {
            "kind" => row.set(ColumnId(2), ColumnValue::Uint(999)),
            "payload" => row.set(ColumnId(3), ColumnValue::Bytes(vec![0])),
            _ => row.set(ColumnId(1), ColumnValue::Uint(c.task() + 1)),
        };
        let mut b = WriteBatch::new();
        b.put(ColumnFamily::Default, k, row.encode());
        e.write(b).unwrap();
        assert!(
            observe(e.snapshot().unwrap().as_ref(), &r).is_err(),
            "{defect}"
        );
    }
    let (s, r) = fixture();
    let c = group(&s, 1);
    let e = s.engine().as_ref();
    complete(e, &r);
    let o = collect(e, &r, &c)[0];
    let mut b = WriteBatch::new();
    let mut forged = o;
    forged.source_digest = RootDigest::sha256(b"other source");
    b.put(
        ColumnFamily::Default,
        origin_key(&o),
        encode_origin(r.digest(), &forged),
    );
    e.write(b).unwrap();
    assert!(origins(
        e.snapshot().unwrap().as_ref(),
        &r,
        c.region(),
        c.digest(),
        None
    )
    .is_err());
}
#[test]
fn census_request_upgrade_empty_catalog_and_orphan_fences_are_canonical() {
    let (s, r) = fixture();
    let e = s.engine().as_ref();
    let req = request(e, &r);
    let encoded = encode_request(r.digest(), &req).unwrap();
    assert_eq!(decode_request(&encoded, r.digest()).unwrap(), req);
    for cut in 0..encoded.len() {
        assert!(decode_request(&encoded[..cut], r.digest()).is_err());
    }
    assert!(encode_request(
        r.digest(),
        &LedgerRequest::AdvanceDataStoreHistory {
            expected: 0,
            revision: 0,
            after: Some(99)
        }
    )
    .is_err());
    let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
    h.truncate(h.len() - 32);
    h[..8].copy_from_slice(DATA_FENCE_HEADER_MAGIC);
    let mut b = WriteBatch::new();
    b.put(ColumnFamily::Default, key(1, &[]), seal(h.clone()));
    e.write(b).unwrap();
    assert!(matches!(
        observe(e.snapshot().unwrap().as_ref(), &r).unwrap(),
        Work::Upgrade
    ));
    assert!(plan_retention(e.snapshot().unwrap().as_ref(), &r, &req).is_err());
    apply(e, &r, LedgerRequest::Initialize);
    assert_eq!(complete(e, &r), 1);
    assert!(matches!(
        observe(e.snapshot().unwrap().as_ref(), &r).unwrap(),
        Work::Complete { revision: 0 }
    ));
    let mut b = WriteBatch::new();
    b.put(ColumnFamily::Default, key(1, &[]), seal(h));
    e.write(b).unwrap();
    assert!(observe(e.snapshot().unwrap().as_ref(), &r).is_err());
    assert!(plan_retention(
        e.snapshot().unwrap().as_ref(),
        &r,
        &LedgerRequest::Initialize
    )
    .is_err());
    let mut b = WriteBatch::new();
    b.delete(ColumnFamily::Default, key(1, &[]));
    e.write(b).unwrap();
    assert!(observe(e.snapshot().unwrap().as_ref(), &r).is_err());
}
#[test]
fn persisted_page_and_task_revision_survive_wal_reopen_and_continue() {
    let (s, r) = fixture();
    let groups = (1..=17).map(|n| group(&s, n)).collect::<Vec<_>>();
    let source = s.engine();
    let dir = std::env::temp_dir().join(format!(
        "kv9-data-store-history-{}",
        StoreIncarnation::mint().unwrap()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("metadata.wal");
    let view = source.snapshot().unwrap();
    let mut seed = WriteBatch::new();
    for row in view.iter(ColumnFamily::Default, &[], &[255]).unwrap() {
        let (k, v) = row.unwrap();
        seed.put(ColumnFamily::Default, k, v);
    }
    drop(view);
    let old;
    {
        let e = WalEngine::open(&path).unwrap().0;
        e.write(seed).unwrap();
        old = request(&e, &r);
        apply(&e, &r, old.clone());
    }
    {
        let e = WalEngine::open(&path).unwrap().0;
        apply(&e, &r, old);
        assert_eq!(complete(&e, &r), 4);
        for c in &groups {
            assert_eq!(collect(&e, &r, c).len(), 3);
        }
    }
    {
        let e = WalEngine::open(&path).unwrap().0;
        assert_eq!(complete(&e, &r), 0);
        assert_eq!(
            task_history::revision(e.snapshot().unwrap().as_ref())
                .unwrap()
                .sequence,
            17
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn missing_origins_counts_and_borrowed_or_stale_cursors_never_prove_coverage() {
    let (s, r) = fixture();
    let mut t = s.begin().unwrap();
    let c = data_groups::plan_empty_group(
        &mut t,
        [19; 16],
        &[NodeId(1), NodeId(2), NodeId(3), NodeId(4), NodeId(5)],
    )
    .unwrap();
    t.commit().unwrap();
    let e = s.engine().as_ref();
    complete(e, &r);
    let page = origins(
        e.snapshot().unwrap().as_ref(),
        &r,
        c.region(),
        c.digest(),
        None,
    )
    .unwrap();
    assert_eq!(page.total_origins, 5);
    let cursor = page.next.unwrap();
    let last = origins(
        e.snapshot().unwrap().as_ref(),
        &r,
        c.region(),
        c.digest(),
        Some(cursor),
    )
    .unwrap()
    .origins[0];
    let k = origin_key(&last);
    let bytes = e.get(ColumnFamily::Default, &k).unwrap().unwrap();
    let mut b = WriteBatch::new();
    b.delete(ColumnFamily::Default, k.clone());
    e.write(b).unwrap();
    assert!(origins(
        e.snapshot().unwrap().as_ref(),
        &r,
        c.region(),
        c.digest(),
        Some(cursor)
    )
    .is_err());
    assert!(origins(
        e.snapshot().unwrap().as_ref(),
        &r,
        c.region(),
        c.digest(),
        None
    )
    .is_err());
    let mut b = WriteBatch::new();
    b.put(ColumnFamily::Default, k, bytes);
    e.write(b).unwrap();
    let other = group(&s, 20);
    complete(e, &r);
    assert!(origins(
        e.snapshot().unwrap().as_ref(),
        &r,
        other.region(),
        other.digest(),
        Some(cursor)
    )
    .is_err());
    assert!(origins(
        e.snapshot().unwrap().as_ref(),
        &r,
        c.region(),
        c.digest(),
        Some(cursor)
    )
    .is_err());
    assert_eq!(collect(e, &r, &c).len(), 5);
    let mut b = WriteBatch::new();
    b.delete(ColumnFamily::Default, count_key(c.region(), c.digest()));
    e.write(b).unwrap();
    assert!(origins(
        e.snapshot().unwrap().as_ref(),
        &r,
        c.region(),
        c.digest(),
        None
    )
    .is_err());
    // Force a rescan: existing origin rows without their atomic count cannot be repaired by guessing.
    group(&s, 21);
    let request = request(e, &r);
    assert!(plan_retention(e.snapshot().unwrap().as_ref(), &r, &request).is_err());
}

#[test]
fn each_census_page_visits_at_most_four_tasks_and_one_lookahead() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Counted<'a> {
        view: &'a dyn ReadView,
        gets: AtomicUsize,
        rows: AtomicUsize,
    }
    impl ReadView for Counted<'_> {
        fn scan(
            &self,
            cf: ColumnFamily,
            start: &[u8],
            end: &[u8],
            limit: usize,
        ) -> Result<Vec<kv9_engine::ScanEntry>> {
            let rows = self.view.scan(cf, start, end, limit)?;
            self.rows.fetch_add(rows.len(), Ordering::SeqCst);
            Ok(rows)
        }
        fn seek_le(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<kv9_engine::ScanEntry>> {
            self.gets.fetch_add(1, Ordering::SeqCst);
            self.view.seek_le(cf, key)
        }
        fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Vec<u8>>> {
            self.gets.fetch_add(1, Ordering::SeqCst);
            self.view.get(cf, key)
        }
        fn iter<'a>(
            &'a self,
            cf: ColumnFamily,
            start: &[u8],
            end: &[u8],
        ) -> Result<Box<dyn Iterator<Item = Result<kv9_engine::ScanEntry>> + 'a>> {
            Ok(Box::new(self.view.iter(cf, start, end)?.inspect(|_| {
                self.rows.fetch_add(1, Ordering::SeqCst);
            })))
        }
        fn iter_rev<'a>(
            &'a self,
            cf: ColumnFamily,
            start: &[u8],
            end: &[u8],
        ) -> Result<Box<dyn Iterator<Item = Result<kv9_engine::ScanEntry>> + 'a>> {
            Ok(Box::new(self.view.iter_rev(cf, start, end)?.inspect(
                |_| {
                    self.rows.fetch_add(1, Ordering::SeqCst);
                },
            )))
        }
    }
    let (s, r) = fixture();
    for n in 1..=41 {
        group(&s, n);
    }
    let e = s.engine().as_ref();
    let req = request(e, &r);
    let view = e.snapshot().unwrap();
    let counted = Counted {
        view: view.as_ref(),
        gets: AtomicUsize::new(0),
        rows: AtomicUsize::new(0),
    };
    let plan = plan_retention(&counted, &r, &req).unwrap();
    assert_eq!(counted.rows.load(Ordering::SeqCst), PAGE + 1);
    assert!(
        counted.gets.load(Ordering::SeqCst) < 40,
        "unexpected unbounded point reads"
    );
    e.write(plan.into_batch()).unwrap();
    let Work::Page { progress, .. } = observe(e.snapshot().unwrap().as_ref(), &r).unwrap() else {
        panic!()
    };
    assert_eq!(progress.sequence, 1);
    assert!(!progress.complete);
}
