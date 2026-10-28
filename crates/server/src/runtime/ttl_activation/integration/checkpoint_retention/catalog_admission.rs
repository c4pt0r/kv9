use super::*;

async fn until(mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("catalog worker did not reach the expected state");
}

async fn hold_catalog(f: &Fixture) -> (std::sync::mpsc::Sender<()>, tokio::task::JoinHandle<()>) {
    let backend = f.backends[1].clone();
    let (release, released) = std::sync::mpsc::channel();
    let (ready, locked) = tokio::sync::oneshot::channel();
    let holder = tokio::task::spawn_blocking(move || {
        let _guard = backend.node.meta_raft.lock_catalog_txn();
        ready.send(()).unwrap();
        released.recv_timeout(Duration::from_secs(5)).unwrap();
    });
    locked.await.unwrap();
    (release, holder)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn retention_short_catalog_contention_completes_original_rpc_without_replay() {
    let f = Fixture::new().await;
    let root = root(&f);
    let before = f.meta[1].driver_applied();
    let (release, holder) = hold_catalog(&f).await;
    let channel = f.public[1].clone();
    let query =
        LedgerQuery::Apply(encode_request(root.digest(), &LedgerRequest::Initialize).unwrap());
    let digest = root.digest();
    let request = tokio::spawn(async move {
        on_channel(
            channel,
            digest,
            &query,
            NodeId(1),
            Some("cluster-test".into()),
            Duration::from_secs(2),
        )
        .await
    });
    // Wait for the real admitted worker, not an assumed transport delay.
    until(|| f.backends[1].node.meta_raft.catalog_waiters() == 1 || request.is_finished()).await;
    assert!(
        !request.is_finished(),
        "contention must be absorbed before returning to caller"
    );
    assert_eq!(
        f.meta[1].driver_applied(),
        before,
        "no proposal before catalog ownership"
    );
    release.send(()).unwrap();
    holder.await.unwrap();
    assert!(matches!(
        request.await.unwrap().unwrap(),
        LedgerReply::Applied { changed: true, .. }
    ));
    assert_eq!(f.hooks[1].retention_calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.hooks[1].retention_commits.load(Ordering::SeqCst), 1);
    assert_eq!(f.hooks[1].retention_catalog_busy.load(Ordering::SeqCst), 0);
    assert_eq!(f.backends[1].node.meta_raft.catalog_waiters(), 0);
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn retention_expired_catalog_wait_cannot_append_after_lock_release() {
    let f = Fixture::new().await;
    let root = root(&f);
    let before = f.meta[1].driver_applied();
    let (release, holder) = hold_catalog(&f).await;
    let query =
        LedgerQuery::Apply(encode_request(root.digest(), &LedgerRequest::Initialize).unwrap());
    let result = on_channel(
        f.public[1].clone(),
        root.digest(),
        &query,
        NodeId(1),
        Some("cluster-test".into()),
        Duration::from_millis(30),
    )
    .await;
    assert!(result.is_err());
    until(|| f.backends[1].node.meta_raft.catalog_waiters() == 0).await;
    assert_eq!(f.hooks[1].retention_calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.hooks[1].retention_commits.load(Ordering::SeqCst), 0);
    assert_eq!(f.meta[1].driver_applied(), before);
    release.send(()).unwrap();
    holder.await.unwrap();
    // A separate explicit request may now execute; only its two confirmation/
    // mutation entries may advance the leader, with no orphan timed-out worker.
    let fresh = on_channel(
        f.public[1].clone(),
        root.digest(),
        &query,
        NodeId(1),
        Some("cluster-test".into()),
        Duration::from_secs(2),
    )
    .await
    .unwrap();
    assert!(matches!(fresh, LedgerReply::Applied { changed: true, .. }));
    assert_eq!(f.hooks[1].retention_calls.load(Ordering::SeqCst), 2);
    assert_eq!(f.hooks[1].retention_commits.load(Ordering::SeqCst), 1);
    assert_eq!(
        f.meta[1].driver_applied().unwrap().index,
        before.unwrap().index + 2
    );
    f.close().await;
}
