use super::*;
use crate::checkpoint_retention::CheckpointRead;
use kv9_meta::retention::{checkpoint_reader, reader_lifecycle, OwnerBinding};

async fn apply(f: &Fixture, root: &RootDescriptor, request: LedgerRequest) -> Result<()> {
    send(f, encode_request(root.digest(), &request)?).await
}
fn observed(f: &Fixture, root: &RootDescriptor, b: &OwnerBinding) -> PinPhase {
    let leader = f
        .meta
        .iter()
        .position(|d| d.status().role == Role::Leader)
        .unwrap();
    kv9_meta::retention::retention_owner(
        f.backends[leader]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap()
            .as_ref(),
        root,
        b.descriptor.id,
    )
    .unwrap()
    .unwrap()
    .phase
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refused_pending_fences_new_reads_while_actual_get_drains_across_leader_and_receipt_loss() {
    let f = Fixture::new().await;
    let (root, cases) = remote::committed(&f, true).await;
    let pending = cases
        .iter()
        .find(|c| c.positive == Some(false))
        .unwrap()
        .pending;
    let intent = kv9_meta::retention::checkpoint_intent::checkpoint_intent(
        f.backends[1]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap()
            .as_ref(),
        &root,
        pending,
    )
    .unwrap()
    .unwrap();
    let source = intent.binding(OwnerKind::Pending).unwrap();
    let active =
        checkpoint_reader::binding(&source, f.backends[0].ttl_context().unwrap().lifetime).unwrap();
    let mut lifetime = f.backends[0].ttl_context().unwrap().lifetime;
    lifetime.process = [210; 16];
    let held = checkpoint_reader::binding(&source, lifetime).unwrap();
    lifetime.process = [211; 16];
    let fresh = checkpoint_reader::binding(&source, lifetime).unwrap();
    apply(
        &f,
        &root,
        LedgerRequest::Share {
            from: source.token(),
            to: held.clone(),
        },
    )
    .await
    .unwrap();

    let (store, pause) =
        kv9_engine::MinioObjectStore::with_paused_memory_worker(Duration::from_millis(100))
            .unwrap();
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    let r = root.clone();
    let manifest = intent.manifest().unwrap();
    let predecessor = intent.predecessor();
    let reader = tokio::task::spawn_blocking(move || {
        let uploader = RemoteUploader::new(Arc::new(store));
        let mut reader =
            CheckpointRead::new(&r, &manifest, predecessor, OwnerKind::Pending, &ledger).unwrap();
        let (result, report) = uploader
            .with_local_read_report(|scope| {
                reader.before_io(&ledger, scope)?;
                scope.restore(&manifest)
            })
            .unwrap();
        reader.observe_drain(report).unwrap();
        (result, reader.finish(&ledger))
    });
    let pause = tokio::task::spawn_blocking(move || {
        assert!(pause.wait_for_closed_execution(Duration::from_secs(10)));
        pause
    })
    .await
    .unwrap();
    assert!(
        !reader.is_finished(),
        "GET timeout must still wait for actual object task drain"
    );
    assert_eq!(observed(&f, &root, &active), PinPhase::Published);
    let winner: [u8; 32] = f.data[0]
        .manifest_pair(f.range.region.0)
        .unwrap()
        .last_change_id
        .try_into()
        .unwrap();
    let calls = f.hooks[1].retention_calls.load(Ordering::SeqCst);
    f.hooks[1]
        .retention_fail_after
        .store(calls + 1, Ordering::SeqCst);
    let refusal = request(&root, pending, NodeId(1), winner);
    assert!(send(&f, refusal.clone()).await.is_err());
    assert!(fact(&f, 1, &root, pending).is_some());
    f.hooks[1].retention_fail_after.store(0, Ordering::SeqCst);
    f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
    until(|| f.meta[2].status().role == Role::Leader && f.meta[1].status().role != Role::Leader)
        .await;
    for hook in &f.hooks {
        hook.history_refuse.store(true, Ordering::SeqCst);
    }
    send(&f, refusal).await.unwrap(); // exact committed fact, no source lookup
    for binding in [&fresh, &held, &active] {
        assert!(apply(
            &f,
            &root,
            LedgerRequest::Share {
                from: source.token(),
                to: binding.clone()
            }
        )
        .await
        .is_err());
        assert!(apply(&f, &root, LedgerRequest::Publish(binding.token()))
            .await
            .is_err());
        assert!(apply(&f, &root, LedgerRequest::Acquire(binding.clone()))
            .await
            .is_err());
    }
    assert_eq!(observed(&f, &root, &held), PinPhase::Held);
    assert_eq!(observed(&f, &root, &active), PinPhase::Published);
    assert_eq!(
        settlement::phase(&f, &root, pending),
        Some(PinPhase::Published)
    );
    assert!(!reader.is_finished());

    // The original process drains only after the paused object task exits.
    // Its exact terminal generation commits, while the receipt is dropped.
    f.hooks[2].retention_read_fault.store(2, Ordering::SeqCst);
    drop(pause);
    let (get, finish) = tokio::time::timeout(Duration::from_secs(5), reader)
        .await
        .unwrap()
        .unwrap();
    assert!(get.is_err());
    assert!(finish.is_err());
    assert_eq!(observed(&f, &root, &active), PinPhase::Released);
    let outbox = f.backends[0]
        .ttl_context()
        .unwrap()
        .drain_outbox
        .as_ref()
        .unwrap()
        .clone();
    assert!(outbox.has_pending());
    f.hooks[2].retention_read_fault.store(0, Ordering::SeqCst);
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    let replay = outbox.clone();
    assert!(
        tokio::task::spawn_blocking(move || replay.replay_one(&ledger))
            .await
            .unwrap()
            .unwrap()
    );
    assert!(!outbox.has_pending());
    apply(
        &f,
        &root,
        LedgerRequest::FinishCheckpointRead {
            successor: source.token(),
            reader: held.clone(),
        },
    )
    .await
    .unwrap();
    let mut next = active.clone();
    next.generation = std::num::NonZeroU64::new(2).unwrap();
    assert!(apply(
        &f,
        &root,
        LedgerRequest::Share {
            from: source.token(),
            to: next.clone()
        }
    )
    .await
    .is_err());
    assert_eq!(observed(&f, &root, &active), PinPhase::Released);
    let path = f.shutdown_keep().await;
    let (engine, _) = WalEngine::open(path.join("n3/meta.wal")).unwrap();
    let view = engine.snapshot().unwrap();
    assert!(checkpoint_refusal::refusal(view.as_ref(), &root, pending)
        .unwrap()
        .is_some());
    for b in [&active, &held] {
        let record = reader_lifecycle::generation(view.as_ref(), &root, b.token())
            .unwrap()
            .unwrap();
        assert_eq!(record.origin, reader_lifecycle::Origin::Entered);
        assert!(record.finished_at > record.entered_at);
        assert_eq!(record.published_at != 0, b == &active);
    }
    assert!(
        kv9_meta::retention::retention_owner(view.as_ref(), &root, fresh.descriptor.id)
            .unwrap()
            .is_none()
    );
    assert!(kv9_meta::retention::plan_retention(
        view.as_ref(),
        &root,
        &LedgerRequest::Share {
            from: source.token(),
            to: next,
        }
    )
    .is_err());
    assert_eq!(
        kv9_meta::retention::retention_owner(view.as_ref(), &root, pending)
            .unwrap()
            .unwrap()
            .phase,
        PinPhase::Published
    );
    drop(view);
    drop(engine);
    drop(outbox);
    std::fs::remove_dir_all(path).unwrap();
}
