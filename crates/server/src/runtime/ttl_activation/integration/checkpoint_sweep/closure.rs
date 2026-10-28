use super::*;
use kv9_meta::retention::{build_index, checkpoint_closure as closure};

fn observed(f: &Fixture, leader: usize, root: &RootDescriptor, pending: OwnerId) -> closure::Work {
    closure::observe(
        f.backends[leader]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap()
            .as_ref(),
        root,
        pending,
    )
    .unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn closure_resumes_lost_receipt_new_leader_late_cancel_and_wal_without_erasing_unknown_puts()
{
    let mut f = Fixture::new().await;
    let backend = f.backends[1].clone();
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    let (root, zero, unknown, mut late) = tokio::task::spawn_blocking(move || {
        let root = backend.ttl_context().unwrap().root.clone();
        let engine = backend.node.meta_raft.store.engine();
        let planned = settlement::planned(engine, &root, kv9_common::META_REGION_0);
        let manifest = planned.manifest().clone();
        let zero = CheckpointOwners::new(&root, &manifest, 0).unwrap();
        let unknown = CheckpointOwners::new(&root, &manifest, 1).unwrap();
        let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()));
        zero.before_io(&ledger).unwrap();
        unknown.before_io(&ledger).unwrap();
        for _ in 0..9 {
            let mut execution = CheckpointOwners::new(&root, &manifest, 0)
                .unwrap()
                .execution(&ledger)
                .unwrap();
            let (result, report) = uploader
                .with_local_execution_report(|scoped| execution.before_io_scoped(&ledger, scoped))
                .unwrap();
            result.unwrap();
            assert_eq!((report.issued_puts(), report.resolved_puts()), (0, 0));
            execution.observe_drain(report).unwrap();
            execution.after_local_drain(&ledger).unwrap();
        }
        let mut execution = CheckpointOwners::new(&root, &manifest, 1)
            .unwrap()
            .execution(&ledger)
            .unwrap();
        let (result, report) = uploader
            .with_local_execution_report(|scoped| {
                execution.before_io_scoped(&ledger, scoped)?;
                scoped.upload_planned(planned)
            })
            .unwrap();
        result.unwrap();
        assert!(report.issued_puts() > 0);
        assert_eq!(report.resolved_puts(), 0);
        execution.observe_drain(report).unwrap();
        execution.after_local_drain(&ledger).unwrap();
        zero.close_admission(&ledger).unwrap();
        unknown.close_admission(&ledger).unwrap();
        loop {
            match build_index::observe(engine.snapshot().unwrap().as_ref(), &root).unwrap() {
                build_index::Work::Page { progress, next, .. } => ledger
                    .apply(encode_request(root.digest(), &progress.request(next)).unwrap())
                    .unwrap(),
                build_index::Work::Complete => break,
                _ => panic!("current ledger"),
            }
        }
        let intent = ledger.intent(&root, zero.pending_id()).unwrap().unwrap();
        let late = kv9_meta::retention::checkpoint_admission::executor_binding(
            &intent,
            ledger.local_lifetime().unwrap(),
        )
        .unwrap();
        (root, zero.pending_id(), unknown.pending_id(), late)
    })
    .await
    .unwrap();
    let closure::Work::Advance { cursor, .. } = observed(&f, 1, &root, zero) else {
        panic!("first closure page")
    };
    let request = encode_request(root.digest(), &cursor.request(zero)).unwrap();
    f.hooks[1].retention_commits.store(0, Ordering::SeqCst);
    f.hooks[1].retention_fail_after.store(1, Ordering::SeqCst);
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    let retry = request.clone();
    assert!(tokio::task::spawn_blocking(move || ledger.apply(request))
        .await
        .unwrap()
        .is_err());
    assert_eq!(f.hooks[1].retention_commits.load(Ordering::SeqCst), 1);
    let closure::Work::Advance { cursor: first, .. } = observed(&f, 1, &root, zero) else {
        panic!("partial")
    };
    assert_eq!((first.sequence, first.counts.generations), (1, 4));
    f.hooks[1].retention_fail_after.store(0, Ordering::SeqCst);
    f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
    until(|| f.meta[2].status().role == Role::Leader && f.meta[1].status().role != Role::Leader)
        .await;
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    tokio::task::spawn_blocking(move || ledger.apply(retry))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        observed(&f, 2, &root, zero),
        closure::Work::Advance {
            cursor: first,
            restart: false
        }
    );
    let tasks: Vec<_> = f
        .backends
        .iter()
        .map(|b| Maintenance::start(b.clone(), &tokio::runtime::Handle::current()))
        .collect();
    until(|| {
        matches!(observed(&f, 2, &root, zero), closure::Work::Complete(_))
            && matches!(observed(&f, 2, &root, unknown), closure::Work::Complete(_))
    })
    .await;
    assert!(observed(&f, 2, &root, zero).registered_zero_put_closed());
    let original_unknown = observed(&f, 2, &root, unknown);
    let closure::Work::Complete(ref debt) = original_unknown else {
        panic!("complete unknown report")
    };
    assert_eq!(debt.counts.nonzero_put, 1);
    assert!(debt.counts.unresolved_puts > 0);
    assert!(!original_unknown.registered_zero_put_closed());
    for task in &tasks {
        task.cancel();
    }
    for task in tasks {
        task.drain().await;
    }
    late.generation = std::num::NonZeroU64::new(10).unwrap();
    let request = encode_request(
        root.digest(),
        &LedgerRequest::CancelUnenteredCheckpoint(late),
    )
    .unwrap();
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    tokio::task::spawn_blocking(move || ledger.apply(request))
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        observed(&f, 2, &root, zero),
        closure::Work::Advance { restart: true, .. }
    ));
    assert_eq!(observed(&f, 2, &root, unknown), original_unknown);
    let tasks: Vec<_> = f
        .backends
        .iter()
        .map(|b| Maintenance::start(b.clone(), &tokio::runtime::Handle::current()))
        .collect();
    until(|| observed(&f, 2, &root, zero).registered_zero_put_closed()).await;
    for task in &tasks {
        task.cancel();
    }
    for task in tasks {
        task.drain().await;
    }
    assert!(f
        .backends
        .iter()
        .all(|b| b.maintenance.snapshot().active == 0));
    let closure::Work::Complete(zero_done) = observed(&f, 2, &root, zero) else {
        panic!("complete")
    };
    assert_eq!(
        (
            zero_done.counts.owners,
            zero_done.counts.generations,
            zero_done.counts.zero_put
        ),
        (1, 10, 10)
    );
    for id in [zero, unknown] {
        assert_eq!(settlement::phase(&f, &root, id), Some(PinPhase::Published));
    }
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
    assert_eq!(
        closure::observe(view.as_ref(), &root, zero).unwrap(),
        closure::Work::Complete(zero_done)
    );
    assert_eq!(
        closure::observe(view.as_ref(), &root, unknown).unwrap(),
        original_unknown
    );
    for id in [zero, unknown] {
        assert_eq!(
            kv9_meta::retention::retention_owner(view.as_ref(), &root, id)
                .unwrap()
                .unwrap()
                .phase,
            PinPhase::Published
        );
    }
    drop(view);
    drop(engine);
    f.close().await;
}
