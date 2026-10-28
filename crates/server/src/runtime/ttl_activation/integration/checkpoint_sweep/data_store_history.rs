use super::build_index::census;
use super::*;
use kv9_meta::data_groups::CreationIntent;
use kv9_meta::retention::data_store_history::{self, Work};

fn work(f: &Fixture, leader: usize, root: &RootDescriptor) -> Work {
    data_store_history::observe(
        f.backends[leader]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap()
            .as_ref(),
        root,
    )
    .unwrap()
}
async fn create(f: &Fixture, leader: usize, n: u8) -> CreationIntent {
    let backend = f.backends[leader].clone();
    let root = f.range.root;
    tokio::task::spawn_blocking(move || {
        backend
            .create_data_group("fixture", root, [n; 16], &[NodeId(1), NodeId(2), NodeId(3)])
            .unwrap()
            .intent
    })
    .await
    .unwrap()
}
fn collect(
    view: &dyn kv9_engine::ReadView,
    root: &RootDescriptor,
    c: &CreationIntent,
) -> Vec<kv9_meta::data_groups::store_history::Origin> {
    let page = data_store_history::origins(view, root, c.region(), c.digest(), None).unwrap();
    assert!(page.next.is_none());
    assert_eq!(page.origins.len(), 3);
    for (origin, replica) in page.origins.iter().zip(c.replicas()) {
        assert_eq!(origin.replica, *replica);
        assert_eq!(origin.kind, 100);
    }
    page.origins
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn retained_data_history_worker_resumes_lost_receipt_after_leader_change_and_all_wal_reopen()
{
    let f = Fixture::new().await;
    let mut groups = Vec::new();
    for n in 30..35 {
        groups.push(create(&f, 1, n).await);
    }
    let (root, attempts) = seed(&f, 1).await;
    // Advance the five earlier catalogs in their production lane, then stop
    // exactly before DATA's first page to inject a committed-but-lost receipt.
    for _ in 0..80 {
        let view = f.backends[1]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap();
        let ready = matches!(
            kv9_meta::retention::image_publisher_index::observe(view.as_ref(), &root).unwrap(),
            kv9_meta::retention::image_publisher_index::Work::Complete
        );
        drop(view);
        if ready {
            break;
        }
        census(&f, 1, CancellationToken::new()).await.unwrap();
    }
    let Work::Page {
        progress,
        revision,
        after,
        examined,
    } = work(&f, 1, &root)
    else {
        panic!()
    };
    assert_eq!(progress.sequence, 0);
    assert_eq!(after, None);
    assert_eq!(examined, 4);
    let receipt = progress.request(revision, after);
    f.hooks[1].retention_calls.store(0, Ordering::SeqCst);
    f.hooks[1].retention_commits.store(0, Ordering::SeqCst);
    f.hooks[1].retention_fail_after.store(1, Ordering::SeqCst);
    assert!(census(&f, 1, CancellationToken::new()).await.is_err());
    assert_eq!(f.hooks[1].retention_commits.load(Ordering::SeqCst), 1);
    let Work::Page { progress, .. } = work(&f, 1, &root) else {
        panic!()
    };
    assert_eq!(progress.sequence, 1);
    f.hooks[1].retention_fail_after.store(0, Ordering::SeqCst);
    f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
    until(|| f.meta[2].status().role == Role::Leader).await;
    assert!(census(&f, 1, CancellationToken::new()).await.is_err());
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(census(&f, 2, cancelled).await.is_err());
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    let digest = root.digest();
    tokio::task::spawn_blocking(move || {
        ledger
            .apply(encode_request(digest, &receipt).unwrap())
            .unwrap()
    })
    .await
    .unwrap();
    let Work::Page { progress, .. } = work(&f, 2, &root) else {
        panic!()
    };
    assert_eq!(progress.sequence, 1);
    let tasks: Vec<_> = f
        .backends
        .iter()
        .map(|b| Maintenance::start(b.clone(), &tokio::runtime::Handle::current()))
        .collect();
    until(|| tasks[2].snapshot().data_store_history_catalog_complete).await;
    let first_revision = tasks[2]
        .snapshot()
        .data_store_history_source_revision
        .unwrap();
    // New committed placement after apparent completion must invalidate the
    // exact source revision and be discovered by the same running worker.
    groups.push(create(&f, 2, 40).await);
    let revision = kv9_meta::task_history::revision(
        f.backends[2]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap()
            .as_ref(),
    )
    .unwrap()
    .sequence;
    assert!(revision > first_revision);
    until(|| {
        let s = tasks[2].snapshot();
        s.data_store_history_catalog_complete
            && s.data_store_history_source_revision == Some(revision)
    })
    .await;
    assert_eq!(tasks[2].snapshot().peak_in_flight, 1);
    assert!(tasks[2].snapshot().data_store_history_pages >= 4);
    assert!(!tasks[2].snapshot().data_store_history_uncertain);
    for task in &tasks {
        task.cancel();
    }
    for task in tasks {
        task.drain().await;
    }
    let target = f.meta[2].status().applied_index;
    until(|| f.meta.iter().all(|d| d.status().applied_index >= target)).await;
    let expected = {
        let view = f.backends[2]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap();
        groups
            .iter()
            .map(|c| collect(view.as_ref(), &root, c))
            .collect::<Vec<_>>()
    };
    let path = f.shutdown_keep().await;
    for node in 1..=3 {
        let (e, _) = WalEngine::open(path.join(format!("n{node}/meta.wal"))).unwrap();
        let view = e.snapshot().unwrap();
        assert!(
            matches!(data_store_history::observe(view.as_ref(),&root).unwrap(),Work::Complete{revision:r} if r==revision)
        );
        for (c, origins) in groups.iter().zip(&expected) {
            assert_eq!(collect(view.as_ref(), &root, c), *origins);
        }
        for a in &attempts {
            assert_eq!(
                kv9_meta::retention::retention_owner(view.as_ref(), &root, a.id)
                    .unwrap()
                    .unwrap()
                    .phase,
                PinPhase::Published
            );
        }
    }
    std::fs::remove_dir_all(path).unwrap();
}
