use super::*;
use crate::checkpoint_retention::{drain_outbox::DrainOutbox, CheckpointRead};
use kv9_meta::retention::checkpoint_reader;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn terminal_reader_and_bootstrap_receipts_replay_after_version_transfer_leader_change_and_reopen(
) {
    let f = Fixture::new().await;
    let root = f.backends[0].ttl_context().unwrap().root.clone();
    let identity = StoreIdentity::for_voter(&root, NodeId(1)).unwrap();
    let directory = f.path.join("n1");
    let objects = Arc::new(Objects::default());
    let uploader = Arc::new(RemoteUploader::for_harness(objects.clone()));
    let ledger = Arc::new(Fault {
        inner: RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current()),
        lose: AtomicBool::new(false),
        saved: Mutex::default(),
    });
    let (first, p) = publish(&f, uploader.clone()).await;
    select(&f, first.clone(), p, ledger.clone()).await.unwrap();
    let intent = CheckpointIntent::new(&root, &first, p).unwrap();
    let version = intent.binding(OwnerKind::Version).unwrap();
    let bootstrap = bootstrap_reader::binding(&root, &first, p, identity).unwrap();
    let reader =
        checkpoint_reader::binding(&version, f.backends[0].ttl_context().unwrap().lifetime)
            .unwrap();
    let read_finish = LedgerRequest::FinishCheckpointRead {
        successor: version.token(),
        reader: reader.clone(),
    };
    f.hooks[1].retention_read_fault.store(2, Ordering::SeqCst);
    let b = f.backends[0].clone();
    let u = uploader.clone();
    let r = root.clone();
    let m = first.clone();
    let runtime = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        let ledger = RoutedLedger::new(&b, runtime);
        let mut read = CheckpointRead::new(&r, &m, p, OwnerKind::Version, &ledger).unwrap();
        let (result, drained) = u
            .with_local_read_report(|scope| {
                read.before_io(&ledger, scope)?;
                scope.restore(&m)
            })
            .unwrap();
        result.unwrap();
        read.observe_drain(drained).unwrap();
        assert!(read.finish(&ledger).is_err());
    })
    .await
    .unwrap();
    assert_eq!(owner(&f, &root, &reader), PinPhase::Released);
    assert!(f.backends[0]
        .ttl_context()
        .unwrap()
        .drain_outbox
        .as_ref()
        .unwrap()
        .has_pending());
    let (second, q) = publish(&f, uploader.clone()).await;
    ledger.lose.store(true, Ordering::SeqCst);
    assert!(select(&f, second.clone(), q, ledger.clone()).await.is_err());
    assert_eq!(owner(&f, &root, &bootstrap), PinPhase::Released);
    let bootstrap_finish = ledger.saved.lock().unwrap().clone().unwrap();
    // Valid transfer to identical full-image coverage, not a newer-image
    // retirement certificate. This fixture never makes any object collectible.
    let mut holder = version.clone();
    holder.descriptor.kind = OwnerKind::Snapshot;
    holder.descriptor.id = OwnerId::new([86; 16]).unwrap();
    for request in [
        LedgerRequest::Share {
            from: version.token(),
            to: holder.clone(),
        },
        LedgerRequest::Publish(holder.token()),
        LedgerRequest::QuiesceAfterTransfer {
            from: version.token(),
            to: holder.token(),
        },
        LedgerRequest::Release(version.token()),
    ] {
        send(
            ledger.clone(),
            encode_request(root.digest(), &request).unwrap(),
        )
        .await
        .unwrap();
    }
    assert_eq!(owner(&f, &root, &version), PinPhase::Released);
    assert_eq!(owner(&f, &root, &holder), PinPhase::Published);
    assert!(
        kv9_meta::retention::checkpoint_intent::checkpoint_intent(
            f.backends[1]
                .node
                .meta_raft
                .store
                .engine()
                .snapshot()
                .unwrap()
                .as_ref(),
            &root,
            intent.pending_id()
        )
        .is_err(),
        "old intent must no longer satisfy Published source lookup"
    );
    f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
    until(|| f.meta[2].status().role == Role::Leader).await;
    f.hooks[0].bootstrap_refuse.store(true, Ordering::SeqCst);
    let inspections = f.hooks[0].bootstrap_reads.load(Ordering::SeqCst);
    let gets = objects.gets.load(Ordering::SeqCst);
    ledger.lose.store(false, Ordering::SeqCst);
    // Re-open the actual persisted outbox, preserving the original lifetime in
    // its saved request. No active Reader survives the earlier blocking scope.
    let outbox = DrainOutbox::open(&directory, identity).unwrap();
    let b = f.backends[0].clone();
    let r = root.clone();
    let dir = directory.clone();
    let runtime = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        let ledger = RoutedLedger::new(&b, runtime);
        assert!(outbox.replay_one(&ledger).unwrap());
        assert!(!outbox.has_pending());
        crate::bootstrap_retention::resume_retirement(&dir, &r, &ledger).unwrap();
    })
    .await
    .unwrap();
    assert_eq!(
        f.hooks[0].bootstrap_reads.load(Ordering::SeqCst),
        inspections
    );
    assert_eq!(objects.gets.load(Ordering::SeqCst), gets);
    assert!(fenced_image(&directory, identity, p, Some(&second))
        .unwrap()
        .is_none());
    // Caller-supplied first releases still cannot substitute this terminal path.
    let mut unentered = reader.clone();
    unentered.generation = std::num::NonZeroU64::new(2).unwrap();
    assert!(send(
        ledger.clone(),
        encode_request(
            root.digest(),
            &LedgerRequest::FinishCheckpointRead {
                successor: version.token(),
                reader: unentered,
            }
        )
        .unwrap()
    )
    .await
    .is_err());
    let at = f.meta[2].driver_applied().unwrap();
    until(|| {
        f.meta
            .iter()
            .all(|d| d.driver_applied().is_some_and(|v| v.index >= at.index))
    })
    .await;
    drop(ledger);
    let path = f.shutdown_keep().await;
    tokio::task::spawn_blocking(move || {
        let _guard = StoreGuard::lock(&directory).unwrap();
        let mut storage = DiskRaftStorage::recover(&directory.join("meta-raft")).unwrap();
        let reader = BootstrapReader {
            directory: &directory,
            root: &root,
            identity,
            uploader: &uploader,
        };
        let (engine, _, _) = kv9_meta::recovery::open_initial_checkpoint_engine_with_reader(
            &mut storage,
            directory.join("meta.wal"),
            Some(&reader),
            &root,
        )
        .unwrap();
        let view = engine.snapshot().unwrap();
        assert!(
            !kv9_meta::retention::plan_retention(view.as_ref(), &root, &read_finish)
                .unwrap()
                .changed()
        );
        let LedgerRequest::RetireBootstrapRead { pending, .. } =
            decode_request(&bootstrap_finish, root.digest()).unwrap()
        else {
            panic!("exact bootstrap report")
        };
        assert!(
            !bootstrap_reader::plan_retry(view.as_ref(), &root, pending, identity)
                .unwrap()
                .changed()
        );
        assert_eq!(
            kv9_meta::retention::retention_owner(view.as_ref(), &root, version.descriptor.id)
                .unwrap()
                .unwrap()
                .phase,
            PinPhase::Released
        );
        assert_eq!(
            kv9_meta::retention::retention_owner(view.as_ref(), &root, holder.descriptor.id)
                .unwrap()
                .unwrap()
                .phase,
            PinPhase::Published
        );
    })
    .await
    .unwrap();
    std::fs::remove_dir_all(path).unwrap();
}
