use super::*;
use kv9_meta::retention::checkpoint_refusal::{self, Refusal};

fn fact(f: &Fixture, leader: usize, root: &RootDescriptor, pending: OwnerId) -> Option<Refusal> {
    checkpoint_refusal::refusal(
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
fn request(root: &RootDescriptor, pending: OwnerId, source: NodeId, winner: [u8; 32]) -> Vec<u8> {
    encode_request(
        root.digest(),
        &LedgerRequest::RecordCheckpointRefusal {
            pending: OwnerToken {
                owner: pending,
                generation: std::num::NonZeroU64::new(1).unwrap(),
            },
            source,
            winner,
        },
    )
    .unwrap()
}
async fn send(f: &Fixture, bytes: Vec<u8>) -> Result<()> {
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    tokio::task::spawn_blocking(move || ledger.apply(bytes))
        .await
        .unwrap()
}
fn protected(f: &Fixture, root: &RootDescriptor, pending: OwnerId) {
    assert!(fact(f, 1, root, pending).is_none());
    assert!(matches!(
        observation(f, 1, root, pending),
        sweep::SweepObservation::Open
    ));
    assert_eq!(
        settlement::phase(f, root, pending),
        Some(PinPhase::Published)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn authoritative_refusal_rejects_forgery_unknown_and_endpoint_change_before_recording() {
    let f = Fixture::new().await;
    let (root, cases) = remote::committed(&f, true).await;
    let negative = cases
        .iter()
        .find(|c| c.positive == Some(false))
        .unwrap()
        .pending;
    let positive = cases
        .iter()
        .find(|c| c.positive == Some(true))
        .unwrap()
        .pending;
    let unknown = cases.iter().find(|c| c.positive.is_none()).unwrap().pending;
    let winner: [u8; 32] = f.data[0]
        .manifest_pair(f.range.region.0)
        .unwrap()
        .last_change_id
        .try_into()
        .unwrap();
    let bytes = request(&root, negative, NodeId(1), winner);
    // The admin path holds the catalog lock and cannot mint a history witness.
    let backend = f.backends[1].clone();
    let raw = bytes.clone();
    assert!(
        tokio::task::spawn_blocking(move || backend.apply_retention("test", raw))
            .await
            .unwrap()
            .is_err()
    );
    protected(&f, &root, negative);
    for (pending, source, claimed) in [
        (negative, NodeId(1), [8; 32]),
        (positive, NodeId(1), winner),
        (unknown, NodeId(1), winner),
        (negative, NodeId(99), winner),
    ] {
        assert!(send(&f, request(&root, pending, source, claimed))
            .await
            .is_err());
        protected(&f, &root, pending);
    }
    assert_eq!(f.hooks[0].history_reads.load(Ordering::SeqCst), 2);
    f.hooks[0].history_unknown.store(true, Ordering::SeqCst);
    assert!(send(&f, bytes.clone()).await.is_err());
    f.hooks[0].history_unknown.store(false, Ordering::SeqCst);
    f.hooks[0].history_refuse.store(true, Ordering::SeqCst);
    assert!(send(&f, bytes.clone()).await.is_err());
    f.hooks[0].history_refuse.store(false, Ordering::SeqCst);
    protected(&f, &root, negative);
    // The actual history response changes the source endpoint while the
    // receiving metadata leader is verifying it. No catalog lock is held.
    let txn = f.backends[1].node.meta_raft.store.begin().unwrap();
    let old = node_endpoint(&txn, NodeId(1)).unwrap().unwrap();
    drop(txn);
    *f.hooks[0].history_change_endpoint.lock().unwrap() = Some(f.backends[1].clone());
    assert!(send(&f, bytes.clone()).await.is_err());
    protected(&f, &root, negative);
    let txn = f.backends[1].node.meta_raft.store.begin().unwrap();
    assert_eq!(
        node_endpoint(&txn, NodeId(1)).unwrap().unwrap().generation,
        old.generation + 1
    );
    drop(txn);
    send(&f, bytes).await.unwrap();
    let fact = fact(&f, 1, &root, negative).unwrap();
    assert_eq!(fact.endpoint_generation, old.generation + 1);
    assert_eq!(fact.winner, winner);
    assert_eq!(fact.closed_at, fact.recorded_at);
    assert_eq!(
        settlement::phase(&f, &root, negative),
        Some(PinPhase::Published)
    );
    protected(&f, &root, positive);
    protected(&f, &root, unknown);
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn durable_refusal_survives_lost_receipt_new_leader_unavailable_history_and_wal() {
    let mut f = Fixture::new().await;
    let (root, cases) = remote::committed(&f, true).await;
    let negative = cases
        .iter()
        .find(|c| c.positive == Some(false))
        .unwrap()
        .pending;
    let winner: [u8; 32] = f.data[0]
        .manifest_pair(f.range.region.0)
        .unwrap()
        .last_change_id
        .try_into()
        .unwrap();
    let bytes = request(&root, negative, NodeId(1), winner);
    f.hooks[1].retention_commits.store(0, Ordering::SeqCst);
    f.hooks[1].retention_fail_after.store(1, Ordering::SeqCst);
    assert!(send(&f, bytes.clone()).await.is_err());
    assert_eq!(f.hooks[1].retention_commits.load(Ordering::SeqCst), 1);
    let original = fact(&f, 1, &root, negative).unwrap();
    assert_eq!(f.hooks[0].history_reads.load(Ordering::SeqCst), 1);
    f.hooks[1].retention_fail_after.store(0, Ordering::SeqCst);
    // Remove all runtime lookups for this group, including the next metadata
    // leader, and refuse history RPCs. The immutable fact is the only evidence.
    for (backend, hook) in f.backends.iter().zip(&f.hooks) {
        backend.raw_directory.remove(f.range.region);
        hook.history_refuse.store(true, Ordering::SeqCst);
        assert!(!backend
            .group_handles
            .lock()
            .unwrap()
            .contains_key(&f.range.region));
    }
    f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
    until(|| f.meta[2].status().role == Role::Leader && f.meta[1].status().role != Role::Leader)
        .await;
    // Retire the source directory row after its exact identity was persisted.
    let metadata = f.backends[2].clone();
    tokio::task::spawn_blocking(move || {
        let _guard = metadata.node.meta_raft.lock_catalog_txn();
        let at = metadata.driver.propose(&kv9_raft::Command::Noop).unwrap();
        metadata
            .driver
            .wait_applied(at, Duration::from_secs(3))
            .unwrap();
        let mut txn = metadata.node.meta_raft.store.begin().unwrap();
        let pk = [kv9_meta::codec::memcmp_uint(1)];
        txn.update(
            &kv9_meta::schema::NODES_DESC,
            &pk,
            vec![(
                kv9_meta::schema::ColumnId(3),
                kv9_meta::codec::ColumnValue::Uint(3),
            )],
        )
        .unwrap();
        let at = metadata
            .driver
            .propose_in_term(&kv9_raft::Command::from_batch(&txn.into_batch()), at.term)
            .unwrap();
        metadata
            .driver
            .wait_applied(at, Duration::from_secs(3))
            .unwrap();
    })
    .await
    .unwrap();
    send(&f, bytes).await.unwrap();
    assert_eq!(fact(&f, 2, &root, negative), Some(original.clone()));
    assert_eq!(f.hooks[0].history_reads.load(Ordering::SeqCst), 1);
    assert!(send(&f, request(&root, negative, NodeId(1), [8; 32]))
        .await
        .is_err());
    assert!(send(&f, request(&root, negative, NodeId(3), winner))
        .await
        .is_err());
    let tasks: Vec<_> = f
        .backends
        .iter()
        .map(|b| Maintenance::start(b.clone(), &tokio::runtime::Handle::current()))
        .collect();
    until(|| {
        kv9_meta::retention::checkpoint_closure::observe(
            f.backends[2]
                .node
                .meta_raft
                .store
                .engine()
                .snapshot()
                .unwrap()
                .as_ref(),
            &root,
            negative,
        )
        .is_ok_and(|work| work.registered_zero_put_closed())
    })
    .await;
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
    assert_eq!(fact(&f, 2, &root, negative), Some(original.clone()));
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
        checkpoint_refusal::refusal(view.as_ref(), &root, negative).unwrap(),
        Some(original)
    );
    for case in &cases {
        assert_eq!(
            kv9_meta::retention::retention_owner(view.as_ref(), &root, case.pending)
                .unwrap()
                .unwrap()
                .phase,
            PinPhase::Published
        );
        assert!(
            kv9_meta::retention::retention_owner(view.as_ref(), &root, case.version)
                .unwrap()
                .is_none()
        );
    }
    drop(view);
    drop(engine);
    f.close().await;
}

mod reader_admission;

mod reader_closure;
