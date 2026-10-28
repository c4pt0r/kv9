use super::*;
use crate::configuration_origin::tests::{change, origin};
use crate::configuration_origin::Action;
use kv9_common::fs::testing::{Crash, Fault, ModelFs, Operation};
use raft::prelude::{EntryType, Snapshot};

fn path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "kv9-timeline-{}",
        kv9_common::StoreIncarnation::mint().unwrap()
    ))
}
fn commit<F: FileSystem>(
    s: &DiskRaftStorage<F>,
    index: u64,
    term: u64,
    o: Option<crate::configuration_origin::ConfigurationOrigin>,
) {
    let data = o
        .map(|o| change(o).write_to_bytes().unwrap())
        .unwrap_or_default();
    s.persist_ready(
        &[Entry {
            term,
            index,
            entry_type: EntryType::EntryConfChangeV2,
            data: data.into(),
            ..Default::default()
        }],
        Some(&HardState {
            term,
            commit: index,
            ..Default::default()
        }),
    )
    .unwrap();
}
fn all<F: FileSystem>(s: &DiskRaftStorage<F>) -> Vec<TimelinePage> {
    let mut pages = Vec::new();
    let mut cursor = None;
    loop {
        let p = s.configuration_timeline_page(cursor).unwrap();
        p.validate_progress(cursor).unwrap();
        cursor = p.next;
        pages.push(p);
        if cursor.is_none() {
            return pages;
        }
    }
}
fn joint() -> ConfState {
    let mut cs = ConfState::from((vec![1, 2, 4], vec![5]));
    cs.voters_outgoing = vec![1, 2, 3];
    cs.learners_next = vec![3];
    cs.auto_leave = true;
    cs
}
#[test]
fn timeline_keeps_native_applied_joint_and_snapshot_boundaries_through_real_replay() {
    let dir = path();
    let s = DiskRaftStorage::open(&dir, &[1, 2, 3]).unwrap().0;
    commit(&s, 1, 2, Some(origin()));
    assert_eq!(
        all(&s)[0].total,
        1,
        "committed but not applied is not a configuration observation"
    );
    s.set_conf_state(&ConfState::from((vec![1, 2, 3], vec![4])), 1)
        .unwrap();
    commit(&s, 2, 2, None);
    s.set_conf_state(&joint(), 2).unwrap();
    let mut image = Snapshot {
        data: b"timeline image".to_vec().into(),
        ..Default::default()
    };
    image.mut_metadata().term = 3;
    image.mut_metadata().index = 10;
    image
        .mut_metadata()
        .set_conf_state(ConfState::from((vec![1, 2, 6], vec![7])));
    s.install_protocol_snapshot(
        &image,
        &HardState {
            term: 3,
            commit: 10,
            ..Default::default()
        },
    )
    .unwrap();
    commit(&s, 11, 3, None);
    s.set_conf_state(&ConfState::from((vec![1, 2, 6], vec![8])), 11)
        .unwrap();
    let pages = all(&s);
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].total, 5);
    assert!(pages[0].gaps.snapshot_prefix_unknown);
    let mut wrong = pages[0].clone();
    wrong.gaps.snapshot_prefix_unknown = false;
    assert!(wrong.encode().is_err());
    let records: Vec<_> = pages.iter().flat_map(|p| &p.records).collect();
    assert_eq!(records[0].kind, RecordKind::Initial);
    assert_eq!(records[0].state.as_ref().unwrap().voters, vec![1, 2, 3]);
    assert_eq!(records[1].origin.unwrap().origin, origin());
    assert!(records[2].origin.is_none());
    let cs = records[2].state.as_ref().unwrap();
    assert_eq!(cs.outgoing, vec![1, 2, 3]);
    assert_eq!(cs.learners_next, vec![3]);
    assert_eq!(cs.learners, vec![5]);
    assert!(cs.auto_leave);
    assert_eq!(
        records[3].kind,
        RecordKind::Snapshot {
            at: AppliedPosition { term: 3, index: 10 }
        }
    );
    assert_eq!(records[4].state.as_ref().unwrap().learners, vec![8]);
    drop(s);
    let s = DiskRaftStorage::recover(&dir).unwrap();
    assert_eq!(all(&s), pages);
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn timeline_rewrite_preserves_legacy_states_origins_and_exact_page_fingerprint() {
    let dir = path();
    let s = DiskRaftStorage::open(&dir, &[1, 2, 3]).unwrap().0;
    let learner = ConfState::from((vec![1, 2, 3], vec![4]));
    let native = ConfState::from((vec![1, 2, 3], vec![]));
    for (index, o, cs) in [
        (1, Some(origin()), &learner),
        (
            2,
            Some(crate::configuration_origin::ConfigurationOrigin {
                action: Action::Detach,
                task: 105,
                ..origin()
            }),
            &native,
        ),
        (3, None, &joint()),
    ] {
        commit(&s, index, 2, o);
        s.set_conf_state(cs, index).unwrap();
    }
    let before = all(&s);
    s.set_conf_state(&joint(), 3).unwrap();
    assert_eq!(all(&s), before, "identical indexed replay is idempotent");
    s.compact_retained_prefix(
        AppliedPosition { term: 2, index: 3 },
        &joint(),
        b"authorized floor",
    )
    .unwrap();
    assert!(s.rewrite_log().unwrap());
    drop(s);
    let s = DiskRaftStorage::recover(&dir).unwrap();
    assert_eq!(all(&s), before);
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn timeline_observation_and_source_are_atomic_under_sync_failure_and_crash() {
    let setup = || {
        let fs = ModelFs::default();
        let s = DiskRaftStorage::open_on(fs.clone(), Path::new("/timeline"), &[1, 2, 3])
            .unwrap()
            .0;
        commit(&s, 1, 2, Some(origin()));
        fs.clear_events();
        (fs, s)
    };
    let cs = ConfState::from((vec![1, 2, 3], vec![4]));
    let (probe, s) = setup();
    s.set_conf_state(&cs, 1).unwrap();
    for event in probe
        .events()
        .into_iter()
        .filter(|e| matches!(e.operation, Operation::Write | Operation::SyncData))
    {
        for fault in [Fault::Before(5), Fault::After(5)] {
            for crash in [Crash::LoseUnsynced, Crash::KeepUnsynced] {
                let (fs, s) = setup();
                fs.fail_at(event.number, fault);
                assert!(s.set_conf_state(&cs, 1).is_err());
                assert!(fs.fault_arrived());
                assert!(s.configuration_timeline_page(None).is_err());
                drop(s);
                fs.crash(crash);
                let s = DiskRaftStorage::open_on(fs, Path::new("/timeline"), &[])
                    .unwrap()
                    .0;
                let p = s.configuration_timeline_page(None).unwrap();
                assert_eq!(p.total == 2, s.recovered_conf_index() == 1);
                if p.total == 2 {
                    assert_eq!(p.records[1].origin.unwrap().origin, origin());
                    assert_eq!(p.records[1].state, ConfigurationState::observe(&cs));
                }
            }
        }
    }
}
#[test]
fn timeline_unknown_state_and_out_of_order_configuration_are_not_erased() {
    let mut h = membership_history::MembershipHistory::default();
    let native = ConfState::from((vec![1, 2, 3], vec![]));
    h.initial(&native);
    let mut future = native.clone();
    future.mut_unknown_fields().add_varint(100, 42);
    h.applied(2, &future);
    h.applied(1, &native);
    let p = h.timeline_page(None).unwrap();
    assert!(p.gaps.unsupported_configuration && p.gaps.ambiguous_records);
    assert!(p.records[1].state.is_none());
    assert!(p.records[1].origin.is_none());
    assert_eq!(p.records[2].kind, RecordKind::Applied { index: 1 });
    h.installed(AppliedPosition { term: 3, index: 8 }, &native);
    let p = h.timeline_page(None).unwrap();
    assert!(p.gaps.snapshot_prefix_unknown && p.gaps.ambiguous_records);
    assert_eq!(p.total, 4);
}
#[test]
fn timeline_wire_rejects_forged_cursor_truncation_gaps_and_unbounded_counts() {
    let fs = ModelFs::default();
    let s = DiskRaftStorage::open_on(fs, Path::new("/timeline-wire"), &[1, 2, 3])
        .unwrap()
        .0;
    for index in 1..=5 {
        commit(&s, index, 2, None);
        s.set_conf_state(&ConfState::from((vec![1, 2, 3], vec![index + 3])), index)
            .unwrap();
    }
    let p = s.configuration_timeline_page(None).unwrap();
    let b = p.encode().unwrap();
    assert_eq!(TimelinePage::decode(&b).unwrap(), p);
    for cut in 0..b.len() {
        assert!(TimelinePage::decode(&b[..cut]).is_err());
    }
    let cursor = p.next.unwrap();
    let mut forged = cursor.encode();
    forged[40..48].copy_from_slice(&8u64.to_be_bytes());
    assert!(s
        .configuration_timeline_page(Some(TimelineCursor::decode(&forged).unwrap()))
        .is_err());
    let last = s.configuration_timeline_page(p.next).unwrap();
    assert!(last.validate_progress(None).is_err());
    assert_eq!(TimelinePage::decode(&last.encode().unwrap()).unwrap(), last);
    let mut bad = b.clone();
    bad[49] = 5;
    assert!(TimelinePage::decode(&bad).is_err());
    let mut bad = b.clone();
    bad[50..54].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(TimelinePage::decode(&bad).is_err());
    let mut bad = p.clone();
    bad.records[1].ordinal += 1;
    assert!(bad.encode().is_err());
    commit(&s, 6, 2, None);
    s.set_conf_state(&ConfState::from((vec![1, 2, 3], vec![9])), 6)
        .unwrap();
    assert!(s.configuration_timeline_page(Some(cursor)).is_err());
}
#[test]
fn timeline_page_scratch_is_independent_of_total_history_and_large_joint_sets_fit() {
    let mut h = membership_history::MembershipHistory::default();
    let cs = ConfState::from((vec![1, 2, 3], vec![]));
    h.initial(&cs);
    let mut peaks = Vec::new();
    for index in 1..=10000 {
        h.applied(index, &cs);
        if index == 10 || index == 10000 {
            let (p, peak) = crate::test_alloc::peak(|| h.timeline_page(None).unwrap());
            assert_eq!(p.records.len(), 4);
            assert_eq!(p.total, index + 1);
            peaks.push(peak);
        }
    }
    assert!(peaks.iter().all(|p| *p < 8192), "{peaks:?}");
    assert!(peaks[1] <= peaks[0] + 128, "{peaks:?}");
    let mut h = membership_history::MembershipHistory::default();
    let mut cs = ConfState::from((
        (1..=1024).collect::<Vec<_>>(),
        (2049..=3072).collect::<Vec<_>>(),
    ));
    cs.voters_outgoing = (1025..=2048).collect();
    cs.learners_next = cs.voters_outgoing.clone();
    cs.auto_leave = true;
    h.initial(&cs);
    for index in 1..=4 {
        h.applied(index, &cs);
    }
    let p = h.timeline_page(None).unwrap();
    let bytes = p.encode().unwrap();
    assert!(bytes.len() > 128 * 1024 && bytes.len() <= MAX_PAGE_BYTES);
    assert_eq!(TimelinePage::decode(&bytes).unwrap(), p);
    cs.voters.push(4097);
    h.applied(5, &cs);
    assert!(h.timeline_page(p.next).is_err());
    let changed = h.timeline_page(None).unwrap();
    assert!(changed.gaps.unsupported_configuration);
}
