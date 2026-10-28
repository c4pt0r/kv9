use super::*;
use kv9_engine::checkpoint_pending_budget::PendingDiskBudget;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn data_pending_above_reopened_quota_still_publishes_adopts_and_clears_on_three_replicas() {
    let f = Fixture::new().await;
    assert!(matches!(
        f.client(0).activate().await.outcome,
        TtlOutcome::Success { .. }
    ));
    assert!(matches!(
        f.client(0)
            .call(Operation::Put {
                pairs: vec![(b"disk-pressure".to_vec(), vec![7; 4096])],
                ttl_secs: Some(60)
            })
            .await
            .outcome,
        TtlOutcome::Success { .. }
    ));
    let root = f.backends[0].ttl_context().unwrap().root.clone();
    let source = engine(&f, 0);
    let (_, scope, generation) =
        kv9_raft::state_machine::checkpoint_recovery::data_group::checkpoint_state(
            source.snapshot().unwrap().as_ref(),
            kv9_raft::state_machine::checkpoint_recovery::data_group::DataCheckpointIdentity {
                cluster: root.cluster_id,
                root: root.digest(),
                region: f.range.region,
                creation: f.range.creation,
            },
        )
        .unwrap();
    let plan = RemoteUploader::plan(source.freeze(scope).unwrap()).unwrap();
    let original = plan.manifest().clone();
    let path = source.path().with_extension("pending");
    FlushJournal::new(&path)
        .stage_planned(&plan, generation)
        .unwrap();
    drop(plan);
    // Startup can lower the limit below existing debt. Resumption must not
    // allocate a replacement journal or require a fresh candidate to make room.
    let budget = Arc::new(PendingDiskBudget::new(1).unwrap());
    budget.inventory_journal(&path).unwrap();
    let debt = budget.snapshot().retained_bytes;
    assert!(debt > 1);
    let uploader = Arc::new(
        RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()))
            .with_pending_budget(budget.clone()),
    );
    let tasks = f
        .backends
        .iter()
        .map(|backend| {
            Maintenance::start(
                backend.clone(),
                uploader.clone(),
                false,
                Duration::from_millis(100),
                &tokio::runtime::Handle::current(),
            )
        })
        .collect::<Vec<_>>();
    until(|| {
        (0..3).all(|i| selected(&f, i).is_some_and(|manifest| manifest == original))
            && !path.exists()
    })
    .await;
    for task in &tasks {
        task.cancel();
    }
    for task in tasks {
        task.drain().await;
    }
    let state = budget.snapshot();
    assert_eq!(state.retained_bytes, 0);
    assert_eq!(state.open_readers, 0);
    assert_eq!(state.open_writers, 0);
    assert!(state.prepared_updates_deferred >= 1);
    assert_eq!(state.peak_bytes, debt);
    assert!(
        matches!(f.client(0).call(Operation::Get { keys: vec![b"disk-pressure".to_vec()] }).await.outcome,
        TtlOutcome::Success { reply: Reply::Values { values } } if values == vec![Some(vec![7; 4096])])
    );
    eprintln!(
        "data_pending_disk recovery_debt={debt} limit=1 peak={} after={} prepared_deferred={}",
        state.peak_bytes, state.retained_bytes, state.prepared_updates_deferred
    );
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn metadata_pending_above_reopened_quota_finishes_without_replacement_space() {
    let f = Fixture::new().await;
    let backend = &f.backends[1];
    let root = backend.ttl_context().unwrap().root.clone();
    let engine = backend.node.meta_raft.store.engine();
    let plan = RemoteUploader::plan(
        engine
            .freeze_with_scope(|view| {
                Ok(
                    kv9_meta::checkpoint::inspect_initial_checkpoint_base(view, &root)?
                        .flush_scope(),
                )
            })
            .unwrap(),
    )
    .unwrap();
    let original = plan.manifest().clone();
    let path = f.path.join("n2/catalog.pending");
    FlushJournal::new(&path).stage_planned(&plan, 0).unwrap();
    drop(plan);
    let budget = Arc::new(PendingDiskBudget::new(1).unwrap());
    budget.inventory_journal(&path).unwrap();
    let debt = budget.snapshot().retained_bytes;
    let uploader = Arc::new(
        RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()))
            .with_pending_budget(budget.clone()),
    );
    let remote = crate::remote_storage::RemoteStorage::start(
        backend.node.clone(),
        backend.driver.clone(),
        crate::remote_storage::PreparedRemote::for_test(&f.path.join("n2"), &root, uploader),
        backend.maintenance.clone(),
        Arc::new(crate::runtime::checkpoint_retention::RoutedLedger::new(
            backend,
            tokio::runtime::Handle::current(),
        )),
    )
    .unwrap();
    until(|| {
        WalEngine::checkpoint_reference(engine.path())
            .unwrap()
            .is_some_and(|m| m == original)
            && !path.exists()
    })
    .await;
    tokio::task::spawn_blocking(move || drop(remote))
        .await
        .unwrap();
    let state = budget.snapshot();
    assert_eq!(state.retained_bytes, 0);
    assert_eq!(state.open_readers, 0);
    assert_eq!(state.open_writers, 0);
    assert!(state.prepared_updates_deferred >= 1);
    assert_eq!(state.peak_bytes, debt);
    eprintln!(
        "metadata_pending_disk recovery_debt={debt} limit=1 peak={} after={} prepared_deferred={}",
        state.peak_bytes, state.retained_bytes, state.prepared_updates_deferred
    );
    f.close().await;
}
