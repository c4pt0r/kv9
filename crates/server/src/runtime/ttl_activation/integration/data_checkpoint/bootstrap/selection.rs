use super::*;
use kv9_meta::retention::{
    checkpoint_intent::CheckpointIntent,
    checkpoint_selection::{self, Selection},
    encode_request,
};

fn bytes(root: &RootDescriptor, intent: &CheckpointIntent, source: usize) -> Vec<u8> {
    let identity = StoreIdentity::for_voter(root, NodeId(source as u64 + 1)).unwrap();
    encode_request(
        root.digest(),
        &LedgerRequest::RecordCheckpointSelection {
            pending: intent.binding(OwnerKind::Pending).unwrap().token(),
            source: identity.node_id,
            store: identity.store_incarnation,
        },
    )
    .unwrap()
}
fn fact(f: &Fixture, root: &RootDescriptor, leader: usize, source: usize) -> Option<Selection> {
    let identity = StoreIdentity::for_voter(root, NodeId(source as u64 + 1)).unwrap();
    checkpoint_selection::selection(
        f.backends[leader]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap()
            .as_ref(),
        root,
        identity.node_id,
        identity.store_incarnation,
        META_REGION_0,
    )
    .unwrap()
}
async fn send(f: &Fixture, bytes: Vec<u8>) -> Result<()> {
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    tokio::task::spawn_blocking(move || ledger.apply(bytes))
        .await
        .unwrap()
}
pub(super) async fn select(
    f: &Fixture,
    source: usize,
    manifest: CheckpointManifest,
    predecessor: u64,
) {
    try_select(f, source, manifest, predecessor).await.unwrap();
}
pub(super) async fn try_select(
    f: &Fixture,
    source: usize,
    manifest: CheckpointManifest,
    predecessor: u64,
) -> Result<()> {
    let b = f.backends[source].clone();
    let directory = f.path.join(format!("n{}", source + 1));
    let root = b.ttl_context().unwrap().root.clone();
    let ledger = RoutedLedger::new(&b, tokio::runtime::Handle::current());
    tokio::task::spawn_blocking(move || {
        let engine = b.node.meta_raft.store.engine();
        crate::bootstrap_retention::before_selection(
            &directory,
            &root,
            &ledger,
            engine.selected_checkpoint().unwrap().as_ref(),
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
        crate::bootstrap_retention::after_selection(
            &directory,
            &root,
            &ledger,
            engine.selected_checkpoint().unwrap().as_ref(),
        )
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn metadata_selection_requires_actual_selector_authorized_grant_and_endpoint_recheck() {
    let f = Fixture::new().await;
    let root = f.backends[0].ttl_context().unwrap().root.clone();
    let objects = Arc::new(Objects::default());
    let uploader = Arc::new(RemoteUploader::for_harness(objects));
    let (manifest, predecessor) = retirement::publish(&f, uploader).await;
    let intent = CheckpointIntent::new(&root, &manifest, predecessor).unwrap();
    let request = bytes(&root, &intent, 0);
    // Committed metadata history alone cannot establish local recovery selection.
    assert!(send(&f, request.clone()).await.is_err());
    assert!(fact(&f, &root, 1, 0).is_none());
    f.backends[0]
        .node
        .meta_raft
        .store
        .engine()
        .checkpoint_applied(&manifest)
        .unwrap();
    // A selector without its locally authorized bootstrap receipt is also insufficient.
    assert!(send(&f, request.clone()).await.is_err());
    select(&f, 0, manifest.clone(), predecessor).await;
    let mut wrong = decode_request(&request, root.digest()).unwrap();
    if let LedgerRequest::RecordCheckpointSelection { store, .. } = &mut wrong {
        *store = kv9_common::StoreIncarnation::from_bytes([99; 16]);
    }
    assert!(send(&f, encode_request(root.digest(), &wrong).unwrap())
        .await
        .is_err());
    for unknown in [true, false] {
        f.hooks[0]
            .selection_unknown
            .store(unknown, Ordering::SeqCst);
        f.hooks[0]
            .selection_refuse
            .store(!unknown, Ordering::SeqCst);
        assert!(send(&f, request.clone()).await.is_err());
        assert!(fact(&f, &root, 1, 0).is_none());
    }
    f.hooks[0].selection_refuse.store(false, Ordering::SeqCst);
    *f.hooks[0].selection_change_endpoint.lock().unwrap() = Some(f.backends[1].clone());
    assert!(send(&f, request.clone()).await.is_err());
    assert!(fact(&f, &root, 1, 0).is_none());
    send(&f, request).await.unwrap();
    let selected = fact(&f, &root, 1, 0).unwrap();
    assert_eq!(selected.intent(), &intent);
    assert_eq!(selected.endpoint_generation, 1);
    let identity = StoreIdentity::for_voter(&root, NodeId(1)).unwrap();
    let grant = bootstrap_reader::binding(&root, &manifest, predecessor, identity).unwrap();
    assert_eq!(selected.bootstrap, Some(grant.token()));
    assert!(send(&f, bytes(&root, &intent, 2)).await.is_err());
    select(&f, 2, manifest.clone(), predecessor).await;
    f.backends[2]
        .node
        .meta_raft
        .store
        .engine()
        .checkpoint_history()
        .unwrap()
        .with_compaction(manifest.position(), || {
            Err::<(), _>(Error::Engine(
                "injected uncertain metadata compaction".into(),
            ))
        })
        .unwrap_err();
    assert!(send(&f, bytes(&root, &intent, 2)).await.is_err());
    assert!(fact(&f, &root, 1, 2).is_none());
    assert_eq!(
        retirement::owner(&f, &root, &intent.binding(OwnerKind::Version).unwrap()),
        PinPhase::Published
    );
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn metadata_worker_retries_lost_selection_without_io_and_reconstructs_report_after_cache_loss(
) {
    let f = Fixture::new().await;
    let root = f.backends[0].ttl_context().unwrap().root.clone();
    let objects = Arc::new(Objects::default());
    let uploader = Arc::new(RemoteUploader::for_harness(objects.clone()));
    let (manifest, predecessor) = retirement::publish(&f, uploader.clone()).await;
    let intent = CheckpointIntent::new(&root, &manifest, predecessor).unwrap();
    f.hooks[1]
        .retention_selection_fault
        .store(2, Ordering::SeqCst);
    let b = &f.backends[0];
    let mut work = crate::remote_storage::checkpoint_passes_for_test(
        b.node.clone(),
        b.driver.clone(),
        PreparedRemote::for_test(&f.path.join("n1"), &root, uploader),
        Arc::new(RoutedLedger::new(b, tokio::runtime::Handle::current())),
    )
    .unwrap();
    let (next, result) = tokio::task::spawn_blocking(move || {
        let result = work();
        (work, result)
    })
    .await
    .unwrap();
    work = next;
    assert!(result.is_err());
    assert_eq!(
        f.hooks[1].retention_selection_fault.load(Ordering::SeqCst),
        1
    );
    let original = fact(&f, &root, 1, 0).unwrap();
    let gets = objects.gets.load(Ordering::SeqCst);
    let puts = objects.puts.load(Ordering::SeqCst);
    for _ in 0..2 {
        let calls = f.hooks[1].retention_calls.load(Ordering::SeqCst);
        let (next, result) = tokio::task::spawn_blocking(move || {
            let result = work();
            (work, result)
        })
        .await
        .unwrap();
        work = next;
        assert!(result.is_err());
        assert!(f.hooks[1].retention_calls.load(Ordering::SeqCst) > calls);
        assert_eq!(objects.gets.load(Ordering::SeqCst), gets);
        assert_eq!(objects.puts.load(Ordering::SeqCst), puts);
    }
    f.hooks[0].selection_refuse.store(true, Ordering::SeqCst);
    let reads = f.hooks[0].selection_reads.load(Ordering::SeqCst);
    f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
    until(|| f.meta[2].status().role == Role::Leader && f.meta[1].status().role != Role::Leader)
        .await;
    // Both the lost receipt and loss of a subsequently confirmed report cache
    // reconstruct the same request from the durable selector and local receipt.
    for _ in 0..2 {
        let commits = f.hooks[2].retention_commits.load(Ordering::SeqCst);
        let (next, result) = tokio::task::spawn_blocking(move || {
            let result = work();
            (work, result)
        })
        .await
        .unwrap();
        work = next;
        assert_eq!(result.unwrap(), predecessor + 1);
        assert!(f.hooks[2].retention_commits.load(Ordering::SeqCst) > commits);
    }
    drop(work);
    assert_eq!(fact(&f, &root, 2, 0), Some(original.clone()));
    assert_eq!(f.hooks[0].selection_reads.load(Ordering::SeqCst), reads);
    assert_eq!(objects.gets.load(Ordering::SeqCst), gets);
    assert_eq!(objects.puts.load(Ordering::SeqCst), puts);
    assert!(send(&f, bytes(&root, &intent, 1)).await.is_err());
    let directory = f.shutdown_keep().await;
    let engine = WalEngine::open(directory.join("n3/meta.wal")).unwrap().0;
    let view = engine.snapshot().unwrap();
    let identity = StoreIdentity::for_voter(&root, NodeId(1)).unwrap();
    assert_eq!(
        checkpoint_selection::selection(
            view.as_ref(),
            &root,
            identity.node_id,
            identity.store_incarnation,
            META_REGION_0
        )
        .unwrap(),
        Some(original)
    );
    for binding in [
        intent.binding(OwnerKind::Version).unwrap(),
        bootstrap_reader::binding(&root, &manifest, predecessor, identity).unwrap(),
    ] {
        assert_eq!(
            kv9_meta::retention::retention_owner(view.as_ref(), &root, binding.descriptor.id)
                .unwrap()
                .unwrap()
                .phase,
            PinPhase::Published
        );
    }
    drop(view);
    drop(engine);
    std::fs::remove_dir_all(directory).unwrap();
}
