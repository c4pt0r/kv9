use super::*;
use kv9_engine::checkpoint::{CheckpointManifest, FlushScope, PlannedFlush};
use kv9_raft::driver::ApplyWaitOutcome;
use kv9_raft::{Command, ManifestChangePayload, ManifestVerdict};

pub(super) struct Case {
    pub(super) pending: OwnerId,
    pub(super) version: OwnerId,
    pub(super) positive: Option<bool>,
}

struct CancelAfterApply {
    ledger: RoutedLedger,
    cancel: CancellationToken,
    calls: AtomicUsize,
}
impl CheckpointLedger for CancelAfterApply {
    fn apply(&self, bytes: Vec<u8>) -> Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.ledger.apply(bytes)?;
        self.cancel.cancel();
        Ok(())
    }
    fn owner(&self, root: RootDigest, id: OwnerId) -> Result<Option<Vec<u8>>> {
        self.ledger.owner(root, id)
    }
}

pub(super) fn planned(engine: &WalEngine, root: &RootDescriptor, region: RegionId) -> PlannedFlush {
    RemoteUploader::plan(
        engine
            .freeze(FlushScope {
                cluster: root.cluster_id.to_string(),
                region: region.0,
                conf_ver: 1,
                version: 1,
            })
            .unwrap(),
    )
    .unwrap()
}

pub(super) fn register(
    root: &RootDescriptor,
    ledger: &RoutedLedger,
    manifest: &CheckpointManifest,
    predecessor: u64,
    positive: Option<bool>,
) -> Case {
    let owners = CheckpointOwners::new(root, manifest, predecessor).unwrap();
    owners.before_io(ledger).unwrap();
    Case {
        pending: owners.pending_id(),
        version: owners.version_id(),
        positive,
    }
}

pub(super) fn publish(
    driver: &Driver,
    uploader: &RemoteUploader,
    plan: PlannedFlush,
    predecessor: u64,
) {
    let prepared = uploader.upload_planned(plan).unwrap();
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
}

pub(super) fn phase(f: &Fixture, root: &RootDescriptor, id: OwnerId) -> Option<PinPhase> {
    kv9_meta::retention::retention_owner(
        f.backends[1]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap()
            .as_ref(),
        root,
        id,
    )
    .unwrap()
    .map(|o| o.phase)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn journal_free_background_settles_old_metadata_and_data_windows_without_an_uploader() {
    let f = Fixture::new().await;
    assert!(matches!(
        f.client(0).activate().await.outcome,
        TtlOutcome::Success { .. }
    ));
    let backend = f.backends[1].clone();
    let data = f.backends[0]
        .raw_directory
        .by_region(f.range.region)
        .unwrap();
    let data_engine = data.capture_parts().1.clone();
    let data_driver = data.capture_parts().0.clone();
    let region = f.range.region;
    let range = f.range.clone();
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    let (root, cases, execution_tokens) = tokio::task::spawn_blocking(move || {
        let root = backend.ttl_context().unwrap().root.clone();
        let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()));
        let mut cases = Vec::new();
        let mut executors = Vec::new();
        for (region, engine, driver) in [
            (
                kv9_common::META_REGION_0,
                backend.node.meta_raft.store.engine().clone(),
                backend.driver.clone(),
            ),
            (region, data_engine, data_driver),
        ] {
            let loser = planned(&engine, &root, region);
            let lost = register(&root, &ledger, loser.manifest(), 0, Some(false));
            let intent = ledger.intent(&root, lost.pending).unwrap().unwrap();
            let pending = intent.binding(OwnerKind::Pending).unwrap();
            for n in 1..=2 {
                let mut lifetime = ledger.local_lifetime().unwrap();
                lifetime.process = [n; 16];
                let owner =
                    kv9_meta::retention::checkpoint_admission::executor_binding(&intent, lifetime)
                        .unwrap();
                ledger
                    .apply(
                        encode_request(
                            root.digest(),
                            &LedgerRequest::Share {
                                from: pending.token(),
                                to: owner.clone(),
                            },
                        )
                        .unwrap(),
                    )
                    .unwrap();
                if n == 2 {
                    ledger
                        .apply(
                            encode_request(root.digest(), &LedgerRequest::Publish(owner.token()))
                                .unwrap(),
                        )
                        .unwrap();
                }
                executors.push((owner.token(), n == 2));
            }
            cases.push(lost);
            for predecessor in 0..2 {
                if region == range.region {
                    // Planning the same frozen image is deterministic. Commit
                    // an actual different TTL clock state before the winner.
                    let at = driver
                        .propose(&Command::RawTtl(
                            kv9_raft::command::RawTtlCommand::new(
                                &range,
                                kv9_raft::command::RawTtlAction::Observe {
                                    observed: kv9_common::raw_ttl::ClockSample::with_uncertainty(
                                        2000 + predecessor,
                                        0,
                                    )
                                    .unwrap()
                                    .advance(kv9_common::raw_ttl::ClockWatermark::default())
                                    .unwrap(),
                                },
                            )
                            .unwrap(),
                        ))
                        .unwrap();
                    driver.wait_applied(at, Duration::from_secs(3)).unwrap();
                }
                let winner = planned(&engine, &root, region);
                assert_ne!(
                    winner.manifest().change_id(0).unwrap(),
                    loser.manifest().change_id(0).unwrap()
                );
                cases.push(register(
                    &root,
                    &ledger,
                    winner.manifest(),
                    predecessor,
                    Some(true),
                ));
                publish(&driver, &uploader, winner, predecessor);
            }
            // This exact window has not been consumed. Newer histories in the
            // OTHER group must not authorize its negative settlement.
            cases.push(register(&root, &ledger, loser.manifest(), 2, None));
        }
        (root, cases, executors)
    })
    .await
    .unwrap();
    assert!(f.backends.iter().all(|b| b.uploader.is_none()));
    assert!(f.backends.iter().all(|b| !b
        .node
        .meta_raft
        .store
        .engine()
        .path()
        .with_extension("pending")
        .exists()));
    for case in &cases {
        assert_eq!(phase(&f, &root, case.pending), Some(PinPhase::Published));
    }
    // The publishing task and its uploader are gone. Only the global retention
    // maintenance runs; its metadata leader is a follower of the data group.
    let task = Maintenance::start(f.backends[1].clone(), &tokio::runtime::Handle::current());
    until(|| {
        cases.iter().all(|case| {
            let view = f.backends[1]
                .node
                .meta_raft
                .store
                .engine()
                .snapshot()
                .unwrap();
            let closed = executors(view.as_ref(), &root, case.pending, None, 1)
                .unwrap()
                .admission
                .is_closed();
            match case.positive {
                Some(true) => closed && phase(&f, &root, case.pending) == Some(PinPhase::Released),
                Some(false) => {
                    closed
                        && matches!(
                            observation(&f, 1, &root, case.pending),
                            sweep::SweepObservation::Done
                        )
                }
                None => !closed,
            }
        })
    })
    .await;
    until(|| task.snapshot().history_unknown >= 2).await;
    task.cancel();
    let stats = task.snapshot();
    task.drain().await;
    assert_eq!(stats.positive_handoffs, 4);
    assert_eq!(stats.negative_closures, 2);
    assert_eq!(stats.retirements_confirmed, 2);
    assert_eq!(stats.failures, 0);
    for case in &cases {
        assert_eq!(
            phase(&f, &root, case.version),
            if case.positive == Some(true) {
                Some(PinPhase::Published)
            } else {
                None
            }
        );
        if case.positive != Some(true) {
            assert_eq!(phase(&f, &root, case.pending), Some(PinPhase::Published));
        }
    }
    for (token, published) in execution_tokens {
        assert_eq!(
            phase(&f, &root, token.owner),
            Some(if published {
                PinPhase::Published
            } else {
                PinPhase::Released
            })
        );
    }
    assert_eq!(f.backends[1].maintenance.snapshot().active, 0);
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn all_positive_handoff_receipt_losses_resume_after_done_cursor_leader_change_and_wal_reopen()
{
    for cut in 1..=6 {
        let mut f = Fixture::new().await;
        let backend = f.backends[1].clone();
        let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
        let (root, case) = tokio::task::spawn_blocking(move || {
            let root = backend.ttl_context().unwrap().root.clone();
            let plan = planned(
                backend.node.meta_raft.store.engine(),
                &root,
                kv9_common::META_REGION_0,
            );
            let case = register(&root, &ledger, plan.manifest(), 0, Some(true));
            publish(
                &backend.driver,
                &RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new())),
                plan,
                0,
            );
            (root, case)
        })
        .await
        .unwrap();
        f.hooks[1].retention_calls.store(0, Ordering::SeqCst);
        f.hooks[1].retention_commits.store(0, Ordering::SeqCst);
        f.hooks[1].retention_fail_after.store(cut, Ordering::SeqCst);
        let mut state = Reconciler::default();
        assert!(pass(&mut state, &f, 1).await.is_err(), "prefix {cut}");
        assert_eq!(f.hooks[1].retention_commits.load(Ordering::SeqCst), cut);
        f.hooks[1].retention_fail_after.store(0, Ordering::SeqCst);
        if cut == 1 {
            let backend = f.backends[1].clone();
            let ledger = CancelAfterApply {
                ledger: RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current()),
                cancel: CancellationToken::new(),
                calls: AtomicUsize::new(0),
            };
            tokio::task::spawn_blocking(move || {
                let result = Reconciler::default().pass(
                    &backend,
                    &ledger,
                    &ledger.cancel,
                    Instant::now() + Duration::from_secs(5),
                    &mut Stats::default(),
                );
                assert!(result.is_err());
                assert_eq!(
                    ledger.calls.load(Ordering::SeqCst),
                    1,
                    "cancel must stop the following handoff mutations"
                );
            })
            .await
            .unwrap();
            assert!(matches!(
                observation(&f, 1, &root, case.pending),
                sweep::SweepObservation::Open
            ));
        }
        if cut >= 2 {
            // The old Held-only sweep could already have finished. Done cannot
            // suppress a positive owner handoff left between Close and Release.
            let (cursor, _) = cursor(&f, 1, &root, case.pending);
            let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
            let command =
                encode_request(root.digest(), &cursor.advance_request(case.pending, None)).unwrap();
            tokio::task::spawn_blocking(move || ledger.apply(command))
                .await
                .unwrap()
                .unwrap();
            assert!(matches!(
                observation(&f, 1, &root, case.pending),
                sweep::SweepObservation::Done
            ));
        }
        f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
        until(|| {
            f.meta[2].status().role == Role::Leader && f.meta[1].status().role != Role::Leader
        })
        .await;
        assert!(pass(&mut state, &f, 1).await.is_err());
        state = Reconciler::default();
        for _ in 0..4 {
            pass(&mut state, &f, 2).await.unwrap();
        }
        let applied = f.meta[2].status().applied_index;
        for _ in 0..4 {
            pass(&mut state, &f, 2).await.unwrap();
        }
        assert_eq!(
            f.meta[2].status().applied_index,
            applied,
            "terminal settlement cannot append forever"
        );
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
        for (id, phase) in [
            (case.pending, PinPhase::Released),
            (case.version, PinPhase::Published),
        ] {
            assert_eq!(
                kv9_meta::retention::retention_owner(view.as_ref(), &root, id)
                    .unwrap()
                    .unwrap()
                    .phase,
                phase
            );
        }
        assert!(matches!(
            sweep::observe(view.as_ref(), &root, case.pending).unwrap(),
            sweep::SweepObservation::Done
        ));
        drop(view);
        drop(engine);
        f.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn settlement_keeps_protection_without_local_replica_or_exact_valid_history() {
    let f = Fixture::new().await;
    // This test isolates absent/damaged local evidence. Remote fallback has
    // its own real RPC tests; these peers simulate having no retained window.
    for hook in &f.hooks {
        hook.history_unknown.store(true, Ordering::SeqCst);
    }
    let group = f.backends[0]
        .raw_directory
        .by_region(f.range.region)
        .unwrap();
    let engine = group.capture_parts().1.clone();
    let driver = group.capture_parts().0.clone();
    let root = f.backends[1].ttl_context().unwrap().root.clone();
    let expected_root = root.clone();
    let region = f.range.region;
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    let case = tokio::task::spawn_blocking(move || {
        let plan = planned(&engine, &root, region);
        let case = register(&root, &ledger, plan.manifest(), 0, Some(true));
        publish(
            &driver,
            &RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new())),
            plan,
            0,
        );
        case
    })
    .await
    .unwrap();
    let root = expected_root;
    until(|| f.data[1].manifest_pair(region.0).unwrap().generation == 1).await;
    let local = f.backends[1].raw_directory.by_region(region).unwrap();
    let engine = local.capture_parts().1.clone();
    assert!(!f.backends[1]
        .group_handles
        .lock()
        .unwrap()
        .contains_key(&region));
    f.backends[1].raw_directory.remove(region);
    let stats = pass(&mut Reconciler::default(), &f, 1).await.unwrap();
    assert_eq!(stats.history_unknown, 1);
    assert!(matches!(
        observation(&f, 1, &root, case.pending),
        sweep::SweepObservation::Open
    ));
    f.backends[1].raw_directory.insert(local);

    // Find the actual retained values rather than fabricating a parallel log.
    let (term, index, bytes) = f.data[1].manifest_at(region.0, 1).unwrap().unwrap();
    let mut history = term.to_be_bytes().to_vec();
    history.extend_from_slice(&index.to_be_bytes());
    history.extend_from_slice(&bytes);
    let current = f.data[1].manifest_pair(region.0).unwrap();
    let mut pair = current.generation.to_be_bytes().to_vec();
    pair.extend_from_slice(&current.last_change_id);
    let view = engine.snapshot().unwrap();
    let key_for = |value: &[u8]| {
        let rows: Vec<_> = view
            .iter(ColumnFamily::Default, b"", &[255])
            .unwrap()
            .map(|r| r.unwrap())
            .filter(|(_, v)| v == value)
            .collect();
        assert_eq!(rows.len(), 1);
        rows[0].0.clone()
    };
    let history_key = key_for(&history);
    let pair_key = key_for(&pair);
    drop(view);
    for fault in 0..5 {
        let mut batch = kv9_engine::WriteBatch::new();
        match fault {
            0 => {
                batch.delete(ColumnFamily::Default, history_key.clone());
            }
            1 => {
                batch.put(ColumnFamily::Default, history_key.clone(), vec![0; 8]);
            }
            2 => {
                let mut opaque = history[..16].to_vec();
                opaque.extend_from_slice(b"legacy opaque descriptor");
                batch.put(ColumnFamily::Default, history_key.clone(), opaque);
            }
            3 => {
                let mut mismatch = history.clone();
                mismatch[0] ^= 1;
                batch.put(ColumnFamily::Default, history_key.clone(), mismatch);
            }
            _ => {
                let mut mismatch = pair.clone();
                *mismatch.last_mut().unwrap() ^= 1;
                batch.put(ColumnFamily::Default, pair_key.clone(), mismatch);
            }
        }
        engine.write(batch).unwrap();
        let result = pass(&mut Reconciler::default(), &f, 1).await;
        if matches!(fault, 0 | 2) {
            assert_eq!(result.unwrap().history_unknown, 1);
        } else {
            assert!(result.is_err(), "damaged evidence {fault}");
        }
        assert_eq!(phase(&f, &root, case.pending), Some(PinPhase::Published));
        assert_eq!(phase(&f, &root, case.version), None);
        assert!(matches!(
            observation(&f, 1, &root, case.pending),
            sweep::SweepObservation::Open
        ));
        let mut restore = kv9_engine::WriteBatch::new();
        restore.put(ColumnFamily::Default, history_key.clone(), history.clone());
        restore.put(ColumnFamily::Default, pair_key.clone(), pair.clone());
        engine.write(restore).unwrap();
    }
    let stats = pass(&mut Reconciler::default(), &f, 1).await.unwrap();
    assert_eq!(stats.positive_handoffs, 1);
    assert_eq!(phase(&f, &root, case.pending), Some(PinPhase::Released));
    assert_eq!(phase(&f, &root, case.version), Some(PinPhase::Published));
    drop(engine);
    drop(group);
    f.close().await;
}

async fn coexist_until(
    stage: &str,
    condition: impl FnMut() -> bool,
    diagnostic: impl Fn() -> serde_json::Value,
) {
    // A liveness budget, not a timing claim: a loaded test run gets the same verdict, later.
    coexist_until_for(Duration::from_secs(120), stage, condition, diagnostic).await;
}

async fn coexist_until_for(
    budget: Duration,
    stage: &str,
    mut condition: impl FnMut() -> bool,
    diagnostic: impl Fn() -> serde_json::Value,
) {
    let started = Instant::now();
    let outcome = tokio::time::timeout(budget, async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(outcome.is_ok(), "{stage}: {}", diagnostic());
    eprintln!(
        "checkpoint coexist stage={stage} elapsed={:?}",
        started.elapsed()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn global_reconciliation_coexists_with_actual_metadata_and_data_checkpoint_workers() {
    let f = Fixture::new().await;
    assert!(matches!(
        f.client(0).activate().await.outcome,
        TtlOutcome::Success { .. }
    ));
    let root = f.backends[1].ttl_context().unwrap().root.clone();
    let uploader = Arc::new(RemoteUploader::for_harness(Arc::new(
        MemoryObjectStore::new(),
    )));
    let runtime = tokio::runtime::Handle::current();
    // Match NodeRuntime: the follower tasks must already exist so a new
    // metadata leader can finish a handoff after the publishing workers stop.
    let sweeps: Vec<_> = f
        .backends
        .iter()
        .map(|backend| Maintenance::start(backend.clone(), &runtime))
        .collect();
    let data = crate::runtime::data_checkpoint::Maintenance::start(
        f.backends[0].clone(),
        uploader.clone(),
        true,
        Duration::from_millis(100),
        &runtime,
    );
    let remote = crate::remote_storage::RemoteStorage::start(
        f.backends[1].node.clone(),
        f.backends[1].driver.clone(),
        crate::remote_storage::PreparedRemote::for_test(&f.path.join("n2"), &root, uploader),
        f.backends[1].maintenance.clone(),
        Arc::new(RoutedLedger::new(&f.backends[1], runtime)),
    )
    .unwrap();
    let diagnostic = || {
        serde_json::json!({
            "data_generation": f.data[0].manifest_pair(f.range.region.0).unwrap().generation,
            "metadata_generation": f.meta[1].manifest_pair(kv9_common::META_REGION_0.0).unwrap().generation,
            "data": data.snapshot(),
            "data_roles": f.data.iter().map(|d| format!("{:?}", d.status())).collect::<Vec<_>>(),
            "metadata_roles": f.meta.iter().map(|d| format!("{:?}", d.status())).collect::<Vec<_>>(),
            "sweeps": sweeps.iter().map(Maintenance::snapshot).collect::<Vec<_>>(),
            "maintenance": f.backends.iter().map(|b| b.maintenance.snapshot()).collect::<Vec<_>>(),
        })
    };
    coexist_until(
        "first data checkpoint",
        || {
            f.data[0]
                .manifest_pair(f.range.region.0)
                .unwrap()
                .generation
                >= 1
        },
        &diagnostic,
    )
    .await;
    assert!(matches!(
        f.client(0)
            .call(Operation::Put {
                pairs: vec![(b"concurrent-checkpoint".to_vec(), b"new".to_vec())],
                ttl_secs: Some(60),
            })
            .await
            .outcome,
        TtlOutcome::Success { .. }
    ));
    // Publication may precede a deferred owner handoff. Six catalog refusals
    // cause 0.5+1+2+4+8+16 seconds of production retry delays; the old 20-second
    // stage could expire before the next eligible attempt. This is a recovery
    // coexistence check, not a 20-second latency SLO. Allow the existing maximum
    // 30-second delay plus two 30-second worker passes without changing policy.
    coexist_until_for(
        Duration::from_secs(90),
        "second data and first metadata checkpoint",
        || {
            f.data[0]
                .manifest_pair(f.range.region.0)
                .unwrap()
                .generation
                >= 2
                && f.meta[1]
                    .manifest_pair(kv9_common::META_REGION_0.0)
                    .unwrap()
                    .generation
                    >= 1
        },
        &diagnostic,
    )
    .await;
    data.drain().await;
    tokio::task::spawn_blocking(move || drop(remote))
        .await
        .unwrap();
    let mut latest = Vec::new();
    for (driver, region) in [
        (&f.meta[1], kv9_common::META_REGION_0),
        (&f.data[0], f.range.region),
    ] {
        let generation = driver.manifest_pair(region.0).unwrap().generation;
        let (_, _, bytes) = driver.manifest_at(region.0, generation).unwrap().unwrap();
        let manifest = CheckpointManifest::decode(&bytes).unwrap();
        let owners = CheckpointOwners::new(&root, &manifest, generation - 1).unwrap();
        latest.push((owners.pending_id(), owners.version_id()));
    }
    coexist_until("latest owner handoffs", || {
        latest.iter().all(|(pending, version)| {
            phase(&f, &root, *pending) == Some(PinPhase::Released)
                && phase(&f, &root, *version) == Some(PinPhase::Published)
        })
    }, || serde_json::json!({
        "sweeps": sweeps.iter().map(Maintenance::snapshot).collect::<Vec<_>>(),
        "metadata_roles": f.meta.iter().map(|d| format!("{:?}", d.status())).collect::<Vec<_>>(),
        "serving": f.backends.iter().map(|b| format!("{:?}", b.ensure_serving())).collect::<Vec<_>>(),
        "owners": latest.iter().map(|(pending, version)| format!("{:?}: {:?}, {:?}: {:?}", pending, phase(&f, &root, *pending), version, phase(&f, &root, *version))).collect::<Vec<_>>(),
        "maintenance": f.backends.iter().map(|b| b.maintenance.snapshot()).collect::<Vec<_>>(),
    }))
    .await;
    assert!(sweeps.iter().all(|sweep| !sweep.stopped_unexpectedly()));
    for sweep in &sweeps {
        sweep.cancel();
    }
    for sweep in sweeps {
        sweep.drain().await;
    }
    assert!(f
        .backends
        .iter()
        .all(|b| b.maintenance.snapshot().active == 0));
    assert!(
        matches!(f.client(0).call(Operation::Get { keys: vec![b"concurrent-checkpoint".to_vec()] }).await.outcome,
        TtlOutcome::Success { reply: Reply::Values { values } } if values == vec![Some(b"new".to_vec())])
    );
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn background_handoff_survives_metadata_leader_change_with_original_catalog_busy() {
    let f = Fixture::new().await;
    let root = f.backends[1].ttl_context().unwrap().root.clone();
    let data = f.backends[0]
        .raw_directory
        .by_region(f.range.region)
        .unwrap();
    let engine = data.capture_parts().1.clone();
    let driver = f.data[0].clone();
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    let root_for_plan = root.clone();
    let region = f.range.region;
    let case = tokio::task::spawn_blocking(move || {
        let plan = planned(&engine, &root_for_plan, region);
        let case = register(&root_for_plan, &ledger, plan.manifest(), 0, Some(true));
        publish(
            &driver,
            &RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new())),
            plan,
            0,
        );
        case
    })
    .await
    .unwrap();
    assert_eq!(phase(&f, &root, case.pending), Some(PinPhase::Published));
    assert_eq!(phase(&f, &root, case.version), None);
    // The original node's catalog lock stays held across the actual transfer.
    // It cannot finish the handoff; a new leader must use its own worker/lock.
    let backend = f.backends[1].clone();
    let (locked, ready) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    let holder = tokio::task::spawn_blocking(move || {
        let _guard = backend.node.meta_raft.lock_catalog_txn();
        locked.send(()).unwrap();
        released.recv_timeout(Duration::from_secs(30)).unwrap();
    });
    ready.await.unwrap();
    let tasks: Vec<_> = f
        .backends
        .iter()
        .map(|backend| Maintenance::start(backend.clone(), &tokio::runtime::Handle::current()))
        .collect();
    until(|| tasks[1].snapshot().discovered > 0 && tasks[1].snapshot().failures > 0).await;
    assert_eq!(tasks[1].snapshot().positive_handoffs, 0);
    assert_eq!(phase(&f, &root, case.pending), Some(PinPhase::Published));
    f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
    until(|| f.meta[2].status().role == Role::Leader && f.meta[1].status().role != Role::Leader)
        .await;
    coexist_until("new metadata leader handoff", || {
        tasks[2].snapshot().positive_handoffs == 1
            && phase(&f, &root, case.pending) == Some(PinPhase::Released)
            && phase(&f, &root, case.version) == Some(PinPhase::Published)
    }, || serde_json::json!({
        "tasks": tasks.iter().map(Maintenance::snapshot).collect::<Vec<_>>(),
        "metadata_roles": f.meta.iter().map(|d| format!("{:?}", d.status())).collect::<Vec<_>>(),
    })).await;
    assert_eq!(tasks[1].snapshot().positive_handoffs, 0);
    assert!(
        !holder.is_finished(),
        "the old node remains blocked during takeover"
    );
    release.send(()).unwrap();
    holder.await.unwrap();
    for task in &tasks {
        task.cancel();
    }
    for task in tasks {
        task.drain().await;
    }
    assert!(f
        .backends
        .iter()
        .all(|backend| backend.maintenance.snapshot().active == 0));
    f.close().await;
}
