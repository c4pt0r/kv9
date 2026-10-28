use super::*;
use kv9_engine::checkpoint::FlushJournal;
use std::time::Duration;

#[test]
fn pending_recovery_refuses_missing_access_foreign_corrupt_or_compacted_plan() {
    for fault in 0..4 {
        let f = Fixture::new();
        f.put(b"key", b"candidate".to_vec(), Some(60));
        let mut candidate_scope = scope();
        if fault == 1 {
            candidate_scope.region += 1;
        }
        let (frozen, mut candidate) = f
            .engine
            .freeze_checkpoint_candidate(|_| Ok(candidate_scope))
            .unwrap();
        let plan = RemoteUploader::plan(frozen).unwrap();
        let journal_path = f.path.join("data.pending");
        let mut journal = FlushJournal::new(&journal_path);
        candidate.stage(&mut journal, &plan, 0).unwrap();
        let tail = f.put(b"key", b"tail".to_vec(), Some(90));
        let path = f.path.clone();
        let objects = f.objects.clone();
        drop(candidate);
        f.driver.stop();
        drop(f);
        if fault == 2 {
            std::fs::write(&journal_path, b"truncated pending descriptor").unwrap();
        }
        let before = std::fs::read(&journal_path).unwrap();
        let mut storage = DiskRaftStorage::recover(&path.join("raft")).unwrap();
        if fault == 3 {
            // Deliberately damage stopped files by bypassing the protected peer.
            let crate::storage::ConfigurationLookup::Found(conf) =
                storage.configuration_at_committed(tail).unwrap()
            else {
                panic!("actual tail configuration");
            };
            storage
                .compact_retained_prefix(tail, conf.state(), b"test-only evidence removal")
                .unwrap();
        }
        let uploader = RemoteUploader::for_harness(objects);
        assert!(
            open(
                &mut storage,
                path.join("data.wal"),
                (fault != 0).then_some(&uploader),
                identity(),
            )
            .is_err(),
            "fault {fault} must refuse before constructing a voting driver"
        );
        assert_eq!(std::fs::read(&journal_path).unwrap(), before);
        drop(storage);
        std::fs::remove_dir_all(path).unwrap();
    }
}

fn publish_pending(f: &Fixture, journal: &mut FlushJournal) -> CheckpointManifest {
    let pending = journal.load().unwrap().unwrap();
    let manifest = pending.manifest().clone();
    let (prepared, generation) = pending.recover(&f.uploader).unwrap();
    journal.stage(&prepared, generation).unwrap();
    let command = Command::ManifestChange(
        ManifestChangePayload::from_prepared(prepared, generation).unwrap(),
    );
    let at = f.driver.propose(&command).unwrap();
    pump(&f.driver);
    assert!(matches!(
        f.driver.wait_applied(at, Duration::from_secs(1)).unwrap(),
        crate::driver::ApplyWaitOutcome::Manifest {
            verdict: crate::ManifestVerdict::Applied { .. },
            ..
        }
    ));
    manifest
}
#[test]
fn candidate_blocks_both_compaction_paths_before_selection_and_survives_executor_drop() {
    let f = Fixture::new();
    f.put(b"key", b"candidate".to_vec(), Some(60));
    let (frozen, mut candidate) = f
        .engine
        .freeze_checkpoint_candidate(|_| Ok(scope()))
        .unwrap();
    let cut = frozen.position();
    let plan = RemoteUploader::plan(frozen).unwrap();
    let mut journal = FlushJournal::new(f.path.join("data.pending"));
    candidate.stage(&mut journal, &plan, 0).unwrap();
    let tail = f.put(b"key", b"renewed".to_vec(), Some(90));
    for leader in [true, false] {
        let result = if leader {
            f.driver
                .peer()
                .truncate_retained_log(tail, b"actual test authority")
        } else {
            f.driver
                .peer()
                .compact_confirmed_prefix(tail, b"actual test authority")
        };
        assert!(result.is_err());
    }
    assert_eq!(
        f.engine
            .checkpoint_history()
            .unwrap()
            .compaction_ceiling()
            .unwrap(),
        Some(cut)
    );
    assert!(
        f.engine
            .claim_checkpoint_candidate(&journal.load().unwrap().unwrap())
            .is_err(),
        "single execution owner"
    );
    drop(candidate);
    assert!(
        f.driver
            .peer()
            .compact_confirmed_prefix(tail, b"test authority")
            .is_err(),
        "dropping an executor does not discard a durable plan"
    );
    assert!(f
        .engine
        .freeze_checkpoint_candidate(|_| Ok(scope()))
        .is_err());
    let mut candidate = f
        .engine
        .claim_checkpoint_candidate(&journal.load().unwrap().unwrap())
        .unwrap();
    let manifest = publish_pending(&f, &mut journal);
    f.engine.checkpoint_applied(&manifest).unwrap();
    candidate.finish(&mut journal).unwrap();
    assert!(journal.load().unwrap().is_none());
    assert!(
        f.driver
            .peer()
            .compact_confirmed_prefix(tail, b"test authority")
            .is_err(),
        "the selected image still protects its cut"
    );
    f.driver
        .peer()
        .truncate_retained_log(cut, b"test authority")
        .unwrap();
    let path = f.path.clone();
    drop(candidate);
    f.driver.stop();
    drop(f);
    std::fs::remove_dir_all(path).unwrap();
}
#[test]
fn prepublication_stage_failure_cancels_only_after_durable_absence_and_finish_failure_keeps_pin() {
    let f = Fixture::new();
    f.put(b"key", b"v".to_vec(), Some(60));
    let (frozen, mut candidate) = f
        .engine
        .freeze_checkpoint_candidate(|_| Ok(scope()))
        .unwrap();
    let plan = RemoteUploader::plan(frozen).unwrap();
    let path = f.path.join("data.pending");
    let mut journal = FlushJournal::new(&path);
    std::fs::create_dir(path.with_extension("pending.tmp")).unwrap();
    assert!(candidate.stage(&mut journal, &plan, 0).is_err());
    assert!(!candidate.is_durable());
    assert_eq!(
        f.engine
            .checkpoint_history()
            .unwrap()
            .compaction_ceiling()
            .unwrap(),
        None
    );
    drop(candidate);
    std::fs::remove_dir(path.with_extension("pending.tmp")).unwrap();
    let (frozen, mut candidate) = f
        .engine
        .freeze_checkpoint_candidate(|_| Ok(scope()))
        .unwrap();
    let plan = RemoteUploader::plan(frozen).unwrap();
    candidate.stage(&mut journal, &plan, 0).unwrap();
    let manifest = publish_pending(&f, &mut journal);
    // A later selected image would normally permit more compaction, but the
    // older unresolved local journal must remain restrictive until deletion fsync.
    let (newer, _) = f.publish();
    f.engine.checkpoint_applied(&newer).unwrap();
    std::fs::rename(&path, path.with_extension("backup")).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(candidate.finish(&mut journal).is_err());
    assert_eq!(
        f.engine
            .checkpoint_history()
            .unwrap()
            .compaction_ceiling()
            .unwrap(),
        Some(manifest.position())
    );
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(path.with_extension("backup"), &path).unwrap();
    candidate.finish(&mut journal).unwrap();
    assert_eq!(
        f.engine
            .checkpoint_history()
            .unwrap()
            .compaction_ceiling()
            .unwrap(),
        Some(newer.position())
    );
    let directory = f.path.clone();
    drop(candidate);
    f.driver.stop();
    drop(f);
    std::fs::remove_dir_all(directory).unwrap();
}
#[test]
fn frozen_scope_callback_does_not_hold_history_or_wal_and_cut_is_already_reserved() {
    let f = Fixture::new();
    let cut = f.put(b"key", b"old".to_vec(), Some(60));
    let (entered, receive) = std::sync::mpsc::channel();
    let (resume, pause) = std::sync::mpsc::channel();
    let engine = f.engine.clone();
    let freezer = std::thread::spawn(move || {
        engine
            .freeze_checkpoint_candidate(|_| {
                entered.send(()).unwrap();
                pause.recv_timeout(Duration::from_secs(5)).unwrap();
                Ok(scope())
            })
            .unwrap()
    });
    receive.recv_timeout(Duration::from_secs(5)).unwrap();
    let tail = f.put(b"key", b"new".to_vec(), Some(90));
    // These calls acquire peer/history while validation remains paused. Their
    // completion proves the callback does not hold either history or WAL lock.
    assert_eq!(
        f.engine
            .checkpoint_history()
            .unwrap()
            .compaction_ceiling()
            .unwrap(),
        Some(cut)
    );
    assert!(f
        .driver
        .peer()
        .truncate_retained_log(tail, b"test authority")
        .is_err());
    resume.send(()).unwrap();
    let (frozen, candidate) = freezer.join().unwrap();
    assert_eq!(frozen.position(), cut);
    drop(candidate); // No durable plan or I/O ever occurred.
    f.driver
        .peer()
        .truncate_retained_log(tail, b"test authority")
        .unwrap();
    let path = f.path.clone();
    f.driver.stop();
    drop(f);
    std::fs::remove_dir_all(path).unwrap();
}
#[test]
fn pending_journal_rebuilds_reservation_before_recovered_driver_can_compact() {
    let f = Fixture::new();
    let cut = f.put(b"key", b"image".to_vec(), Some(60));
    let (frozen, mut candidate) = f
        .engine
        .freeze_checkpoint_candidate(|_| Ok(scope()))
        .unwrap();
    let plan = RemoteUploader::plan(frozen).unwrap();
    let mut journal = FlushJournal::new(f.path.join("data.pending"));
    candidate.stage(&mut journal, &plan, 0).unwrap();
    let tail = f.put(b"key", b"tail".to_vec(), Some(90));
    let path = f.path.clone();
    let objects = f.objects.clone();
    drop(candidate);
    f.driver.stop();
    drop(f);
    let mut storage = DiskRaftStorage::recover(&path.join("raft")).unwrap();
    let uploader = RemoteUploader::for_harness(objects);
    let (engine, _, _) = open(
        &mut storage,
        path.join("data.wal"),
        Some(&uploader),
        identity(),
    )
    .unwrap();
    assert_eq!(
        engine
            .checkpoint_history()
            .unwrap()
            .compaction_ceiling()
            .unwrap(),
        Some(cut)
    );
    let engine = Arc::new(engine);
    let peer = Arc::new(RaftPeer::with_storage(NodeId(1), range().region, storage).unwrap());
    let mut sm = MemStateMachine::with_engine(engine.clone()).unwrap();
    sm.set_data_group(range().root, range().region, range().creation)
        .unwrap();
    let driver = NodeDriver::new(
        peer,
        Arc::new(InProcHub::new().endpoint(NodeId(1))) as Arc<dyn RaftTransport>,
        sm,
    )
    .unwrap();
    driver.peer().campaign().unwrap();
    pump(&driver);
    assert!(driver
        .peer()
        .truncate_retained_log(tail, b"test authority")
        .is_err());
    assert!(driver
        .peer()
        .compact_confirmed_prefix(tail, b"test authority")
        .is_err());
    let resumed = engine
        .claim_checkpoint_candidate(&journal.load().unwrap().unwrap())
        .unwrap();
    assert_eq!(resumed.position(), cut);
    assert_eq!(
        engine
            .get(ColumnFamily::Default, &physical(b"key"))
            .unwrap(),
        Some(b"tail".to_vec())
    );
    driver.stop();
    drop(resumed);
    drop(driver);
    drop(engine);
    std::fs::remove_dir_all(path).unwrap();
}
