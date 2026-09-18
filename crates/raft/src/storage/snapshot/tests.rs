use super::*;
use crate::rawnode::RaftPeer;
use kv9_common::fs::testing::{Crash, Fault, ModelFs, Operation};
use kv9_common::{AppliedPosition, NodeId, RegionId};
use raft::prelude::{Message, MessageType};

const DIR: &str = "/snapshot-protocol/raft";
fn opened(fs: &ModelFs) -> DiskRaftStorage<ModelFs> {
    DiskRaftStorage::open_on(fs.clone(), Path::new(DIR), &[1, 2, 3])
        .unwrap()
        .0
}
fn prepared(fs: &ModelFs) -> DiskRaftStorage<ModelFs> {
    let store = opened(fs);
    store
        .persist_ready(
            &[Entry {
                index: 1,
                term: 3,
                data: b"old".to_vec().into(),
                ..Default::default()
            }],
            Some(&HardState {
                term: 7,
                vote: 2,
                commit: 1,
                ..Default::default()
            }),
        )
        .unwrap();
    store
}
fn image() -> (Snapshot, HardState) {
    let mut image = Snapshot {
        data: b"exact-image-anchor".to_vec().into(),
        ..Default::default()
    };
    let meta = image.mut_metadata();
    meta.index = 12;
    meta.term = 5;
    let mut cs = ConfState::from((vec![1, 2, 4], vec![5]));
    cs.set_voters_outgoing(vec![1, 2, 3]);
    cs.set_learners_next(vec![3]);
    cs.auto_leave = true;
    meta.set_conf_state(cs);
    (
        image,
        HardState {
            term: 7,
            vote: 2,
            commit: 12,
            ..Default::default()
        },
    )
}
fn assert_state<F: FileSystem>(store: &DiskRaftStorage<F>, expected: &raft::RaftState) {
    let actual = store.initial_state().unwrap();
    assert_eq!(actual.hard_state, expected.hard_state);
    assert_eq!(actual.conf_state, expected.conf_state);
}
fn check_new<F: FileSystem>(store: &DiskRaftStorage<F>, image: &Snapshot, hs: &HardState) {
    assert_eq!(store.initial_state().unwrap().hard_state, *hs);
    assert_eq!(
        store.initial_state().unwrap().conf_state,
        *image.get_metadata().get_conf_state()
    );
    assert_eq!(store.first_index().unwrap(), 13);
    assert_eq!(store.last_index().unwrap(), 12);
    assert_eq!(store.committed_term(12).unwrap(), 5);
    assert!(store.committed_term(1).is_err());
    assert_eq!(store.recovered_conf_index(), 12);
    assert_eq!(store.snapshot(0, 4).unwrap(), *image);
    assert_eq!(store.snapshot(12, 4).unwrap(), *image);
    assert!(matches!(
        store.snapshot(13, 4),
        Err(raft::Error::Store(
            raft::StorageError::SnapshotTemporarilyUnavailable
        ))
    ));
    // The configuration AT the durable base (installed snapshot metadata,
    // or a REC_COMPACTION record) is defensibly known and now resolves —
    // this is what lets a second compaction floor, and image capture at a
    // snapshot base, proceed. A cut BELOW the base is still gone.
    match store
        .configuration_at_committed(AppliedPosition { term: 5, index: 12 })
        .unwrap()
    {
        ConfigurationLookup::Found(found) => {
            assert_eq!(found.state(), image.get_metadata().get_conf_state());
            assert_eq!(found.cut(), AppliedPosition { term: 5, index: 12 });
        }
        other => panic!("base configuration did not resolve: {other:?}"),
    }
    assert!(
        store
            .configuration_at_committed(AppliedPosition { term: 5, index: 11 })
            .is_err(),
        "a cut below the durable base must not resolve"
    );
}

#[test]
fn snapshot_pair_survives_recovery_without_synthesizing_later_images() {
    let fs = ModelFs::default();
    let store = prepared(&fs);
    assert!(
        store.snapshot(0, 4).is_err(),
        "invented an empty image from HardState"
    );
    let (image, hs) = image();
    fs.clear_events();
    store.install_protocol_snapshot(&image, &hs).unwrap();
    assert_eq!(
        fs.events().iter().map(|e| e.operation).collect::<Vec<_>>(),
        [Operation::Write, Operation::SyncData]
    );
    check_new(&store, &image, &hs);
    fs.clear_events();
    store.install_protocol_snapshot(&image, &hs).unwrap();
    assert!(
        fs.events().is_empty(),
        "identical retry wrote a duplicate record"
    );
    drop(store);
    fs.crash(Crash::LoseUnsynced);
    let store = opened(&fs);
    check_new(&store, &image, &hs);
    let next = HardState {
        term: 8,
        vote: 4,
        commit: 13,
        ..Default::default()
    };
    store
        .persist_ready(
            &[Entry {
                index: 13,
                term: 8,
                data: b"tail".to_vec().into(),
                ..Default::default()
            }],
            Some(&next),
        )
        .unwrap();
    assert_eq!(
        store.snapshot(0, 4).unwrap(),
        image,
        "relabeled the old image as new commit"
    );
    assert!(store.snapshot(13, 4).is_err());
    assert!(store.install_protocol_snapshot(&image, &hs).is_err());
    drop(store);
    fs.crash(Crash::LoseUnsynced);
    let store = opened(&fs);
    assert_eq!(store.initial_state().unwrap().hard_state, next);
    assert_eq!(store.committed_term(13).unwrap(), 8);
    assert_eq!(store.snapshot(0, 4).unwrap(), image);
    assert!(
        RaftPeer::with_storage(NodeId(2), RegionId(100), store).is_err(),
        "protocol image bypassed engine installation"
    );
}

#[test]
fn every_snapshot_io_cut_recovers_one_complete_protocol_state() {
    let fs = ModelFs::default();
    let store = prepared(&fs);
    let (image, hs) = image();
    fs.clear_events();
    store.install_protocol_snapshot(&image, &hs).unwrap();
    let cuts = fs.events();
    let mut cases = 0;
    for cut in cuts {
        for errno in [5, 28] {
            let mut faults = vec![Fault::Before(errno), Fault::After(errno)];
            if cut.operation == Operation::Write {
                faults.push(Fault::ShortWrite { bytes: 17, errno });
            }
            for fault in faults {
                for crash in [Crash::LoseUnsynced, Crash::KeepUnsynced]
                    .into_iter()
                    .chain((0..16).map(Crash::Seeded))
                {
                    let fs = ModelFs::default();
                    let store = prepared(&fs);
                    let old = store.initial_state().unwrap();
                    fs.clear_events();
                    fs.fail_at(cut.number, fault);
                    assert!(store.install_protocol_snapshot(&image, &hs).is_err());
                    assert!(fs.fault_arrived());
                    assert_state(&store, &old);
                    assert!(store.snapshot(0, 4).is_err());
                    let events = fs.events().len();
                    assert!(store.install_protocol_snapshot(&image, &hs).is_err());
                    assert!(store.set_hardstate(&hs).is_err());
                    assert_eq!(fs.events().len(), events, "failed writer continued");
                    drop(store);
                    fs.crash(crash);
                    let recovered = opened(&fs);
                    match recovered.first_index().unwrap() {
                        1 => {
                            assert_state(&recovered, &old);
                            assert!(recovered.snapshot(0, 4).is_err());
                        }
                        13 => check_new(&recovered, &image, &hs),
                        _ => panic!("mixed snapshot prefix"),
                    }
                    if cut.operation == Operation::SyncData && matches!(fault, Fault::After(_)) {
                        check_new(&recovered, &image, &hs);
                    }
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 180);
}

#[test]
fn invalid_snapshot_and_vote_rollback_refuse_before_any_io() {
    let (valid, hs) = image();
    let mut invalids = Vec::new();
    for n in 0..12 {
        let mut s = valid.clone();
        let mut h = hs.clone();
        match n {
            0 => h.term = 6,
            1 => h.vote = 3,
            2 => h.vote = 0,
            3 => h.commit = 13,
            4 => s.mut_metadata().term = 8,
            5 => s.mut_metadata().index = 1,
            6 => s.data.clear(),
            7 => s.mut_metadata().mut_conf_state().voters.clear(),
            8 => s.mut_metadata().mut_conf_state().voters.push(1),
            9 => s.mut_metadata().mut_conf_state().learners.push(2),
            10 => s.mut_metadata().mut_conf_state().learners_next.push(99),
            11 => s.mut_unknown_fields().add_varint(99, 1),
            _ => unreachable!(),
        }
        invalids.push((s, h));
    }
    for (s, h) in invalids {
        let fs = ModelFs::default();
        let store = prepared(&fs);
        let old = store.initial_state().unwrap();
        fs.clear_events();
        assert!(store.install_protocol_snapshot(&s, &h).is_err());
        assert!(fs.events().is_empty());
        assert_state(&store, &old);
        store.install_protocol_snapshot(&valid, &hs).unwrap();
    }
}

#[test]
fn bounded_record_codec_rejects_every_truncation_and_trailing_data() {
    let (image, hs) = image();
    let encoded = encode(&image, &hs).unwrap();
    assert_eq!(decode(&encoded).unwrap(), (image.clone(), hs.clone()));
    for end in 0..encoded.len() {
        assert!(
            decode(&encoded[..end]).is_err(),
            "accepted truncation {end}"
        );
    }
    let mut extra = encoded.clone();
    extra.push(0);
    assert!(decode(&extra).is_err());
    let mut huge = encoded.clone();
    huge[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(decode(&huge).is_err());
    let mut oversized = image;
    oversized.data = vec![1; MAX_PROTOCOL_SNAPSHOT_BYTES].into();
    assert!(encode(&oversized, &hs).is_err());
}

#[test]
fn receiver_drops_snapshot_before_rawnode_can_change_commit_or_membership() {
    let fs = ModelFs::default();
    let store = prepared(&fs);
    let peer = RaftPeer::with_storage(NodeId(2), RegionId(100), store).unwrap();
    let before = peer.status_snapshot();
    let (image, _) = image();
    let mut msg = Message {
        from: 1,
        to: 2,
        term: 8,
        msg_type: MessageType::MsgSnapshot,
        ..Default::default()
    };
    msg.set_snapshot(image);
    peer.step_message(msg);
    let after = peer.status_snapshot();
    assert_eq!(
        after.term, before.term,
        "uninstalled image advanced local term"
    );
    assert_eq!(
        after.committed, before.committed,
        "uninstalled image advanced commit"
    );
    assert_eq!(
        after.voters, before.voters,
        "uninstalled image changed membership"
    );
    assert_eq!(after.step_errors, before.step_errors + 1);
    assert!(
        peer.pump().unwrap().is_empty(),
        "snapshot was acknowledged without an engine"
    );
}

#[test]
fn real_file_snapshot_reopens_and_refuses_uncoordinated_peer_start() {
    let base = std::env::var_os("KV9_TEST_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join(format!("kv9-protocol-snapshot-{}", std::process::id()));
    assert!(!dir.exists());
    let (store, _) = DiskRaftStorage::open(&dir, &[1, 2, 3]).unwrap();
    let (image, hs) = image();
    store.install_protocol_snapshot(&image, &hs).unwrap();
    drop(store);
    let recovered = DiskRaftStorage::recover(&dir).unwrap();
    check_new(&recovered, &image, &hs);
    assert!(RaftPeer::with_storage(NodeId(2), RegionId(100), recovered).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn every_torn_frame_boundary_selects_old_state_and_complete_frame_selects_new() {
    let fs = ModelFs::default();
    let store = prepared(&fs);
    let path = Path::new(DIR).join("raft.log");
    let old_length = fs.visible_bytes(&path).unwrap().len();
    let (image, hs) = image();
    store.install_protocol_snapshot(&image, &hs).unwrap();
    let frame_length = fs.visible_bytes(&path).unwrap().len() - old_length;
    drop(store);
    for retained in 0..=frame_length {
        let fs = ModelFs::default();
        let store = prepared(&fs);
        let old = store.initial_state().unwrap();
        store.install_protocol_snapshot(&image, &hs).unwrap();
        drop(store);
        let file = fs.open_existing_append(&path).unwrap();
        file.set_len((old_length + retained) as u64).unwrap();
        file.sync_data().unwrap();
        drop(file);
        fs.crash(Crash::LoseUnsynced);
        let store = opened(&fs);
        if retained < frame_length {
            assert_state(&store, &old);
            assert_eq!(store.first_index().unwrap(), 1);
            assert_eq!(fs.visible_bytes(&path).unwrap().len(), old_length);
        } else {
            check_new(&store, &image, &hs);
        }
    }
}

#[test]
fn checksum_valid_invalid_record_refuses_without_repairing_away_evidence() {
    for invalid in [b"unknown-envelope".to_vec(), {
        let (s, h) = image();
        let mut b = encode(&s, &h).unwrap();
        b[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        b
    }] {
        let fs = ModelFs::default();
        let store = prepared(&fs);
        store
            .with_writer(|f| {
                DiskRaftStorage::<ModelFs>::write_record(
                    &store.io_metrics,
                    f,
                    REC_SNAPSHOT,
                    &invalid,
                )
            })
            .unwrap();
        drop(store);
        fs.crash(Crash::LoseUnsynced);
        let path = Path::new(DIR).join("raft.log");
        let before = fs.visible_bytes(&path).unwrap();
        assert!(DiskRaftStorage::open_on(fs.clone(), Path::new(DIR), &[1, 2, 3]).is_err());
        assert_eq!(fs.visible_bytes(&path).unwrap(), before);
    }
}

#[test]
fn snapshot_and_fixed_lease_policies_cannot_be_combined_in_either_order() {
    let policy = crate::lease_policy::LeasePolicy {
        node: 2,
        group: 100,
        configuration: 7,
        voters: vec![1, 2, 3],
        promise_ns: 100,
        drift_ppb: 100_000,
        margin_ns: 1,
    };
    let (image, hs) = image();
    let fs = ModelFs::default();
    let store = prepared(&fs);
    store.begin_lease_incarnation(&policy).unwrap();
    fs.clear_events();
    assert!(store.install_protocol_snapshot(&image, &hs).is_err());
    assert!(fs.events().is_empty());
    let fs = ModelFs::default();
    let store = prepared(&fs);
    store.install_protocol_snapshot(&image, &hs).unwrap();
    fs.clear_events();
    assert!(store.begin_lease_incarnation(&policy).is_err());
    assert!(fs.events().is_empty());
}

/// Runtime adoption seam: the ONE constructor allowed past the
/// coordinated-install refusal re-checks the exact durable base shape and
/// admits nothing else — no zero cut, no foreign term, no shifted index.
#[test]
fn installed_base_constructor_admits_only_the_exact_verified_cut() {
    let fs = ModelFs::default();
    let store = prepared(&fs);
    let (image, hs) = image();
    store.install_protocol_snapshot(&image, &hs).unwrap();
    drop(store);
    assert!(
        RaftPeer::with_storage(NodeId(4), RegionId(100), opened(&fs)).is_err(),
        "snapshot-backed store bypassed the coordinated-install refusal"
    );
    for wrong in [
        AppliedPosition { term: 0, index: 12 },
        AppliedPosition { term: 5, index: 0 },
        AppliedPosition { term: 4, index: 12 },
        AppliedPosition { term: 5, index: 11 },
        AppliedPosition { term: 5, index: 13 },
    ] {
        assert!(
            RaftPeer::with_installed_storage(NodeId(4), RegionId(100), opened(&fs), wrong).is_err(),
            "admitted a base the storage does not carry: {wrong:?}"
        );
    }
    let peer = RaftPeer::with_installed_storage(
        NodeId(4),
        RegionId(100),
        opened(&fs),
        AppliedPosition { term: 5, index: 12 },
    )
    .unwrap();
    // The peer starts as exactly what the image configuration says.
    let (voters, learners) = peer.membership();
    assert_eq!(voters, vec![1, 2, 4]);
    assert_eq!(learners, vec![5]);
}
