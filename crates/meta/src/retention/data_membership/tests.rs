use super::*;
use crate::codec::{memcmp_uint, ColumnValue, RowValue};
use crate::data_groups::{self, activation, migration, CreationIntent};
use crate::schema::{ColumnId, NODES_DESC};
use crate::store::MetaStore;
use kv9_common::RootVoter;
use kv9_engine::{Engine, MemEngine, WalEngine};
use kv9_raft::storage::DiskRaftStorage;
use std::sync::Arc;
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
fn complete(e: &impl Engine, r: &RootDescriptor) -> u64 {
    for _ in 0..100 {
        match data_store_history::observe(e.snapshot().unwrap().as_ref(), r).unwrap() {
            data_store_history::Work::Complete { revision } => return revision,
            data_store_history::Work::Page {
                progress,
                revision,
                after,
                ..
            } => apply(e, r, progress.request(revision, after)),
            _ => panic!("expected current placement census"),
        }
    }
    panic!("census stalled")
}
fn q(c: &CreationIntent, revision: u64, expected: u64) -> Request {
    Request {
        source: Source {
            region: c.region(),
            creation: c.digest(),
            node: c.replicas()[0].node,
            store: c.replicas()[0].incarnation,
        },
        task: c.task(),
        revision,
        expected,
    }
}
fn endpoint(root: &RootDescriptor, source: Source) -> NodeEndpoint {
    NodeEndpoint {
        node: source.node,
        incarnation: source.store,
        address: root.voter(source.node).unwrap().addr,
        generation: 0,
        previous_address: None,
        active: true,
    }
}
fn record(
    e: &impl Engine,
    r: &RootDescriptor,
    q: Request,
    cursor: Option<MembershipCursor>,
    seen: usize,
    page: &MembershipPage,
) {
    let plan = plan_verified(
        e.snapshot().unwrap().as_ref(),
        r,
        q,
        endpoint(r, q.source),
        cursor,
        seen,
        page,
    )
    .unwrap();
    e.write(plan.into_batch()).unwrap();
}
struct History {
    path: std::path::PathBuf,
    storage: DiskRaftStorage,
}
impl History {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "kv9-member-ledger-{}",
            StoreIncarnation::mint().unwrap()
        ));
        let storage = DiskRaftStorage::open(&path, &[1, 2, 3, 4, 5]).unwrap().0;
        Self { path, storage }
    }
}
impl Drop for History {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
#[test]
fn membership_request_canonical_plain_planner_refuses_and_source_revision_is_exact() {
    let (s, r) = fixture();
    let c = group(&s, 1);
    let e = s.engine().as_ref();
    let revision = complete(e, &r);
    let q = q(&c, revision, 0);
    let request = LedgerRequest::RecordDataMembership(q);
    let bytes = encode_request(r.digest(), &request).unwrap();
    assert_eq!(decode_request(&bytes, r.digest()).unwrap(), request);
    for end in 0..bytes.len() {
        assert!(decode_request(&bytes[..end], r.digest()).is_err());
    }
    for field in 0..5 {
        let mut invalid = q;
        match field {
            0 => invalid.source.node = NodeId(0),
            1 => invalid.source.region = RegionId(1),
            2 => invalid.task = 0,
            3 => invalid.revision = 0,
            _ => invalid.expected = u64::MAX,
        }
        assert!(encode_request(r.digest(), &LedgerRequest::RecordDataMembership(invalid)).is_err());
    }
    assert!(plan_retention(e.snapshot().unwrap().as_ref(), &r, &request).is_err());
    let h = History::new();
    let page = h.storage.membership_history_page(None).unwrap();
    for field in 0..4 {
        let mut bad = q;
        match field {
            0 => bad.source.store = StoreIncarnation::from_bytes([9; 16]),
            1 => bad.source.creation = RootDigest::sha256(b"wrong"),
            2 => bad.revision += 1,
            _ => bad.expected += 1,
        }
        assert!(plan_verified(
            e.snapshot().unwrap().as_ref(),
            &r,
            bad,
            endpoint(&r, bad.source),
            None,
            0,
            &page
        )
        .is_err());
    }
    assert!(observation(e.snapshot().unwrap().as_ref(), &r, q.source)
        .unwrap()
        .is_none());
}
#[test]
fn membership_pages_preserve_unknown_nodes_retry_and_counted_queries_across_wal_reopen() {
    let (s, r) = fixture();
    let c = group(&s, 1);
    let e = s.engine().as_ref();
    let revision = complete(e, &r);
    let first = q(&c, revision, 0);
    let h = History::new();
    let page = h.storage.membership_history_page(None).unwrap();
    record(e, &r, first, None, 0, &page);
    let p = observation(e.snapshot().unwrap().as_ref(), &r, first.source)
        .unwrap()
        .unwrap();
    assert_eq!(
        (p.sequence, p.seen, p.unmatched_nodes, p.retained_nodes),
        (1, 4, 1, 4)
    );
    assert!(!p.round_complete());
    let before = crate::retention::tests::all(e);
    assert!(is_retry(e.snapshot().unwrap().as_ref(), &r, first).unwrap());
    e.write(
        plan_retry(e.snapshot().unwrap().as_ref(), &r, first)
            .unwrap()
            .into_batch(),
    )
    .unwrap();
    assert_eq!(crate::retention::tests::all(e), before);
    let (cursor, seen) =
        continuation(e.snapshot().unwrap().as_ref(), &r, q(&c, revision, 1)).unwrap();
    let last = h.storage.membership_history_page(cursor).unwrap();
    record(e, &r, q(&c, revision, 1), cursor, seen, &last);
    let view = e.snapshot().unwrap();
    let all = members(view.as_ref(), &r, first.source, None).unwrap();
    assert!(all.progress.round_complete());
    assert!(all.progress.has_known_gaps());
    assert_eq!(all.progress.unmatched_nodes, 2);
    assert_eq!(all.members[3].node, NodeId(4));
    assert!(all.members[3].placement.is_none());
    let tail = members(view.as_ref(), &r, first.source, all.next).unwrap();
    assert_eq!(tail.members[0].node, NodeId(5));
    assert!(tail.next.is_none());
    // Reconfirming a stable complete history produces no new durable fact.
    assert!(!plan_verified(
        view.as_ref(),
        &r,
        q(&c, revision, 2),
        endpoint(&r, first.source),
        None,
        0,
        &page
    )
    .unwrap()
    .changed());
    let path = h.path.join("meta.wal");
    let (wal, _) = WalEngine::open(&path).unwrap();
    let mut batch = WriteBatch::new();
    for cf in [
        ColumnFamily::Default,
        ColumnFamily::Write,
        ColumnFamily::Lock,
    ] {
        for row in view.iter(cf, &[], &[255]).unwrap() {
            let (k, v) = row.unwrap();
            batch.put(cf, k, v);
        }
    }
    wal.write(batch).unwrap();
    drop(wal);
    let (wal, _) = WalEngine::open(&path).unwrap();
    let reopened = members(wal.snapshot().unwrap().as_ref(), &r, first.source, None).unwrap();
    assert_eq!(reopened.progress, all.progress);
    assert_eq!(reopened.members, all.members);
    let mut corrupt = WriteBatch::new();
    corrupt.delete(
        ColumnFamily::Default,
        member_key(first.source, NodeId(2)).unwrap(),
    );
    e.write(corrupt).unwrap();
    assert!(members(e.snapshot().unwrap().as_ref(), &r, first.source, None).is_err());
}
#[test]
fn membership_placement_changes_restart_progress_without_remapping_unknown_node_to_current_store() {
    let (s, r) = fixture();
    let c = group(&s, 1);
    let e = s.engine().as_ref();
    let revision = complete(e, &r);
    let first = q(&c, revision, 0);
    let h = History::new();
    let page = h.storage.membership_history_page(None).unwrap();
    record(e, &r, first, None, 0, &page);
    let stale = q(&c, revision, 1);
    let mut t = s.begin().unwrap();
    activation::plan_activation(&mut t, &c).unwrap();
    migration::plan_migration(&mut t, [9; 16], c.task(), NodeId(4)).unwrap();
    t.commit().unwrap();
    assert!(continuation(e.snapshot().unwrap().as_ref(), &r, stale).is_err());
    assert!(
        !members(e.snapshot().unwrap().as_ref(), &r, first.source, None)
            .unwrap()
            .placement_current
    );
    // Historical receipt remains confirmable, but never changes its old revision.
    assert!(!plan_retry(e.snapshot().unwrap().as_ref(), &r, first)
        .unwrap()
        .changed());
    let revision = complete(e, &r);
    let restart = q(&c, revision, 1);
    assert_eq!(
        continuation(e.snapshot().unwrap().as_ref(), &r, restart).unwrap(),
        (None, 0)
    );
    record(e, &r, restart, None, 0, &page);
    let members = members(e.snapshot().unwrap().as_ref(), &r, first.source, None).unwrap();
    assert_eq!(members.progress.round, 2);
    assert_eq!(members.progress.unmatched_nodes, 0);
    let witness = members.members[3].placement.unwrap();
    assert_eq!(witness.replica.node, NodeId(4));
    assert_eq!(witness.kind, 103);
    assert_eq!(
        witness.replica.incarnation,
        StoreIncarnation::from_bytes([4; 16])
    );
}

#[test]
fn membership_source_discovery_keeps_both_replacement_stores_and_skips_duplicate_task_origins() {
    let (s, r) = fixture();
    let c = group(&s, 1);
    let mut t = s.begin().unwrap();
    activation::plan_activation(&mut t, &c).unwrap();
    let (old, _) = migration::plan_migration(&mut t, [2; 16], c.task(), NodeId(4)).unwrap();
    data_groups::abort::plan_abort(&mut t, old.operation(), [1; 32]).unwrap();
    t.commit().unwrap();
    let mut t = s.begin().unwrap();
    t.update(
        &NODES_DESC,
        &[memcmp_uint(4)],
        vec![(ColumnId(5), ColumnValue::Bytes(vec![44; 16]))],
    )
    .unwrap();
    let (new, _) = migration::plan_migration(&mut t, [3; 16], c.task(), NodeId(4)).unwrap();
    data_groups::abort::plan_abort(&mut t, new.operation(), [2; 32]).unwrap();
    t.commit().unwrap();
    let e = s.engine().as_ref();
    let revision = complete(e, &r);
    let view = e.snapshot().unwrap();
    let mut cursor = None;
    let mut stores = Vec::new();
    while let Some((next, o, observed)) =
        data_store_history::next_source(view.as_ref(), &r, cursor.as_ref()).unwrap()
    {
        assert_eq!(observed, revision);
        stores.push((o.replica.node, o.replica.incarnation));
        cursor = Some(next);
    }
    assert_eq!(stores.len(), 5);
    assert!(stores.contains(&(NodeId(4), StoreIncarnation::from_bytes([4; 16]))));
    assert!(stores.contains(&(NodeId(4), StoreIncarnation::from_bytes([44; 16]))));
    drop(view);
    let h = History::new();
    let page = h.storage.membership_history_page(None).unwrap();
    record(e, &r, q(&c, revision, 0), None, 0, &page);
    let observed = members(
        e.snapshot().unwrap().as_ref(),
        &r,
        q(&c, revision, 0).source,
        None,
    )
    .unwrap();
    assert_eq!(observed.members[3].placement.unwrap().replica.incarnation,StoreIncarnation::from_bytes([4;16]),
        "a historical node witness must not be silently replaced by the current node directory's store");
}
#[test]
fn membership_upgrade_rejects_orphans_and_snapshot_unknown_prefix_remains_a_gap() {
    let (s, r) = fixture();
    let c = group(&s, 1);
    let e = s.engine().as_ref();
    let revision = complete(e, &r);
    let first = q(&c, revision, 0);
    let h = History::new();
    let mut page = h.storage.membership_history_page(None).unwrap();
    page.gaps.snapshot_prefix_unknown = true;
    page.last_snapshot = Some(kv9_common::AppliedPosition { term: 1, index: 10 });
    page.last_configuration_index = 10;
    record(e, &r, first, None, 0, &page);
    assert!(
        observation(e.snapshot().unwrap().as_ref(), &r, first.source)
            .unwrap()
            .unwrap()
            .has_known_gaps()
    );
    let mut clean_page = h.storage.membership_history_page(None).unwrap();
    clean_page.last_configuration_index = 11;
    record(e, &r, q(&c, revision, 1), None, 0, &clean_page);
    assert!(
        observation(e.snapshot().unwrap().as_ref(), &r, first.source)
            .unwrap()
            .unwrap()
            .source_history_unknown,
        "later observations cannot certify the missing snapshot prefix"
    );
    clean_page.last_configuration_index = 9;
    assert!(plan_verified(
        e.snapshot().unwrap().as_ref(),
        &r,
        q(&c, revision, 2),
        endpoint(&r, first.source),
        None,
        0,
        &clean_page
    )
    .is_err());
    let header = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
    let mut old = header[..header.len() - 32].to_vec();
    old[..8].copy_from_slice(DATA_HISTORY_HEADER_MAGIC);
    let mut batch = WriteBatch::new();
    batch.put(ColumnFamily::Default, key(1, &[]), seal(old));
    e.write(batch).unwrap();
    assert!(plan_retention(
        e.snapshot().unwrap().as_ref(),
        &r,
        &LedgerRequest::Initialize
    )
    .is_err());
    let (clean, root) = fixture();
    let engine = clean.engine().as_ref();
    let header = engine
        .get(ColumnFamily::Default, &key(1, &[]))
        .unwrap()
        .unwrap();
    let mut old = header[..header.len() - 32].to_vec();
    old[..8].copy_from_slice(DATA_HISTORY_HEADER_MAGIC);
    let mut batch = WriteBatch::new();
    batch.put(ColumnFamily::Default, key(1, &[]), seal(old));
    engine.write(batch).unwrap();
    apply(engine, &root, LedgerRequest::Initialize);
    assert!(engine
        .get(ColumnFamily::Default, &key(1, &[]))
        .unwrap()
        .unwrap()
        .starts_with(HEADER_MAGIC));
}

#[test]
fn membership_page_cost_does_not_grow_with_the_retained_placement_catalog() {
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

    let mut costs = Vec::new();
    for size in [13, 113] {
        let (s, r) = fixture();
        let c = group(&s, 1);
        for n in 2..=size {
            group(&s, n);
        }
        let e = s.engine().as_ref();
        let revision = complete(e, &r);
        let history = History::new();
        let page = history.storage.membership_history_page(None).unwrap();
        let view = e.snapshot().unwrap();
        let counted = Counted {
            view: view.as_ref(),
            gets: AtomicUsize::new(0),
            rows: AtomicUsize::new(0),
        };
        let request = q(&c, revision, 0);
        let plan = plan_verified(
            &counted,
            &r,
            request,
            endpoint(&r, request.source),
            None,
            0,
            &page,
        )
        .unwrap();
        let cost = (
            counted.gets.load(Ordering::SeqCst),
            counted.rows.load(Ordering::SeqCst),
        );
        assert!(
            cost.0 < 160 && cost.1 <= 16,
            "unbounded membership census work: {cost:?}"
        );
        costs.push(cost);
        e.write(plan.into_batch()).unwrap();
    }
    assert_eq!(costs[0], costs[1]);
}
