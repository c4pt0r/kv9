use super::*;
use crate::codec::{memcmp_uint, ColumnValue, RowValue};
use crate::data_groups::{self, activation, migration, CreationIntent};
use crate::schema::{ColumnId, NODES_DESC};
use crate::store::MetaStore;
use kv9_common::data_range::DataRange;
use kv9_common::RootVoter;
use kv9_common::{KeyspaceId, NodeId, RegionId, StoreIncarnation, TenantId};
use kv9_engine::{Engine, MemEngine, WalEngine};
use kv9_raft::configuration_origin::Action;
use kv9_raft::storage::membership_history::MembershipGaps;
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

fn history(
    s: &MetaStore<MemEngine>,
    c: &CreationIntent,
    root: &RootDescriptor,
) -> Vec<AppliedConfigurationOrigin> {
    let mut t = s.begin().unwrap();
    activation::plan_activation(&mut t, c).unwrap();
    t.commit().unwrap();
    let range = DataRange {
        root: root.digest(),
        creation: c.digest(),
        region: c.region(),
        keyspace: KeyspaceId(100),
        tenant: TenantId(1),
        conf_ver: 1,
        version: 1,
        start: vec![],
        end: vec![],
        sealed: false,
    };
    let mut entries = Vec::new();
    for n in 2..=4 {
        let mut t = s.begin().unwrap();
        let (migration, _) =
            migration::plan_migration(&mut t, [n; 16], c.task(), NodeId(4)).unwrap();
        let (abort, _) =
            data_groups::abort::plan_abort(&mut t, migration.operation(), [n; 32]).unwrap();
        t.commit().unwrap();
        let view = s.engine().snapshot().unwrap();
        for (task, action) in [
            (migration.task(), Action::Attach),
            (abort.task(), Action::Detach),
        ] {
            let origin = data_groups::store_history::configuration_origin(
                view.as_ref(),
                &range,
                task,
                migration.operation(),
                migration.destination(),
                action,
            )
            .unwrap();
            entries.push(AppliedConfigurationOrigin {
                at: AppliedPosition {
                    term: 2,
                    index: entries.len() as u64 + 1,
                },
                origin,
            });
        }
    }
    entries
}
// Metadata planner tests supply descriptive pages at the trusted seam. Actual
// source Raft/WAL and authenticated RPC provenance are tested in server runtime.
fn page(all: &[AppliedConfigurationOrigin], after: Option<OriginCursor>) -> OriginPage {
    let mut bytes = Vec::new();
    for e in all {
        bytes.extend_from_slice(&e.origin.encode().unwrap());
    }
    let fingerprint = RootDigest::sha256(&bytes);
    let start = after.map_or(0, |c| c.seen()) as usize;
    let entries: Vec<_> = all.iter().skip(start).take(4).copied().collect();
    let seen = start + entries.len();
    let next = (seen < all.len()).then(|| {
        let mut b = b"KV9OCP01".to_vec();
        b.extend_from_slice(fingerprint.as_bytes());
        uint(&mut b, entries.last().unwrap().at.index);
        uint(&mut b, seen as u64);
        OriginCursor::decode(&b).unwrap()
    });
    let out = OriginPage {
        initial: None,
        fingerprint,
        membership_gaps: MembershipGaps::default(),
        legacy_contexts_unknown: false,
        last_configuration_index: all.last().map_or(0, |e| e.at.index),
        last_snapshot: None,
        total_known: all.len() as u64,
        entries,
        next,
    };
    out.validate_progress(after).unwrap();
    out
}
fn record(
    e: &impl Engine,
    root: &RootDescriptor,
    q: Request,
    cursor: Option<OriginCursor>,
    page: &OriginPage,
) {
    let plan = plan_verified(
        e.snapshot().unwrap().as_ref(),
        root,
        q,
        endpoint(root, q.source),
        cursor,
        page,
    )
    .unwrap();
    e.write(plan.into_batch()).unwrap();
}
#[test]
fn origin_census_request_is_canonical_and_plain_planner_cannot_accept_asserted_evidence() {
    let (s, r) = fixture();
    let c = group(&s, 1);
    let h = history(&s, &c, &r);
    let e = s.engine().as_ref();
    let q = q(&c, complete(e, &r), 0);
    let request = LedgerRequest::RecordConfigurationOrigins(q);
    let bytes = encode_request(r.digest(), &request).unwrap();
    assert_eq!(decode_request(&bytes, r.digest()).unwrap(), request);
    for n in 0..bytes.len() {
        assert!(decode_request(&bytes[..n], r.digest()).is_err());
    }
    assert!(plan_retention(e.snapshot().unwrap().as_ref(), &r, &request).is_err());
    let first = page(&h, None);
    for field in 0..6 {
        let mut bad = first.clone();
        match field {
            0 => bad.entries[0].origin.task = 9999,
            1 => bad.entries[0].origin.source_digest = RootDigest::sha256(b"wrong task row"),
            2 => bad.entries[0].origin.store = StoreIncarnation::from_bytes([99; 16]),
            3 => bad.entries[0].origin.region = RegionId(9999),
            4 => bad.entries[0].origin.operation = [99; 16],
            _ => bad.entries[0].origin.action = Action::Promote,
        }
        assert!(
            plan_verified(
                e.snapshot().unwrap().as_ref(),
                &r,
                q,
                endpoint(&r, q.source),
                None,
                &bad
            )
            .is_err(),
            "origin must be independently matched to the original metadata task: {field}"
        );
    }
    assert!(observation(e.snapshot().unwrap().as_ref(), &r, q.source)
        .unwrap()
        .is_none());
}
#[test]
fn origin_census_resumes_persisted_page_and_exact_receipt_after_wal_reopen() {
    let (s, r) = fixture();
    let c = group(&s, 1);
    let h = history(&s, &c, &r);
    let e = s.engine().as_ref();
    let revision = complete(e, &r);
    let first = q(&c, revision, 0);
    let initial = page(&h, None);
    record(e, &r, first, None, &initial);
    let observed = observation(e.snapshot().unwrap().as_ref(), &r, first.source)
        .unwrap()
        .unwrap();
    assert_eq!(
        (observed.sequence, observed.seen, observed.retained_entries),
        (1, 4, 4)
    );
    assert!(!observed.round_complete());
    let path = std::env::temp_dir().join(format!(
        "kv9-origin-census-{}",
        StoreIncarnation::mint().unwrap()
    ));
    let (wal, _) = WalEngine::open(&path).unwrap();
    let mut batch = WriteBatch::new();
    let view = e.snapshot().unwrap();
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
    assert_eq!(
        observation(wal.snapshot().unwrap().as_ref(), &r, first.source)
            .unwrap()
            .unwrap(),
        observed
    );
    assert!(!plan_retry(wal.snapshot().unwrap().as_ref(), &r, first)
        .unwrap()
        .changed());
    let cursor = continuation(wal.snapshot().unwrap().as_ref(), &r, q(&c, revision, 1)).unwrap();
    record(&wal, &r, q(&c, revision, 1), cursor, &page(&h, cursor));
    let view = wal.snapshot().unwrap();
    let all = entries(view.as_ref(), &r, first.source, None).unwrap();
    assert!(all.progress.round_complete() && all.placement_current);
    assert_eq!(
        all.entries.iter().map(|e| e.applied).collect::<Vec<_>>(),
        h[..4]
    );
    let tail = entries(view.as_ref(), &r, first.source, all.next).unwrap();
    assert_eq!(
        tail.entries.iter().map(|e| e.applied).collect::<Vec<_>>(),
        h[4..]
    );
    assert!(tail.next.is_none());
    assert!(!plan_verified(
        view.as_ref(),
        &r,
        q(&c, revision, 2),
        endpoint(&r, first.source),
        None,
        &initial
    )
    .unwrap()
    .changed());
    drop(view);
    let mut corrupt = WriteBatch::new();
    corrupt.delete(ColumnFamily::Default, entry_key(first.source, 2).unwrap());
    wal.write(corrupt).unwrap();
    assert!(
        entries(wal.snapshot().unwrap().as_ref(), &r, first.source, None).is_err(),
        "missing prefix row cannot produce a complete query"
    );
    drop(wal);
    std::fs::remove_file(path).unwrap();
}
#[test]
fn origin_census_revision_endpoint_and_immutable_prefix_fence_restart() {
    let (s, r) = fixture();
    let c = group(&s, 1);
    let h = history(&s, &c, &r);
    let e = s.engine().as_ref();
    let revision = complete(e, &r);
    let first = q(&c, revision, 0);
    let initial = page(&h, None);
    record(e, &r, first, None, &initial);
    let cursor = initial.next;
    let last = page(&h, cursor);
    let mut endpoint = endpoint(&r, first.source);
    endpoint.generation = 1;
    assert!(plan_verified(
        e.snapshot().unwrap().as_ref(),
        &r,
        q(&c, revision, 1),
        endpoint,
        cursor,
        &last
    )
    .is_err());
    group(&s, 20);
    assert!(continuation(e.snapshot().unwrap().as_ref(), &r, q(&c, revision, 1)).is_err());
    assert!(!plan_retry(e.snapshot().unwrap().as_ref(), &r, first)
        .unwrap()
        .changed());
    let revision = complete(e, &r);
    let next = q(&c, revision, 1);
    assert_eq!(
        continuation(e.snapshot().unwrap().as_ref(), &r, next).unwrap(),
        None
    );
    let plan = plan_verified(
        e.snapshot().unwrap().as_ref(),
        &r,
        next,
        endpoint,
        None,
        &initial,
    )
    .unwrap();
    e.write(plan.into_batch()).unwrap();
    let p = observation(e.snapshot().unwrap().as_ref(), &r, first.source)
        .unwrap()
        .unwrap();
    assert_eq!((p.round, p.seen, p.endpoint_generation), (2, 4, 1));
    // Both descriptors are valid tasks, but a committed source index is immutable.
    let mut replaced = initial.clone();
    replaced.entries[0].origin = h[2].origin;
    assert!(plan_verified(
        e.snapshot().unwrap().as_ref(),
        &r,
        q(&c, revision, 2),
        endpoint,
        None,
        &replaced
    )
    .is_err());
    let mut regressed = initial.clone();
    regressed.total_known = 5;
    assert!(plan_verified(
        e.snapshot().unwrap().as_ref(),
        &r,
        q(&c, revision, 2),
        endpoint,
        None,
        &regressed
    )
    .is_err());
}
#[test]
fn origin_census_keeps_legacy_and_snapshot_uncertainty_after_clean_round() {
    let (s, r) = fixture();
    let c = group(&s, 1);
    let e = s.engine().as_ref();
    let revision = complete(e, &r);
    let first = q(&c, revision, 0);
    let mut unknown = page(&[], None);
    unknown.legacy_contexts_unknown = true;
    unknown.membership_gaps.snapshot_prefix_unknown = true;
    unknown.last_snapshot = Some(AppliedPosition { term: 2, index: 10 });
    unknown.last_configuration_index = 10;
    record(e, &r, first, None, &unknown);
    let mut clean = page(&[], None);
    clean.last_configuration_index = 11;
    clean.fingerprint = RootDigest::sha256(b"new clean page");
    record(e, &r, q(&c, revision, 1), None, &clean);
    let p = observation(e.snapshot().unwrap().as_ref(), &r, first.source)
        .unwrap()
        .unwrap();
    assert!(p.round_complete() && p.source_history_unknown && p.has_known_gaps());
    assert!(!p.page.legacy_contexts_unknown && !p.page.membership_gaps.any());
}
#[test]
fn origin_census_upgrade_preserves_v23_membership_and_refuses_new_records_under_old_header() {
    let (s, r) = fixture();
    let c = group(&s, 1);
    let e = s.engine().as_ref();
    let revision = complete(e, &r);
    let first = q(&c, revision, 0);
    let path = std::env::temp_dir().join(format!(
        "kv9-origin-upgrade-{}",
        StoreIncarnation::mint().unwrap()
    ));
    let storage = kv9_raft::storage::DiskRaftStorage::open(&path, &[1, 2, 3])
        .unwrap()
        .0;
    let members = storage.membership_history_page(None).unwrap();
    e.write(
        data_membership::plan_verified(
            e.snapshot().unwrap().as_ref(),
            &r,
            first,
            endpoint(&r, first.source),
            None,
            0,
            &members,
        )
        .unwrap()
        .into_batch(),
    )
    .unwrap();
    let original =
        data_membership::observation(e.snapshot().unwrap().as_ref(), &r, first.source).unwrap();
    let downgrade = || {
        let mut header = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
        header.truncate(header.len() - 32);
        header[..8].copy_from_slice(DATA_MEMBERSHIP_HEADER_MAGIC);
        let mut b = WriteBatch::new();
        b.put(ColumnFamily::Default, key(1, &[]), seal(header));
        e.write(b).unwrap();
    };
    downgrade();
    assert!(observation(e.snapshot().unwrap().as_ref(), &r, first.source).is_err());
    apply(e, &r, LedgerRequest::Initialize);
    assert_eq!(
        data_membership::observation(e.snapshot().unwrap().as_ref(), &r, first.source).unwrap(),
        original
    );
    record(e, &r, first, None, &page(&[], None));
    downgrade();
    assert!(plan_retention(
        e.snapshot().unwrap().as_ref(),
        &r,
        &LedgerRequest::Initialize
    )
    .is_err());
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

mod budget;
mod initial;

#[test]
fn origin_census_separates_replacement_stores_of_the_same_source_node() {
    let (s, r) = fixture();
    let c = group(&s, 1);
    let h = history(&s, &c, &r);
    let mut t = s.begin().unwrap();
    t.update(
        &NODES_DESC,
        &[memcmp_uint(4)],
        vec![(ColumnId(5), ColumnValue::Bytes(vec![44; 16]))],
    )
    .unwrap();
    let (replacement, _) =
        migration::plan_migration(&mut t, [44; 16], c.task(), NodeId(4)).unwrap();
    t.commit().unwrap();
    let e = s.engine().as_ref();
    let revision = complete(e, &r);
    let initial = page(&h, None);
    for (task, store) in [
        (h[0].origin.task, h[0].origin.store),
        (replacement.task(), replacement.destination().incarnation),
    ] {
        let source = Source {
            region: c.region(),
            creation: c.digest(),
            node: NodeId(4),
            store,
        };
        let q = Request {
            source,
            task,
            revision,
            expected: 0,
        };
        let endpoint = NodeEndpoint {
            node: NodeId(4),
            incarnation: store,
            address: "127.0.0.1:18004".parse().unwrap(),
            generation: 0,
            previous_address: None,
            active: true,
        };
        e.write(
            plan_verified(
                e.snapshot().unwrap().as_ref(),
                &r,
                q,
                endpoint,
                None,
                &initial,
            )
            .unwrap()
            .into_batch(),
        )
        .unwrap();
    }
    for store in [
        StoreIncarnation::from_bytes([4; 16]),
        StoreIncarnation::from_bytes([44; 16]),
    ] {
        let source = Source {
            region: c.region(),
            creation: c.digest(),
            node: NodeId(4),
            store,
        };
        let observed = observation(e.snapshot().unwrap().as_ref(), &r, source)
            .unwrap()
            .unwrap();
        assert_eq!(observed.source.store, store);
        assert_eq!((observed.sequence, observed.retained_entries), (1, 4));
        let retained = entries(e.snapshot().unwrap().as_ref(), &r, source, None).unwrap();
        assert_eq!(
            retained.entries[0].applied.origin.store,
            StoreIncarnation::from_bytes([4; 16]),
            "observing on a replacement source never rewrites the origin subject's old store"
        );
    }
}
