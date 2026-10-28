use super::*;

fn populated() -> (PathBuf, DiskRaftStorage) {
    let dir = path();
    let s = DiskRaftStorage::open(&dir, &[1, 2, 3]).unwrap().0;
    for index in 1..=9 {
        let o = ConfigurationOrigin {
            task: 100 + index,
            operation: [index as u8; 16],
            ..origin()
        };
        commit(&s, entry(index, o));
        s.set_conf_state(&ConfState::from((vec![1, 2, 3], vec![4])), index)
            .unwrap();
    }
    (dir, s)
}
#[test]
fn origin_query_pages_reject_changed_history_forged_rank_and_malformed_wire() {
    let (dir, s) = populated();
    let first = s.configuration_origins_page(None).unwrap();
    assert_eq!(first.entries.len(), 4);
    assert_eq!(first.total_known, 9);
    let encoded = first.encode().unwrap();
    assert_eq!(OriginPage::decode(&encoded).unwrap(), first);
    for end in 0..encoded.len() {
        assert!(OriginPage::decode(&encoded[..end]).is_err());
    }
    for field in 0..7 {
        let mut bad = encoded.clone();
        match field {
            0 => bad[40] |= 128,
            1 => bad[65..73].copy_from_slice(&3u64.to_be_bytes()),
            2 => bad[82..90].fill(0),
            3 => bad[74 + 185 + 8..74 + 185 + 16].copy_from_slice(&1u64.to_be_bytes()),
            4 => bad[41..49].copy_from_slice(&1u64.to_be_bytes()),
            5 => bad[73] = 5,
            _ => bad.push(0),
        }
        assert!(
            OriginPage::decode(&bad).is_err(),
            "malformed page field {field}"
        );
    }
    let cursor = first.next.unwrap();
    assert_eq!(OriginCursor::decode(&cursor.encode()).unwrap(), cursor);
    for field in [8..40, 40..48, 48..56] {
        let mut bad = cursor.encode();
        if field.start == 40 {
            bad[field].copy_from_slice(&3u64.to_be_bytes());
        } else if field.start == 48 {
            bad[field].copy_from_slice(&8u64.to_be_bytes());
        } else {
            bad[field].fill(42);
        }
        let forged = OriginCursor::decode(&bad).unwrap();
        assert!(s.configuration_origins_page(Some(forged)).is_err());
    }
    let second = s.configuration_origins_page(Some(cursor)).unwrap();
    second.validate_progress(Some(cursor)).unwrap();
    assert!(second.validate_progress(None).is_err());
    let last = s.configuration_origins_page(second.next).unwrap();
    last.validate_progress(second.next).unwrap();
    assert_eq!(last.entries.len(), 1);
    assert!(last.next.is_none());
    commit(
        &s,
        entry(
            10,
            ConfigurationOrigin {
                task: 110,
                ..origin()
            },
        ),
    );
    s.set_conf_state(&ConfState::from((vec![1, 2, 3], vec![4])), 10)
        .unwrap();
    assert!(s.configuration_origins_page(Some(cursor)).is_err());
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn origin_query_legacy_gap_and_cached_fingerprint_survive_rewrite_and_snapshot() {
    let (dir, s) = populated();
    commit(
        &s,
        Entry {
            term: 2,
            index: 10,
            ..Default::default()
        },
    );
    s.set_conf_state(&ConfState::from((vec![1, 2, 3], vec![4])), 10)
        .unwrap();
    let before = s.configuration_origins_page(None).unwrap();
    assert!(before.legacy_contexts_unknown);
    s.compact_retained_prefix(
        AppliedPosition { term: 2, index: 9 },
        &ConfState::from((vec![1, 2, 3], vec![4])),
        b"authorized floor",
    )
    .unwrap();
    s.rewrite_log().unwrap();
    drop(s);
    let s = DiskRaftStorage::recover(&dir).unwrap();
    assert_eq!(s.configuration_origins_page(None).unwrap(), before);
    let mut image = Snapshot {
        data: b"image".to_vec().into(),
        ..Default::default()
    };
    image.mut_metadata().index = 20;
    image.mut_metadata().term = 3;
    image
        .mut_metadata()
        .set_conf_state(ConfState::from((vec![1, 2, 3], vec![5])));
    s.install_protocol_snapshot(
        &image,
        &HardState {
            term: 3,
            commit: 20,
            ..Default::default()
        },
    )
    .unwrap();
    let after = s.configuration_origins_page(None).unwrap();
    assert!(after.legacy_contexts_unknown && after.membership_gaps.snapshot_prefix_unknown);
    assert_eq!(before.entries, after.entries);
    assert!(s.configuration_origins_page(before.next).is_err());
    drop(s);
    let s = DiskRaftStorage::recover(&dir).unwrap();
    assert_eq!(s.configuration_origins_page(None).unwrap(), after);
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn origin_query_scratch_stays_bounded_as_retained_history_grows() {
    let mut history = super::super::super::membership_history::MembershipHistory::default();
    history.initial(&ConfState::from((vec![1, 2, 3], vec![])));
    history.applied(10000, &ConfState::from((vec![1, 2, 3], vec![4])));
    let membership = history.page(None).unwrap();
    let mut origins = Origins::new();
    let mut peaks = Vec::new();
    for index in 1..=10000 {
        insert(
            &mut origins,
            AppliedConfigurationOrigin {
                at: AppliedPosition { term: 2, index },
                origin: origin(),
            },
        )
        .unwrap();
        if index == 10 || index == 10000 {
            let (page, peak) = crate::test_alloc::peak(|| {
                origins
                    .page(
                        membership.clone(),
                        crate::storage::membership_history::InitialConfiguration::Missing,
                        None,
                    )
                    .unwrap()
            });
            assert_eq!(page.entries.len(), 4);
            assert_eq!(page.total_known, index);
            peaks.push(peak);
        }
    }
    assert!(peaks.iter().all(|p| *p < 4096), "page scratch: {peaks:?}");
    assert!(
        peaks[1] <= peaks[0] + 256,
        "page scratch grew with history: {peaks:?}"
    );
}
