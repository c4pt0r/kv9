use super::*;
use crate::checkpoint_retention::CheckpointLedger;
use crate::runtime::checkpoint_retention::RoutedLedger;
use crate::runtime::checkpoint_sweep::Maintenance;
use kv9_meta::retention::{
    data_configuration_origins as census, data_membership, data_store_history, encode_request,
    LedgerRequest,
};

async fn until(mut f: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(30), async {
        while !f() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("origin census progress deadline");
}
async fn six(f: &Fixture, root: &RootDescriptor) -> (StoreIdentity, StoreGuard) {
    let (identity, guard, creation) = prepare_migration(f, root, [125; 16]).await;
    query::detach(f, [125; 16]).await;
    for operation in [[126; 16], [127; 16]] {
        let mut t = f.backends[1].node.meta_raft.store.begin().unwrap();
        kv9_meta::data_groups::migration::plan_migration(&mut t, operation, creation, NodeId(4))
            .unwrap();
        let at = f.meta[1]
            .propose(&Command::from_batch(&t.into_batch()))
            .unwrap();
        applied(&f.meta, at).await;
        let backend = f.backends[0].clone();
        let digest = root.digest();
        tokio::task::spawn_blocking(move || {
            backend.attach_migration_learner("test", digest, operation)
        })
        .await
        .unwrap()
        .unwrap();
        query::detach(f, operation).await;
    }
    // The historical destination is now unavailable. Keep its original tasks,
    // and refuse locally before connecting to its nonexistent fixture endpoint.
    let mut t = f.backends[1].node.meta_raft.store.begin().unwrap();
    t.update(
        &NODES_DESC,
        &[memcmp_uint(4)],
        vec![(ColumnId(3), ColumnValue::Uint(0))],
    )
    .unwrap();
    let at = f.meta[1]
        .propose(&Command::from_batch(&t.into_batch()))
        .unwrap();
    applied(&f.meta, at).await;
    (identity, guard)
}
async fn send(f: &Fixture, root: &RootDescriptor, request: LedgerRequest) -> Result<()> {
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    let bytes = encode_request(root.digest(), &request)?;
    tokio::task::spawn_blocking(move || ledger.apply(bytes))
        .await
        .unwrap()
}
async fn ready(f: &Fixture, root: &RootDescriptor, leader: usize) {
    send(f, root, LedgerRequest::Initialize).await.unwrap();
    for _ in 0..100 {
        let work = data_store_history::observe(
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
        .unwrap();
        match work {
            data_store_history::Work::Complete { .. } => return,
            data_store_history::Work::Page {
                progress,
                revision,
                after,
                ..
            } => send(f, root, progress.request(revision, after))
                .await
                .unwrap(),
            _ => panic!("expected initialized placement census"),
        }
    }
    panic!("placement census stalled");
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
    let page =
        data_store_history::origins(view.as_ref(), root, f.range.region, f.range.creation, None)
            .unwrap();
    let origin = page
        .origins
        .iter()
        .find(|o| o.replica.node == NodeId(node))
        .unwrap();
    LedgerRequest::RecordConfigurationOrigins(census::Request {
        source: source(f, root, node),
        task: origin.task,
        revision: page.source_revision,
        expected: progress(f, root, leader, node).map_or(0, |p| p.sequence),
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn origin_census_lost_receipt_new_leader_stale_source_cursor_and_three_wal_reopens() {
    let f = Fixture::new().await;
    let root = f.backends[1].ttl_context().unwrap().root.clone();
    let (identity, guard) = six(&f, &root).await;
    ready(&f, &root, 1).await;
    let first = request(&f, &root, 1, 3);
    f.hooks[1].retention_calls.store(0, Ordering::SeqCst);
    f.hooks[1].retention_commits.store(0, Ordering::SeqCst);
    f.hooks[1].retention_fail_after.store(1, Ordering::SeqCst);
    assert!(send(&f, &root, first.clone()).await.is_err());
    let initial = progress(&f, &root, 1, 3).unwrap();
    let native = census::initial::observation(
        f.backends[1]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap()
            .as_ref(),
        &root,
        source(&f, &root, 3),
    )
    .unwrap()
    .unwrap();
    assert_eq!(native.recorded_at, initial.recorded_at);
    assert_eq!(native.replicas.len(), 3);
    for replica in &native.replicas {
        assert_eq!(
            replica.incarnation,
            root.voter(replica.node).unwrap().store_incarnation
        );
    }
    assert_eq!(
        (initial.sequence, initial.seen, initial.retained_entries),
        (1, 4, 4)
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
        "durable receipt does not query an unavailable source"
    );
    f.hooks[2].membership_refuse.store(false, Ordering::SeqCst);
    let at = f.data[0].add_learner(NodeId(5)).unwrap();
    until(|| {
        f.data
            .iter()
            .all(|d| d.peer().status_snapshot().conf_applied >= at.index.0)
    })
    .await;
    send(&f, &root, request(&f, &root, 2, 3)).await.unwrap();
    let reset = progress(&f, &root, 2, 3).unwrap();
    assert_eq!((reset.sequence, reset.round, reset.seen), (2, 2, 4));
    assert_ne!(reset.page.fingerprint, initial.page.fingerprint);
    assert!(reset.source_history_unknown);
    send(&f, &root, request(&f, &root, 2, 3)).await.unwrap();
    let expected = progress(&f, &root, 2, 3).unwrap();
    assert!(expected.round_complete() && expected.has_known_gaps());
    assert_eq!(expected.retained_entries, 6);
    let index = f.meta[2].status().applied_index;
    until(|| f.meta.iter().all(|d| d.status().applied_index >= index)).await;
    let scope = source(&f, &root, 3);
    drop(guard);
    let path = f.shutdown_keep().await;
    for node in 1..=3 {
        let (wal, _) = WalEngine::open(path.join(format!("n{node}/meta.wal"))).unwrap();
        let view = wal.snapshot().unwrap();
        let first = census::entries(view.as_ref(), &root, scope, None).unwrap();
        assert_eq!(
            census::initial::observation(view.as_ref(), &root, scope)
                .unwrap()
                .unwrap(),
            native
        );
        assert_eq!(first.progress, expected);
        let last = census::entries(view.as_ref(), &root, scope, first.next).unwrap();
        assert!(last.next.is_none());
        let records: Vec<_> = first.entries.iter().chain(&last.entries).collect();
        assert_eq!(records.len(), 6);
        assert!(records
            .iter()
            .all(|r| r.applied.origin.store == identity.store_incarnation));
        assert_eq!(
            records
                .iter()
                .filter(|r| r.applied.origin.action == Action::Detach)
                .count(),
            3
        );
    }
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn origin_census_background_alternates_membership_and_origins_past_unknown_sources() {
    let f = Fixture::new().await;
    let root = f.backends[1].ttl_context().unwrap().root.clone();
    let (_, guard) = six(&f, &root).await;
    ready(&f, &root, 1).await;
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
        [2, 3].iter().all(|&node| {
            let origins = progress(&f, &root, 1, node).is_some_and(|p| p.round_complete());
            let members = data_membership::observation(
                f.backends[1]
                    .node
                    .meta_raft
                    .store
                    .engine()
                    .snapshot()
                    .unwrap()
                    .as_ref(),
                &root,
                source(&f, &root, node),
            )
            .unwrap();
            origins && members.is_some_and(|p| p.round_complete())
        })
    })
    .await;
    assert!(progress(&f, &root, 1, 1).is_none());
    let stats = tasks[1].snapshot();
    assert!(stats.origin_failures > 0 && stats.membership_failures > 0);
    assert_eq!(stats.peak_in_flight, 1);
    assert_eq!(progress(&f, &root, 1, 3).unwrap().endpoint_generation, 1);
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
        assert_eq!(p.retained_entries, 6);
        let native = census::initial::observation(
            f.backends[1]
                .node
                .meta_raft
                .store
                .engine()
                .snapshot()
                .unwrap()
                .as_ref(),
            &root,
            source(&f, &root, node),
        )
        .unwrap()
        .unwrap();
        assert_eq!(native.source.node, NodeId(node));
        assert_eq!(native.replicas.len(), 3);
        assert!(native
            .replicas
            .iter()
            .all(|r| r.incarnation == root.voter(r.node).unwrap().store_incarnation));
    }
    drop(guard);
    f.close().await;
}
