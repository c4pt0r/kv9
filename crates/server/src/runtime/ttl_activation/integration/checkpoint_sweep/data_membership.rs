use super::*;
use kv9_meta::retention::{data_membership as census, data_store_history};

async fn ready(f: &Fixture) -> RootDescriptor {
    let (root, _) = seed(f, 1).await;
    for _ in 0..100 {
        let view = f.backends[1]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap();
        if matches!(
            data_store_history::observe(view.as_ref(), &root).unwrap(),
            data_store_history::Work::Complete { .. }
        ) {
            return root;
        }
        drop(view);
        build_index::census(f, 1, CancellationToken::new())
            .await
            .unwrap();
    }
    panic!("placement census stalled")
}
fn source(f: &Fixture, root: &RootDescriptor, node: u64) -> census::Source {
    census::Source {
        region: f.range.region,
        creation: f.range.creation,
        node: NodeId(node),
        store: root.voter(NodeId(node)).unwrap().store_incarnation,
    }
}
fn progress(
    f: &Fixture,
    root: &RootDescriptor,
    leader: usize,
    node: u64,
) -> Option<census::Progress> {
    census::observation(
        f.backends[leader]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap()
            .as_ref(),
        root,
        source(f, root, node),
    )
    .unwrap()
}
fn request(f: &Fixture, root: &RootDescriptor, leader: usize, node: u64) -> LedgerRequest {
    let view = f.backends[leader]
        .node
        .meta_raft
        .store
        .engine()
        .snapshot()
        .unwrap();
    let origins =
        data_store_history::origins(view.as_ref(), root, f.range.region, f.range.creation, None)
            .unwrap();
    let origin = origins
        .origins
        .iter()
        .find(|o| o.replica.node == NodeId(node))
        .unwrap();
    LedgerRequest::RecordDataMembership(census::Request {
        source: source(f, root, node),
        task: origin.task,
        expected: progress(f, root, leader, node).map_or(0, |p| p.sequence),
        revision: origins.source_revision,
    })
}
async fn send(f: &Fixture, root: &RootDescriptor, request: LedgerRequest) -> Result<()> {
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    let bytes = encode_request(root.digest(), &request)?;
    tokio::task::spawn_blocking(move || ledger.apply(bytes))
        .await
        .unwrap()
}
async fn removed(f: &Fixture, node: u64) {
    let at = f.data[0].add_learner(NodeId(node)).unwrap();
    until(|| {
        f.data.iter().all(|d| {
            d.peer().status_snapshot().conf_applied >= at.index.0
                && d.status().learners.contains(&node)
        })
    })
    .await;
    let at = f.data[0].remove_voter(NodeId(node)).unwrap();
    until(|| {
        f.data.iter().all(|d| {
            d.peer().status_snapshot().conf_applied >= at.index.0
                && !d.status().learners.contains(&node)
        })
    })
    .await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn durable_membership_receipt_survives_leader_change_and_restarts_changed_remote_history() {
    let f = Fixture::new().await;
    for node in 4..=6 {
        removed(&f, node).await;
    }
    let root = ready(&f).await;
    let first = request(&f, &root, 1, 3);
    f.hooks[1].retention_calls.store(0, Ordering::SeqCst);
    f.hooks[1].retention_commits.store(0, Ordering::SeqCst);
    f.hooks[1].retention_fail_after.store(1, Ordering::SeqCst);
    assert!(send(&f, &root, first.clone()).await.is_err());
    let initial = progress(&f, &root, 1, 3).unwrap();
    assert_eq!(
        (initial.sequence, initial.seen, initial.unmatched_nodes),
        (1, 4, 1)
    );
    f.hooks[1].retention_fail_after.store(0, Ordering::SeqCst);
    f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
    until(|| f.meta[2].status().role == Role::Leader).await;
    f.hooks[2].membership_refuse.store(true, Ordering::SeqCst);
    let reads = f.hooks[2].membership_reads.load(Ordering::SeqCst);
    send(&f, &root, first).await.unwrap();
    assert_eq!(progress(&f, &root, 2, 3).unwrap(), initial);
    assert_eq!(
        f.hooks[2].membership_reads.load(Ordering::SeqCst),
        reads,
        "receipt retry must not inspect the source again"
    );
    f.hooks[2].membership_refuse.store(false, Ordering::SeqCst);
    removed(&f, 7).await;
    // Persisted continuation is stale; only a fresh independently read prefix
    // restarts the round. The removed members remain explicit unknown origins.
    send(&f, &root, request(&f, &root, 2, 3)).await.unwrap();
    let reset = progress(&f, &root, 2, 3).unwrap();
    assert_eq!((reset.sequence, reset.round, reset.seen), (2, 2, 4));
    assert_ne!(reset.page.fingerprint, initial.page.fingerprint);
    send(&f, &root, request(&f, &root, 2, 3)).await.unwrap();
    let expected = progress(&f, &root, 2, 3).unwrap();
    assert!(expected.round_complete());
    assert_eq!(expected.unmatched_nodes, 4);
    assert!(expected.has_known_gaps());
    let target = f.meta[2].status().applied_index;
    until(|| f.meta.iter().all(|d| d.status().applied_index >= target)).await;
    let scope = source(&f, &root, 3);
    let path = f.shutdown_keep().await;
    for node in 1..=3 {
        let (wal, _) = WalEngine::open(path.join(format!("n{node}/meta.wal"))).unwrap();
        let view = wal.snapshot().unwrap();
        let page = census::members(view.as_ref(), &root, scope, None).unwrap();
        assert_eq!(page.progress, expected);
        assert!(page.placement_current);
        let tail = census::members(view.as_ref(), &root, scope, page.next).unwrap();
        assert_eq!(tail.members.len(), 3);
        assert!(tail.members.iter().all(|m| m.placement.is_none()));
        assert!(tail.next.is_none());
    }
    std::fs::remove_dir_all(path).unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn durable_membership_worker_rotates_past_unknown_and_rejects_changed_endpoint() {
    let f = Fixture::new().await;
    for node in 4..=5 {
        removed(&f, node).await;
    }
    let root = ready(&f).await;
    *f.hooks[2].membership_change_endpoint.lock().unwrap() = Some(f.backends[1].clone());
    assert!(send(&f, &root, request(&f, &root, 1, 3)).await.is_err());
    assert!(progress(&f, &root, 1, 3).is_none());
    f.hooks[0].membership_unknown.store(true, Ordering::SeqCst);
    let tasks: Vec<_> = f
        .backends
        .iter()
        .map(|b| Maintenance::start(b.clone(), &tokio::runtime::Handle::current()))
        .collect();
    until(|| {
        progress(&f, &root, 1, 2).is_some_and(|p| p.round_complete())
            && progress(&f, &root, 1, 3).is_some_and(|p| p.round_complete())
    })
    .await;
    assert!(progress(&f, &root, 1, 1).is_none());
    assert!(tasks[1].snapshot().membership_failures > 0);
    assert_eq!(progress(&f, &root, 1, 3).unwrap().endpoint_generation, 1);
    assert_eq!(tasks[1].snapshot().peak_in_flight, 1);
    f.hooks[0].membership_unknown.store(false, Ordering::SeqCst);
    until(|| progress(&f, &root, 1, 1).is_some_and(|p| p.round_complete())).await;
    for task in &tasks {
        task.cancel();
    }
    for task in tasks {
        task.drain().await;
    }
    for node in 1..=3 {
        let p = progress(&f, &root, 1, node).unwrap();
        assert_eq!(p.unmatched_nodes, 2);
        assert!(p.has_known_gaps());
    }
    f.close().await;
}
