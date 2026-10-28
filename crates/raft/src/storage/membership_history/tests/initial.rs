use super::*;

#[test]
fn initial_origin_native_record_survives_joint_rewrite_snapshot_and_replay() {
    let dir = std::env::temp_dir().join(format!(
        "kv9-initial-origin-{}",
        kv9_common::StoreIncarnation::mint().unwrap()
    ));
    let s = DiskRaftStorage::open(&dir, &[1, 2, 3]).unwrap().0;
    let expected = Some(InitialConfiguration::Voters(vec![
        NodeId(1),
        NodeId(2),
        NodeId(3),
    ]));
    assert_eq!(
        s.configuration_origins_page(None).unwrap().initial,
        expected
    );
    commit(&s);
    s.set_conf_state(&joint(), 2).unwrap();
    s.compact_retained_prefix(
        AppliedPosition { term: 2, index: 3 },
        &joint(),
        b"authorized cut",
    )
    .unwrap();
    s.rewrite_log().unwrap();
    drop(s);
    let s = DiskRaftStorage::recover(&dir).unwrap();
    assert_eq!(
        s.configuration_origins_page(None).unwrap().initial,
        expected
    );
    install(&s, 12, ConfState::from((vec![1, 4, 5], vec![6])));
    let observed = s.configuration_origins_page(None).unwrap();
    assert_eq!(observed.initial, expected);
    assert!(observed.membership_gaps.snapshot_prefix_unknown);
    drop(s);
    let s = DiskRaftStorage::recover(&dir).unwrap();
    assert_eq!(s.configuration_origins_page(None).unwrap(), observed);
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn initial_origin_missing_ambiguous_and_unsupported_records_do_not_infer_from_snapshot() {
    let normal = ConfState::from((vec![3, 1, 2], vec![]));
    let expected = InitialConfiguration::Voters(vec![NodeId(1), NodeId(2), NodeId(3)]);
    let mut h = MembershipHistory::default();
    h.retained_base(&normal);
    h.installed(AppliedPosition { term: 3, index: 10 }, &normal);
    assert_eq!(h.initial_observation(), InitialConfiguration::Missing);
    h.initial(&normal);
    assert_eq!(h.initial_observation(), InitialConfiguration::Ambiguous);
    let mut h = MembershipHistory::default();
    h.initial(&normal);
    assert_eq!(h.initial_observation(), expected);
    h.initial(&normal);
    assert_eq!(h.initial_observation(), InitialConfiguration::Ambiguous);
    for cs in [
        ConfState::from((vec![1], vec![])),
        ConfState::from((vec![1, 1, 2], vec![])),
        ConfState::from((vec![0, 1, 2], vec![])),
        ConfState::from((vec![1, 2, 3], vec![4])),
        joint(),
    ] {
        let mut h = MembershipHistory::default();
        h.initial(&cs);
        assert_eq!(h.initial_observation(), InitialConfiguration::Unsupported);
        h.installed(AppliedPosition { term: 3, index: 10 }, &normal);
        assert_eq!(h.initial_observation(), InitialConfiguration::Unsupported);
    }
}

#[test]
fn initial_origin_wire_is_canonical_bounded_and_legacy_page_stays_legacy() {
    for n in [3, 5, 7] {
        let cs = ConfState::from(((1..=n).rev().collect::<Vec<_>>(), vec![]));
        let initial = InitialConfiguration::observe(&cs);
        let encoded = initial.encode().unwrap();
        assert_eq!(encoded.len(), 2 + n as usize * 8);
        assert_eq!(InitialConfiguration::decode(&encoded).unwrap(), initial);
        for cut in 0..encoded.len() {
            assert!(InitialConfiguration::decode(&encoded[..cut]).is_err());
        }
        for mutation in 0..5 {
            let mut b = encoded.clone();
            match mutation {
                0 => b[1] = 8,
                1 => b[2..10].fill(0),
                2 => b[10..18].copy_from_slice(&1u64.to_be_bytes()),
                3 => b[0] = 4,
                _ => b.push(0),
            }
            assert!(InitialConfiguration::decode(&b).is_err());
        }
    }
    let s = opened(&ModelFs::default());
    let mut p = s.configuration_origins_page(None).unwrap();
    assert_eq!(&p.encode().unwrap()[..8], b"KV9OPG02");
    p.initial = None;
    let old = p.encode().unwrap();
    assert_eq!(&old[..8], b"KV9OPG01");
    let decoded = crate::storage::configuration_origins::OriginPage::decode(&old).unwrap();
    assert!(decoded.initial.is_none());
    assert_eq!(decoded.encode().unwrap(), old);
}
