use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn adoption_retries_catalog_contention_before_advancing_data_floor() {
    let f = Fixture::new().await;
    let (root, old, objects) = readers::published(&f).await;
    let uploader = Arc::new(RemoteUploader::for_harness(objects.clone()));
    let jobs: Vec<_> = (0..3)
        .map(|i| readers::task(&f, i, uploader.clone()))
        .collect();
    selected_floor(&f, &jobs, 1).await;
    let (next, command) = prepare_next_image(&f, &root, objects.clone()).await;
    assert_eq!(
        owner_phases(&f, &root, &next, 1),
        (Some(PinPhase::Published), None)
    );
    let gets = objects.gets.load(Ordering::SeqCst);
    let failures: Vec<_> = jobs.iter().map(|j| j.snapshot().failures).collect();
    let busy = f.hooks[1].retention_catalog_busy.load(Ordering::SeqCst);

    // Hold the actual metadata leader's transaction lock. DATA publication
    // still commits, but every new handoff attempt must stop before adoption.
    let backend = f.backends[1].clone();
    let (locked, ready) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let holder = tokio::task::spawn_blocking(move || {
        let _guard = backend.node.meta_raft.lock_catalog_txn();
        locked.send(()).unwrap();
        released.recv_timeout(Duration::from_secs(30)).unwrap();
    });
    ready.await.unwrap();
    publish_next_image(&f, command).await;

    // The old fixture unwrapped this one-shot call. Capture the ambiguous
    // wire result AND the original busy error without changing RPC semantics.
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    let owners = CheckpointOwners::new(&root, &next, 1).unwrap();
    let error = tokio::task::spawn_blocking(move || owners.after_positive_settlement(&ledger))
        .await
        .unwrap()
        .unwrap_err();
    assert!(matches!(error, Error::MetaNotReady(ref message)
        if message == "checkpoint retention outcome unconfirmed"));
    until(|| {
        jobs.iter()
            .zip(&failures)
            .all(|(job, before)| job.snapshot().failures > *before)
    })
    .await;
    assert!(f.hooks[1].retention_catalog_busy.load(Ordering::SeqCst) > busy);
    assert_eq!(objects.gets.load(Ordering::SeqCst), gets);
    assert_eq!(
        owner_phases(&f, &root, &next, 1),
        (Some(PinPhase::Published), None)
    );
    for (i, job) in jobs.iter().enumerate() {
        assert_eq!(job.snapshot().adoptions, 1);
        assert_eq!(selected(&f, i), Some(old.clone()));
        assert_eq!(
            f.backends[i]
                .ttl_context()
                .unwrap()
                .data_read_fences
                .floor(f.range.region)
                .unwrap()
                .unwrap()
                .generation,
            1
        );
    }
    assert!(!holder.is_finished());
    release.send(()).unwrap();
    holder.await.unwrap();

    // No new upload, replacement intent, or test retry loop completes this:
    // the same three production workers retry their retained history.
    selected_floor(&f, &jobs, 2).await;
    for job in jobs {
        job.drain().await;
    }
    for i in 0..3 {
        assert_eq!(selected(&f, i), Some(next.clone()));
        let floor = f.backends[i]
            .ttl_context()
            .unwrap()
            .data_read_fences
            .floor(f.range.region)
            .unwrap()
            .unwrap();
        assert_eq!(floor.generation, 2);
        assert_eq!(
            floor.image,
            crate::checkpoint_retention::data_read_image(&next)
                .unwrap()
                .unwrap()
        );
        assert_eq!(f.backends[i].maintenance.snapshot().active, 0);
    }
    assert!(objects.gets.load(Ordering::SeqCst) > gets);
    assert_eq!(
        owner_phases(&f, &root, &old, 0),
        (Some(PinPhase::Released), Some(PinPhase::Published)),
        "advancing the read floor does not authorize old Version release"
    );
    f.close().await;
}
