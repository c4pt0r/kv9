use super::*;
use crate::checkpoint_retention::CheckpointLedger;
use crate::runtime::checkpoint_retention::RoutedLedger;
use kv9_meta::retention::{
    checkpoint_intent::CheckpointIntent,
    data_read_fence::{self, Observation},
    encode_request, LedgerRequest, OwnerKind,
};

fn request(
    f: &Fixture,
    root: &RootDescriptor,
    manifest: &CheckpointManifest,
    source: usize,
) -> Vec<u8> {
    let identity = StoreIdentity::for_voter(root, NodeId(source as u64 + 1)).unwrap();
    encode_request(
        root.digest(),
        &LedgerRequest::RecordDataReadFence {
            pending: CheckpointIntent::new(root, manifest, 0)
                .unwrap()
                .binding(OwnerKind::Pending)
                .unwrap()
                .token(),
            source: identity.node_id,
            store: identity.store_incarnation,
            creation: f.range.creation,
        },
    )
    .unwrap()
}
fn fact(f: &Fixture, root: &RootDescriptor, leader: usize, source: usize) -> Option<Observation> {
    let identity = StoreIdentity::for_voter(root, NodeId(source as u64 + 1)).unwrap();
    data_read_fence::observation(
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
        f.range.region,
    )
    .unwrap()
}
async fn send(f: &Fixture, bytes: Vec<u8>) -> Result<()> {
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    tokio::task::spawn_blocking(move || ledger.apply(bytes))
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn independent_data_fence_rejects_selection_only_unknown_and_changed_endpoint() {
    let f = Fixture::new().await;
    let (root, manifest, store) = readers::published(&f).await;
    let bytes = request(&f, &root, &manifest, 0);
    engine(&f, 0).checkpoint_applied(&manifest).unwrap();
    assert!(
        send(&f, bytes.clone()).await.is_err(),
        "selection is not durable future read admission"
    );
    assert!(fact(&f, &root, 1, 0).is_none());
    f.hooks[0].data_fence_unknown.store(true, Ordering::SeqCst);
    let work = readers::task(&f, 0, Arc::new(RemoteUploader::for_harness(store)));
    until(|| {
        f.backends[0]
            .ttl_context()
            .unwrap()
            .data_read_fences
            .floor(f.range.region)
            .unwrap()
            .is_some()
            && work.snapshot().failures > 0
    })
    .await;
    work.cancel();
    work.drain().await;
    assert!(fact(&f, &root, 1, 0).is_none());
    assert!(send(&f, bytes.clone()).await.is_err());
    let backend = f.backends[1].clone();
    let raw = bytes.clone();
    assert!(
        tokio::task::spawn_blocking(move || backend.apply_retention("test", raw))
            .await
            .unwrap()
            .is_err()
    );
    f.hooks[0].data_fence_unknown.store(false, Ordering::SeqCst);
    f.hooks[0].data_fence_refuse.store(true, Ordering::SeqCst);
    assert!(send(&f, bytes.clone()).await.is_err());
    f.hooks[0].data_fence_refuse.store(false, Ordering::SeqCst);
    *f.hooks[0].data_fence_change_endpoint.lock().unwrap() = Some(f.backends[1].clone());
    assert!(send(&f, bytes.clone()).await.is_err());
    assert!(fact(&f, &root, 1, 0).is_none());
    send(&f, bytes).await.unwrap();
    let observed = fact(&f, &root, 1, 0).unwrap();
    assert_eq!(observed.endpoint_generation, 1);
    assert_eq!(
        Some(observed.floor),
        f.backends[0]
            .ttl_context()
            .unwrap()
            .data_read_fences
            .floor(f.range.region)
            .unwrap()
    );
    assert!(
        send(&f, request(&f, &root, &manifest, 2)).await.is_err(),
        "another store cannot borrow the observation"
    );
    let version = CheckpointIntent::new(&root, &manifest, 0)
        .unwrap()
        .binding(OwnerKind::Version)
        .unwrap();
    assert_eq!(
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
            version.descriptor.id
        )
        .unwrap()
        .unwrap()
        .phase,
        kv9_common::retention::PinPhase::Published
    );
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn data_fence_lost_receipt_retries_without_get_and_survives_failover_removed_handles_and_wal()
{
    let f = Fixture::new().await;
    let (root, manifest, store) = readers::published(&f).await;
    f.hooks[1]
        .retention_data_fence_fault
        .store(2, Ordering::SeqCst);
    let work = readers::task(&f, 0, Arc::new(RemoteUploader::for_harness(store.clone())));
    until(|| fact(&f, &root, 1, 0).is_some() && work.snapshot().failures > 0).await;
    let original = fact(&f, &root, 1, 0).unwrap();
    let gets = store.gets.load(Ordering::SeqCst);
    let failures = work.snapshot().failures;
    until(|| work.snapshot().failures >= failures + 2).await;
    assert_eq!(
        store.gets.load(Ordering::SeqCst),
        gets,
        "confirmation retry must not repeat image GET"
    );
    work.cancel();
    work.drain().await;
    let request = request(&f, &root, &manifest, 0);
    let region = f.range.region;
    for b in &f.backends {
        b.raw_directory.remove(region);
        b.group_handles.lock().unwrap().remove(&region);
    }
    // The actual local fence remains observable without a running DATA group.
    assert_eq!(
        f.backends[0]
            .observe_data_read_fence(
                root.digest(),
                region,
                f.range.creation,
                Instant::now() + Duration::from_secs(1)
            )
            .unwrap(),
        Some(original.floor)
    );
    f.hooks[0].data_fence_refuse.store(true, Ordering::SeqCst);
    let reads = f.hooks[0].data_fence_reads.load(Ordering::SeqCst);
    f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
    until(|| f.meta[2].status().role == Role::Leader && f.meta[1].status().role != Role::Leader)
        .await;
    send(&f, request).await.unwrap();
    assert_eq!(fact(&f, &root, 2, 0), Some(original));
    assert_eq!(
        f.hooks[0].data_fence_reads.load(Ordering::SeqCst),
        reads,
        "committed retry acknowledges only historical evidence"
    );
    let path = f.shutdown_keep().await;
    let disk = WalEngine::open(path.join("n3/meta.wal")).unwrap().0;
    let actual = data_read_fence::observation(
        disk.snapshot().unwrap().as_ref(),
        &root,
        original.floor.identity.node_id,
        original.floor.identity.store_incarnation,
        region,
    )
    .unwrap();
    assert_eq!(actual, Some(original));
    drop(disk);
    std::fs::remove_dir_all(path).unwrap();
}
