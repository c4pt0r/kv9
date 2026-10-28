use super::*;
use crate::checkpoint_retention::CheckpointLedger;
use crate::runtime::checkpoint_retention::RoutedLedger;
use kv9_common::retention::PinPhase;
use kv9_meta::retention::{
    checkpoint_intent::CheckpointIntent,
    checkpoint_selection::{self, Selection},
    encode_request, LedgerRequest, OwnerKind,
};

fn intent(root: &RootDescriptor, manifest: &CheckpointManifest) -> CheckpointIntent {
    CheckpointIntent::new(root, manifest, 0).unwrap()
}
fn bytes(root: &RootDescriptor, manifest: &CheckpointManifest, source: usize) -> Vec<u8> {
    let identity = StoreIdentity::for_voter(root, NodeId(source as u64 + 1)).unwrap();
    encode_request(
        root.digest(),
        &LedgerRequest::RecordCheckpointSelection {
            pending: intent(root, manifest)
                .binding(OwnerKind::Pending)
                .unwrap()
                .token(),
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
async fn committed_history_cannot_replace_selected_replica_evidence_or_endpoint_recheck() {
    let f = Fixture::new().await;
    let (root, manifest, _) = readers::published(&f).await;
    let request = bytes(&root, &manifest, 0);
    assert!(f
        .data
        .iter()
        .all(|d| d.manifest_at(f.range.region.0, 1).unwrap().is_some()));
    assert!((0..3).all(|i| engine(&f, i).selected_checkpoint().unwrap().is_none()));
    assert!(send(&f, request.clone()).await.is_err());
    assert!(fact(&f, &root, 1, 0).is_none());
    engine(&f, 0).checkpoint_applied(&manifest).unwrap();
    let wrong_store = encode_request(
        root.digest(),
        &LedgerRequest::RecordCheckpointSelection {
            pending: intent(&root, &manifest)
                .binding(OwnerKind::Pending)
                .unwrap()
                .token(),
            source: NodeId(1),
            store: kv9_common::StoreIncarnation::from_bytes([9; 16]),
        },
    )
    .unwrap();
    assert!(send(&f, wrong_store).await.is_err());
    let backend = f.backends[1].clone();
    let raw = request.clone();
    assert!(
        tokio::task::spawn_blocking(move || backend.apply_retention("test", raw))
            .await
            .unwrap()
            .is_err()
    );
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
    assert_eq!(selected.intent(), &intent(&root, &manifest));
    assert_eq!(selected.endpoint_generation, 1);
    assert!(send(&f, bytes(&root, &manifest, 2)).await.is_err());
    engine(&f, 2).checkpoint_applied(&manifest).unwrap();
    engine(&f, 2)
        .checkpoint_history()
        .unwrap()
        .with_compaction(manifest.position(), || {
            Err::<(), _>(Error::Engine("injected uncertain compaction".into()))
        })
        .unwrap_err();
    assert!(send(&f, bytes(&root, &manifest, 2)).await.is_err());
    assert!(fact(&f, &root, 1, 2).is_none());
    let version = intent(&root, &manifest)
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
        PinPhase::Published
    );
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn worker_retries_lost_selection_receipt_without_get_and_fact_survives_leader_change_and_wal()
{
    let mut f = Fixture::new().await;
    let (root, manifest, store) = readers::published(&f).await;
    f.hooks[1]
        .retention_selection_fault
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
        "receipt retry cannot reread the image"
    );
    work.cancel();
    work.drain().await;
    for (backend, hook) in f.backends.iter().zip(&f.hooks) {
        backend.raw_directory.remove(f.range.region);
        backend
            .group_handles
            .lock()
            .unwrap()
            .remove(&f.range.region);
        hook.selection_refuse.store(true, Ordering::SeqCst);
    }
    let reads = f.hooks[0].selection_reads.load(Ordering::SeqCst);
    f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
    until(|| f.meta[2].status().role == Role::Leader && f.meta[1].status().role != Role::Leader)
        .await;
    send(&f, bytes(&root, &manifest, 0)).await.unwrap();
    assert_eq!(fact(&f, &root, 2, 0), Some(original.clone()));
    assert_eq!(f.hooks[0].selection_reads.load(Ordering::SeqCst), reads);
    assert!(
        send(&f, bytes(&root, &manifest, 1)).await.is_err(),
        "another store cannot borrow the fact"
    );
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
    let engine = WalEngine::open(f.path.join("n3/meta.wal")).unwrap().0;
    let identity = StoreIdentity::for_voter(&root, NodeId(1)).unwrap();
    let view = engine.snapshot().unwrap();
    assert_eq!(
        checkpoint_selection::selection(
            view.as_ref(),
            &root,
            identity.node_id,
            identity.store_incarnation,
            f.range.region
        )
        .unwrap(),
        Some(original)
    );
    let version = intent(&root, &manifest)
        .binding(OwnerKind::Version)
        .unwrap();
    assert_eq!(
        kv9_meta::retention::retention_owner(view.as_ref(), &root, version.descriptor.id)
            .unwrap()
            .unwrap()
            .phase,
        PinPhase::Published
    );
    drop(view);
    drop(engine);
    f.close().await;
}
