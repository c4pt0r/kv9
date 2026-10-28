use super::timeline_census::{ready, request, send, six, source, until};
use super::*;
use crate::runtime::checkpoint_sweep::Maintenance;
use kv9_meta::retention::{
    data_configuration_intervals as projection, data_configuration_origins as origins,
    LedgerRequest,
};

async fn collect(f: &Fixture, root: &RootDescriptor, leader: usize, node: u64) {
    for _ in 0..2 {
        let timeline = request(f, root, leader, node);
        let LedgerRequest::RecordConfigurationTimeline(mut q) = timeline.clone() else {
            unreachable!()
        };
        q.expected = origins::observation(
            f.backends[leader]
                .node
                .meta_raft
                .store
                .engine()
                .snapshot()
                .unwrap()
                .as_ref(),
            root,
            q.source,
        )
        .unwrap()
        .map_or(0, |p| p.sequence);
        send(f, root, LedgerRequest::RecordConfigurationOrigins(q))
            .await
            .unwrap();
        send(f, root, timeline).await.unwrap();
    }
}
fn progress(
    f: &Fixture,
    root: &RootDescriptor,
    leader: usize,
    node: u64,
) -> Option<projection::Progress> {
    projection::observation(
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
fn next(f: &Fixture, root: &RootDescriptor, leader: usize, node: u64) -> Option<LedgerRequest> {
    projection::next(
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
    .map(LedgerRequest::AdvanceConfigurationIntervals)
}
fn entries(
    view: &dyn kv9_engine::ReadView,
    root: &RootDescriptor,
    source: projection::Source,
) -> Vec<projection::Entry> {
    let mut out = Vec::new();
    let mut cursor = None;
    loop {
        let page = projection::intervals(view, root, source, cursor).unwrap();
        out.extend(page.intervals.into_iter().map(|i| i.entry));
        cursor = page.next;
        if cursor.is_none() {
            return out;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn configuration_intervals_survive_lost_receipt_new_leader_append_and_three_metadata_reopens()
{
    let f = Fixture::new().await;
    let root = f.backends[1].ttl_context().unwrap().root.clone();
    let (identity, guard) = six(&f, &root).await;
    ready(&f, &root, 1).await;
    collect(&f, &root, 1, 3).await;
    let first = next(&f, &root, 1, 3).unwrap();
    f.hooks[1].retention_calls.store(0, Ordering::SeqCst);
    f.hooks[1].retention_commits.store(0, Ordering::SeqCst);
    f.hooks[1].retention_fail_after.store(1, Ordering::SeqCst);
    assert!(send(&f, &root, first.clone()).await.is_err());
    let partial = progress(&f, &root, 1, 3).unwrap();
    assert_eq!(partial.processed, 4);
    f.hooks[1].retention_fail_after.store(0, Ordering::SeqCst);
    f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
    until(|| f.meta[2].status().role == Role::Leader).await;
    f.hooks[2].membership_refuse.store(true, Ordering::SeqCst);
    let reads = f.hooks[2].membership_reads.load(Ordering::SeqCst);
    send(&f, &root, first).await.unwrap();
    assert_eq!(progress(&f, &root, 2, 3).unwrap(), partial);
    send(&f, &root, next(&f, &root, 2, 3).unwrap())
        .await
        .unwrap();
    assert_eq!(
        f.hooks[2].membership_reads.load(Ordering::SeqCst),
        reads,
        "projection of durable observations needs no live source query"
    );
    let scope = source(&f, &root, 3);
    let original = entries(
        f.backends[2]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap()
            .as_ref(),
        &root,
        scope,
    );
    assert_eq!(original.len(), 7);
    for (i, entry) in original.iter().enumerate() {
        assert_eq!(entry.gaps, 0);
        let members = entry.members.as_ref().unwrap();
        assert_eq!(members.len(), if i % 2 == 1 { 4 } else { 3 });
        for member in members {
            let expected = if member.node == NodeId(4) {
                identity.store_incarnation
            } else {
                root.voter(member.node).unwrap().store_incarnation
            };
            assert_eq!(member.store, Some(expected));
            assert_eq!(member.roles, if member.node == NodeId(4) { 4 } else { 1 });
        }
    }
    f.hooks[2].membership_refuse.store(false, Ordering::SeqCst);
    let at = f.data[0].add_learner(NodeId(5)).unwrap();
    until(|| {
        f.data
            .iter()
            .all(|d| d.peer().status_snapshot().conf_applied >= at.index.0)
    })
    .await;
    for _ in 0..2 {
        send(&f, &root, request(&f, &root, 2, 3)).await.unwrap();
    }
    send(&f, &root, next(&f, &root, 2, 3).unwrap())
        .await
        .unwrap();
    let expected = progress(&f, &root, 2, 3).unwrap();
    assert_eq!(
        (expected.processed, expected.generation),
        (8, partial.generation)
    );
    assert!(expected.has_known_gaps());
    let index = f.meta[2].status().applied_index;
    until(|| f.meta.iter().all(|d| d.status().applied_index >= index)).await;
    drop(guard);
    let path = f.shutdown_keep().await;
    for node in 1..=3 {
        let (wal, _) = WalEngine::open(path.join(format!("n{node}/meta.wal"))).unwrap();
        let view = wal.snapshot().unwrap();
        assert_eq!(
            projection::observation(view.as_ref(), &root, scope)
                .unwrap()
                .unwrap(),
            expected
        );
        let all = entries(view.as_ref(), &root, scope);
        assert_eq!(all.len(), 8);
        assert_eq!(&all[..7], original.as_slice());
        assert_ne!(all[7].gaps & projection::LEGACY_CHANGE, 0);
        assert!(all[7]
            .members
            .as_ref()
            .unwrap()
            .iter()
            .all(|m| m.store.is_none()));
    }
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn configuration_intervals_background_rotates_past_unknown_and_preserves_exact_sources() {
    let f = Fixture::new().await;
    let root = f.backends[1].ttl_context().unwrap().root.clone();
    let (_, guard) = six(&f, &root).await;
    ready(&f, &root, 1).await;
    *f.hooks[2].membership_change_endpoint.lock().unwrap() = Some(f.backends[1].clone());
    assert!(send(&f, &root, request(&f, &root, 1, 3)).await.is_err());
    assert!(next(&f, &root, 1, 3).is_none());
    f.hooks[0].membership_unknown.store(true, Ordering::SeqCst);
    let tasks: Vec<_> = f
        .backends
        .iter()
        .map(|b| Maintenance::start(b.clone(), &tokio::runtime::Handle::current()))
        .collect();
    until(|| {
        [2, 3].iter().all(|&node| {
            progress(&f, &root, 1, node)
                .is_some_and(|p| p.round_complete() && p.processed == 7 && !p.has_known_gaps())
        })
    })
    .await;
    assert!(progress(&f, &root, 1, 1).is_none());
    let stats = tasks[1].snapshot();
    assert!(stats.interval_pages >= 4);
    assert_eq!(stats.peak_in_flight, 1);
    f.hooks[0].membership_unknown.store(false, Ordering::SeqCst);
    until(|| {
        progress(&f, &root, 1, 1)
            .is_some_and(|p| p.round_complete() && p.processed == 7 && !p.has_known_gaps())
    })
    .await;
    for task in &tasks {
        task.cancel();
    }
    for task in tasks {
        task.drain().await;
    }
    for node in 1..=3 {
        let scope = source(&f, &root, node);
        let all = entries(
            f.backends[1]
                .node
                .meta_raft
                .store
                .engine()
                .snapshot()
                .unwrap()
                .as_ref(),
            &root,
            scope,
        );
        assert_eq!(all.len(), 7);
        assert!(all.iter().all(|e| e.gaps == 0));
        assert!(all[0]
            .members
            .as_ref()
            .unwrap()
            .iter()
            .all(|m| m.store == Some(root.voter(m.node).unwrap().store_incarnation)));
    }
    drop(guard);
    f.close().await;
}
