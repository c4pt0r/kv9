use super::*;
use kv9_engine::MemoryObjectStore;
use kv9_meta::retention::checkpoint_admission::drain::checkpoint_drain;

fn reopen_report_backend(f: &mut Fixture, root: &RootDescriptor) {
    let old = &f.backends[0];
    let identity = StoreIdentity::for_voter(root, NodeId(1)).unwrap();
    let mut context = ActivationContext::new(
        root.clone(),
        f._guards[0].raw_ttl_format_fence(root, &identity).unwrap(),
        "cluster-test".into(),
    )
    .unwrap()
    .with_drain_outbox(&f.path.join("n1"))
    .unwrap();
    *context.channels.get_mut().unwrap() =
        old.ttl_context().unwrap().channels.lock().unwrap().clone();
    assert_ne!(
        context.lifetime.process,
        old.ttl_context().unwrap().lifetime.process
    );
    f.backends[0] = Arc::new(RuntimeBackend {
        maintenance: old.maintenance.clone(),
        ttl_activation: Some(Arc::new(context)),
        raw_directory: old.raw_directory.clone(),
        group_handles: old.group_handles.clone(),
        adoption_receipts: old.adoption_receipts.clone(),
        node: old.node.clone(),
        driver: old.driver.clone(),
        transport: old.transport.clone(),
        endpoint_ready: old.endpoint_ready.clone(),
        initial_voters: old.initial_voters.clone(),
        uploader: None,
        status_cache: Arc::default(),
        catalog_ring: crate::catalog_scope::ring::ChangeRing::from_env(None),
        slice_watch: Arc::default(),
        scope_stats: Arc::default(),
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn report_outbox_recovers_original_process_on_metadata_follower_without_uploader_and_preserves_unknown(
) {
    let mut f = Fixture::new().await;
    let backend = f.backends[0].clone();
    let root = backend.ttl_context().unwrap().root.clone();
    let engine = backend
        .raw_directory
        .by_region(f.range.region)
        .unwrap()
        .capture_parts()
        .1
        .clone();
    let range = f.range.clone();
    let ledger = RoutedLedger::new(&backend, tokio::runtime::Handle::current());
    let expected = tokio::task::spawn_blocking(move || {
        let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()));
        let mut expected = Vec::new();
        // Metadata and data observations use the same node outbox. The third
        // executor deliberately loses its report before any local persistence.
        for (n, (region, engine)) in [
            (META_REGION_0, backend.node.meta_raft.store.engine().clone()),
            (range.region, engine.clone()),
            (range.region, engine),
        ]
        .into_iter()
        .enumerate()
        {
            let planned = RemoteUploader::plan(
                engine
                    .freeze(FlushScope {
                        cluster: root.cluster_id.to_string(),
                        region: region.0,
                        conf_ver: 1,
                        version: 1,
                    })
                    .unwrap(),
            )
            .unwrap();
            let owners = CheckpointOwners::new(&root, planned.manifest(), n as u64).unwrap();
            owners.before_io(&ledger).unwrap();
            let pending = owners.pending_id();
            let mut execution = owners.execution(&ledger).unwrap();
            let (result, report) = uploader
                .with_local_execution_report(|scoped| {
                    execution.before_io_scoped(&ledger, scoped)?;
                    if n == 1 {
                        scoped.upload_planned(planned)?;
                    }
                    Ok::<_, Error>(())
                })
                .unwrap();
            result.unwrap();
            let token = report.identity().unwrap().executor;
            let counts = (report.issued_puts(), report.resolved_puts());
            if n == 1 {
                assert!(counts.0 > 0);
                assert_eq!(counts.1, 0);
            }
            if n < 2 {
                execution.observe_drain(report).unwrap();
                let fault = Fault {
                    inner: &ledger,
                    calls: AtomicUsize::new(0),
                    fail_at: 1,
                    before: n == 0,
                    lifetime: ledger.local_lifetime().unwrap(),
                };
                assert!(execution.after_local_drain(&fault).is_err());
            } // n==2 loses the actual report; recovery must not fabricate zero.
            drop(execution);
            expected.push((pending, token, (n < 2).then_some(counts)));
        }
        expected
    })
    .await
    .unwrap();
    let root = f.backends[0].ttl_context().unwrap().root.clone();
    let outbox_path = f.path.join("n1/checkpoint-drains");
    assert_eq!(std::fs::read_dir(&outbox_path).unwrap().count(), 2);
    let snapshot = f.backends[1]
        .node
        .meta_raft
        .store
        .engine()
        .snapshot()
        .unwrap();
    assert!(
        checkpoint_drain(snapshot.as_ref(), &root, expected[0].0, expected[0].1)
            .unwrap()
            .is_none()
    );
    let committed = checkpoint_drain(snapshot.as_ref(), &root, expected[1].0, expected[1].1)
        .unwrap()
        .unwrap();
    drop(snapshot);
    // Transfer the already-finished upload's exact image twice. Its original
    // Pending and Version are no longer Published before the lost report is
    // acknowledged. The Snapshot keeps all objects protected throughout.
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    let r = root.clone();
    let pending = expected[1].0;
    let (version, holder) = tokio::task::spawn_blocking(move || {
        use kv9_meta::retention::{encode_request, LedgerRequest, OwnerKind};
        let intent = ledger.intent(&r, pending).unwrap().unwrap();
        let p = intent.binding(OwnerKind::Pending).unwrap();
        let v = intent.binding(OwnerKind::Version).unwrap();
        let mut holder = v.clone();
        holder.descriptor.kind = OwnerKind::Snapshot;
        holder.descriptor.id = OwnerId::new([98; 16]).unwrap();
        for (from, to) in [(&p, &v), (&v, &holder)] {
            for request in [
                LedgerRequest::Share {
                    from: from.token(),
                    to: to.clone(),
                },
                LedgerRequest::Publish(to.token()),
                LedgerRequest::QuiesceAfterTransfer {
                    from: from.token(),
                    to: to.token(),
                },
                LedgerRequest::Release(from.token()),
            ] {
                ledger
                    .apply(encode_request(r.digest(), &request).unwrap())
                    .unwrap();
            }
        }
        assert!(ledger.intent(&r, pending).is_err());
        (v, holder)
    })
    .await
    .unwrap();
    f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
    tokio::time::timeout(Duration::from_secs(10), async {
        while f.meta[2].status().role != Role::Leader {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    // Reconstruct the process identity and disk queue, retaining the live Raft
    // fixture. This tests component recovery, not an OS process-kill deployment.
    reopen_report_backend(&mut f, &root);
    assert_ne!(f.backends[0].driver.status().role, Role::Leader);
    let queue = f.backends[0]
        .ttl_context()
        .unwrap()
        .drain_outbox
        .clone()
        .unwrap();
    let task = crate::runtime::checkpoint_sweep::Maintenance::start(
        f.backends[0].clone(),
        &tokio::runtime::Handle::current(),
    );
    tokio::time::timeout(Duration::from_secs(15), async {
        while queue.has_pending() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        task.snapshot().discovered,
        0,
        "metadata follower only replays its local reports"
    );
    task.drain().await;
    assert_eq!(std::fs::read_dir(&outbox_path).unwrap().count(), 0);
    let at = f.meta[2].propose(&Command::Noop).unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !f
            .meta
            .iter()
            .all(|d| d.driver_applied().is_some_and(|p| p.index >= at.index.0))
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let snapshot = f.backends[1]
        .node
        .meta_raft
        .store
        .engine()
        .snapshot()
        .unwrap();
    assert_eq!(
        checkpoint_drain(snapshot.as_ref(), &root, expected[1].0, expected[1].1)
            .unwrap()
            .unwrap(),
        committed
    );
    drop(snapshot);
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
    for n in 1..=3 {
        let (engine, _) = WalEngine::open(f.path.join(format!("n{n}/meta.wal"))).unwrap();
        let view = engine.snapshot().unwrap();
        for &(pending, token, counts) in &expected {
            let report = checkpoint_drain(view.as_ref(), &root, pending, token).unwrap();
            assert_eq!(report.map(|r| (r.issued, r.resolved)), counts);
            let executor = kv9_meta::retention::retention_owner(view.as_ref(), &root, token.owner)
                .unwrap()
                .unwrap();
            assert_eq!(
                executor.phase,
                if counts.is_some() {
                    PinPhase::Released
                } else {
                    PinPhase::Published
                }
            );
            assert_eq!(
                kv9_meta::retention::retention_owner(view.as_ref(), &root, pending)
                    .unwrap()
                    .unwrap()
                    .phase,
                if pending == expected[1].0 {
                    PinPhase::Released
                } else {
                    PinPhase::Published
                }
            );
        }
        for (binding, phase) in [
            (&version, PinPhase::Released),
            (&holder, PinPhase::Published),
        ] {
            assert_eq!(
                kv9_meta::retention::retention_owner(view.as_ref(), &root, binding.descriptor.id)
                    .unwrap()
                    .unwrap()
                    .phase,
                phase
            );
        }
    }
}
