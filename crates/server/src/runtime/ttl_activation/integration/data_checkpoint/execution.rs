//! Actual checkpoint dispatchers + the actual object job queue/deadline loop.
//! HTTP/2 is duplex; the object SDK transport is InMemory, not remote MinIO.
use super::*;
use kv9_engine::{MinioObjectStore, ObjectStore};

fn assert_no_put_report(
    backend: &RuntimeBackend,
    root: &RootDescriptor,
    manifest: &CheckpointManifest,
    id: kv9_common::retention::OwnerId,
) {
    let pending = crate::checkpoint_retention::CheckpointOwners::new(root, manifest, 0)
        .unwrap()
        .pending_id();
    let view = backend.node.meta_raft.store.engine().snapshot().unwrap();
    let token = kv9_meta::retention::retention_owner(view.as_ref(), root, id)
        .unwrap()
        .unwrap()
        .binding
        .token();
    let report = kv9_meta::retention::checkpoint_admission::drain::checkpoint_drain(
        view.as_ref(),
        root,
        pending,
        token,
    )
    .unwrap()
    .unwrap();
    assert_eq!((report.issued, report.resolved), (0, 0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unconfirmed_executor_close_blocks_new_uploads_and_resumes_the_same_generation() {
    unconfirmed_close(1, kv9_common::retention::PinPhase::Published).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unconfirmed_drain_report_blocks_new_uploads_even_after_executor_release() {
    unconfirmed_close(2, kv9_common::retention::PinPhase::Released).await;
}
async fn unconfirmed_close(fault: usize, expected_phase: kv9_common::retention::PinPhase) {
    let f = Fixture::new().await;
    assert!(matches!(
        f.client(0).activate().await.outcome,
        TtlOutcome::Success { .. }
    ));
    assert!(matches!(
        f.client(0)
            .call(Operation::Put {
                pairs: vec![(b"execution".to_vec(), b"first".to_vec())],
                ttl_secs: Some(60),
            })
            .await
            .outcome,
        TtlOutcome::Success { .. }
    ));
    let store = Arc::new(GatedStore {
        memory: MemoryObjectStore::new(),
        region: 0,
        entered: AtomicBool::new(false),
        blocked_puts: AtomicUsize::new(0),
        blocked: Mutex::new(false),
        wake: Condvar::new(),
    });
    let uploader = Arc::new(RemoteUploader::for_harness(store.clone()));
    f.hooks[1]
        .retention_finish_fault
        .store(fault, Ordering::SeqCst);
    let backend = &f.backends[0];
    let task = Maintenance::start(
        backend.clone(),
        uploader,
        true,
        Duration::from_millis(100),
        &tokio::runtime::Handle::current(),
    );
    until(|| task.snapshot().failures >= 1).await;
    let calls = f.hooks[1].retention_calls.load(Ordering::SeqCst);
    let original = selected(&f, 0).unwrap();
    let root = backend.ttl_context().unwrap().root.clone();
    let id = crate::checkpoint_retention::CheckpointOwners::new(&root, &original, 0)
        .unwrap()
        .execution_id(backend.ttl_context().unwrap().lifetime)
        .unwrap();
    let owner = || {
        kv9_meta::retention::retention_owner(
            f.backends[1]
                .node
                .meta_raft
                .store
                .engine()
                .snapshot()
                .unwrap()
                .as_ref(),
            &root,
            id,
        )
        .unwrap()
        .unwrap()
    };
    assert_eq!(owner().phase, expected_phase);
    assert_eq!(owner().binding.generation.get(), 1);
    let view = f.backends[1]
        .node
        .meta_raft
        .store
        .engine()
        .snapshot()
        .unwrap();
    let owners = crate::checkpoint_retention::CheckpointOwners::new(&root, &original, 0).unwrap();
    let observed = kv9_meta::retention::retention_owner(view.as_ref(), &root, id)
        .unwrap()
        .unwrap();
    assert_eq!(observed.phase, expected_phase);
    let report = kv9_meta::retention::checkpoint_admission::drain::checkpoint_drain(
        view.as_ref(),
        &root,
        owners.pending_id(),
        observed.binding.token(),
    )
    .unwrap();
    assert_eq!(
        report.is_some(),
        fault == 2,
        "release and report have one commit boundary"
    );
    assert!(
        kv9_meta::retention::checkpoint_admission::executors(
            view.as_ref(),
            &root,
            owners.pending_id(),
            None,
            1,
        )
        .unwrap()
        .admission
        .is_closed(),
        "positive settlement closes admission before local drain"
    );
    drop(view);
    let puts = store.blocked_puts.load(Ordering::SeqCst);
    assert!(puts > 0);
    assert!(matches!(
        f.client(0)
            .call(Operation::Put {
                pairs: vec![(b"execution".to_vec(), b"next".to_vec())],
                ttl_secs: Some(60),
            })
            .await
            .outcome,
        TtlOutcome::Success { .. }
    ));
    until(|| f.hooks[1].retention_calls.load(Ordering::SeqCst) >= calls + 3).await;
    assert_eq!(store.blocked_puts.load(Ordering::SeqCst), puts);
    assert_eq!(
        f.data[0]
            .manifest_pair(f.range.region.0)
            .unwrap()
            .generation,
        1
    );
    assert_eq!(owner().phase, expected_phase);
    f.hooks[1].retention_finish_fault.store(0, Ordering::SeqCst);
    until(|| {
        f.data[0]
            .manifest_pair(f.range.region.0)
            .unwrap()
            .generation
            >= 2
    })
    .await;
    assert!(store.blocked_puts.load(Ordering::SeqCst) > puts);
    task.drain().await;
    assert_eq!(owner().phase, kv9_common::retention::PinPhase::Released);
    assert_eq!(
        owner().binding.generation.get(),
        1,
        "closure retries never allocate a new generation"
    );
    f.close().await;
}

async fn closed(
    pause: kv9_engine::minio::ObjectWorkerPause,
) -> kv9_engine::minio::ObjectWorkerPause {
    tokio::task::spawn_blocking(move || {
        assert!(pause.wait_for_closed_execution(Duration::from_secs(10)));
        pause
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn data_checkpoint_cancellation_keeps_capacity_until_the_timed_out_object_job_exits() {
    let f = Fixture::new().await;
    assert!(matches!(
        f.client(0).activate().await.outcome,
        TtlOutcome::Success { .. }
    ));
    assert!(matches!(
        f.client(0)
            .call(Operation::Put {
                pairs: vec![(b"pending-drain".to_vec(), b"expire".to_vec())],
                ttl_secs: Some(0),
            })
            .await
            .outcome,
        TtlOutcome::Success { .. }
    ));
    let (store, pause) =
        MinioObjectStore::with_paused_memory_worker(Duration::from_millis(100)).unwrap();
    let store = Arc::new(store);
    let uploader = Arc::new(RemoteUploader::new(store.clone()));
    let backend = &f.backends[0];
    let checkpoint = Maintenance::start(
        backend.clone(),
        uploader.clone(),
        true,
        Duration::from_millis(100),
        &tokio::runtime::Handle::current(),
    );
    let pause = closed(pause).await;
    assert_eq!(backend.maintenance.snapshot().heavy_active, 1);
    assert_eq!(
        checkpoint.snapshot().failures,
        0,
        "the pass has not exited its local drain"
    );
    let journal = uploader.journal(engine(&f, 0).path().with_extension("pending"));
    let original = journal.load().unwrap().unwrap().manifest().clone();
    let root = backend.ttl_context().unwrap().root.clone();
    let execution_id = crate::checkpoint_retention::CheckpointOwners::new(&root, &original, 0)
        .unwrap()
        .execution_id(backend.ttl_context().unwrap().lifetime)
        .unwrap();
    let executor = |phase| {
        let metadata = f.backends[1]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap();
        let owner = kv9_meta::retention::retention_owner(metadata.as_ref(), &root, execution_id)
            .unwrap()
            .unwrap();
        assert_eq!(owner.phase, phase);
        assert_eq!(
            owner.binding.descriptor.kind,
            kv9_meta::retention::OwnerKind::Build
        );
        assert_eq!(
            owner.binding.descriptor.local,
            Some(backend.ttl_context().unwrap().lifetime)
        );
    };
    executor(kv9_common::retention::PinPhase::Published);
    assert!(journal.load().unwrap().unwrap().requires_upload());
    assert!(store.object_memory_snapshot().retained_bytes > 0);
    assert!(selected(&f, 0).is_none());

    // The real TTL dispatcher still runs while the heavier checkpoint waits
    // for its own local object job. Foreground/consensus never take this permit.
    let ttl = crate::runtime::ttl_maintenance::Maintenance::start(
        backend.clone(),
        &tokio::runtime::Handle::current(),
    );
    until(|| ttl.snapshot().candidates_applied == 1).await;
    ttl.drain().await;
    checkpoint.cancel();
    let drain = tokio::spawn(checkpoint.drain());
    tokio::task::yield_now().await;
    assert!(!drain.is_finished());
    assert_eq!(backend.maintenance.snapshot().active, 1);
    assert_eq!(journal.load().unwrap().unwrap().manifest(), &original);

    drop(pause);
    tokio::time::timeout(Duration::from_secs(5), drain)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(backend.maintenance.snapshot().active, 0);
    executor(kv9_common::retention::PinPhase::Released);
    assert_no_put_report(&f.backends[1], &root, &original, execution_id);
    assert_eq!(store.object_memory_snapshot().retained_bytes, 0);
    assert_eq!(journal.load().unwrap().unwrap().manifest(), &original);
    for file in &original.files {
        assert!(store
            .get(&ObjectKey::new(file.key.clone()).unwrap())
            .unwrap()
            .is_none());
    }
    let root = backend.ttl_context().unwrap().root.clone();
    let owners = crate::checkpoint_retention::CheckpointOwners::new(&root, &original, 0).unwrap();
    let metadata = f.backends[1]
        .node
        .meta_raft
        .store
        .engine()
        .snapshot()
        .unwrap();
    let pending =
        kv9_meta::retention::retention_owner(metadata.as_ref(), &root, owners.pending_id())
            .unwrap()
            .unwrap();
    assert_eq!(pending.phase, kv9_common::retention::PinPhase::Published);
    // Retry reconstructs the same prepared identity from the unchanged durable plan.
    tokio::task::spawn_blocking(move || {
        let (prepared, generation) = uploader
            .with_local_execution(|scoped| journal.load().unwrap().unwrap().recover(scoped))
            .unwrap();
        assert_eq!(generation, 0);
        assert_eq!(prepared.into_manifest(), original);
    })
    .await
    .unwrap();
    drop(metadata);
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn metadata_checkpoint_shutdown_drains_its_timed_out_object_job_before_releasing_capacity() {
    let f = Fixture::new().await;
    let backend = &f.backends[1];
    let root = backend.ttl_context().unwrap().root.clone();
    let (store, pause) =
        MinioObjectStore::with_paused_memory_worker(Duration::from_millis(100)).unwrap();
    let store = Arc::new(store);
    let uploader = Arc::new(RemoteUploader::new(store.clone()));
    let remote = crate::remote_storage::RemoteStorage::start(
        backend.node.clone(),
        backend.driver.clone(),
        crate::remote_storage::PreparedRemote::for_test(&f.path.join("n2"), &root, uploader),
        backend.maintenance.clone(),
        Arc::new(crate::runtime::checkpoint_retention::RoutedLedger::new(
            backend,
            tokio::runtime::Handle::current(),
        )),
    )
    .unwrap();
    let pause = closed(pause).await;
    assert_eq!(backend.maintenance.snapshot().active, 1);
    let journal = FlushJournal::new(f.path.join("n2/catalog.pending"));
    let original = journal.load().unwrap().unwrap().manifest().clone();
    let execution_id = crate::checkpoint_retention::CheckpointOwners::new(&root, &original, 0)
        .unwrap()
        .execution_id(backend.ttl_context().unwrap().lifetime)
        .unwrap();
    let phase = || {
        kv9_meta::retention::retention_owner(
            backend
                .node
                .meta_raft
                .store
                .engine()
                .snapshot()
                .unwrap()
                .as_ref(),
            &root,
            execution_id,
        )
        .unwrap()
        .unwrap()
        .phase
    };
    assert_eq!(phase(), kv9_common::retention::PinPhase::Published);
    let drain = tokio::task::spawn_blocking(move || drop(remote));
    tokio::task::yield_now().await;
    assert!(!drain.is_finished());
    assert!(store.object_memory_snapshot().retained_bytes > 0);
    assert_eq!(backend.maintenance.snapshot().heavy_active, 1);
    drop(pause);
    tokio::time::timeout(Duration::from_secs(5), drain)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(backend.maintenance.snapshot().active, 0);
    assert_eq!(phase(), kv9_common::retention::PinPhase::Released);
    assert_no_put_report(backend, &root, &original, execution_id);
    assert_eq!(journal.load().unwrap().unwrap().manifest(), &original);
    assert_eq!(
        backend
            .driver
            .manifest_pair(kv9_common::META_REGION_0.0)
            .unwrap()
            .generation,
        0
    );
    assert_eq!(store.object_memory_snapshot().retained_bytes, 0);
    f.close().await;
}
