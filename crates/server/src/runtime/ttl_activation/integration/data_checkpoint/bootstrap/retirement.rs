use super::*;
use crate::bootstrap_retention::{after_selection, before_selection, fenced_image};
use crate::checkpoint_retention::CheckpointOwners;
use kv9_meta::retention::{checkpoint_intent::CheckpointIntent, encode_request, OwnerBinding};

pub(super) async fn publish(
    f: &Fixture,
    uploader: Arc<RemoteUploader>,
) -> (CheckpointManifest, u64) {
    let backend = f.backends[1].clone();
    let root = backend.ttl_context().unwrap().root.clone();
    let ledger = RoutedLedger::new(&backend, tokio::runtime::Handle::current());
    let (manifest, predecessor) = tokio::task::spawn_blocking(move || {
        let predecessor = backend
            .driver
            .manifest_pair(META_REGION_0.0)
            .unwrap()
            .generation;
        let frozen = backend
            .node
            .meta_raft
            .store
            .engine()
            .freeze_with_scope(|view| {
                Ok(
                    kv9_meta::checkpoint::inspect_initial_checkpoint_base(view, &root)?
                        .flush_scope(),
                )
            })
            .unwrap();
        let plan = RemoteUploader::plan(frozen).unwrap();
        let manifest = plan.manifest().clone();
        let owners = CheckpointOwners::new(&root, &manifest, predecessor).unwrap();
        owners.before_io(&ledger).unwrap();
        let prepared = uploader.upload_planned(plan).unwrap();
        let at = backend
            .driver
            .propose(&kv9_raft::Command::ManifestChange(
                kv9_raft::ManifestChangePayload::from_prepared(prepared, predecessor).unwrap(),
            ))
            .unwrap();
        assert!(matches!(
            backend
                .driver
                .wait_applied(at, Duration::from_secs(3))
                .unwrap(),
            kv9_raft::driver::ApplyWaitOutcome::Manifest {
                verdict: kv9_raft::ManifestVerdict::Applied { .. },
                ..
            }
        ));
        owners.after_positive_settlement(&ledger).unwrap();
        (manifest, predecessor)
    })
    .await
    .unwrap();
    until(|| {
        f.meta
            .iter()
            .all(|d| d.manifest_pair(META_REGION_0.0).unwrap().generation > predecessor)
    })
    .await;
    (manifest, predecessor)
}
pub(super) async fn select(
    f: &Fixture,
    manifest: CheckpointManifest,
    predecessor: u64,
    ledger: Arc<dyn CheckpointLedger>,
) -> Result<()> {
    let b = f.backends[0].clone();
    let directory = f.path.join("n1");
    let root = b.ttl_context().unwrap().root.clone();
    tokio::task::spawn_blocking(move || {
        let engine = b.node.meta_raft.store.engine();
        before_selection(
            &directory,
            &root,
            ledger.as_ref(),
            engine.selected_checkpoint()?.as_ref(),
            &manifest,
            predecessor,
            |p| {
                let (_, _, bytes) = b
                    .driver
                    .manifest_at(META_REGION_0.0, p + 1)?
                    .ok_or_else(|| Error::Engine("test history absent".into()))?;
                CheckpointManifest::decode(&bytes)
            },
        )?;
        engine.checkpoint_applied(&manifest)?;
        after_selection(
            &directory,
            &root,
            ledger.as_ref(),
            engine.selected_checkpoint()?.as_ref(),
        )
    })
    .await
    .unwrap()
}
async fn send(ledger: Arc<dyn CheckpointLedger>, bytes: Vec<u8>) -> Result<()> {
    tokio::task::spawn_blocking(move || ledger.apply(bytes))
        .await
        .unwrap()
}
pub(super) fn owner(f: &Fixture, root: &RootDescriptor, binding: &OwnerBinding) -> PinPhase {
    let leader = f
        .meta
        .iter()
        .position(|d| d.status().role == Role::Leader)
        .unwrap();
    kv9_meta::retention::retention_owner(
        f.backends[leader]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap()
            .as_ref(),
        root,
        binding.descriptor.id,
    )
    .unwrap()
    .unwrap()
    .phase
}
struct Fault {
    inner: RoutedLedger,
    lose: AtomicBool,
    saved: Mutex<Option<Vec<u8>>>,
}
impl CheckpointLedger for Fault {
    fn apply(&self, bytes: Vec<u8>) -> Result<()> {
        let is_retire = bytes.get(40) == Some(&23);
        self.inner.apply(bytes.clone())?;
        if is_retire && self.lose.load(Ordering::SeqCst) {
            *self.saved.lock().unwrap() = Some(bytes);
            return Err(Error::MetaNotReady(
                "lost committed bootstrap retirement".into(),
            ));
        }
        Ok(())
    }
    fn owner(&self, root: RootDigest, id: OwnerId) -> Result<Option<Vec<u8>>> {
        self.inner.owner(root, id)
    }
    fn local_lifetime(&self) -> Result<LocalLifetime> {
        self.inner.local_lifetime()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn retirement_requires_independent_fence_rechecks_endpoint_and_replays_after_failover_and_reopen(
) {
    let f = Fixture::new().await;
    let root = f.backends[0].ttl_context().unwrap().root.clone();
    let identity = StoreIdentity::for_voter(&root, NodeId(1)).unwrap();
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
    let grant = bootstrap_reader::binding(&root, &first, p, identity).unwrap();
    let version = intent.binding(OwnerKind::Version).unwrap();
    let request = LedgerRequest::RetireBootstrapRead {
        pending: intent.binding(OwnerKind::Pending).unwrap().token(),
        source: identity.node_id,
        store: identity.store_incarnation,
    };
    let bytes = encode_request(root.digest(), &request).unwrap();
    assert!(send(ledger.clone(), bytes.clone()).await.is_err()); // Still admitted and selected.
    assert_eq!(owner(&f, &root, &grant), PinPhase::Published);
    let (second, q) = publish(&f, uploader.clone()).await;
    f.hooks[0].bootstrap_unknown.store(true, Ordering::SeqCst);
    assert!(select(&f, second.clone(), q, ledger.clone()).await.is_err());
    assert_eq!(owner(&f, &root, &grant), PinPhase::Published);
    f.hooks[0].bootstrap_unknown.store(false, Ordering::SeqCst);
    f.hooks[0].bootstrap_refuse.store(true, Ordering::SeqCst);
    assert!(send(ledger.clone(), bytes.clone()).await.is_err());
    f.hooks[0].bootstrap_refuse.store(false, Ordering::SeqCst);
    // Independently reject a checksum-valid local fence naming another
    // Pending, even though its image digest and predecessor still match.
    let receipt = f.path.join("n1/catalog.bootstrap-reads");
    let saved = std::fs::read(&receipt).unwrap();
    let mut corrupt = saved.clone();
    let offset = corrupt
        .windows(16)
        .position(|w| w == intent.pending_id().as_bytes())
        .unwrap();
    corrupt[offset] ^= 1;
    let end = corrupt.len() - 32;
    let hash = RootDigest::sha256(&corrupt[..end]);
    corrupt[end..].copy_from_slice(hash.as_bytes());
    std::fs::write(&receipt, corrupt).unwrap();
    assert!(send(ledger.clone(), bytes.clone()).await.is_err());
    assert_eq!(owner(&f, &root, &grant), PinPhase::Published);
    std::fs::write(&receipt, saved).unwrap();
    let mut wrong = request.clone();
    if let LedgerRequest::RetireBootstrapRead { store, .. } = &mut wrong {
        *store = kv9_common::StoreIncarnation::from_bytes([99; 16]);
    }
    assert!(send(
        ledger.clone(),
        encode_request(root.digest(), &wrong).unwrap()
    )
    .await
    .is_err());
    *f.hooks[0].bootstrap_change_endpoint.lock().unwrap() = Some(f.backends[1].clone());
    assert!(send(ledger.clone(), bytes.clone()).await.is_err());
    assert!(f.hooks[0]
        .bootstrap_change_endpoint
        .lock()
        .unwrap()
        .is_none());
    assert_eq!(owner(&f, &root, &grant), PinPhase::Published);
    ledger.lose.store(true, Ordering::SeqCst);
    assert!(select(&f, second.clone(), q, ledger.clone()).await.is_err());
    assert_eq!(ledger.saved.lock().unwrap().as_ref(), Some(&bytes));
    assert_eq!(owner(&f, &root, &grant), PinPhase::Released);
    assert_eq!(owner(&f, &root, &version), PinPhase::Published);
    let directory = f.path.join("n1");
    assert_eq!(
        fenced_image(&directory, identity, p, Some(&second))
            .unwrap()
            .map(|f| f.image),
        Some(*RootDigest::sha256(&first.encode().unwrap()).as_bytes())
    );
    let gets = objects.gets.load(Ordering::SeqCst);
    use kv9_engine::checkpoint::CheckpointRestore;
    assert!(BootstrapReader {
        directory: &directory,
        root: &root,
        identity,
        uploader: &uploader
    }
    .restore(&first)
    .is_err());
    assert_eq!(objects.gets.load(Ordering::SeqCst), gets);
    f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
    until(|| f.meta[2].status().role == Role::Leader).await;
    f.hooks[0].bootstrap_refuse.store(true, Ordering::SeqCst);
    let queries = f.hooks[0].bootstrap_reads.load(Ordering::SeqCst);
    ledger.lose.store(false, Ordering::SeqCst);
    select(&f, second.clone(), q, ledger.clone()).await.unwrap();
    assert_eq!(f.hooks[0].bootstrap_reads.load(Ordering::SeqCst), queries);
    assert!(fenced_image(&directory, identity, p, Some(&second))
        .unwrap()
        .is_none());
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
        let mut storage = DiskRaftStorage::recover(&directory.join("meta-raft")).unwrap();
        let _guard = StoreGuard::lock(&directory).unwrap();
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
        assert_eq!(
            kv9_meta::retention::retention_owner(view.as_ref(), &root, grant.descriptor.id)
                .unwrap()
                .unwrap()
                .phase,
            PinPhase::Released
        );
        assert_eq!(
            kv9_meta::retention::retention_owner(view.as_ref(), &root, version.descriptor.id)
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn metadata_worker_replays_older_publication_without_grants_and_still_adopts_the_next_image()
{
    let f = Fixture::new().await;
    let root = f.backends[0].ttl_context().unwrap().root.clone();
    let uploader = Arc::new(RemoteUploader::for_harness(Arc::new(Objects::default())));
    let ledger = Arc::new(Fault {
        inner: RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current()),
        lose: AtomicBool::new(false),
        saved: Mutex::default(),
    });
    let (first, p) = publish(&f, uploader.clone()).await;
    select(&f, first.clone(), p, ledger.clone()).await.unwrap();
    let (second, q) = publish(&f, uploader.clone()).await;
    ledger.lose.store(true, Ordering::SeqCst);
    assert!(select(&f, second.clone(), q, ledger.clone()).await.is_err());
    let r = root.clone();
    let b = f.backends[1].clone();
    let old = first.clone();
    let old_ledger = RoutedLedger::new(&b, tokio::runtime::Handle::current());
    let u = uploader.clone();
    let older_predecessor = tokio::task::spawn_blocking(move || {
        let p = b.driver.manifest_pair(META_REGION_0.0).unwrap().generation;
        let owners = CheckpointOwners::new(&r, &old, p).unwrap();
        owners.before_io(&old_ledger).unwrap();
        u.verify_checkpoint_objects(&old).unwrap();
        // Explicit harness republication of the exact original, verified image.
        let command =
            kv9_raft::Command::ManifestChange(kv9_raft::ManifestChangePayload::for_harness(
                META_REGION_0.0,
                old.change_id(p).unwrap(),
                p,
                old.encode().unwrap(),
                old.term,
                old.index,
            ));
        let at = b.driver.propose(&command).unwrap();
        assert!(matches!(
            b.driver.wait_applied(at, Duration::from_secs(3)).unwrap(),
            kv9_raft::driver::ApplyWaitOutcome::Manifest {
                verdict: kv9_raft::ManifestVerdict::Applied { .. },
                ..
            }
        ));
        owners.after_positive_settlement(&old_ledger).unwrap();
        p
    })
    .await
    .unwrap();
    until(|| f.meta[0].manifest_pair(META_REGION_0.0).unwrap().generation == older_predecessor + 1)
        .await;
    let b = f.backends[0].clone();
    let dir = f.path.join("n1");
    let mut worker = crate::remote_storage::checkpoint_passes_for_test(
        b.node.clone(),
        b.driver.clone(),
        PreparedRemote::for_test(&dir, &root, uploader.clone()),
        ledger.clone(),
    )
    .unwrap();
    let (next_worker, result) = tokio::task::spawn_blocking(move || {
        let result = worker();
        (worker, result)
    })
    .await
    .unwrap();
    worker = next_worker;
    assert!(
        result.is_err(),
        "retirement debt must run even for an older publication"
    );
    ledger.lose.store(false, Ordering::SeqCst);
    for _ in 0..3 {
        let (next_worker, generation) = tokio::task::spawn_blocking(move || {
            let generation = worker();
            (worker, generation)
        })
        .await
        .unwrap();
        worker = next_worker;
        assert_eq!(generation.unwrap(), older_predecessor + 1);
        assert_eq!(
            f.backends[0]
                .node
                .meta_raft
                .store
                .engine()
                .selected_checkpoint()
                .unwrap(),
            Some(second.clone())
        );
    }
    assert!(fenced_image(
        &f.path.join("n1"),
        StoreIdentity::for_voter(&root, NodeId(1)).unwrap(),
        p,
        Some(&second)
    )
    .unwrap()
    .is_none());
    let unneeded = bootstrap_reader::binding(
        &root,
        &first,
        older_predecessor,
        StoreIdentity::for_voter(&root, NodeId(1)).unwrap(),
    )
    .unwrap();
    assert!(kv9_meta::retention::retention_owner(
        f.backends[1]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap()
            .as_ref(),
        &root,
        unneeded.descriptor.id
    )
    .unwrap()
    .is_none());
    let (next, predecessor) = publish(&f, uploader.clone()).await;
    assert_eq!(
        tokio::task::spawn_blocking(worker).await.unwrap().unwrap(),
        predecessor + 1
    );
    assert_eq!(
        f.backends[0]
            .node
            .meta_raft
            .store
            .engine()
            .selected_checkpoint()
            .unwrap(),
        Some(next)
    );
    f.close().await;
}

mod acknowledgement;
