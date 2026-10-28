use super::*;
use crate::checkpoint_retention::CheckpointRead;
use kv9_engine::checkpoint::CheckpointManifest;
use kv9_meta::retention::{reader_closure as closure, reader_index};

fn observed(f: &Fixture, leader: usize, root: &RootDescriptor, pending: OwnerId) -> closure::Work {
    closure::observe(
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
fn tasks(f: &Fixture) -> Vec<Maintenance> {
    f.backends
        .iter()
        .map(|b| Maintenance::start(b.clone(), &tokio::runtime::Handle::current()))
        .collect()
}
async fn stop(tasks: Vec<Maintenance>) {
    for task in &tasks {
        task.cancel();
    }
    for task in tasks {
        task.drain().await;
    }
}
fn read_scope(
    root: &RootDescriptor,
    manifest: &CheckpointManifest,
    predecessor: u64,
    ledger: &RoutedLedger,
    uploader: &RemoteUploader,
    get: bool,
) -> Result<()> {
    let mut reader = CheckpointRead::new(root, manifest, predecessor, OwnerKind::Pending, ledger)?;
    let (result, report) = uploader.with_local_read_report(|scope| {
        reader.before_io(ledger, scope)?;
        if get {
            scope.restore(manifest)?;
        }
        Ok(())
    })?;
    reader.observe_drain(report)?;
    reader.finish(ledger)?;
    result
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reader_closure_resumes_lost_page_and_leader_change_then_restarts_after_actual_late_drain()
{
    let f = Fixture::new().await;
    let (root, cases) = remote::committed(&f, true).await;
    let pending = cases
        .iter()
        .find(|c| c.positive == Some(false))
        .unwrap()
        .pending;
    let intent = kv9_meta::retention::checkpoint_intent::checkpoint_intent(
        f.backends[1]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap()
            .as_ref(),
        &root,
        pending,
    )
    .unwrap()
    .unwrap();
    let manifest = intent.manifest().unwrap();
    let predecessor = intent.predecessor();
    let r = root.clone();
    let m = manifest.clone();
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    tokio::task::spawn_blocking(move || {
        let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()));
        // Real production admission/drain scopes, with no object jobs in the
        // first nine generations. The tenth below has an actual blocked GET.
        for _ in 0..9 {
            read_scope(&r, &m, predecessor, &ledger, &uploader, false).unwrap();
        }
    })
    .await
    .unwrap();
    let (store, pause) =
        kv9_engine::MinioObjectStore::with_paused_memory_worker(Duration::from_millis(100))
            .unwrap();
    let r = root.clone();
    let m = manifest.clone();
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    let active = tokio::task::spawn_blocking(move || {
        read_scope(
            &r,
            &m,
            predecessor,
            &ledger,
            &RemoteUploader::new(Arc::new(store)),
            true,
        )
    });
    let pause = tokio::task::spawn_blocking(move || {
        assert!(pause.wait_for_closed_execution(Duration::from_secs(10)));
        pause
    })
    .await
    .unwrap();
    assert!(!active.is_finished());
    let winner: [u8; 32] = f.data[0]
        .manifest_pair(f.range.region.0)
        .unwrap()
        .last_change_id
        .try_into()
        .unwrap();
    send(&f, request(&root, pending, NodeId(1), winner))
        .await
        .unwrap();
    // Use the actual global census pass, but keep attempt maintenance stopped
    // until the first Reader aggregate response has been dropped deliberately.
    let backend = f.backends[1].clone();
    let r = root.clone();
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    tokio::task::spawn_blocking(move || loop {
        let view = backend.node.meta_raft.store.engine().snapshot().unwrap();
        if matches!(
            reader_index::observe(view.as_ref(), &r).unwrap(),
            reader_index::Work::Complete
        ) {
            break;
        }
        drop(view);
        crate::runtime::checkpoint_sweep::build_index::pass(
            &backend,
            &ledger,
            &CancellationToken::new(),
            Instant::now() + Duration::from_secs(5),
            &mut Stats::default(),
        )
        .unwrap();
    })
    .await
    .unwrap();
    let closure::Work::Advance { cursor, .. } = observed(&f, 1, &root, pending) else {
        panic!("first page")
    };
    let bytes = encode_request(root.digest(), &cursor.request(pending)).unwrap();
    f.hooks[1].retention_calls.store(0, Ordering::SeqCst);
    f.hooks[1].retention_commits.store(0, Ordering::SeqCst);
    f.hooks[1].retention_fail_after.store(1, Ordering::SeqCst);
    assert!(send(&f, bytes.clone()).await.is_err());
    assert_eq!(f.hooks[1].retention_commits.load(Ordering::SeqCst), 1);
    let closure::Work::Advance { cursor: first, .. } = observed(&f, 1, &root, pending) else {
        panic!("partial")
    };
    assert_eq!((first.sequence, first.counts.generations), (1, 4));
    f.hooks[1].retention_fail_after.store(0, Ordering::SeqCst);
    f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
    until(|| f.meta[2].status().role == Role::Leader && f.meta[1].status().role != Role::Leader)
        .await;
    send(&f, bytes).await.unwrap();
    assert_eq!(
        observed(&f, 2, &root, pending),
        closure::Work::Advance {
            cursor: first,
            restart: false
        }
    );
    let maintenance = tasks(&f);
    until(|| matches!(observed(&f, 2, &root, pending), closure::Work::Complete(_))).await;
    assert_eq!(maintenance[2].snapshot().peak_in_flight, 1);
    stop(maintenance).await;
    let closure::Work::Complete(blocked) = observed(&f, 2, &root, pending) else {
        panic!()
    };
    assert_eq!(
        (
            blocked.counts.generations,
            blocked.counts.drained,
            blocked.counts.active
        ),
        (10, 9, 1)
    );
    assert!(!observed(&f, 2, &root, pending).registered_generations_closed());
    assert!(!active.is_finished());
    drop(pause);
    assert!(tokio::time::timeout(Duration::from_secs(5), active)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    assert!(matches!(
        observed(&f, 2, &root, pending),
        closure::Work::Advance { restart: true, .. }
    ));
    let maintenance = tasks(&f);
    until(|| observed(&f, 2, &root, pending).registered_generations_closed()).await;
    assert!(maintenance[2].snapshot().reader_closure_restarts > 0);
    stop(maintenance).await;
    // A new local scope is refused before I/O; its actual empty drain records
    // an Unentered terminal generation and invalidates the finished aggregate.
    let r = root.clone();
    let m = manifest.clone();
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    assert!(tokio::task::spawn_blocking(move || read_scope(
        &r,
        &m,
        predecessor,
        &ledger,
        &RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new())),
        false
    ))
    .await
    .unwrap()
    .is_err());
    assert!(matches!(
        observed(&f, 2, &root, pending),
        closure::Work::Advance { restart: true, .. }
    ));
    let maintenance = tasks(&f);
    until(|| observed(&f, 2, &root, pending).registered_generations_closed()).await;
    stop(maintenance).await;
    let closure::Work::Complete(done) = observed(&f, 2, &root, pending) else {
        panic!()
    };
    assert_eq!(
        done.counts,
        closure::Counts {
            owners: 1,
            generations: 11,
            drained: 10,
            unentered: 1,
            ..closure::Counts::default()
        }
    );
    assert_eq!(
        settlement::phase(&f, &root, pending),
        Some(PinPhase::Published)
    );
    assert!(!f.backends[0]
        .ttl_context()
        .unwrap()
        .drain_outbox
        .as_ref()
        .unwrap()
        .has_pending());
    let path = f.shutdown_keep().await;
    let (engine, _) = WalEngine::open(path.join("n3/meta.wal")).unwrap();
    let view = engine.snapshot().unwrap();
    assert_eq!(
        closure::observe(view.as_ref(), &root, pending).unwrap(),
        closure::Work::Complete(done)
    );
    assert_eq!(
        kv9_meta::retention::retention_owner(view.as_ref(), &root, pending)
            .unwrap()
            .unwrap()
            .phase,
        PinPhase::Published
    );
    drop(view);
    drop(engine);
    std::fs::remove_dir_all(path).unwrap();
}
