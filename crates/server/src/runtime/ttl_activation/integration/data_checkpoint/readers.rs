use super::*;
use crate::checkpoint_retention::{CheckpointOwners, CheckpointRead};
use crate::runtime::checkpoint_retention::RoutedLedger;
use kv9_common::retention::PinPhase;
use kv9_engine::MinioObjectStore;
use kv9_meta::retention::{checkpoint_reader, LedgerOwner, OwnerBinding, OwnerKind};

#[derive(Debug, Default)]
pub(super) struct ReadCount {
    memory: MemoryObjectStore,
    pub(super) gets: AtomicUsize,
}
impl ObjectStore for ReadCount {
    fn put(&self, k: &ObjectKey, b: &[u8]) -> Result<()> {
        self.memory.put(k, b)
    }
    fn get(&self, k: &ObjectKey) -> Result<Option<Vec<u8>>> {
        self.gets.fetch_add(1, Ordering::SeqCst);
        self.memory.get(k)
    }
    fn delete(&self, _: &ObjectKey) -> Result<()> {
        panic!("no GC")
    }
    fn list(&self, _: &str) -> Result<Vec<ObjectKey>> {
        panic!("no listing")
    }
}
pub(super) async fn published(f: &Fixture) -> (RootDescriptor, CheckpointManifest, Arc<ReadCount>) {
    assert!(matches!(
        f.client(0).activate().await.outcome,
        TtlOutcome::Success { .. }
    ));
    let root = f.backends[0].ttl_context().unwrap().root.clone();
    let r = root.clone();
    let engine = engine(f, 0);
    let driver = f.data[0].clone();
    let region = f.range.region;
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    let store = Arc::new(ReadCount::default());
    let memory = store.clone();
    let manifest = tokio::task::spawn_blocking(move || {
        let uploader = RemoteUploader::for_harness(memory);
        let plan = RemoteUploader::plan(
            engine
                .freeze(kv9_engine::checkpoint::FlushScope {
                    cluster: r.cluster_id.to_string(),
                    region: region.0,
                    conf_ver: 1,
                    version: 1,
                })
                .unwrap(),
        )
        .unwrap();
        let manifest = plan.manifest().clone();
        let owners = CheckpointOwners::new(&r, &manifest, 0).unwrap();
        owners.before_io(&ledger).unwrap();
        let prepared = uploader.upload_planned(plan).unwrap();
        let at = driver
            .propose(&kv9_raft::Command::ManifestChange(
                kv9_raft::ManifestChangePayload::from_prepared(prepared, 0).unwrap(),
            ))
            .unwrap();
        assert!(matches!(
            driver.wait_applied(at, Duration::from_secs(3)).unwrap(),
            kv9_raft::driver::ApplyWaitOutcome::Manifest {
                verdict: kv9_raft::ManifestVerdict::Applied { .. },
                ..
            }
        ));
        owners.after_positive_settlement(&ledger).unwrap();
        manifest
    })
    .await
    .unwrap();
    until(|| {
        f.data
            .iter()
            .all(|d| d.manifest_pair(f.range.region.0).unwrap().generation == 1)
    })
    .await;
    (root, manifest, store)
}
fn reader(f: &Fixture, index: usize, r: &RootDescriptor, m: &CheckpointManifest) -> OwnerBinding {
    let source = kv9_meta::retention::checkpoint_intent::CheckpointIntent::new(r, m, 0)
        .unwrap()
        .binding(OwnerKind::Version)
        .unwrap();
    checkpoint_reader::binding(&source, f.backends[index].ttl_context().unwrap().lifetime).unwrap()
}
fn observed(f: &Fixture, r: &RootDescriptor, b: &OwnerBinding) -> Option<LedgerOwner> {
    kv9_meta::retention::retention_owner(
        f.backends[1]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap()
            .as_ref(),
        r,
        b.descriptor.id,
    )
    .unwrap()
}
pub(super) fn task(f: &Fixture, index: usize, uploader: Arc<RemoteUploader>) -> Maintenance {
    Maintenance::start(
        f.backends[index].clone(),
        uploader,
        false,
        Duration::from_millis(100),
        &tokio::runtime::Handle::current(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn adoption_reader_keeps_pin_and_permit_until_timed_out_get_really_exits() {
    let f = Fixture::new().await;
    let (root, manifest, _) = published(&f).await;
    let binding = reader(&f, 2, &root, &manifest);
    let (store, pause) =
        MinioObjectStore::with_paused_memory_worker(Duration::from_millis(100)).unwrap();
    let store = Arc::new(store);
    let work = task(&f, 2, Arc::new(RemoteUploader::new(store.clone())));
    let pause = tokio::task::spawn_blocking(move || {
        assert!(pause.wait_for_closed_execution(Duration::from_secs(10)));
        pause
    })
    .await
    .unwrap();
    assert_eq!(
        observed(&f, &root, &binding).unwrap().phase,
        PinPhase::Published
    );
    assert_eq!(f.backends[2].maintenance.snapshot().heavy_active, 1);
    assert_eq!(work.snapshot().failures, 0);
    work.cancel();
    let drain = tokio::spawn(work.drain());
    tokio::task::yield_now().await;
    assert!(!drain.is_finished());
    assert_eq!(
        observed(&f, &root, &binding).unwrap().phase,
        PinPhase::Published
    );
    drop(pause);
    tokio::time::timeout(Duration::from_secs(5), drain)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        observed(&f, &root, &binding).unwrap().phase,
        PinPhase::Released
    );
    assert_eq!(f.backends[2].maintenance.snapshot().active, 0);
    assert!(selected(&f, 2).is_none());
    assert!(!f.backends[2]
        .ttl_context()
        .unwrap()
        .drain_outbox
        .as_ref()
        .unwrap()
        .has_pending());
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn uncertain_reader_entry_issues_no_get_and_blocks_new_reads_until_exact_finish() {
    let f = Fixture::new().await;
    let (root, manifest, store) = published(&f).await;
    let binding = reader(&f, 0, &root, &manifest);
    let before = store.gets.load(Ordering::SeqCst);
    f.hooks[1].retention_calls.store(0, Ordering::SeqCst);
    f.hooks[1].retention_commits.store(0, Ordering::SeqCst);
    f.hooks[1].retention_fail_after.store(3, Ordering::SeqCst);
    f.hooks[1].retention_refuse_from.store(4, Ordering::SeqCst);
    let work = task(&f, 0, Arc::new(RemoteUploader::for_harness(store.clone())));
    until(|| work.snapshot().failures > 0).await;
    assert_eq!(observed(&f, &root, &binding).unwrap().phase, PinPhase::Held);
    let calls = f.hooks[1].retention_calls.load(Ordering::SeqCst);
    until(|| f.hooks[1].retention_calls.load(Ordering::SeqCst) >= calls + 3).await;
    assert_eq!(store.gets.load(Ordering::SeqCst), before);
    assert!(selected(&f, 0).is_none());
    f.hooks[1].retention_refuse_from.store(0, Ordering::SeqCst);
    f.hooks[1].retention_fail_after.store(0, Ordering::SeqCst);
    until(|| selected(&f, 0).is_some()).await;
    work.drain().await;
    let observed = observed(&f, &root, &binding).unwrap();
    assert_eq!(
        (observed.phase, observed.binding.generation.get()),
        (PinPhase::Released, 2)
    );
    assert!(store.gets.load(Ordering::SeqCst) > before);
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn read_finish_outbox_survives_lost_receipt_new_process_new_leader_and_wal_but_missing_report_stays_pinned(
) {
    let mut f = Fixture::new().await;
    let (root, manifest, store) = published(&f).await;
    let binding = reader(&f, 0, &root, &manifest);
    f.hooks[1].retention_read_fault.store(2, Ordering::SeqCst);
    let work = task(&f, 0, Arc::new(RemoteUploader::for_harness(store.clone())));
    until(|| work.snapshot().failures > 0).await;
    assert_eq!(
        observed(&f, &root, &binding).unwrap().phase,
        PinPhase::Released
    );
    let gets = store.gets.load(Ordering::SeqCst);
    let calls = f.hooks[1].retention_calls.load(Ordering::SeqCst);
    until(|| f.hooks[1].retention_calls.load(Ordering::SeqCst) >= calls + 3).await;
    assert_eq!(store.gets.load(Ordering::SeqCst), gets);
    work.drain().await;
    let path = f.path.join("n1/checkpoint-drains");
    assert_eq!(std::fs::read_dir(&path).unwrap().count(), 1);
    // A different process scope loses its report entirely. Local exit is not
    // permission to clear its durable Reader, even though these GETs completed.
    let missing = reader(&f, 2, &root, &manifest);
    let ledger = RoutedLedger::new(&f.backends[2], tokio::runtime::Handle::current());
    let r = root.clone();
    let m = manifest.clone();
    let s = store.clone();
    tokio::task::spawn_blocking(move || {
        let mut read = CheckpointRead::new(&r, &m, 0, OwnerKind::Version, &ledger).unwrap();
        let uploader = RemoteUploader::for_harness(s);
        let (result, _unrecorded_report) = uploader
            .with_local_read_report(|scoped| {
                read.before_io(&ledger, scoped)?;
                scoped.restore(&m)
            })
            .unwrap();
        result.unwrap();
        drop(read);
    })
    .await
    .unwrap();
    assert_eq!(
        observed(&f, &root, &missing).unwrap().phase,
        PinPhase::Published
    );
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
    until(|| f.meta[2].status().role == Role::Leader && f.meta[1].status().role != Role::Leader)
        .await;
    let queue = f.backends[0]
        .ttl_context()
        .unwrap()
        .drain_outbox
        .clone()
        .unwrap();
    let task = crate::runtime::checkpoint_sweep::Maintenance::start(
        f.backends[0].clone(),
        &tokio::runtime::Handle::current(),
    );
    until(|| !queue.has_pending()).await;
    assert_eq!(task.snapshot().discovered, 0);
    task.drain().await;
    assert_eq!(std::fs::read_dir(&path).unwrap().count(), 0);
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
    for (b, phase) in [
        (binding, PinPhase::Released),
        (missing, PinPhase::Published),
    ] {
        let owner = kv9_meta::retention::retention_owner(view.as_ref(), &root, b.descriptor.id)
            .unwrap()
            .unwrap();
        assert_eq!(owner.binding, b);
        assert_eq!(owner.phase, phase);
    }
    let version = CheckpointOwners::new(&root, &manifest, 0)
        .unwrap()
        .version_id();
    assert_eq!(
        kv9_meta::retention::retention_owner(view.as_ref(), &root, version)
            .unwrap()
            .unwrap()
            .phase,
        PinPhase::Published
    );
    drop(view);
    drop(engine);
    f.close().await;
}
