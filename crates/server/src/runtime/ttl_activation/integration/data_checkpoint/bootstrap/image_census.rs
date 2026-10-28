use super::*;
use crate::installation_retention::RetainedImages;
use crate::runtime::checkpoint_sweep::{Reconciler, Stats};
use kv9_engine::checkpoint::CheckpointRestore;
use kv9_meta::retention::{
    checkpoint_intent::CheckpointIntent, image_reader_closure, image_reader_index,
};
use read_gate::{select, send, stop, tasks, wait};

fn observed(
    f: &Fixture,
    r: &RootDescriptor,
    i: &CheckpointIntent,
    leader: usize,
) -> image_reader_closure::Work {
    image_reader_closure::observe(
        f.backends[leader]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap()
            .as_ref(),
        r,
        i.pending_id(),
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unrelated_snapshot_reader_is_counted_across_lost_page_failover_real_get_drain_and_all_store_recovery(
) {
    let f = Fixture::new().await;
    let root = f.backends[0].ttl_context().unwrap().root.clone();
    let objects = Arc::new(Objects::default());
    let uploader = Arc::new(RemoteUploader::for_harness(objects.clone()));
    let (old, p) = retirement::publish(&f, uploader.clone()).await;
    for n in 0..3 {
        select(&f, &root, &old, p, n).await;
    }
    let i = CheckpointIntent::new(&root, &old, p).unwrap();
    let mut source = i.binding(OwnerKind::Version).unwrap();
    source.descriptor.id = OwnerId::new([81; 16]).unwrap();
    source.descriptor.kind = OwnerKind::Snapshot;
    source.descriptor.operation = [81; 32];
    send(&f, &root, LedgerRequest::Acquire(source.clone()))
        .await
        .unwrap();
    send(&f, &root, LedgerRequest::Publish(source.token()))
        .await
        .unwrap();
    assert_ne!(
        source.descriptor.operation,
        i.binding(OwnerKind::Version).unwrap().descriptor.operation
    );
    let (store, pause) =
        kv9_engine::MinioObjectStore::with_paused_memory_worker(Duration::from_millis(100))
            .unwrap();
    let r = root.clone();
    let m = old.clone();
    let id = source.descriptor.id;
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    let active = tokio::task::spawn_blocking(move || {
        let u = RemoteUploader::new(Arc::new(store));
        RetainedImages::new(&u, &ledger, &r, &[id])
            .unwrap()
            .restore(&m)
    });
    let pause = tokio::task::spawn_blocking(move || {
        assert!(pause.wait_for_closed_execution(Duration::from_secs(10)));
        pause
    })
    .await
    .unwrap();
    assert!(!active.is_finished());
    let (new, q) = retirement::publish(&f, uploader.clone()).await;
    for n in 0..3 {
        select(&f, &root, &new, q, n).await;
    }
    let pending = i.binding(OwnerKind::Pending).unwrap().token();
    send(
        &f,
        &root,
        LedgerRequest::AdvanceMetadataReadGate {
            pending,
            expected: 0,
        },
    )
    .await
    .unwrap();
    send(&f, &root, LedgerRequest::CloseMetadataImageReads(pending))
        .await
        .unwrap();
    let gets = objects.gets.load(Ordering::SeqCst);
    let puts = objects.puts.load(Ordering::SeqCst);
    // Use the production global census driver and actual routed ledger; there
    // is no direct catalog seeding or substitute apply on this path.
    for _ in 0..128 {
        if matches!(
            image_reader_index::observe(
                f.backends[1]
                    .node
                    .meta_raft
                    .store
                    .engine()
                    .snapshot()
                    .unwrap()
                    .as_ref(),
                &root
            )
            .unwrap(),
            image_reader_index::Work::Complete
        ) {
            break;
        }
        let backend = f.backends[1].clone();
        let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
        tokio::task::spawn_blocking(move || {
            crate::runtime::checkpoint_sweep::build_index::pass(
                &backend,
                &ledger,
                &tokio_util::sync::CancellationToken::new(),
                Instant::now() + Duration::from_secs(5),
                &mut Stats::default(),
            )
        })
        .await
        .unwrap()
        .unwrap();
    }
    assert!(matches!(
        observed(&f, &root, &i, 1),
        image_reader_closure::Work::Advance { .. }
    ));
    f.hooks[1]
        .retention_image_census_fault
        .store(2, Ordering::SeqCst);
    let mut state = Reconciler::default();
    for _ in 0..64 {
        let backend = f.backends[1].clone();
        let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
        let (next, result) = tokio::task::spawn_blocking(move || {
            let result = state.pass(
                &backend,
                &ledger,
                &tokio_util::sync::CancellationToken::new(),
                Instant::now() + Duration::from_secs(5),
                &mut Stats::default(),
            );
            (state, result)
        })
        .await
        .unwrap();
        state = next;
        if f.hooks[1]
            .retention_image_census_fault
            .load(Ordering::SeqCst)
            == 1
        {
            assert!(result.is_err());
            break;
        }
        result.unwrap();
    }
    assert_eq!(
        f.hooks[1]
            .retention_image_census_fault
            .load(Ordering::SeqCst),
        1
    );
    f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
    wait("image census leader", || {
        f.meta[2].status().role == Role::Leader && f.meta[1].status().role != Role::Leader
    })
    .await;
    send(
        &f,
        &root,
        LedgerRequest::AdvanceImageReaderClosure {
            pending,
            expected: 0,
        },
    )
    .await
    .unwrap();
    let maintenance = tasks(&f);
    wait("unrelated active image reader", || {
        matches!(
            observed(&f, &root, &i, 2),
            image_reader_closure::Work::Complete(_)
        )
    })
    .await;
    stop(maintenance).await;
    let image_reader_closure::Work::Complete(c) = observed(&f, &root, &i, 2) else {
        panic!()
    };
    assert_eq!(c.counts.active, 1);
    assert!(!active.is_finished());
    let view = f.backends[2]
        .node
        .meta_raft
        .store
        .engine()
        .snapshot()
        .unwrap();
    assert!(
        kv9_meta::retention::reader_closure::observe(view.as_ref(), &root, i.pending_id())
            .unwrap()
            .registered_generations_closed(),
        "per-operation closure alone misses the Snapshot operation"
    );
    drop(view);
    let r = root.clone();
    let m = old.clone();
    let u = uploader.clone();
    let ledger = RoutedLedger::new(&f.backends[1], tokio::runtime::Handle::current());
    assert!(tokio::task::spawn_blocking(move || RetainedImages::new(
        u.as_ref(),
        &ledger,
        &r,
        &[id]
    )
    .unwrap()
    .restore(&m))
    .await
    .unwrap()
    .is_err());
    assert_eq!(objects.gets.load(Ordering::SeqCst), gets);
    assert_eq!(objects.puts.load(Ordering::SeqCst), puts);
    drop(pause);
    assert!(tokio::time::timeout(Duration::from_secs(5), active)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    assert!(matches!(
        observed(&f, &root, &i, 2),
        image_reader_closure::Work::Advance { restart: true, .. }
    ));
    let maintenance = tasks(&f);
    wait("image reader actual drain", || {
        observed(&f, &root, &i, 2).registered_generations_closed()
    })
    .await;
    assert_eq!(maintenance[2].snapshot().peak_in_flight, 1);
    stop(maintenance).await;
    let image_reader_closure::Work::Complete(done) = observed(&f, &root, &i, 2) else {
        panic!()
    };
    assert!(done.counts.drained >= 1);
    assert_eq!(done.counts.unentered, 1);
    let at = f.meta[2].driver_applied().unwrap();
    wait("image census all stores", || {
        f.meta
            .iter()
            .all(|d| d.driver_applied().is_some_and(|p| p.index >= at.index))
    })
    .await;
    let directory = f.shutdown_keep().await;
    for n in 1..=3 {
        let path = directory.join(format!("n{n}"));
        // Stopped V8 format fixture, not a running old binary. Retain the real
        // checkpoint, Raft tail, bootstrap authorization and census verbatim.
        let lifecycle = path.join(kv9_common::store_lifecycle::STORE_LIFECYCLE_FILE);
        let current = std::fs::read(&lifecycle).unwrap();
        let mut legacy = current.clone();
        legacy[..8].copy_from_slice(b"KV9LIFE8");
        let end = legacy.len() - 32;
        let digest = kv9_common::root::RootDigest::sha256(&legacy[..end]);
        legacy[end..].copy_from_slice(digest.as_bytes());
        std::fs::write(&lifecycle, &legacy).unwrap();
        let receipt_path = path.join("catalog.bootstrap-reads");
        let receipt = std::fs::read(&receipt_path).unwrap();
        let catalog = path.join("meta.wal");
        let wal = std::fs::read(&catalog).unwrap();
        let mut guard = StoreGuard::lock(&path).unwrap();
        let record = guard.record();
        let mut storage = DiskRaftStorage::recover(&path.join("meta-raft")).unwrap();
        let bootstrap = BootstrapReader {
            directory: &path,
            root: &root,
            identity: StoreIdentity::for_voter(&root, NodeId(n)).unwrap(),
            uploader: &uploader,
        };
        let gets = objects.gets.load(Ordering::SeqCst);
        let puts = objects.puts.load(Ordering::SeqCst);
        assert!(guard.require_current_writer_format().is_err());
        assert!(
            crate::bootstrap_retention::startup::open_metadata::<RoutedLedger>(
                &guard,
                &mut storage,
                &catalog,
                &bootstrap,
                || panic!("unfenced offline startup queried quorum")
            )
            .is_err()
        );
        assert_eq!(objects.gets.load(Ordering::SeqCst), gets);
        assert_eq!(objects.puts.load(Ordering::SeqCst), puts);
        assert_eq!(std::fs::read(&lifecycle).unwrap(), legacy);
        assert_eq!(std::fs::read(&catalog).unwrap(), wal);
        assert_eq!(std::fs::read(&receipt_path).unwrap(), receipt);
        guard.fence_legacy_writers().unwrap();
        assert_eq!(guard.record(), record);
        assert_eq!(std::fs::read(&lifecycle).unwrap(), current);
        assert_eq!(std::fs::read(&catalog).unwrap(), wal);
        assert_eq!(std::fs::read(&receipt_path).unwrap(), receipt);
        let (e, _, anchor) = crate::bootstrap_retention::startup::open_metadata::<RoutedLedger>(
            &guard,
            &mut storage,
            &catalog,
            &bootstrap,
            || panic!("authorized offline startup queried quorum"),
        )
        .unwrap();
        assert!(objects.gets.load(Ordering::SeqCst) > gets);
        assert_eq!(objects.puts.load(Ordering::SeqCst), puts);
        assert_eq!(std::fs::read(&receipt_path).unwrap(), receipt);
        assert_eq!(anchor.unwrap().descriptor().manifest, new.encode().unwrap());
        let view = e.snapshot().unwrap();
        assert_eq!(
            image_reader_closure::observe(view.as_ref(), &root, i.pending_id()).unwrap(),
            image_reader_closure::Work::Complete(done.clone())
        );
        for b in [&source, &i.binding(OwnerKind::Version).unwrap()] {
            assert_eq!(
                kv9_meta::retention::retention_owner(view.as_ref(), &root, b.descriptor.id)
                    .unwrap()
                    .unwrap()
                    .phase,
                PinPhase::Published
            );
        }
        drop(view);
        drop(e);
        drop(storage);
        drop(guard);
    }
    std::fs::remove_dir_all(directory).unwrap();
}
