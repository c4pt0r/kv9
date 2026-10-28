use super::*;
use crate::runtime::checkpoint_sweep::{build_index, Reconciler, Stats};
use kv9_meta::retention::{
    bootstrap_closure::{self, Work},
    bootstrap_index,
    checkpoint_intent::CheckpointIntent,
};
use read_gate::{select, send, stop, tasks, wait};

fn observed(f: &Fixture, root: &RootDescriptor, pending: OwnerId, leader: usize) -> Work {
    bootstrap_closure::observe(
        f.backends[leader]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap()
            .as_ref(),
        root,
        pending,
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bootstrap_image_census_survives_lost_receipt_leader_change_late_fence_and_offline_recovery(
) {
    let f = Fixture::new().await;
    let root = f.backends[0].ttl_context().unwrap().root.clone();
    let objects = Arc::new(Objects::default());
    let uploader = Arc::new(RemoteUploader::for_harness(objects.clone()));
    let (old, p) = retirement::publish(&f, uploader.clone()).await;
    for n in 0..3 {
        select(&f, &root, &old, p, n).await;
    }
    let intent = CheckpointIntent::new(&root, &old, p).unwrap();
    let pending = intent.pending_id();
    let (new, q) = retirement::publish(&f, uploader.clone()).await;
    for n in 1..3 {
        select(&f, &root, &new, q, n).await;
    }
    // The selector moves and locally fences the old grant, but the independent
    // store query fails. Its durable retirement debt and catalog pin remain.
    f.hooks[0].bootstrap_refuse.store(true, Ordering::SeqCst);
    assert!(retirement::select(
        &f,
        new.clone(),
        q,
        Arc::new(RoutedLedger::new(
            &f.backends[0],
            tokio::runtime::Handle::current()
        )),
    )
    .await
    .is_err());
    assert_eq!(
        retirement::owner(
            &f,
            &root,
            &bootstrap_reader::binding(
                &root,
                &old,
                p,
                StoreIdentity::for_voter(&root, NodeId(1)).unwrap(),
            )
            .unwrap()
        ),
        PinPhase::Published
    );
    let new_intent = CheckpointIntent::new(&root, &new, q).unwrap();
    let identity = StoreIdentity::for_voter(&root, NodeId(1)).unwrap();
    send(
        &f,
        &root,
        LedgerRequest::RecordCheckpointSelection {
            pending: new_intent.binding(OwnerKind::Pending).unwrap().token(),
            source: identity.node_id,
            store: identity.store_incarnation,
        },
    )
    .await
    .unwrap();
    send(
        &f,
        &root,
        LedgerRequest::AdvanceMetadataReadGate {
            pending: intent.binding(OwnerKind::Pending).unwrap().token(),
            expected: 0,
        },
    )
    .await
    .unwrap();
    // Use the production bounded catalog lane before the per-image census.
    for _ in 0..256 {
        if matches!(
            bootstrap_index::observe(
                f.backends[1]
                    .node
                    .meta_raft
                    .store
                    .engine()
                    .snapshot()
                    .unwrap()
                    .as_ref(),
                &root,
            )
            .unwrap(),
            bootstrap_index::Work::Complete
        ) {
            break;
        }
        let backend = f.backends[1].clone();
        let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
        tokio::task::spawn_blocking(move || {
            build_index::pass(
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
        observed(&f, &root, pending, 1),
        Work::Advance { restart: false, .. }
    ));
    f.hooks[1]
        .retention_bootstrap_closure_fault
        .store(2, Ordering::SeqCst);
    let mut state = Reconciler::default();
    for _ in 0..32 {
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
            .retention_bootstrap_closure_fault
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
            .retention_bootstrap_closure_fault
            .load(Ordering::SeqCst),
        1
    );
    let Work::Complete(blocked) = observed(&f, &root, pending, 1) else {
        panic!()
    };
    assert_eq!(
        (
            blocked.counts.grants,
            blocked.counts.retired,
            blocked.counts.active
        ),
        (3, 2, 1)
    );
    assert!(!observed(&f, &root, pending, 1).registered_grants_retired());
    let gets = objects.gets.load(Ordering::SeqCst);
    let puts = objects.puts.load(Ordering::SeqCst);
    f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
    wait("bootstrap closure new leader", || {
        f.meta[2].status().role == Role::Leader && f.meta[1].status().role != Role::Leader
    })
    .await;
    send(
        &f,
        &root,
        LedgerRequest::AdvanceBootstrapClosure {
            pending: intent.binding(OwnerKind::Pending).unwrap().token(),
            expected: 0,
        },
    )
    .await
    .unwrap();
    assert_eq!(observed(&f, &root, pending, 2), Work::Complete(blocked));
    f.hooks[0].bootstrap_refuse.store(false, Ordering::SeqCst);
    let directory = f.path.join("n1");
    let r = root.clone();
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    tokio::task::spawn_blocking(move || {
        crate::bootstrap_retention::resume_retirement(&directory, &r, &ledger)
    })
    .await
    .unwrap()
    .unwrap();
    assert!(matches!(
        observed(&f, &root, pending, 2),
        Work::Advance { restart: true, .. }
    ));
    let maintenance = tasks(&f);
    wait("bootstrap census after actual retirement", || {
        observed(&f, &root, pending, 2).registered_grants_retired()
            && maintenance[2].snapshot().bootstrap_closure_restarts > 0
    })
    .await;
    assert_eq!(maintenance[2].snapshot().peak_in_flight, 1);
    stop(maintenance).await;
    let Work::Complete(done) = observed(&f, &root, pending, 2) else {
        panic!()
    };
    assert_eq!(
        (
            done.counts.grants,
            done.counts.retired,
            done.counts.active,
            done.counts.legacy
        ),
        (3, 3, 0, 0)
    );
    assert_eq!(
        retirement::owner(&f, &root, &intent.binding(OwnerKind::Version).unwrap()),
        PinPhase::Published
    );
    assert_eq!(objects.gets.load(Ordering::SeqCst), gets);
    assert_eq!(objects.puts.load(Ordering::SeqCst), puts);
    let applied = f.meta[2].driver_applied().unwrap();
    wait(
        "bootstrap census replicated before offline recovery",
        || {
            f.meta
                .iter()
                .all(|d| d.driver_applied().is_some_and(|p| p.index >= applied.index))
        },
    )
    .await;
    let directory = f.shutdown_keep().await;
    for node in 1..=3 {
        let path = directory.join(format!("n{node}"));
        let guard = StoreGuard::lock(&path).unwrap();
        let mut storage = DiskRaftStorage::recover(&path.join("meta-raft")).unwrap();
        let bootstrap = BootstrapReader {
            directory: &path,
            root: &root,
            identity: StoreIdentity::for_voter(&root, NodeId(node)).unwrap(),
            uploader: &uploader,
        };
        let (e, _, anchor) = kv9_meta::recovery::open_initial_checkpoint_engine_with_reader(
            &mut storage,
            path.join("meta.wal"),
            Some(&bootstrap),
            &root,
        )
        .unwrap();
        assert_eq!(anchor.unwrap().descriptor().manifest, new.encode().unwrap());
        let view = e.snapshot().unwrap();
        assert_eq!(
            bootstrap_closure::observe(view.as_ref(), &root, pending).unwrap(),
            Work::Complete(done.clone())
        );
        assert_eq!(
            kv9_meta::retention::retention_owner(
                view.as_ref(),
                &root,
                intent.binding(OwnerKind::Version).unwrap().descriptor.id,
            )
            .unwrap()
            .unwrap()
            .phase,
            PinPhase::Published
        );
        drop(view);
        drop(e);
        drop(storage);
        drop(guard);
    }
    std::fs::remove_dir_all(directory).unwrap();
}
