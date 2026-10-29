use super::*;
use crate::runtime::checkpoint_retention::RoutedLedger;
use kv9_meta::retention::{migration_publisher, OwnerBinding};

fn observed(
    f: &Fixture,
    root: &RootDescriptor,
    binding: &OwnerBinding,
) -> migration_publisher::PublisherGeneration {
    migration_publisher::generation(
        f.backends[1]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap()
            .as_ref(),
        root,
        binding.token(),
    )
    .unwrap()
    .unwrap()
}

pub(super) fn capture_fact(
    f: &Fixture,
    root: &RootDescriptor,
    descriptor: &kv9_raft::snapshot_install::capture::CaptureDescriptor,
    node_index: usize,
    generation: u64,
) -> migration_publisher::PublisherGeneration {
    let mut binding = migration_publisher::binding(
        descriptor,
        f.backends[node_index].ttl_context().unwrap().lifetime,
    )
    .unwrap();
    binding.generation = std::num::NonZeroU64::new(generation).unwrap();
    let leader = f
        .meta
        .iter()
        .position(|d| d.status().role == Role::Leader)
        .unwrap();
    migration_publisher::generation(
        f.backends[leader]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap()
            .as_ref(),
        root,
        binding.token(),
    )
    .unwrap()
    .unwrap()
}
async fn capture(
    backend: Arc<RuntimeBackend>,
    root: RootDigest,
    operation: [u8; 16],
) -> Result<crate::api::CaptureMigrationImageResult> {
    tokio::task::spawn_blocking(move || backend.capture_migration_image("test", root, operation))
        .await
        .unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn publisher_rechecks_generation_after_reserving_without_saving_a_conflicting_report() {
    let f = Fixture::new().await;
    let root = f.backends[0].ttl_context().unwrap().root.clone();
    let digest = root.digest();
    let operation = [95; 16];
    let (_identity, _guard, _) = prepare_migration(&f, &root, operation).await;
    let objects = Arc::new(Objects::default());
    enable_payload_replicas(&f, &objects);
    let source = with_uploader(
        &f.backends[0],
        Arc::new(RemoteUploader::for_harness(objects.clone())),
    );
    *f.hooks[0].capture_backend.lock().unwrap() = Some(source.clone());
    let backend = source.clone();
    let plan = tokio::task::spawn_blocking(move || {
        backend.plan_migration_image("test", digest, operation)
    })
    .await
    .unwrap()
    .unwrap();
    let metadata = f.backends[1].clone();
    tokio::task::spawn_blocking(move || {
        metadata.bind_migration_image("test", digest, operation, &plan.manifest)
    })
    .await
    .unwrap()
    .unwrap();
    let at = f.meta[1].propose(&Command::Noop).unwrap();
    applied(&f.meta, at).await;
    let descriptor = kv9_meta::data_groups::capture::committed_capture(
        f.backends[1]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap()
            .as_ref(),
        &root,
        operation,
    )
    .unwrap()
    .unwrap()
    .descriptor()
    .clone();
    // Both calls use the same node/process owner. Use the successor path so
    // there is no source journal lock serializing the calls before admission.
    let successor = with_uploader(
        &f.backends[2],
        Arc::new(RemoteUploader::for_harness(objects.clone())),
    );
    f.data[0].peer().transfer_leader_for_tests(NodeId(3));
    leader(&f.data[2]).await;
    let binding =
        migration_publisher::binding(&descriptor, successor.ttl_context().unwrap().lifetime)
            .unwrap();
    let (notify, read_observed) = std::sync::mpsc::channel();
    let (resume, paused) = std::sync::mpsc::channel();
    *f.hooks[1].publisher_owner_gate.lock().unwrap() = Some(PublisherOwnerGate {
        owner: binding.descriptor.id,
        observed: notify,
        resume: paused,
    });
    let slow_backend = successor.clone();
    let slow = tokio::spawn(async move { capture(slow_backend, digest, operation).await });
    tokio::task::spawn_blocking(move || read_observed.recv_timeout(Duration::from_secs(10)))
        .await
        .unwrap()
        .unwrap();
    capture(successor.clone(), digest, operation).await.unwrap();
    let original = observed(&f, &root, &binding);
    assert!(original.finished_at > original.admitted_at && original.issued > 0);
    let puts = objects.puts.load(Ordering::SeqCst);
    resume.send(()).unwrap();
    let error = slow.await.unwrap().unwrap_err();
    assert!(
        error.to_string().contains("advanced before reservation"),
        "{error}"
    );
    assert_eq!(objects.puts.load(Ordering::SeqCst), puts);
    assert_eq!(observed(&f, &root, &binding), original);
    let queue = successor
        .ttl_context()
        .unwrap()
        .drain_outbox
        .clone()
        .unwrap();
    assert!(!queue.has_pending());
    assert_eq!(
        std::fs::read_dir(f.path.join("n3/checkpoint-drains"))
            .unwrap()
            .count(),
        0
    );
    // A fresh observation can immediately start generation 2, whose GET-only
    // report must not replace generation 1's unknown PUTs.
    capture(successor, digest, operation).await.unwrap();
    let second = capture_fact(&f, &root, &descriptor, 2, 2);
    assert_eq!(second.issued, 0);
    assert_eq!(observed(&f, &root, &binding), original);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn publisher_uncertain_entry_and_unknown_put_report_survive_lost_finish_new_process_and_leader(
) {
    let mut f = Fixture::new().await;
    let root = f.backends[0].ttl_context().unwrap().root.clone();
    let digest = root.digest();
    let operation = [94; 16];
    let (_identity, _guard, _) = prepare_migration(&f, &root, operation).await;
    let objects = Arc::new(Objects::default());
    enable_payload_replicas(&f, &objects);
    let uploader = Arc::new(RemoteUploader::for_harness(objects.clone()));
    let source = with_uploader(&f.backends[0], uploader);
    *f.hooks[0].capture_backend.lock().unwrap() = Some(source.clone());
    let backend = source.clone();
    let plan = tokio::task::spawn_blocking(move || {
        backend.plan_migration_image("test", digest, operation)
    })
    .await
    .unwrap()
    .unwrap();
    let metadata = f.backends[1].clone();
    let pins = tokio::task::spawn_blocking(move || {
        metadata.bind_migration_image("test", digest, operation, &plan.manifest)
    })
    .await
    .unwrap()
    .unwrap();
    let at = f.meta[1].propose(&Command::Noop).unwrap();
    applied(&f.meta, at).await;
    let descriptor = kv9_meta::data_groups::capture::committed_capture(
        f.backends[1]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap()
            .as_ref(),
        &root,
        operation,
    )
    .unwrap()
    .unwrap()
    .descriptor()
    .clone();
    let mut binding =
        migration_publisher::binding(&descriptor, source.ttl_context().unwrap().lifetime).unwrap();
    // A refusal before append records a never-entered terminal generation.
    f.hooks[1].publisher_enter_fault.store(1, Ordering::SeqCst);
    assert!(capture(source.clone(), digest, operation).await.is_err());
    assert_eq!(objects.puts.load(Ordering::SeqCst), 0);
    let cancelled = observed(&f, &root, &binding);
    assert_eq!((cancelled.admitted_at, cancelled.issued), (0, 0));
    assert!(cancelled.finished_at > 0);
    // A lost entry receipt preserves actual admission, but still issues no PUT.
    f.hooks[1].publisher_enter_fault.store(2, Ordering::SeqCst);
    binding.generation = std::num::NonZeroU64::new(2).unwrap();
    assert!(capture(source.clone(), digest, operation).await.is_err());
    let entered = observed(&f, &root, &binding);
    assert!(entered.admitted_at > 0 && entered.finished_at > entered.admitted_at);
    assert_eq!(objects.puts.load(Ordering::SeqCst), 0);
    f.hooks[1].publisher_enter_fault.store(0, Ordering::SeqCst);
    f.hooks[1].publisher_finish_fault.store(1, Ordering::SeqCst);
    objects.refuse.store(true, Ordering::SeqCst);
    binding.generation = std::num::NonZeroU64::new(3).unwrap();
    assert!(capture(source.clone(), digest, operation).await.is_err());
    assert_eq!(observed(&f, &root, &binding).finished_at, 0);
    let puts = objects.puts.load(Ordering::SeqCst);
    assert_eq!(puts, 1);
    assert!(capture(source.clone(), digest, operation).await.is_err());
    assert_eq!(objects.puts.load(Ordering::SeqCst), puts);
    let queue = source.ttl_context().unwrap().drain_outbox.clone().unwrap();
    assert!(queue.has_pending());
    let path = f.path.join("n1/checkpoint-drains");
    assert_eq!(std::fs::read_dir(&path).unwrap().count(), 1);
    // Actual report commits once, its receipt is lost, and a new RPC still
    // cannot reserve a later generation while the old outbox slot remains.
    f.hooks[1].publisher_finish_fault.store(2, Ordering::SeqCst);
    let ledger = RoutedLedger::new(&source, tokio::runtime::Handle::current());
    let outbox = queue.clone();
    assert!(
        tokio::task::spawn_blocking(move || outbox.replay_one(&ledger))
            .await
            .unwrap()
            .is_err()
    );
    let drained = observed(&f, &root, &binding);
    assert!(drained.finished_at > drained.admitted_at);
    assert_eq!((drained.issued, drained.resolved), (1, 0));
    assert!(capture(source.clone(), digest, operation).await.is_err());
    assert_eq!(objects.puts.load(Ordering::SeqCst), puts);
    assert!(queue.has_pending());
    drop(queue);
    // Simulate future capture-history archival only after this Publisher's
    // terminal record committed. Do not remove either Published image owner.
    // The old outbox must acknowledge its exact fact without re-reading the
    // capture plan; this test does not authorize production catalog archival.
    let mut capture_key = b"\0kv9\0migration-capture-v1\0".to_vec();
    capture_key.extend(operation);
    let mut archive = kv9_engine::WriteBatch::new();
    archive.delete(kv9_engine::ColumnFamily::Default, capture_key);
    let at = f.meta[1].propose(&Command::from_batch(&archive)).unwrap();
    applied(&f.meta, at).await;
    f.hooks[0].capture_backend.lock().unwrap().take();
    drop(source);
    // Rebuild process context/outbox with no uploader, while retaining the
    // original report bytes. This is a fixture restart, not a process kill.
    let old = &f.backends[0];
    let identity = StoreIdentity::for_voter(&root, NodeId(1)).unwrap();
    let mut context = ActivationContext::new(
        root.clone(),
        f._guards[0].raw_ttl_format_fence(&root, &identity).unwrap(),
        "cluster-test".into(),
    )
    .unwrap()
    .with_drain_outbox(&f.path.join("n1"))
    .unwrap();
    *context.channels.get_mut().unwrap() =
        old.ttl_context().unwrap().channels.lock().unwrap().clone();
    assert_ne!(
        context.lifetime.process,
        binding.descriptor.local.unwrap().process
    );
    f.backends[0] = Arc::new(RuntimeBackend {
        maintenance: old.maintenance.clone(),
        ttl_activation: Some(Arc::new(context)),
        raw_directory: old.raw_directory.clone(),
        group_handles: old.group_handles.clone(),
        adoption_receipts: old.adoption_receipts.clone(),
        node: old.node.clone(),
        driver: old.driver.clone(),
        transport: old.transport.clone(),
        endpoint_ready: old.endpoint_ready.clone(),
        initial_voters: old.initial_voters.clone(),
        uploader: None,
        status_cache: Arc::default(),
    });
    f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
    leader(&f.meta[2]).await;
    let queue = f.backends[0]
        .ttl_context()
        .unwrap()
        .drain_outbox
        .clone()
        .unwrap();
    let sweep = crate::runtime::checkpoint_sweep::Maintenance::start(
        f.backends[0].clone(),
        &tokio::runtime::Handle::current(),
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while queue.has_pending() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    sweep.drain().await;
    assert_eq!(std::fs::read_dir(&path).unwrap().count(), 0);
    assert_eq!(objects.puts.load(Ordering::SeqCst), puts);
    let at = f.meta[2].propose(&Command::Noop).unwrap();
    applied(&f.meta, at).await;
    assert_eq!(observed(&f, &root, &binding), drained);
    for server in f.servers.drain(..) {
        server.abort();
        let _ = server.await;
    }
    for driver in f.meta.iter().chain(f.data.iter()) {
        driver.stop();
    }
    for thread in f.owners.drain(..) {
        thread.join().unwrap();
    }
    f.backends.clear();
    f.meta.clear();
    f.data.clear();
    f.hooks.clear();
    let (engine, _) = WalEngine::open(f.path.join("n3/meta.wal")).unwrap();
    let view = engine.snapshot().unwrap();
    assert!(
        kv9_meta::data_groups::capture::committed_capture(view.as_ref(), &root, operation)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        migration_publisher::generation(view.as_ref(), &root, binding.token())
            .unwrap()
            .unwrap(),
        drained
    );
    assert_eq!(
        kv9_meta::retention::retention_owner(view.as_ref(), &root, binding.descriptor.id)
            .unwrap()
            .unwrap()
            .phase,
        PinPhase::Released
    );
    for id in [pins.source_owner, pins.destination_owner] {
        assert_eq!(
            kv9_meta::retention::retention_owner(view.as_ref(), &root, id)
                .unwrap()
                .unwrap()
                .phase,
            PinPhase::Published
        );
    }
}
