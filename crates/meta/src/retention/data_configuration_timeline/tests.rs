use super::*;
use crate::codec::{memcmp_uint, ColumnValue, RowValue};
use crate::data_groups::{self, activation, migration, CreationIntent};
use crate::schema::{ColumnId, NODES_DESC};
use crate::store::MetaStore;
use kv9_common::data_range::DataRange;
use kv9_common::{AppliedPosition, RootVoter};
use kv9_common::{KeyspaceId, NodeId, RegionId, StoreIncarnation, TenantId};
use kv9_engine::{Engine, MemEngine, WalEngine};
use kv9_raft::configuration_origin::Action;
use kv9_raft::storage::configuration_origins::AppliedConfigurationOrigin;
use kv9_raft::storage::configuration_timeline::ConfigurationState;
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

fn origins(
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

fn state() -> ConfigurationState {
    ConfigurationState {
        voters: vec![1, 2, 3],
        outgoing: vec![],
        learners: vec![],
        learners_next: vec![],
        auto_leave: false,
    }
}
fn history(
    s: &MetaStore<MemEngine>,
    c: &CreationIntent,
    root: &RootDescriptor,
) -> Vec<ConfigurationRecord> {
    let mut records = vec![ConfigurationRecord {
        ordinal: 1,
        kind: RecordKind::Initial,
        state: Some(state()),
        origin: None,
    }];
    for origin in origins(s, c, root) {
        let mut state = state();
        if origin.origin.action == Action::Attach {
            state.learners.push(4);
        }
        records.push(ConfigurationRecord {
            ordinal: records.len() as u64 + 1,
            kind: RecordKind::Applied {
                index: origin.at.index,
            },
            state: Some(state),
            origin: Some(origin),
        });
    }
    records
}
// Pages are descriptive input at the trusted seam; actual authenticated RPC
// and replica-store provenance are exercised by the runtime integration tests.
fn page(all: &[ConfigurationRecord], after: Option<TimelineCursor>) -> TimelinePage {
    let bytes: Vec<_> = all.iter().flat_map(|e| e.encode().unwrap()).collect();
    let fingerprint = RootDigest::sha256(&bytes);
    let start = after.map_or(0, |c| c.seen()) as usize;
    let records: Vec<_> = all
        .iter()
        .skip(start)
        .take(RECORDS_PER_PAGE)
        .cloned()
        .collect();
    let seen = start + records.len();
    let next = (seen < all.len()).then(|| {
        let mut b = b"KV9CTC01".to_vec();
        b.extend_from_slice(fingerprint.as_bytes());
        uint(&mut b, seen as u64);
        TimelineCursor::decode(&b).unwrap()
    });
    let out = TimelinePage {
        fingerprint,
        gaps: MembershipGaps {
            missing_initial: all.first().is_none_or(|r| r.kind != RecordKind::Initial),
            snapshot_prefix_unknown: all
                .iter()
                .any(|r| matches!(r.kind, RecordKind::Snapshot { .. })),
            ambiguous_records: all
                .iter()
                .any(|r| matches!(r.kind, RecordKind::Applied { index: 0 })),
            unsupported_configuration: all.iter().any(|r| r.state.is_none()),
            member_limit_exceeded: false,
        },
        total: all.len() as u64,
        records,
        next,
    };
    out.validate_progress(after).unwrap();
    out
}
fn record(
    e: &impl Engine,
    root: &RootDescriptor,
    q: Request,
    cursor: Option<TimelineCursor>,
    page: &TimelinePage,
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
fn timeline_census_requires_canonical_request_and_independent_original_tasks() {
    let (s, r) = fixture();
    let c = group(&s, 1);
    let h = history(&s, &c, &r);
    let e = s.engine().as_ref();
    let q = q(&c, complete(e, &r), 0);
    let request = LedgerRequest::RecordConfigurationTimeline(q);
    let bytes = encode_request(r.digest(), &request).unwrap();
    assert_eq!(decode_request(&bytes, r.digest()).unwrap(), request);
    assert_eq!(bytes[40], 40);
    for n in 0..bytes.len() {
        assert!(decode_request(&bytes[..n], r.digest()).is_err());
    }
    assert!(plan_retention(e.snapshot().unwrap().as_ref(), &r, &request).is_err());
    let first = page(&h, None);
    for field in 0..6 {
        let mut bad = first.clone();
        let origin = &mut bad.records[1].origin.as_mut().unwrap().origin;
        match field {
            0 => origin.task = 9999,
            1 => origin.source_digest = RootDigest::sha256(b"different original task row"),
            2 => origin.store = StoreIncarnation::from_bytes([99; 16]),
            3 => origin.region = RegionId(9999),
            4 => origin.operation = [99; 16],
            _ => origin.action = Action::Promote,
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
            "timeline origin needs independent original task verification: {field}"
        );
    }
    assert!(observation(e.snapshot().unwrap().as_ref(), &r, q.source)
        .unwrap()
        .is_none());
}

#[test]
fn timeline_census_wal_reopen_resumes_exact_records_and_original_receipt() {
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
        "kv9-timeline-census-{}",
        StoreIncarnation::mint().unwrap()
    ));
    let (wal, _) = WalEngine::open(&path).unwrap();
    let mut seed = WriteBatch::new();
    for cf in [
        ColumnFamily::Default,
        ColumnFamily::Write,
        ColumnFamily::Lock,
    ] {
        for row in e.snapshot().unwrap().iter(cf, &[], &[255]).unwrap() {
            let (k, v) = row.unwrap();
            seed.put(cf, k, v);
        }
    }
    wal.write(seed).unwrap();
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
    drop(wal);
    let (wal, _) = WalEngine::open(&path).unwrap();
    let view = wal.snapshot().unwrap();
    let head = entries(view.as_ref(), &r, first.source, None).unwrap();
    let tail = entries(view.as_ref(), &r, first.source, head.next).unwrap();
    assert!(tail.next.is_none() && tail.progress.round_complete() && tail.placement_current);
    assert_eq!(
        head.entries
            .iter()
            .chain(&tail.entries)
            .map(|e| e.record.clone())
            .collect::<Vec<_>>(),
        h
    );
    assert_eq!(head.entries[0].first_observed, observed.recorded_at);
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
    // A stable fingerprint cannot cause a missing earlier row to be healed or
    // silently treated as a fully retained prefix.
    let mut corrupt = WriteBatch::new();
    corrupt.delete(ColumnFamily::Default, entry_key(first.source, 2).unwrap());
    wal.write(corrupt).unwrap();
    let view = wal.snapshot().unwrap();
    assert!(entries(view.as_ref(), &r, first.source, None).is_err());
    assert!(plan_verified(
        view.as_ref(),
        &r,
        q(&c, revision, 2),
        endpoint(&r, first.source),
        None,
        &initial
    )
    .is_err());
    drop(view);
    drop(wal);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn timeline_census_revision_endpoint_and_immutable_prefix_fence_restart() {
    let (s, r) = fixture();
    let c = group(&s, 1);
    let h = history(&s, &c, &r);
    let e = s.engine().as_ref();
    let revision = complete(e, &r);
    let first = q(&c, revision, 0);
    let initial = page(&h, None);
    record(e, &r, first, None, &initial);
    let mut endpoint = endpoint(&r, first.source);
    endpoint.generation = 1;
    assert!(plan_verified(
        e.snapshot().unwrap().as_ref(),
        &r,
        q(&c, revision, 1),
        endpoint,
        initial.next,
        &page(&h, initial.next)
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
    for kind in 0..3 {
        let mut changed = initial.clone();
        match kind {
            0 => changed.records[1].origin.as_mut().unwrap().origin = h[3].origin.unwrap().origin,
            1 => changed.records[1].state.as_mut().unwrap().learners = vec![5],
            _ => changed.total = 6,
        }
        assert!(plan_verified(
            e.snapshot().unwrap().as_ref(),
            &r,
            q(&c, revision, 2),
            endpoint,
            None,
            &changed
        )
        .is_err());
    }
}

#[test]
fn timeline_census_keeps_joint_snapshot_and_legacy_records_with_sticky_uncertainty() {
    let (s, r) = fixture();
    let c = group(&s, 1);
    let mut h = history(&s, &c, &r);
    let mut joint = state();
    joint.outgoing = vec![1, 2, 3, 4];
    joint.learners_next = vec![4];
    joint.auto_leave = true;
    h.push(ConfigurationRecord {
        ordinal: 8,
        kind: RecordKind::Applied { index: 7 },
        state: Some(joint.clone()),
        origin: None,
    });
    h.push(ConfigurationRecord {
        ordinal: 9,
        kind: RecordKind::Snapshot {
            at: AppliedPosition { term: 3, index: 7 },
        },
        state: Some(joint),
        origin: None,
    });
    h.push(ConfigurationRecord {
        ordinal: 10,
        kind: RecordKind::Applied { index: 0 },
        state: None,
        origin: None,
    });
    let e = s.engine().as_ref();
    let revision = complete(e, &r);
    let source = q(&c, revision, 0).source;
    let mut cursor = None;
    for sequence in 0..3 {
        let p = page(&h, cursor);
        record(e, &r, q(&c, revision, sequence), cursor, &p);
        cursor = p.next;
    }
    let view = e.snapshot().unwrap();
    let a = entries(view.as_ref(), &r, source, None).unwrap();
    let saved_cursor = a.next;
    let b = entries(view.as_ref(), &r, source, a.next).unwrap();
    let d = entries(view.as_ref(), &r, source, b.next).unwrap();
    assert_eq!(
        a.entries
            .iter()
            .chain(&b.entries)
            .chain(&d.entries)
            .map(|e| e.record.clone())
            .collect::<Vec<_>>(),
        h
    );
    assert!(d.progress.round_complete() && d.progress.has_known_gaps());
    drop(view);
    let mut clean = page(&h, None);
    clean.gaps = MembershipGaps::default();
    clean.fingerprint = RootDigest::sha256(b"later clean-looking prefix");
    let mut bytes = b"KV9CTC01".to_vec();
    bytes.extend_from_slice(clean.fingerprint.as_bytes());
    uint(&mut bytes, 4);
    clean.next = Some(TimelineCursor::decode(&bytes).unwrap());
    record(e, &r, q(&c, revision, 3), None, &clean);
    let view = e.snapshot().unwrap();
    let progress = observation(view.as_ref(), &r, source).unwrap().unwrap();
    assert!(progress.source_history_unknown && !progress.page.gaps.any());
    assert!(
        entries(view.as_ref(), &r, source, saved_cursor).is_err(),
        "query cursor cannot cross persisted revisions"
    );
}

fn downgrade(e: &impl Engine) {
    let mut bytes = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
    bytes.truncate(bytes.len() - 32);
    bytes[..8].copy_from_slice(DATA_INITIAL_HEADER_MAGIC);
    let mut b = WriteBatch::new();
    b.put(ColumnFamily::Default, key(1, &[]), seal(bytes));
    e.write(b).unwrap();
}
#[test]
fn timeline_census_v25_upgrade_preserves_native_mapping_and_refuses_new_namespaces() {
    let (s, r) = fixture();
    let c = group(&s, 1);
    let e = s.engine().as_ref();
    let revision = complete(e, &r);
    let first = q(&c, revision, 0);
    let path = std::env::temp_dir().join(format!(
        "kv9-timeline-upgrade-{}",
        StoreIncarnation::mint().unwrap()
    ));
    let storage = kv9_raft::storage::DiskRaftStorage::open(&path, &[1, 2, 3])
        .unwrap()
        .0;
    let origins = storage.configuration_origins_page(None).unwrap();
    let plan = data_configuration_origins::plan_verified(
        e.snapshot().unwrap().as_ref(),
        &r,
        first,
        endpoint(&r, first.source),
        None,
        &origins,
    )
    .unwrap();
    e.write(plan.into_batch()).unwrap();
    let before: Vec<_> = [54, 56]
        .iter()
        .map(|&ns| {
            (
                ns,
                e.get(ColumnFamily::Default, &first.source.key(ns).unwrap())
                    .unwrap()
                    .unwrap(),
            )
        })
        .collect();
    downgrade(e);
    assert!(observation(e.snapshot().unwrap().as_ref(), &r, first.source).is_err());
    apply(e, &r, LedgerRequest::Initialize);
    for (ns, bytes) in before {
        assert_eq!(
            e.get(ColumnFamily::Default, &first.source.key(ns).unwrap())
                .unwrap()
                .unwrap(),
            bytes
        );
    }
    let page = storage.configuration_timeline_page(None).unwrap();
    record(e, &r, first, None, &page);
    downgrade(e);
    assert!(plan_retention(
        e.snapshot().unwrap().as_ref(),
        &r,
        &LedgerRequest::Initialize
    )
    .is_err());
    // A row in either namespace under an older header prevents upgrade.
    for namespace in [57, 58] {
        let (s, r) = fixture();
        let e = s.engine().as_ref();
        downgrade(e);
        let mut b = WriteBatch::new();
        b.put(ColumnFamily::Default, key(namespace, b"orphan"), vec![1]);
        e.write(b).unwrap();
        assert!(plan_retention(
            e.snapshot().unwrap().as_ref(),
            &r,
            &LedgerRequest::Initialize
        )
        .is_err());
    }
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn timeline_census_missing_or_orphan_page_cannot_be_repaired_by_retry() {
    for corruption in 0..4 {
        let (s, r) = fixture();
        let c = group(&s, 1);
        let h = history(&s, &c, &r);
        let e = s.engine().as_ref();
        let first = q(&c, complete(e, &r), 0);
        let head = page(&h, None);
        record(e, &r, first, None, &head);
        let mut b = WriteBatch::new();
        match corruption {
            0 => {
                b.delete(ColumnFamily::Default, first.source.key(57).unwrap());
            }
            1 => {
                b.delete(ColumnFamily::Default, entry_key(first.source, 3).unwrap());
            }
            2 => {
                b.put(
                    ColumnFamily::Default,
                    entry_key(first.source, 5).unwrap(),
                    vec![1],
                );
            }
            _ => {
                let p = planner(e.snapshot().unwrap().as_ref(), &r)
                    .unwrap()
                    .revision;
                let mut entry = Entry {
                    record: h[1].clone(),
                    first_observed: p,
                    last_observed: p,
                    round: 1,
                };
                entry.record.state.as_mut().unwrap().learners = vec![5];
                b.put(
                    ColumnFamily::Default,
                    entry_key(first.source, 2).unwrap(),
                    entry.encode(r.digest(), first.source).unwrap(),
                );
            }
        }
        e.write(b).unwrap();
        let view = e.snapshot().unwrap();
        assert!(observation(view.as_ref(), &r, first.source).is_err());
        assert!(plan_retry(view.as_ref(), &r, first).is_err());
        assert!(plan_verified(
            view.as_ref(),
            &r,
            first,
            endpoint(&r, first.source),
            None,
            &head
        )
        .is_err());
    }
}

#[test]
fn timeline_census_large_joint_pages_use_u32_framing_and_bounded_records() {
    let (s, r) = fixture();
    let c = group(&s, 1);
    let e = s.engine().as_ref();
    let first = q(&c, complete(e, &r), 0);
    let state = ConfigurationState {
        voters: (1..=1024).collect(),
        outgoing: (1025..=2048).collect(),
        learners: (2049..=3072).collect(),
        learners_next: (1025..=2048).collect(),
        auto_leave: true,
    };
    let h: Vec<_> = (1..=4)
        .map(|ordinal| ConfigurationRecord {
            ordinal,
            kind: RecordKind::Applied { index: ordinal },
            state: Some(state.clone()),
            origin: None,
        })
        .collect();
    let page = page(&h, None);
    assert!(page.encode().unwrap().len() > 128 * 1024);
    record(e, &r, first, None, &page);
    let view = e.snapshot().unwrap();
    let p = observation(view.as_ref(), &r, first.source)
        .unwrap()
        .unwrap();
    assert!(p.round_complete() && p.has_known_gaps());
    let retained = entries(view.as_ref(), &r, first.source, None).unwrap();
    assert_eq!(retained.entries.len(), 4);
    for entry in retained.entries {
        assert!(entry.encode(r.digest(), first.source).unwrap().len() <= MAX_ENTRY_BYTES);
    }
    assert!(
        e.get(ColumnFamily::Default, &first.source.key(57).unwrap())
            .unwrap()
            .unwrap()
            .len()
            <= MAX_PROGRESS_BYTES
    );
}

mod budget;

#[test]
fn timeline_census_separates_replacement_stores_of_the_same_source_node() {
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
        (
            h[1].origin.unwrap().origin.task,
            h[1].origin.unwrap().origin.store,
        ),
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
            retained.entries[1].record.origin.unwrap().origin.store,
            StoreIncarnation::from_bytes([4; 16]),
            "observing on a replacement source never rewrites the origin subject's old store"
        );
    }
}
