use super::*;
use kv9_engine::MemoryObjectStore;
use kv9_meta::retention::checkpoint_admission::drain::checkpoint_drain;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn exact_scope_drain_reports_survive_lost_receipt_later_generation_and_wal_reopen() {
    let mut f = Fixture::new().await;
    let backend = f.backends[0].clone();
    let metadata = f.backends[1].node.meta_raft.store.engine().clone();
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
    let (root, pending, expected) = tokio::task::spawn_blocking(move || {
        let planned = RemoteUploader::plan(
            engine
                .freeze(FlushScope {
                    cluster: root.cluster_id.to_string(),
                    region: range.region.0,
                    conf_ver: range.conf_ver,
                    version: range.version,
                })
                .unwrap(),
        )
        .unwrap();
        let manifest = planned.manifest().clone();
        let make = || CheckpointOwners::new(&root, &manifest, 0).unwrap();
        make().before_io(&ledger).unwrap();
        let pending = make().pending_id();
        let id = make()
            .execution_id(ledger.local_lifetime().unwrap())
            .unwrap();
        let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()));
        let mut first = make().execution(&ledger).unwrap();
        let ((), report) = uploader
            .with_local_execution_report(|scoped| {
                first.before_io_scoped(&ledger, scoped).unwrap();
                assert!(
                    first.after_local_drain(&ledger).is_err(),
                    "a live scope cannot be reported as drained"
                );
            })
            .unwrap();
        let (_, wrong) = uploader
            .with_local_execution_report(|scoped| {
                let mut identity = report.identity().unwrap();
                identity.executor.generation = std::num::NonZeroU64::new(2).unwrap();
                scoped.bind_execution(identity).unwrap();
            })
            .unwrap();
        assert!(first.observe_drain(wrong).is_err());
        first.observe_drain(report).unwrap();
        let fault = Fault {
            inner: &ledger,
            calls: AtomicUsize::new(0),
            fail_at: 1,
            before: false,
            lifetime: ledger.local_lifetime().unwrap(),
        };
        assert!(first.after_local_drain(&fault).is_err());
        assert_eq!(fault.calls.load(Ordering::SeqCst), 1);
        let token1 = observe(&ledger, root.digest(), id).binding.token();
        let read = |token| {
            checkpoint_drain(metadata.snapshot().unwrap().as_ref(), &root, pending, token)
                .unwrap()
                .unwrap()
        };
        let original = read(token1);
        assert_eq!((original.issued, original.resolved), (0, 0));
        first.after_local_drain(&ledger).unwrap();
        assert_eq!(read(token1), original);
        assert!(uploader
            .with_local_execution_report(|scoped| first.before_io_scoped(&ledger, scoped))
            .unwrap()
            .0
            .is_err());

        let mut second = make().execution(&ledger).unwrap();
        let (result, report) = uploader
            .with_local_execution_report(|scoped| {
                second.before_io_scoped(&ledger, scoped).unwrap();
                scoped.upload_planned(planned)
            })
            .unwrap();
        result.unwrap();
        second.observe_drain(report).unwrap();
        second.after_local_drain(&ledger).unwrap();
        let token2 = observe(&ledger, root.digest(), id).binding.token();
        let unknown = read(token2);
        assert!(unknown.issued > 0);
        assert_eq!(
            unknown.resolved, 0,
            "opaque backend success is not qualified remote evidence"
        );

        let mut third = make().execution(&ledger).unwrap();
        let ((), report) = uploader
            .with_local_execution_report(|scoped| {
                third.before_io_scoped(&ledger, scoped).unwrap();
                assert_eq!(
                    read(token2),
                    unknown,
                    "historical evidence survives a new Published generation"
                );
            })
            .unwrap();
        third.observe_drain(report).unwrap();
        third.after_local_drain(&ledger).unwrap();
        let token3 = observe(&ledger, root.digest(), id).binding.token();
        assert_eq!(token3.generation.get(), 3);
        let mut records = vec![original, unknown, read(token3)];
        // Actual scoped execution errors before/after Share and Publish. Even
        // the unentered case finishes and records zero in one committed batch.
        for (n, (fail_at, before)) in [(1, true), (1, false), (2, true), (2, false)]
            .into_iter()
            .enumerate()
        {
            let mut lifetime = ledger.local_lifetime().unwrap();
            lifetime.process = [n as u8 + 10; 16];
            let fault = Fault {
                inner: &ledger,
                calls: AtomicUsize::new(0),
                fail_at,
                before,
                lifetime,
            };
            let mut execution = make().execution(&fault).unwrap();
            let (result, report) = uploader
                .with_local_execution_report(|scoped| execution.before_io_scoped(&fault, scoped))
                .unwrap();
            assert!(result.is_err());
            assert_eq!(fault.calls.load(Ordering::SeqCst), fail_at);
            let token = report.identity().unwrap().executor;
            assert_eq!((report.issued_puts(), report.resolved_puts()), (0, 0));
            execution.observe_drain(report).unwrap();
            execution.after_local_drain(&ledger).unwrap();
            records.push(read(token));
        }
        make().close_admission(&ledger).unwrap();
        // A delayed next-generation entry after gate closure also emits no I/O
        // and can settle atomically without replacing the old unknown report.
        let mut blocked = make().execution(&ledger).unwrap();
        let (result, report) = uploader
            .with_local_execution_report(|scoped| blocked.before_io_scoped(&ledger, scoped))
            .unwrap();
        assert!(result.is_err());
        let token = report.identity().unwrap().executor;
        blocked.observe_drain(report).unwrap();
        blocked.after_local_drain(&ledger).unwrap();
        records.push(read(token));
        for record in &records {
            assert_eq!(read(record.executor), *record);
        }
        (root, pending, records)
    })
    .await
    .unwrap();
    drop(backend);
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
    let (engine, _) = WalEngine::open(f.path.join("n2/meta.wal")).unwrap();
    let view = engine.snapshot().unwrap();
    for record in expected {
        assert_eq!(
            checkpoint_drain(view.as_ref(), &root, pending, record.executor).unwrap(),
            Some(record)
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
    drop(engine);
    f.close().await;
}
