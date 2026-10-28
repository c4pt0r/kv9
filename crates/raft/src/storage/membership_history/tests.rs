use super::*;
use crate::rawnode::PersistentRaftStorage;
use kv9_common::fs::testing::{Crash, Fault, ModelFs, Operation};
use raft::prelude::{EntryType, Snapshot};
mod initial;

#[test]
fn membership_wire_roundtrip_keeps_gaps_and_rejects_corrupt_or_skipped_pages() {
    let fs = ModelFs::default();
    let s = opened(&fs);
    commit(&s);
    s.set_conf_state(&joint(), 2).unwrap();
    install(&s, 12, ConfState::from((vec![1, 2, 6], vec![7])));
    let first = s.membership_history_page(None).unwrap();
    assert!(first.gaps.snapshot_prefix_unknown);
    first.validate_progress(None, 0).unwrap();
    let encoded = first.encode().unwrap();
    assert_eq!(encoded.len(), 112);
    assert_eq!(MembershipPage::decode(&encoded).unwrap(), first);
    for cut in 0..encoded.len() {
        assert!(MembershipPage::decode(&encoded[..cut]).is_err());
    }
    let cursor = first.next.unwrap();
    let bytes = cursor.encode();
    assert_eq!(MembershipCursor::decode(&bytes).unwrap(), cursor);
    for cut in 0..bytes.len() {
        assert!(MembershipCursor::decode(&bytes[..cut]).is_err());
    }
    for range in [0..8, 8..40, 40..48] {
        let mut wrong = bytes.clone();
        wrong[range].fill(0);
        assert!(MembershipCursor::decode(&wrong).is_err());
    }
    let last = s.membership_history_page(Some(cursor)).unwrap();
    last.validate_progress(Some(cursor), 4).unwrap();
    assert_eq!(
        MembershipPage::decode(&last.encode().unwrap()).unwrap(),
        last
    );
    assert!(last.validate_progress(None, 0).is_err());
    assert!(last.validate_progress(Some(cursor), 3).is_err());
    assert!(first.validate_progress(Some(cursor), 4).is_err());
    for mutation in 0..10 {
        let mut wrong = encoded.clone();
        match mutation {
            0 => wrong[40] |= 32,                                    // unknown flags
            1 => wrong[76] = 0,                                      // no role
            2 => wrong[76] = 16,                                     // unknown role
            3 => wrong[77..85].copy_from_slice(&1u64.to_be_bytes()), // duplicate node
            4 => wrong[65..67].copy_from_slice(&4097u16.to_be_bytes()),
            5 => wrong[67] = 5,
            6 => wrong[40] &= !2,       // snapshot without unknown prefix
            7 => wrong[49..57].fill(0), // invalid snapshot term
            8 => wrong[104..112].copy_from_slice(&99u64.to_be_bytes()), // wrong next
            _ => wrong.push(0),
        }
        assert!(
            MembershipPage::decode(&wrong).is_err(),
            "mutation {mutation}"
        );
    }
    let mut history = MembershipHistory::default();
    let empty = history.page(None).unwrap();
    empty.validate_progress(None, 0).unwrap();
    assert!(
        MembershipPage::decode(&empty.encode().unwrap())
            .unwrap()
            .gaps
            .missing_initial
    );
    history.initial(&ConfState::from((vec![1], vec![])));
    let changed = history.page(None).unwrap();
    assert!(changed.validate_progress(Some(cursor), 4).is_err());
}

fn opened(fs: &ModelFs) -> DiskRaftStorage<ModelFs> {
    DiskRaftStorage::open_on(fs.clone(), Path::new("/member-history"), &[1, 2, 3])
        .unwrap()
        .0
}
fn commit<F: FileSystem>(s: &DiskRaftStorage<F>) {
    let entries: Vec<_> = (1..=6)
        .map(|index| Entry {
            index,
            term: 2,
            entry_type: EntryType::EntryConfChangeV2,
            ..Entry::default()
        })
        .collect();
    s.persist_ready(
        &entries,
        Some(&HardState {
            term: 2,
            vote: 2,
            commit: 6,
            ..HardState::default()
        }),
    )
    .unwrap();
}
fn joint() -> ConfState {
    let mut cs = ConfState::from((vec![1, 2, 4], vec![5]));
    cs.voters_outgoing = vec![1, 2, 3];
    cs.learners_next = vec![3];
    cs.auto_leave = true;
    cs
}
fn install<F: FileSystem>(s: &DiskRaftStorage<F>, index: u64, cs: ConfState) {
    let mut image = Snapshot {
        data: b"membership-test-image".to_vec().into(),
        ..Snapshot::default()
    };
    image.mut_metadata().index = index;
    image.mut_metadata().term = 3;
    image.mut_metadata().set_conf_state(cs);
    s.install_protocol_snapshot(
        &image,
        &HardState {
            term: 4,
            vote: 2,
            commit: index,
            ..HardState::default()
        },
    )
    .unwrap();
}
fn all<F: FileSystem>(s: &DiskRaftStorage<F>) -> (MembershipPage, Vec<HistoricalMember>) {
    let first = s.membership_history_page(None).unwrap();
    let mut members = first.members.clone();
    let mut cursor = first.next;
    while let Some(next) = cursor {
        let page = s.membership_history_page(Some(next)).unwrap();
        assert_eq!(page.fingerprint, first.fingerprint);
        assert!(page.members.len() <= MEMBERSHIP_HISTORY_PAGE);
        members.extend(page.members);
        cursor = page.next;
    }
    assert_eq!(members.len(), first.known_nodes);
    (first, members)
}

#[test]
fn removed_and_joint_members_survive_snapshot_replacement_and_reopen_with_unknown_prefix() {
    let fs = ModelFs::default();
    let s = opened(&fs);
    commit(&s);
    s.set_conf_state(&joint(), 2).unwrap();
    s.set_conf_state(&ConfState::from((vec![1, 2, 4], vec![])), 4)
        .unwrap();
    let (before, known) = all(&s);
    assert!(!before.gaps.any());
    assert_eq!(
        known.iter().find(|m| m.node == NodeId(3)).unwrap().roles,
        SEEN_VOTER | SEEN_OUTGOING_VOTER | SEEN_NEXT_LEARNER
    );
    assert_eq!(
        known.iter().find(|m| m.node == NodeId(5)).unwrap().roles,
        SEEN_LEARNER
    );
    install(&s, 12, ConfState::from((vec![1, 2, 6], vec![7])));
    s.persist_ready(
        &[Entry {
            index: 13,
            term: 3,
            entry_type: EntryType::EntryConfChangeV2,
            ..Entry::default()
        }],
        Some(&HardState {
            term: 4,
            vote: 2,
            commit: 13,
            ..HardState::default()
        }),
    )
    .unwrap();
    s.set_conf_state(&ConfState::from((vec![1, 2, 6], vec![8])), 13)
        .unwrap();
    install(&s, 20, ConfState::from((vec![1, 2, 9], vec![])));
    let expected = all(&s);
    assert!(expected.0.gaps.snapshot_prefix_unknown);
    assert!(!expected.0.gaps.missing_initial);
    assert!(!expected.0.gaps.ambiguous_records);
    assert_eq!(
        expected.0.last_snapshot,
        Some(AppliedPosition { term: 3, index: 20 })
    );
    assert_eq!(
        expected.1.iter().map(|m| m.node.0).collect::<Vec<_>>(),
        (1..=9).collect::<Vec<_>>()
    );
    drop(s);
    fs.crash(Crash::LoseUnsynced);
    assert_eq!(all(&opened(&fs)), expected);
}

#[test]
fn pagination_rejects_changed_roles_and_positions_without_losing_removed_nodes() {
    let fs = ModelFs::default();
    let s = opened(&fs);
    commit(&s);
    s.set_conf_state(&joint(), 2).unwrap();
    let first = s.membership_history_page(None).unwrap();
    assert_eq!(first.members.len(), MEMBERSHIP_HISTORY_PAGE);
    let next = first.next.unwrap();
    assert_eq!(
        s.membership_history_page(Some(next)).unwrap().members[0].node,
        NodeId(5)
    );
    s.set_conf_state(&ConfState::from((vec![1, 2, 5], vec![])), 4)
        .unwrap();
    assert!(s.membership_history_page(Some(next)).is_err());
    let (current, members) = all(&s);
    assert_eq!(current.last_configuration_index, 4);
    assert_eq!(current.known_nodes, 5);
    assert_eq!(members.last().unwrap().roles, SEEN_LEARNER | SEEN_VOTER);
    assert!(!current.gaps.any());
}

#[test]
fn ambiguous_unsupported_and_missing_history_stay_unknown_across_later_snapshots() {
    let mut h = MembershipHistory::default();
    assert!(h.page(None).unwrap().gaps.missing_initial);
    h.applied(3, &ConfState::from((vec![1], vec![])));
    h.initial(&ConfState::from((vec![1], vec![])));
    assert!(h.page(None).unwrap().gaps.ambiguous_records);
    let mut cs = joint();
    cs.mut_unknown_fields().add_varint(100, 1);
    h.applied(4, &cs);
    h.installed(
        AppliedPosition { term: 2, index: 10 },
        &ConfState::from((vec![1, 2], vec![])),
    );
    let page = h.page(None).unwrap();
    assert!(
        page.gaps.unsupported_configuration
            && page.gaps.ambiguous_records
            && page.gaps.snapshot_prefix_unknown
    );
    let mut missing = MembershipHistory::default();
    missing.retained_base(&joint());
    assert!(missing.page(None).unwrap().gaps.missing_initial);
    missing.installed(AppliedPosition { term: 2, index: 10 }, &joint());
    assert!(missing.page(None).unwrap().gaps.missing_initial);
}

#[test]
fn bounded_inventory_overflow_is_sticky_and_order_independent() {
    let mut ascending = MembershipHistory::default();
    let mut descending = MembershipHistory::default();
    for h in [&mut ascending, &mut descending] {
        h.initial(&ConfState::from((vec![1], vec![])));
    }
    for node in 2..=(MAX_KNOWN_MEMBERS + 9) as u64 {
        ascending.retained_base(&ConfState::from((vec![node], vec![])));
    }
    for node in (2..=(MAX_KNOWN_MEMBERS + 9) as u64).rev() {
        descending.retained_base(&ConfState::from((vec![node], vec![])));
    }
    assert_eq!(ascending.members.len(), MAX_KNOWN_MEMBERS);
    assert_eq!(
        ascending.page(None).unwrap(),
        descending.page(None).unwrap()
    );
    assert!(ascending.page(None).unwrap().gaps.member_limit_exceeded);
    ascending.installed(
        AppliedPosition { term: 2, index: 10 },
        &ConfState::from((vec![1], vec![])),
    );
    assert!(ascending.page(None).unwrap().gaps.member_limit_exceeded);
}

#[test]
fn failed_configuration_persistence_cannot_expose_a_live_membership_inventory() {
    let fs = ModelFs::default();
    let s = opened(&fs);
    commit(&s);
    let probe = ModelFs::default();
    let probe_store = opened(&probe);
    commit(&probe_store);
    probe.clear_events();
    probe_store.set_conf_state(&joint(), 2).unwrap();
    let sync = probe
        .events()
        .into_iter()
        .find(|e| e.operation == Operation::SyncData)
        .unwrap();
    fs.clear_events();
    fs.fail_at(sync.number, Fault::Before(5));
    assert!(s.set_conf_state(&joint(), 2).is_err());
    assert!(s.membership_history_page(None).is_err());
    drop(s);
    fs.crash(Crash::LoseUnsynced);
    let (page, _) = all(&opened(&fs));
    assert_eq!(page.known_nodes, 3);
}

#[test]
fn ordinary_log_rewrite_preserves_inventory_and_cursor_on_real_disk() {
    let path = std::env::temp_dir().join(format!(
        "kv9-membership-{}",
        kv9_common::StoreIncarnation::mint().unwrap()
    ));
    let s = DiskRaftStorage::open(&path, &[1, 2, 3]).unwrap().0;
    commit(&s);
    s.set_conf_state(&joint(), 2).unwrap();
    s.set_conf_state(&ConfState::from((vec![1, 2, 4], vec![])), 4)
        .unwrap();
    let expected = all(&s);
    s.compact_retained_prefix(
        AppliedPosition { term: 2, index: 3 },
        &joint(),
        b"membership-test-floor",
    )
    .unwrap();
    assert!(s.rewrite_log().unwrap());
    assert_eq!(all(&s), expected);
    drop(s);
    let reopened = DiskRaftStorage::recover(&path).unwrap();
    assert_eq!(all(&reopened), expected);
    assert_eq!(
        reopened
            .membership_history_page(expected.0.next)
            .unwrap()
            .members[0]
            .node,
        NodeId(5)
    );
    drop(reopened);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn rewrite_refuses_to_erase_an_observation_found_only_in_an_older_base() {
    let path = std::env::temp_dir().join(format!(
        "kv9-membership-base-{}",
        kv9_common::StoreIncarnation::mint().unwrap()
    ));
    let s = DiskRaftStorage::open(&path, &[1, 2, 3]).unwrap().0;
    commit(&s);
    // The low-level compaction seam accepts a supplied base. Such old/foreign
    // observations must not disappear just because a newer base agrees with
    // indexed history. They are evidence requiring investigation, not grants.
    s.compact_retained_prefix(
        AppliedPosition { term: 2, index: 2 },
        &ConfState::from((vec![1, 2, 8], vec![])),
        b"older-base",
    )
    .unwrap();
    s.compact_retained_prefix(
        AppliedPosition { term: 2, index: 4 },
        &ConfState::from((vec![1, 2, 3], vec![])),
        b"current-base",
    )
    .unwrap();
    let expected = all(&s);
    assert!(expected.1.iter().any(|m| m.node == NodeId(8)));
    let bytes = std::fs::read(path.join("raft.log")).unwrap();
    assert!(
        s.rewrite_log().is_err(),
        "rewrite would erase a retained membership observation"
    );
    assert_eq!(std::fs::read(path.join("raft.log")).unwrap(), bytes);
    drop(s);
    let recovered = DiskRaftStorage::recover(&path).unwrap();
    assert_eq!(all(&recovered), expected);
    drop(recovered);
    std::fs::remove_dir_all(path).unwrap();
}
