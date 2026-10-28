use super::*;
use crate::configuration_origin::{
    tests::{change, origin},
    Action,
};
use kv9_common::fs::testing::{Crash, Fault, ModelFs, Operation};
use raft::prelude::Snapshot;
mod pages;

fn entry(index: u64, origin: ConfigurationOrigin) -> Entry {
    Entry {
        index,
        term: 2,
        entry_type: EntryType::EntryConfChangeV2,
        data: change(origin).write_to_bytes().unwrap().into(),
        ..Default::default()
    }
}
fn commit<F: FileSystem>(s: &DiskRaftStorage<F>, e: Entry) {
    let index = e.index;
    s.persist_ready(
        &[e],
        Some(&HardState {
            term: 2,
            vote: 1,
            commit: index,
            ..Default::default()
        }),
    )
    .unwrap();
}
fn path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "kv9-conf-origin-{}",
        kv9_common::StoreIncarnation::mint().unwrap()
    ))
}
#[test]
fn configuration_origins_preserve_replaced_stores_through_compaction_and_rewrite() {
    let dir = path();
    let s = DiskRaftStorage::open(&dir, &[1, 2, 3]).unwrap().0;
    let initial = ConfState::from((vec![1, 2, 3], vec![]));
    let learner = ConfState::from((vec![1, 2, 3], vec![4]));
    let first = origin();
    let removed = ConfigurationOrigin {
        action: Action::Detach,
        task: 105,
        ..first
    };
    let replacement = ConfigurationOrigin {
        store: kv9_common::StoreIncarnation::from_bytes([44; 16]),
        task: 106,
        ..first
    };
    for (index, o, cs) in [
        (1, first, &learner),
        (2, removed, &initial),
        (3, replacement, &learner),
    ] {
        commit(&s, entry(index, o));
        assert_eq!(
            s.configuration_origin_at(AppliedPosition { term: 2, index })
                .unwrap(),
            None,
            "commit without apply is not an observation"
        );
        s.set_conf_state(cs, index).unwrap();
    }
    commit(
        &s,
        Entry {
            index: 4,
            term: 2,
            ..Default::default()
        },
    );
    let expected: Vec<_> = (1..=3)
        .map(|index| {
            s.configuration_origin_at(AppliedPosition { term: 2, index })
                .unwrap()
                .unwrap()
        })
        .collect();
    assert_ne!(expected[0].origin.store, expected[2].origin.store);
    s.compact_retained_prefix(
        AppliedPosition { term: 2, index: 3 },
        &learner,
        b"committed truncation",
    )
    .unwrap();
    assert!(s.rewrite_log().unwrap());
    let bytes = s.log_file_bytes();
    drop(s);
    let s = DiskRaftStorage::recover(&dir).unwrap();
    assert_eq!(s.log_file_bytes(), bytes);
    assert_eq!(s.recovered_conf_index(), 3);
    for record in expected {
        assert_eq!(s.configuration_origin_at(record.at).unwrap(), Some(record));
    }
    assert!(s
        .configuration_origin_at(AppliedPosition { term: 3, index: 1 })
        .is_err());
    assert_eq!(
        s.configuration_origin_at(AppliedPosition { term: 2, index: 4 })
            .unwrap(),
        None
    );
    assert!(s.rewrite_log().unwrap());
    assert_eq!(s.log_file_bytes(), bytes);
    drop(s);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn configuration_origins_keep_local_snapshot_ancestors_without_certifying_imported_prefix() {
    let fs = ModelFs::default();
    let dir = Path::new("/conf-origin");
    let s = DiskRaftStorage::open_on(fs.clone(), dir, &[1, 2, 3])
        .unwrap()
        .0;
    commit(&s, entry(1, origin()));
    s.set_conf_state(&ConfState::from((vec![1, 2, 3], vec![4])), 1)
        .unwrap();
    let at = AppliedPosition { term: 2, index: 1 };
    let before = s.configuration_origin_at(at).unwrap().unwrap();
    let mut image = Snapshot {
        data: b"image".to_vec().into(),
        ..Default::default()
    };
    image.mut_metadata().term = 3;
    image.mut_metadata().index = 10;
    image
        .mut_metadata()
        .set_conf_state(ConfState::from((vec![1, 2, 3], vec![5])));
    s.install_protocol_snapshot(
        &image,
        &HardState {
            term: 3,
            vote: 1,
            commit: 10,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(s.configuration_origin_at(at).unwrap(), Some(before));
    drop(s);
    fs.crash(Crash::LoseUnsynced);
    let s = DiskRaftStorage::open_on(fs, dir, &[]).unwrap().0;
    assert_eq!(s.configuration_origin_at(at).unwrap(), Some(before));
    assert_eq!(
        s.configuration_origin_at(AppliedPosition { term: 3, index: 10 })
            .unwrap(),
        None
    );
    assert!(
        s.membership_history_page(None)
            .unwrap()
            .gaps
            .snapshot_prefix_unknown
    );
}
#[test]
fn configuration_origin_record_is_atomic_under_write_and_sync_failures() {
    let setup = || {
        let fs = ModelFs::default();
        let s = DiskRaftStorage::open_on(fs.clone(), Path::new("/conf-origin-crash"), &[1, 2, 3])
            .unwrap()
            .0;
        commit(&s, entry(1, origin()));
        fs.clear_events();
        (fs, s)
    };
    let cs = ConfState::from((vec![1, 2, 3], vec![4]));
    let (probe, s) = setup();
    s.set_conf_state(&cs, 1).unwrap();
    let events: Vec<_> = probe
        .events()
        .into_iter()
        .filter(|e| matches!(e.operation, Operation::Write | Operation::SyncData))
        .collect();
    assert!(!events.is_empty());
    for event in events {
        for fault in [
            Fault::Before(5),
            Fault::After(5),
            Fault::ShortWrite { bytes: 3, errno: 5 },
        ] {
            if event.operation != Operation::Write && matches!(fault, Fault::ShortWrite { .. }) {
                continue;
            }
            for crash in [Crash::LoseUnsynced, Crash::KeepUnsynced, Crash::Seeded(17)] {
                let (fs, s) = setup();
                fs.fail_at(event.number, fault);
                assert!(s.set_conf_state(&cs, 1).is_err());
                assert!(fs.fault_arrived());
                drop(s);
                fs.crash(crash);
                let s = DiskRaftStorage::open_on(fs, Path::new("/conf-origin-crash"), &[])
                    .unwrap()
                    .0;
                let record = s
                    .configuration_origin_at(AppliedPosition { term: 2, index: 1 })
                    .unwrap();
                assert_eq!(record.is_some(), s.recovered_conf_index() == 1);
                assert_eq!(
                    record.is_some(),
                    s.initial_state().unwrap().conf_state.learners.contains(&4)
                );
            }
        }
    }
}
#[test]
fn legacy_configuration_is_unknown_and_uncommitted_origin_cannot_be_recorded() {
    let fs = ModelFs::default();
    let s = DiskRaftStorage::open_on(fs, Path::new("/conf-origin-legacy"), &[1, 2, 3])
        .unwrap()
        .0;
    let cs = ConfState::from((vec![1, 2, 3], vec![4]));
    s.set_conf_state(&cs, 1).unwrap();
    assert_eq!(
        s.configuration_origin_at(AppliedPosition { term: 2, index: 1 })
            .unwrap(),
        None
    );
    s.append(&[entry(1, origin())]).unwrap();
    assert!(s.set_conf_state(&cs, 1).is_err());
    assert!(
        s.configuration_origin_at(AppliedPosition { term: 2, index: 1 })
            .is_err(),
        "failed writer must not supply observations"
    );
}
