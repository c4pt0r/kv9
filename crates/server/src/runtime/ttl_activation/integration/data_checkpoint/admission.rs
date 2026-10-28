use super::*;
use crate::checkpoint_retention::{CheckpointLedger, CheckpointOwners};
use crate::runtime::checkpoint_retention::RoutedLedger;
use kv9_common::retention::PinPhase;
use kv9_engine::checkpoint::FlushScope;
use kv9_meta::retention::{decode_owner_observation, encode_request, LedgerRequest};
use kv9_raft::driver::ApplyWaitOutcome;
use kv9_raft::{Command, ManifestChangePayload, ManifestVerdict};

#[derive(Debug, Default)]
struct NoIo(AtomicUsize);
impl NoIo {
    fn refuse<T>(&self) -> Result<T> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(Error::Engine(
            "reconciliation must not reach object I/O".into(),
        ))
    }
}
impl ObjectStore for NoIo {
    fn put(&self, _: &ObjectKey, _: &[u8]) -> Result<()> {
        self.refuse()
    }
    fn get(&self, _: &ObjectKey) -> Result<Option<Vec<u8>>> {
        self.refuse()
    }
    fn delete(&self, _: &ObjectKey) -> Result<()> {
        self.refuse()
    }
    fn list(&self, _: &str) -> Result<Vec<ObjectKey>> {
        self.refuse()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn metadata_worker_retires_closed_held_executors_in_pages_without_object_io() {
    let f = Fixture::new().await;
    let backend = f.backends[1].clone();
    let root = backend.ttl_context().unwrap().root.clone();
    let engine = backend.node.meta_raft.store.engine().clone();
    let driver = backend.driver.clone();
    let journal_path = f.path.join("n2/catalog.pending");
    let path = journal_path.clone();
    let ledger = RoutedLedger::new(&backend, tokio::runtime::Handle::current());
    let (root, pending, tokens) = tokio::task::spawn_blocking(move || {
        let good = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()));
        let plan = || {
            good.plan_checkpoint(
                engine
                    .freeze_with_scope(|view| {
                        Ok(
                            kv9_meta::checkpoint::inspect_initial_checkpoint_base(view, &root)?
                                .flush_scope(),
                        )
                    })
                    .unwrap(),
            )
            .unwrap()
        };
        let original = plan();
        let owners = CheckpointOwners::new(&root, original.manifest(), 0).unwrap();
        let mut journal = good.journal(&path);
        journal.stage_planned(&original, 0).unwrap();
        drop(original);
        owners.before_io(&ledger).unwrap();
        let intent = ledger.intent(&root, owners.pending_id()).unwrap().unwrap();
        let pending = intent
            .binding(kv9_meta::retention::OwnerKind::Pending)
            .unwrap();
        let mut tokens = Vec::new();
        for n in 1..=8 {
            let mut lifetime = ledger.local_lifetime().unwrap();
            lifetime.process = [n; 16];
            let binding =
                kv9_meta::retention::checkpoint_admission::executor_binding(&intent, lifetime)
                    .unwrap();
            ledger
                .apply(
                    encode_request(
                        root.digest(),
                        &LedgerRequest::Share {
                            from: pending.token(),
                            to: binding.clone(),
                        },
                    )
                    .unwrap(),
                )
                .unwrap();
            if n == 8 {
                ledger
                    .apply(
                        encode_request(root.digest(), &LedgerRequest::Publish(binding.token()))
                            .unwrap(),
                    )
                    .unwrap();
            }
            tokens.push(binding.token());
        }
        let winner = plan();
        let winner_owners = CheckpointOwners::new(&root, winner.manifest(), 0).unwrap();
        let mut winner_journal = good.journal(path.with_extension("winner"));
        winner_journal.stage_planned(&winner, 0).unwrap();
        drop(winner);
        winner_owners.before_io(&ledger).unwrap();
        let (prepared, predecessor) = winner_journal
            .load()
            .unwrap()
            .unwrap()
            .recover(&good)
            .unwrap();
        let at = driver
            .propose(&Command::ManifestChange(
                ManifestChangePayload::from_prepared(prepared, predecessor).unwrap(),
            ))
            .unwrap();
        assert!(matches!(
            driver.wait_applied(at, Duration::from_secs(3)).unwrap(),
            ApplyWaitOutcome::Manifest {
                verdict: ManifestVerdict::Applied { .. },
                ..
            }
        ));
        winner_owners.after_positive_settlement(&ledger).unwrap();
        winner_journal.clear_settled().unwrap();
        owners.close_admission(&ledger).unwrap();
        (root, owners.pending_id(), tokens)
    })
    .await
    .unwrap();
    let no_io = Arc::new(NoIo::default());
    let remote = crate::remote_storage::RemoteStorage::start(
        backend.node.clone(),
        backend.driver.clone(),
        crate::remote_storage::PreparedRemote::for_test(
            &f.path.join("n2"),
            &root,
            Arc::new(RemoteUploader::for_harness(no_io.clone())),
        ),
        backend.maintenance.clone(),
        Arc::new(RoutedLedger::new(
            &backend,
            tokio::runtime::Handle::current(),
        )),
    )
    .unwrap();
    until(|| FlushJournal::new(&journal_path).load().unwrap().is_none()).await;
    tokio::task::spawn_blocking(move || drop(remote))
        .await
        .unwrap();
    assert_eq!(no_io.0.load(Ordering::SeqCst), 0);
    let view = backend.node.meta_raft.store.engine().snapshot().unwrap();
    for (n, token) in tokens.iter().enumerate() {
        let owner = kv9_meta::retention::retention_owner(view.as_ref(), &root, token.owner)
            .unwrap()
            .unwrap();
        assert_eq!(
            owner.phase,
            if n < 7 {
                PinPhase::Released
            } else {
                PinPhase::Published
            }
        );
        assert_eq!(
            kv9_meta::retention::checkpoint_admission::retirement::unpublished_retirement(
                view.as_ref(),
                &root,
                pending,
                *token
            )
            .unwrap()
            .is_some(),
            n < 7
        );
    }
    assert_eq!(
        kv9_meta::retention::retention_owner(view.as_ref(), &root, pending)
            .unwrap()
            .unwrap()
            .phase,
        PinPhase::Published
    );
    drop(view);
    drop(backend);
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn closed_loser_journal_recovers_after_data_wal_reopen_without_object_io() {
    let mut f = Fixture::new().await;
    let mut single = ExtraGroup::new(&f).await;
    f.backends[0].raw_directory.remove(f.range.region);
    assert!(matches!(
        single.client(&f).activate().await.outcome,
        TtlOutcome::Success { .. }
    ));
    let root = f.backends[0].ttl_context().unwrap().root.clone();
    let engine = f.backends[0]
        .raw_directory
        .by_region(single.range.region)
        .unwrap()
        .capture_parts()
        .1
        .clone();
    let driver = single.drivers[0].clone();
    let range = single.range.clone();
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    let uploader = Arc::new(RemoteUploader::for_harness(Arc::new(
        MemoryObjectStore::new(),
    )));
    let good = uploader.clone();
    let (root, pending, executors) = tokio::task::spawn_blocking(move || {
        let scope = FlushScope {
            cluster: root.cluster_id.to_string(),
            region: range.region.0,
            conf_ver: range.conf_ver,
            version: range.version,
        };
        let (frozen, mut candidate) = engine
            .freeze_checkpoint_candidate(|_| Ok(scope.clone()))
            .unwrap();
        let plan = good.plan_checkpoint(frozen).unwrap();
        let manifest = plan.manifest().clone();
        let owners = CheckpointOwners::new(&root, &manifest, 0).unwrap();
        let mut journal = good.journal(engine.path().with_extension("pending"));
        candidate.stage(&mut journal, &plan, 0).unwrap();
        drop(plan);
        owners.before_io(&ledger).unwrap();
        let intent = ledger.intent(&root, owners.pending_id()).unwrap().unwrap();
        let pending = intent
            .binding(kv9_meta::retention::OwnerKind::Pending)
            .unwrap();
        let mut executors = Vec::new();
        // Nine crashed/paused callers stopped after Share; three others must
        // keep their distinct Published, Quiesced and ordinary Released states.
        for n in 1..=12 {
            let mut lifetime = ledger.local_lifetime().unwrap();
            lifetime.process = [n; 16];
            let binding =
                kv9_meta::retention::checkpoint_admission::executor_binding(&intent, lifetime)
                    .unwrap();
            let apply = |request| {
                ledger
                    .apply(encode_request(root.digest(), &request).unwrap())
                    .unwrap()
            };
            apply(LedgerRequest::Share {
                from: pending.token(),
                to: binding.clone(),
            });
            let phase = match n {
                1..=9 => PinPhase::Held,
                10 => PinPhase::Published,
                11 => PinPhase::Quiesced,
                _ => PinPhase::Released,
            };
            if n >= 10 {
                apply(LedgerRequest::Publish(binding.token()));
            }
            if n >= 11 {
                apply(LedgerRequest::QuiesceAfterTransfer {
                    from: binding.token(),
                    to: pending.token(),
                });
            }
            if n == 12 {
                apply(LedgerRequest::Release(binding.token()));
            }
            executors.push((binding.token(), phase));
        }
        assert!(journal.load().unwrap().unwrap().requires_upload());
        // Commit a different image into the old attempt's exact window.
        let at = driver.propose(&Command::Noop).unwrap();
        driver.wait_applied(at, Duration::from_secs(3)).unwrap();
        let winner = good.plan_checkpoint(engine.freeze(scope).unwrap()).unwrap();
        let winner_owners = CheckpointOwners::new(&root, winner.manifest(), 0).unwrap();
        let mut winner_journal = good.journal(engine.path().with_extension("winner-pending"));
        winner_journal.stage_planned(&winner, 0).unwrap();
        drop(winner);
        winner_owners.before_io(&ledger).unwrap();
        let (prepared, predecessor) = winner_journal
            .load()
            .unwrap()
            .unwrap()
            .recover(&good)
            .unwrap();
        let at = driver
            .propose(&Command::ManifestChange(
                ManifestChangePayload::from_prepared(prepared, predecessor).unwrap(),
            ))
            .unwrap();
        assert!(matches!(
            driver.wait_applied(at, Duration::from_secs(3)).unwrap(),
            ApplyWaitOutcome::Manifest {
                verdict: ManifestVerdict::Applied { .. },
                ..
            }
        ));
        winner_owners.after_positive_settlement(&ledger).unwrap();
        winner_journal.clear_settled().unwrap();
        owners.close_admission(&ledger).unwrap();
        // Crash prefix: close committed, original upload journal still durable.
        // Drop/reopen is controlled shutdown, not a process-kill qualification.
        drop(candidate);
        assert!(journal.load().unwrap().is_some());
        (root, owners.pending_id(), executors)
    })
    .await
    .unwrap();
    single.close_owner(&f);
    single.reopen(&f, &uploader);
    // Begin/Close, then first Held retirement commits but loses
    // its receipt. Queries do not increment this Apply-only failure coordinate.
    f.hooks[1].retention_calls.store(0, Ordering::SeqCst);
    f.hooks[1].retention_commits.store(0, Ordering::SeqCst);
    f.hooks[1].retention_fail_after.store(3, Ordering::SeqCst);
    let no_io = Arc::new(NoIo::default());
    let task = Maintenance::start(
        f.backends[0].clone(),
        Arc::new(RemoteUploader::for_harness(no_io.clone())),
        false,
        Duration::from_millis(100),
        &tokio::runtime::Handle::current(),
    );
    until(|| task.snapshot().failures == 1).await;
    task.cancel();
    task.drain().await;
    assert_eq!(f.hooks[1].retention_commits.load(Ordering::SeqCst), 3);
    let metadata = f.backends[1]
        .node
        .meta_raft
        .store
        .engine()
        .snapshot()
        .unwrap();
    let first = kv9_meta::retention::checkpoint_admission::executors(
        metadata.as_ref(),
        &root,
        pending,
        None,
        16,
    )
    .unwrap();
    assert_eq!(
        first
            .owners
            .iter()
            .filter(|o| o.phase == PinPhase::Held)
            .count(),
        8
    );
    assert_eq!(
        first
            .owners
            .iter()
            .filter(|o| o.phase == PinPhase::Released)
            .count(),
        2
    );
    drop(metadata);
    assert!(FlushJournal::new(single.wal().with_extension("pending"))
        .load()
        .unwrap()
        .is_some());
    assert_eq!(no_io.0.load(Ordering::SeqCst), 0);
    single.close_owner(&f);
    single.reopen(&f, &uploader);
    let task = Maintenance::start(
        f.backends[0].clone(),
        Arc::new(RemoteUploader::for_harness(no_io.clone())),
        false,
        Duration::from_millis(100),
        &tokio::runtime::Handle::current(),
    );
    until(|| task.snapshot().refusals == 1).await;
    task.cancel();
    assert_eq!(task.snapshot().failures, 0);
    assert_eq!(task.snapshot().plans, 0);
    task.drain().await;
    assert_eq!(no_io.0.load(Ordering::SeqCst), 0);
    assert!(FlushJournal::new(single.wal().with_extension("pending"))
        .load()
        .unwrap()
        .is_none());
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    let (root, records) = tokio::task::spawn_blocking(move || {
        let page = ledger.executors(root.digest(), pending, None, 16).unwrap();
        assert!(page.admission.is_closed());
        assert_eq!(page.owners.len(), 12);
        for (token, initial) in &executors {
            let observed = page
                .owners
                .iter()
                .find(|o| o.binding.token() == *token)
                .unwrap();
            let expected = if *initial == PinPhase::Held {
                PinPhase::Released
            } else {
                *initial
            };
            assert_eq!(observed.phase, expected);
            assert!(ledger
                .apply(encode_request(root.digest(), &LedgerRequest::Publish(*token)).unwrap())
                .is_err());
        }
        let owner = decode_owner_observation(
            &ledger.owner(root.digest(), pending).unwrap().unwrap(),
            root.digest(),
            pending,
        )
        .unwrap();
        assert_eq!(owner.phase, PinPhase::Published);
        (root, executors)
    })
    .await
    .unwrap();
    single.close_owner(&f);
    drop(single);
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
    let (metadata, _) = WalEngine::open(f.path.join("n2/meta.wal")).unwrap();
    let view = metadata.snapshot().unwrap();
    for (token, initial) in records {
        let proof = kv9_meta::retention::checkpoint_admission::retirement::unpublished_retirement(
            view.as_ref(),
            &root,
            pending,
            token,
        )
        .unwrap();
        assert_eq!(proof.is_some(), initial == PinPhase::Held);
        if let Some(proof) = proof {
            assert!(proof.retired_at > proof.closed_at);
        }
    }
    assert_eq!(
        kv9_meta::retention::retention_owner(view.as_ref(), &root, pending)
            .unwrap()
            .unwrap()
            .phase,
        PinPhase::Published
    );
    drop(view);
    drop(metadata);
    f.close().await;
}
