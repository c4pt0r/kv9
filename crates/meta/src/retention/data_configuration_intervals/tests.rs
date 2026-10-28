use super::*;
use crate::codec::{memcmp_uint, ColumnValue, RowValue};
use crate::data_groups::{self, activation, migration, CreationIntent};
use crate::endpoint::NodeEndpoint;
use crate::schema::{ColumnId, NODES_DESC};
use crate::store::MetaStore;
use kv9_common::data_range::DataRange;
use kv9_common::{AppliedPosition, RootVoter};
use kv9_common::{KeyspaceId, NodeId, StoreIncarnation, TenantId};
use kv9_engine::{Engine, MemEngine, WalEngine};
use kv9_raft::configuration_origin::Action;
use kv9_raft::storage::configuration_origins::AppliedConfigurationOrigin;
use kv9_raft::storage::configuration_timeline::{
    ConfigurationState, TimelineCursor, TimelinePage, RECORDS_PER_PAGE,
};
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

fn native(e: &impl Engine, r: &RootDescriptor, c: &CreationIntent, revision: u64) {
    let page = kv9_raft::storage::configuration_origins::OriginPage {
        fingerprint: RootDigest::sha256(b"native fixture"),
        membership_gaps: MembershipGaps::default(),
        initial: Some(
            kv9_raft::storage::membership_history::InitialConfiguration::Voters(
                c.replicas().iter().map(|r| r.node).collect(),
            ),
        ),
        legacy_contexts_unknown: false,
        last_configuration_index: 0,
        last_snapshot: None,
        total_known: 0,
        entries: vec![],
        next: None,
    };
    let request = q(c, revision, 0);
    let plan = data_configuration_origins::plan_verified(
        e.snapshot().unwrap().as_ref(),
        r,
        request,
        endpoint(r, request.source),
        None,
        &page,
    )
    .unwrap();
    e.write(plan.into_batch()).unwrap();
}
fn seed(
    e: &impl Engine,
    r: &RootDescriptor,
    c: &CreationIntent,
    h: &[ConfigurationRecord],
    with_native: bool,
) -> Source {
    let revision = complete(e, r);
    let source = q(c, revision, 0).source;
    if with_native {
        native(e, r, c, revision);
    }
    let mut cursor = None;
    let mut sequence = 0;
    loop {
        let page = page(h, cursor);
        record(e, r, q(c, revision, sequence), cursor, &page);
        cursor = page.next;
        sequence += 1;
        if cursor.is_none() {
            break;
        }
    }
    source
}
fn step(e: &impl Engine, r: &RootDescriptor, source: Source) -> Request {
    let request = next(e.snapshot().unwrap().as_ref(), r, source)
        .unwrap()
        .unwrap();
    apply(e, r, LedgerRequest::AdvanceConfigurationIntervals(request));
    request
}
fn finish(e: &impl Engine, r: &RootDescriptor, source: Source) {
    for _ in 0..100 {
        let Some(request) = next(e.snapshot().unwrap().as_ref(), r, source).unwrap() else {
            return;
        };
        apply(e, r, LedgerRequest::AdvanceConfigurationIntervals(request));
    }
    panic!("interval projection stalled");
}
fn all(e: &impl Engine, r: &RootDescriptor, source: Source) -> Vec<Interval> {
    let view = e.snapshot().unwrap();
    let mut cursor = None;
    let mut out = Vec::new();
    loop {
        let page = intervals(view.as_ref(), r, source, cursor).unwrap();
        out.extend(page.intervals);
        cursor = page.next;
        if cursor.is_none() {
            return out;
        }
    }
}

#[test]
fn intervals_match_all_four_actions_and_original_stores_not_current_registration() {
    use crate::data_groups::{abort, evidence, removal};
    let (s, r) = fixture();
    let c = group(&s, 31);
    let range = DataRange {
        root: r.digest(),
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
    let mut t = s.begin().unwrap();
    activation::plan_activation(&mut t, &c).unwrap();
    let (old, _) = migration::plan_migration(&mut t, [32; 16], c.task(), NodeId(4)).unwrap();
    let (aborted, _) = abort::plan_abort(&mut t, old.operation(), [9; 32]).unwrap();
    t.commit().unwrap();
    let mut t = s.begin().unwrap();
    let (new, _) = migration::plan_migration(&mut t, [33; 16], c.task(), NodeId(5)).unwrap();
    let receipt = evidence::InstallEvidence::receipt(
        &new,
        StoreIncarnation::from_bytes([21; 16]),
        RootDigest::sha256(b"image"),
        [22; 32],
        AppliedPosition { term: 3, index: 12 },
    )
    .unwrap();
    let (installed, _) = evidence::plan_install_evidence(&mut t, &receipt).unwrap();
    let (removed, _) = removal::plan_removal(&mut t, new.operation(), c.replicas()[0]).unwrap();
    t.commit().unwrap();
    let mut h = vec![ConfigurationRecord {
        ordinal: 1,
        kind: RecordKind::Initial,
        state: Some(state()),
        origin: None,
    }];
    let mut states = vec![state(); 5];
    states[0].learners = vec![4];
    states[2].learners = vec![5];
    states[3].voters = vec![1, 2, 3, 5];
    states[4].voters = vec![2, 3, 5];
    for ((task, op, replica, action), state) in [
        (
            old.task(),
            old.operation(),
            old.destination(),
            Action::Attach,
        ),
        (
            aborted.task(),
            old.operation(),
            old.destination(),
            Action::Detach,
        ),
        (
            new.task(),
            new.operation(),
            new.destination(),
            Action::Attach,
        ),
        (
            installed.task(),
            new.operation(),
            new.destination(),
            Action::Promote,
        ),
        (
            removed.task(),
            new.operation(),
            removed.source(),
            Action::Remove,
        ),
    ]
    .into_iter()
    .zip(states)
    {
        let origin = data_groups::store_history::configuration_origin(
            s.engine().snapshot().unwrap().as_ref(),
            &range,
            task,
            op,
            replica,
            action,
        )
        .unwrap();
        let index = h.len() as u64;
        h.push(ConfigurationRecord {
            ordinal: index + 1,
            kind: RecordKind::Applied { index },
            state: Some(state),
            origin: Some(AppliedConfigurationOrigin {
                at: AppliedPosition { term: 2, index },
                origin,
            }),
        });
    }
    let e = s.engine().as_ref();
    let source = seed(e, &r, &c, &h, true);
    let mut t = s.begin().unwrap();
    t.update(
        &NODES_DESC,
        &[memcmp_uint(4)],
        vec![(ColumnId(5), ColumnValue::Bytes(vec![44; 16]))],
    )
    .unwrap();
    t.commit().unwrap();
    finish(e, &r, source);
    let rows = all(e, &r, source);
    assert_eq!(rows.len(), 6);
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(row.gaps, 0);
        assert_eq!(row.start.index, Some(i as u64));
        assert_eq!(
            row.end.map(|b| b.index),
            if i == 5 {
                None
            } else {
                Some(Some(i as u64 + 1))
            }
        );
        for member in row.entry.members.as_ref().unwrap() {
            assert_eq!(
                member.store,
                Some(StoreIncarnation::from_bytes([member.node.0 as u8; 16]))
            );
        }
    }
    assert_eq!(
        rows[1]
            .entry
            .members
            .as_ref()
            .unwrap()
            .last()
            .unwrap()
            .roles,
        4
    );
    assert_eq!(
        rows[4]
            .entry
            .members
            .as_ref()
            .unwrap()
            .last()
            .unwrap()
            .roles,
        1
    );
    assert!(rows[5]
        .entry
        .members
        .as_ref()
        .unwrap()
        .iter()
        .all(|m| m.node != NodeId(1)));
}

#[test]
fn intervals_compare_actual_result_and_recheck_original_task() {
    let (s, r) = fixture();
    let c = group(&s, 1);
    let mut h = history(&s, &c, &r);
    h[1].state.as_mut().unwrap().learners.push(5);
    let e = s.engine().as_ref();
    let source = seed(e, &r, &c, &h, true);
    finish(e, &r, source);
    let rows = all(e, &r, source);
    assert_ne!(
        rows[1].gaps & TRANSITION_MISMATCH,
        0,
        "valid origin must not replace actual resulting configuration comparison"
    );
    assert!(rows[1]
        .entry
        .members
        .as_ref()
        .unwrap()
        .iter()
        .all(|m| m.store.is_none()));
    let (s, r) = fixture();
    let c = group(&s, 1);
    let h = history(&s, &c, &r);
    let e = s.engine().as_ref();
    let source = seed(e, &r, &c, &h, true);
    let request = next(e.snapshot().unwrap().as_ref(), &r, source)
        .unwrap()
        .unwrap();
    let mut b = WriteBatch::new();
    b.delete(
        ColumnFamily::Default,
        data_groups::store_history::task_key(h[1].origin.unwrap().origin.task).unwrap(),
    );
    e.write(b).unwrap();
    assert!(plan_retention(
        e.snapshot().unwrap().as_ref(),
        &r,
        &LedgerRequest::AdvanceConfigurationIntervals(request)
    )
    .is_err());
    assert!(observation(e.snapshot().unwrap().as_ref(), &r, source)
        .unwrap()
        .is_none());
}

#[test]
fn intervals_joint_snapshot_legacy_and_ambiguous_positions_preserve_unknown_stores() {
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
            at: AppliedPosition { term: 3, index: 8 },
        },
        state: Some(joint),
        origin: None,
    });
    h.push(ConfigurationRecord {
        ordinal: 10,
        kind: RecordKind::Applied { index: 0 },
        state: Some(state()),
        origin: None,
    });
    h.push(ConfigurationRecord {
        ordinal: 11,
        kind: RecordKind::Applied { index: 10 },
        state: None,
        origin: None,
    });
    let e = s.engine().as_ref();
    let source = seed(e, &r, &c, &h, true);
    finish(e, &r, source);
    let rows = all(e, &r, source);
    assert_ne!(rows[7].gaps & LEGACY_CHANGE, 0);
    assert_ne!(
        rows[7].gaps & SNAPSHOT_ANCESTRY,
        0,
        "snapshot end cannot certify intervening configurations"
    );
    assert_ne!(rows[8].gaps & SNAPSHOT_ANCESTRY, 0);
    assert_ne!(rows[9].gaps & AMBIGUOUS_POSITION, 0);
    assert!(rows[10].entry.members.is_none());
    for row in &rows[7..10] {
        assert!(row
            .entry
            .members
            .as_ref()
            .unwrap()
            .iter()
            .all(|m| m.store.is_none()));
    }
    let joint = rows[7]
        .entry
        .members
        .as_ref()
        .unwrap()
        .iter()
        .find(|m| m.node == NodeId(4))
        .unwrap();
    assert_eq!(joint.roles, 2 | 8);
    assert!(observation(e.snapshot().unwrap().as_ref(), &r, source)
        .unwrap()
        .unwrap()
        .has_known_gaps());
}

fn copy_wal(e: &impl Engine) -> (WalEngine, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "kv9-intervals-{}",
        StoreIncarnation::mint().unwrap()
    ));
    let (wal, _) = WalEngine::open(&path).unwrap();
    let mut b = WriteBatch::new();
    for cf in [
        ColumnFamily::Default,
        ColumnFamily::Write,
        ColumnFamily::Lock,
    ] {
        for row in e.snapshot().unwrap().iter(cf, &[], &[255]).unwrap() {
            let (k, v) = row.unwrap();
            b.put(cf, k, v);
        }
    }
    wal.write(b).unwrap();
    drop(wal);
    (WalEngine::open(&path).unwrap().0, path)
}
#[test]
fn intervals_wal_receipt_and_append_resume_without_rebuilding_prefix() {
    let (s, r) = fixture();
    let c = group(&s, 1);
    let mut h = history(&s, &c, &r);
    let e = s.engine().as_ref();
    let source = seed(e, &r, &c, &h, true);
    let request = step(e, &r, source);
    let (wal, path) = copy_wal(e);
    let before = observation(wal.snapshot().unwrap().as_ref(), &r, source)
        .unwrap()
        .unwrap();
    assert!(!plan_retention(
        wal.snapshot().unwrap().as_ref(),
        &r,
        &LedgerRequest::AdvanceConfigurationIntervals(request)
    )
    .unwrap()
    .changed());
    finish(&wal, &r, source);
    let first = intervals(wal.snapshot().unwrap().as_ref(), &r, source, None).unwrap();
    let old_cursor = first.next;
    let bytes = wal
        .get(ColumnFamily::Default, &row_key(source, 1).unwrap())
        .unwrap()
        .unwrap();
    h.push(ConfigurationRecord {
        ordinal: 8,
        kind: RecordKind::Applied { index: 7 },
        state: Some(state()),
        origin: None,
    });
    let revision = complete(&wal, &r);
    let head = page(&h, None);
    record(&wal, &r, q(&c, revision, 2), None, &head);
    record(
        &wal,
        &r,
        q(&c, revision, 3),
        head.next,
        &page(&h, head.next),
    );
    step(&wal, &r, source);
    let p = observation(wal.snapshot().unwrap().as_ref(), &r, source)
        .unwrap()
        .unwrap();
    assert_eq!(p.generation, before.generation);
    assert_eq!(p.processed, 8);
    assert!(p.has_known_gaps());
    assert_eq!(
        wal.get(ColumnFamily::Default, &row_key(source, 1).unwrap())
            .unwrap()
            .unwrap(),
        bytes
    );
    assert!(intervals(wal.snapshot().unwrap().as_ref(), &r, source, old_cursor).is_err());
    let expected = all(&wal, &r, source)
        .into_iter()
        .map(|i| i.entry)
        .collect::<Vec<_>>();
    drop(wal);
    let (wal, _) = WalEngine::open(&path).unwrap();
    assert_eq!(
        all(&wal, &r, source)
            .into_iter()
            .map(|i| i.entry)
            .collect::<Vec<_>>(),
        expected
    );
    drop(wal);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn intervals_late_native_mapping_rebuilds_one_generation_without_changing_raw_history() {
    let (s, r) = fixture();
    let c = group(&s, 1);
    let h = history(&s, &c, &r);
    let e = s.engine().as_ref();
    let source = seed(e, &r, &c, &h, false);
    finish(e, &r, source);
    let old = intervals(e.snapshot().unwrap().as_ref(), &r, source, None).unwrap();
    assert_ne!(old.intervals[0].gaps & MISSING_NATIVE, 0);
    let raw = timeline::entries(e.snapshot().unwrap().as_ref(), &r, source, None)
        .unwrap()
        .entries;
    native(e, &r, &c, complete(e, &r));
    step(e, &r, source);
    assert!(intervals(e.snapshot().unwrap().as_ref(), &r, source, old.next).is_err());
    let current = intervals(e.snapshot().unwrap().as_ref(), &r, source, None).unwrap();
    assert!(current.progress.generation > old.progress.generation);
    assert_eq!(current.progress.processed, 4);
    assert!(current.next.is_none());
    finish(e, &r, source);
    assert!(all(e, &r, source).iter().all(|i| i.gaps == 0));
    assert!(!observation(e.snapshot().unwrap().as_ref(), &r, source)
        .unwrap()
        .unwrap()
        .has_known_gaps());
    assert_eq!(
        timeline::entries(e.snapshot().unwrap().as_ref(), &r, source, None)
            .unwrap()
            .entries,
        raw
    );
}

#[test]
fn intervals_v26_upgrade_keeps_timeline_and_rejects_new_rows_under_old_header() {
    fn downgrade(e: &impl Engine) {
        let mut b = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
        b.truncate(b.len() - 32);
        b[..8].copy_from_slice(DATA_TIMELINE_HEADER_MAGIC);
        let mut batch = WriteBatch::new();
        batch.put(ColumnFamily::Default, key(1, &[]), seal(b));
        e.write(batch).unwrap();
    }
    let (s, r) = fixture();
    let c = group(&s, 1);
    let h = history(&s, &c, &r);
    let e = s.engine().as_ref();
    let source = seed(e, &r, &c, &h, true);
    let old = timeline::observation(e.snapshot().unwrap().as_ref(), &r, source)
        .unwrap()
        .unwrap();
    downgrade(e);
    assert!(next(e.snapshot().unwrap().as_ref(), &r, source).is_err());
    apply(e, &r, LedgerRequest::Initialize);
    assert_eq!(
        timeline::observation(e.snapshot().unwrap().as_ref(), &r, source)
            .unwrap()
            .unwrap(),
        old
    );
    let request = next(e.snapshot().unwrap().as_ref(), &r, source)
        .unwrap()
        .unwrap();
    let wire = LedgerRequest::AdvanceConfigurationIntervals(request);
    let b = encode_request(r.digest(), &wire).unwrap();
    assert_eq!(decode_request(&b, r.digest()).unwrap(), wire);
    for n in 0..b.len() {
        assert!(decode_request(&b[..n], r.digest()).is_err());
    }
    for bad in [
        Request {
            timeline_sequence: request.timeline_sequence + 1,
            ..request
        },
        Request {
            native_at: 0,
            ..request
        },
        Request {
            expected: u64::MAX,
            ..request
        },
    ] {
        assert!(plan_retention(
            e.snapshot().unwrap().as_ref(),
            &r,
            &LedgerRequest::AdvanceConfigurationIntervals(bad)
        )
        .is_err());
    }
    step(e, &r, source);
    downgrade(e);
    assert!(plan_retention(
        e.snapshot().unwrap().as_ref(),
        &r,
        &LedgerRequest::Initialize
    )
    .is_err());
    for namespace in [59, 60] {
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
}

#[test]
fn intervals_missing_rows_and_validly_encoded_wrong_roles_refuse_retry_and_query() {
    for corruption in 0..3 {
        let (s, r) = fixture();
        let c = group(&s, 1);
        let h = history(&s, &c, &r);
        let e = s.engine().as_ref();
        let source = seed(e, &r, &c, &h, true);
        let request = step(e, &r, source);
        let mut b = WriteBatch::new();
        match corruption {
            0 => {
                b.delete(ColumnFamily::Default, source.key(59).unwrap());
            }
            1 => {
                b.delete(ColumnFamily::Default, row_key(source, 4).unwrap());
            }
            _ => {
                let view = e.snapshot().unwrap();
                let p = planner(view.as_ref(), &r).unwrap();
                let mut entry = read_entry(&p, source, 4).unwrap().unwrap();
                entry.members.as_mut().unwrap()[0].roles = 4;
                b.put(
                    ColumnFamily::Default,
                    row_key(source, 4).unwrap(),
                    entry.encode(r.digest(), source).unwrap(),
                );
            }
        }
        e.write(b).unwrap();
        let view = e.snapshot().unwrap();
        assert!(observation(view.as_ref(), &r, source).is_err());
        assert!(intervals(view.as_ref(), &r, source, None).is_err());
        assert!(plan_retention(
            view.as_ref(),
            &r,
            &LedgerRequest::AdvanceConfigurationIntervals(request)
        )
        .is_err());
    }
}

#[test]
fn intervals_reencoded_stores_and_provenance_gaps_are_rejected() {
    for corruption in 0..6 {
        let (s, r) = fixture();
        let c = group(&s, 1);
        let mut h = history(&s, &c, &r);
        h.truncate(4);
        if matches!(corruption, 1 | 2) {
            h[3].kind = RecordKind::Snapshot {
                at: AppliedPosition {
                    term: 4,
                    index: 100,
                },
            };
            h[3].origin = None;
        } else if matches!(corruption, 3 | 4) {
            h[3].origin = None;
        }
        let e = s.engine().as_ref();
        let source = seed(e, &r, &c, &h, true);
        let request = step(e, &r, source);
        let view = e.snapshot().unwrap();
        let p = planner(view.as_ref(), &r).unwrap();
        let mut entry = read_entry(&p, source, 4).unwrap().unwrap();
        let mapped = entry.members.as_mut().unwrap();
        match corruption {
            0 => mapped[1].store = mapped[0].store,
            1 | 3 => mapped[0].store = Some(StoreIncarnation::from_bytes([1; 16])),
            2 => entry.gaps &= !SNAPSHOT_ANCESTRY,
            4 => entry.gaps &= !LEGACY_CHANGE,
            _ => entry.gaps |= UNSUPPORTED_STATE,
        }
        // Recompute framing/checksum: refusal must come from correspondence to
        // the actual source and provenance, not an incidental byte checksum.
        let bytes = entry.encode(r.digest(), source).unwrap();
        let mut b = WriteBatch::new();
        b.put(ColumnFamily::Default, row_key(source, 4).unwrap(), bytes);
        e.write(b).unwrap();
        let view = e.snapshot().unwrap();
        assert!(
            intervals(view.as_ref(), &r, source, None).is_err(),
            "corruption {corruption}"
        );
        assert!(plan_retention(
            view.as_ref(),
            &r,
            &LedgerRequest::AdvanceConfigurationIntervals(request)
        )
        .is_err());
    }
}

#[test]
fn intervals_same_node_replacement_cannot_rebind_an_existing_learner() {
    let (s, r) = fixture();
    let c = group(&s, 1);
    let mut h = history(&s, &c, &r);
    h.truncate(2);
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
    let range = DataRange {
        root: r.digest(),
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
    let origin = data_groups::store_history::configuration_origin(
        s.engine().snapshot().unwrap().as_ref(),
        &range,
        replacement.task(),
        replacement.operation(),
        replacement.destination(),
        Action::Attach,
    )
    .unwrap();
    h.push(ConfigurationRecord {
        ordinal: 3,
        kind: RecordKind::Applied { index: 2 },
        state: h[1].state.clone(),
        origin: Some(AppliedConfigurationOrigin {
            at: AppliedPosition { term: 2, index: 2 },
            origin,
        }),
    });
    let e = s.engine().as_ref();
    let source = seed(e, &r, &c, &h, true);
    finish(e, &r, source);
    let rows = all(e, &r, source);
    assert_eq!(
        rows[1]
            .entry
            .members
            .as_ref()
            .unwrap()
            .last()
            .unwrap()
            .store,
        Some(StoreIncarnation::from_bytes([4; 16]))
    );
    assert_ne!(rows[2].gaps & STORE_CONFLICT, 0);
    assert!(rows[2]
        .entry
        .members
        .as_ref()
        .unwrap()
        .iter()
        .all(|m| m.store.is_none()));
}

mod budget;
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
fn q(c: &CreationIntent, revision: u64, expected: u64) -> timeline::Request {
    timeline::Request {
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
    q: timeline::Request,
    cursor: Option<TimelineCursor>,
    page: &TimelinePage,
) {
    let plan = timeline::plan_verified(
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
