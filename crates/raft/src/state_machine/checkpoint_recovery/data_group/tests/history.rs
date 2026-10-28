use super::*;

#[test]
fn failed_checkpoint_publication_or_unlink_fences_compaction_until_actual_recovery() {
    for after_publication in [false, true] {
        let f = Fixture::new();
        for byte in 0..18 {
            f.put(b"bulk", vec![byte; 1024 * 1024], Some(60));
        }
        f.put(b"bulk", b"survivor".to_vec(), Some(60));
        let (manifest, publication) = f.publish();
        let blocker = if after_publication {
            let stream = std::fs::read_dir(f.path.join("data.segments"))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            let mut segments: Vec<_> = std::fs::read_dir(stream)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| path.extension().is_some_and(|s| s == "wal"))
                .collect();
            segments.sort();
            let first = segments.remove(0);
            std::fs::rename(&first, first.with_extension("backup")).unwrap();
            first
        } else {
            f.path.join("data.topology.tmp")
        };
        std::fs::create_dir(&blocker).unwrap();
        assert!(f.engine.checkpoint_applied(&manifest).is_err());
        assert_eq!(
            WalEngine::checkpoint_reference(f.path.join("data.wal"))
                .unwrap()
                .is_some(),
            after_publication
        );
        let before = f.driver.peer().status_snapshot().log_first_index;
        assert!(f
            .driver
            .peer()
            .compact_confirmed_prefix(publication, b"committed test confirmation")
            .is_err());
        assert_eq!(f.driver.peer().status_snapshot().log_first_index, before);
        assert!(f.engine.checkpoint_applied(&manifest).is_err());
        std::fs::remove_dir(&blocker).unwrap();
        let path = f.path.clone();
        let objects = f.objects.clone();
        f.driver.stop();
        drop(f);
        let mut storage = DiskRaftStorage::recover(&path.join("raft")).unwrap();
        let uploader = RemoteUploader::for_harness(objects);
        let (engine, _, recovered) = open(
            &mut storage,
            path.join("data.wal"),
            Some(&uploader),
            identity(),
        )
        .unwrap();
        assert_eq!(recovered.is_some(), after_publication);
        assert_eq!(
            engine
                .get(ColumnFamily::Default, &physical(b"bulk"))
                .unwrap(),
            Some(b"survivor".to_vec())
        );
        assert!(engine
            .get(ColumnFamily::Write, &physical(b"bulk"))
            .unwrap()
            .is_some());
        // Recovery is what resolves the ambiguous old selection, after which
        // re-adopting this actually published image is legal again.
        assert!(engine.checkpoint_applied(&manifest).unwrap());
        drop(engine);
        drop(storage);
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[test]
fn checkpoint_restores_configuration_changes_in_the_retained_suffix() {
    let f = Fixture::new();
    let older_cut = f.put(b"key", b"before configuration".to_vec(), Some(60));
    let conf = f.driver.add_learner(NodeId(2)).unwrap();
    pump(&f.driver);
    f.driver
        .wait_conf_applied(conf, std::time::Duration::from_secs(1))
        .unwrap();
    f.put(b"key", b"after configuration".to_vec(), Some(60));
    let (manifest, _) = f.publish();
    f.engine.checkpoint_applied(&manifest).unwrap();
    // Current membership deliberately differs from the image's membership.
    // Remove the fixture learner before the real all-matched truncation gate.
    let removed = f.driver.remove_voter(NodeId(2)).unwrap();
    pump(&f.driver);
    f.driver
        .wait_conf_applied(removed, std::time::Duration::from_secs(1))
        .unwrap();
    f.driver
        .peer()
        .truncate_retained_log(older_cut, b"committed test decision")
        .unwrap();
    f.driver.peer().reclaim_log().unwrap();
    let path = f.path.clone();
    let objects = f.objects.clone();
    f.driver.stop();
    drop(f);
    let mut storage = DiskRaftStorage::recover(&path.join("raft")).unwrap();
    let uploader = RemoteUploader::for_harness(objects);
    let (engine, _, recovered) = open(
        &mut storage,
        path.join("data.wal"),
        Some(&uploader),
        identity(),
    )
    .unwrap();
    let recovered = recovered.unwrap();
    let configuration = recovered.publication().configuration();
    assert_eq!(configuration.state().get_voters(), &[1]);
    assert_eq!(configuration.state().get_learners(), &[2]);
    assert_eq!(
        configuration.applied_at(),
        Some(AppliedPosition {
            term: conf.term,
            index: conf.index.0
        })
    );
    assert_eq!(
        engine
            .get(ColumnFamily::Default, &physical(b"key"))
            .unwrap(),
        Some(b"after configuration".to_vec())
    );
    drop(engine);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn both_live_compaction_paths_preserve_checkpoint_evidence_and_advance_with_new_images() {
    let f = Fixture::new();
    for byte in 0..18 {
        f.put(b"bulk", vec![byte; 1024 * 1024], Some(60));
    }
    let written = f.put(b"bulk", b"last".to_vec(), Some(60));
    let (first, publication) = f.publish();
    f.engine.checkpoint_applied(&first).unwrap();
    let tail = f.put(b"tail", b"first tail".to_vec(), None);
    let peer = f.driver.peer();
    let retained = peer.status_snapshot().log_first_index;
    assert!(peer
        .truncate_retained_log(tail, b"committed test decision")
        .is_err());
    assert!(peer
        .compact_confirmed_prefix(publication, b"committed test confirmation")
        .is_err());
    assert_eq!(peer.status_snapshot().log_first_index, retained);

    // At the cut, configuration and term remain in the durable base and the
    // exact later publication remains in the suffix. Reclaim actual bytes.
    let before = peer.log_file_bytes();
    assert_eq!(
        peer.truncate_retained_log(first.position(), b"committed test decision")
            .unwrap(),
        first.index + 1
    );
    assert!(peer.reclaim_log().unwrap());
    let after = peer.log_file_bytes();
    assert!(before > 16 * 1024 * 1024 && after < before / 2);
    println!("checkpoint_history raft_bytes_before={before} raft_bytes_after={after}");

    // A later selected checkpoint raises the restriction, so the protection
    // does not permanently freeze compaction at the first image.
    let (second, second_publication) = f.publish();
    f.engine.checkpoint_applied(&second).unwrap();
    assert_eq!(
        peer.compact_confirmed_prefix(second.position(), b"committed test confirmation")
            .unwrap(),
        second.index + 1
    );
    assert!(peer.reclaim_log().unwrap());
    let last = f.put(b"tail", b"newest".to_vec(), None);
    let path = f.path.clone();
    let objects = f.objects.clone();
    f.driver.stop();
    drop(f);
    let mut storage = DiskRaftStorage::recover(&path.join("raft")).unwrap();
    let base = storage.compacted_base().unwrap().unwrap();
    assert_eq!(base, second.position());
    let uploader = RemoteUploader::for_harness(objects);
    let (engine, _, recovered) = open(
        &mut storage,
        path.join("data.wal"),
        Some(&uploader),
        identity(),
    )
    .unwrap();
    assert_eq!(
        recovered.unwrap().publication().publication(),
        second_publication
    );
    assert_eq!(
        engine.get(ColumnFamily::Write, &physical(b"bulk")).unwrap(),
        Some(
            ExpiryRecord::new(range().creation, written, 61010)
                .unwrap()
                .encode()
                .to_vec()
        )
    );
    assert_eq!(
        engine
            .get(ColumnFamily::Default, &physical(b"tail"))
            .unwrap(),
        Some(b"newest".to_vec())
    );
    // New driver construction automatically restores the selected-image
    // restriction; no runtime caller can forget to register a separate pin.
    let engine = Arc::new(engine);
    let mut sm = MemStateMachine::with_engine(engine.clone()).unwrap();
    sm.set_data_group(range().root, range().region, range().creation)
        .unwrap();
    let peer = Arc::new(
        RaftPeer::with_installed_storage(NodeId(1), range().region, storage, base).unwrap(),
    );
    let hub = InProcHub::new();
    let driver =
        NodeDriver::with_installed_base(peer.clone(), Arc::new(hub.endpoint(NodeId(1))), sm, base)
            .unwrap();
    peer.campaign().unwrap();
    pump(&driver);
    assert!(peer
        .compact_confirmed_prefix(last, b"committed test confirmation")
        .is_err());
    assert!(engine.checkpoint_applied(&first).is_err());
    assert_eq!(peer.status_snapshot().log_first_index, base.index + 1);
    driver.stop();
    drop(driver);
    drop(peer);
    drop(engine);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn compaction_winning_before_first_selection_refuses_that_image_and_allows_a_fresh_one() {
    let f = Fixture::new();
    f.put(b"key", b"preserved in full WAL".to_vec(), Some(60));
    let (first, publication) = f.publish();
    f.driver
        .peer()
        .compact_confirmed_prefix(publication, b"committed test confirmation")
        .unwrap();
    assert!(f.engine.checkpoint_applied(&first).is_err());
    assert!(WalEngine::checkpoint_reference(f.path.join("data.wal"))
        .unwrap()
        .is_none());
    f.put(b"key", b"fresh image".to_vec(), Some(60));
    let (second, _) = f.publish();
    assert!(f.engine.checkpoint_applied(&second).unwrap());
    f.driver
        .peer()
        .compact_confirmed_prefix(second.position(), b"committed test confirmation")
        .unwrap();
    f.driver.peer().reclaim_log().unwrap();
    let path = f.path.clone();
    let objects = f.objects.clone();
    f.driver.stop();
    drop(f);
    let mut storage = DiskRaftStorage::recover(&path.join("raft")).unwrap();
    let uploader = RemoteUploader::for_harness(objects);
    let (engine, _, recovered) = open(
        &mut storage,
        path.join("data.wal"),
        Some(&uploader),
        identity(),
    )
    .unwrap();
    assert_eq!(recovered.unwrap().publication().manifest(), &second);
    assert_eq!(
        engine
            .get(ColumnFamily::Default, &physical(b"key"))
            .unwrap(),
        Some(b"fresh image".to_vec())
    );
    drop(engine);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn actual_checkpoint_selection_racing_compaction_never_loses_both_recovery_sources() {
    for _ in 0..12 {
        let f = Fixture::new();
        f.put(b"key", b"durable".to_vec(), Some(60));
        let (manifest, publication) = f.publish();
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let (sender, receiver) = std::sync::mpsc::channel();
        let select = {
            let barrier = barrier.clone();
            let engine = f.engine.clone();
            let sender = sender.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let selected = engine.checkpoint_applied(&manifest).is_ok_and(|v| v);
                sender.send(selected).unwrap();
            })
        };
        let compact = {
            let barrier = barrier.clone();
            let peer = f.driver.peer().clone();
            std::thread::spawn(move || {
                barrier.wait();
                let compacted = peer
                    .compact_confirmed_prefix(publication, b"committed test confirmation")
                    .is_ok();
                sender.send(compacted).unwrap();
            })
        };
        barrier.wait();
        let timeout = std::time::Duration::from_secs(10);
        let a = receiver
            .recv_timeout(timeout)
            .expect("selection/compaction deadlocked");
        let b = receiver
            .recv_timeout(timeout)
            .expect("selection/compaction deadlocked");
        assert_ne!(a, b, "exactly one operation must succeed");
        select.join().unwrap();
        compact.join().unwrap();
        f.driver.peer().reclaim_log().unwrap();
        let path = f.path.clone();
        let objects = f.objects.clone();
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
                .get(ColumnFamily::Default, &physical(b"key"))
                .unwrap(),
            Some(b"durable".to_vec())
        );
        assert!(engine
            .get(ColumnFamily::Write, &physical(b"key"))
            .unwrap()
            .is_some());
        drop(engine);
        drop(storage);
        std::fs::remove_dir_all(path).unwrap();
    }
}
