use super::*;
use crate::checkpoint_retention::CheckpointOwners;
use kv9_meta::retention::{
    checkpoint_intent::CheckpointIntent, image_publisher_closure, metadata_image_write_gate,
};
use read_gate::{select, send, stop, tasks, wait};
use std::sync::{Condvar, Mutex};

#[derive(Debug, Default)]
struct PausedPut {
    memory: MemoryObjectStore,
    started: AtomicBool,
    released: Mutex<bool>,
    wake: Condvar,
}
impl PausedPut {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.wake.notify_all();
    }
}
struct ResumeOnDrop(Arc<PausedPut>);
impl Drop for ResumeOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}
impl ObjectStore for PausedPut {
    fn put(&self, key: &ObjectKey, bytes: &[u8]) -> Result<()> {
        self.started.store(true, Ordering::SeqCst);
        let guard = self.released.lock().unwrap();
        drop(self.wake.wait_while(guard, |released| !*released).unwrap());
        self.memory.put(key, bytes)
    }
    fn get(&self, key: &ObjectKey) -> Result<Option<Vec<u8>>> {
        self.memory.get(key)
    }
    fn delete(&self, _: &ObjectKey) -> Result<()> {
        panic!("no GC")
    }
    fn list(&self, _: &str) -> Result<Vec<ObjectKey>> {
        panic!("no listing")
    }
}
fn observed(
    f: &Fixture,
    r: &RootDescriptor,
    i: &CheckpointIntent,
    leader: usize,
) -> image_publisher_closure::Work {
    image_publisher_closure::observe(
        f.backends[leader]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap()
            .as_ref(),
        r,
        i.pending_id(),
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn image_upload_gate_and_unknown_publisher_survive_lost_page_failover_and_all_store_recovery()
{
    let f = Fixture::new().await;
    let root = f.backends[0].ttl_context().unwrap().root.clone();
    let objects = Arc::new(Objects::default());
    let uploader = Arc::new(RemoteUploader::for_harness(objects.clone()));
    let (old, p) = retirement::publish(&f, uploader.clone()).await;
    for n in 0..3 {
        select(&f, &root, &old, p, n).await;
    }
    let i = CheckpointIntent::new(&root, &old, p).unwrap();
    // Re-plan the exact recovered immutable image, under an unrelated Pending alias.
    let m = old.clone();
    let u = uploader.clone();
    let replay_path = f.path.join("replan.wal");
    let planned = tokio::task::spawn_blocking(move || {
        let restored = u.restore(&m).unwrap();
        let (replan, _) = kv9_engine::WalEngine::open(&replay_path).unwrap();
        let view = restored.snapshot().unwrap();
        let mut batch = kv9_engine::WriteBatch::new();
        for file in &m.files {
            let cf = kv9_engine::ColumnFamily::ALL[file.cf as usize];
            let mut end = file.largest.clone();
            end.push(0);
            for entry in view.iter(cf, &file.smallest, &end).unwrap() {
                let (k, v) = entry.unwrap();
                batch.put(cf, k, v);
            }
        }
        use kv9_engine::ReplicatedEngine;
        replan
            .write_applied(
                batch,
                kv9_common::AppliedPosition {
                    term: m.term,
                    index: m.index,
                },
            )
            .unwrap();
        let planned = RemoteUploader::plan(replan.freeze(m.scope.clone()).unwrap()).unwrap();
        assert_eq!(planned.manifest(), &m);
        planned
    })
    .await
    .unwrap();
    let alias = CheckpointIntent::new(&root, &old, p + 100).unwrap();
    let blocked = Arc::new(PausedPut::default());
    let _resume = ResumeOnDrop(blocked.clone());
    let r = root.clone();
    let m = old.clone();
    let b = blocked.clone();
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    let active = tokio::task::spawn_blocking(move || {
        let owners = CheckpointOwners::new(&r, &m, p + 100).unwrap();
        owners.before_io(&ledger).unwrap();
        let mut execution = owners.execution(&ledger).unwrap();
        let u = RemoteUploader::for_harness(b);
        let (result, report) = u
            .with_local_execution_report(|scope| {
                execution.before_io_scoped(&ledger, scope)?;
                scope.upload_planned(planned)
            })
            .unwrap();
        result.unwrap();
        let counts = (report.issued_puts(), report.resolved_puts());
        let token = report.identity().unwrap().executor;
        execution.observe_drain(report).unwrap();
        execution.after_local_drain(&ledger).unwrap();
        (token, counts)
    });
    wait("real old-image PUT started", || {
        blocked.started.load(Ordering::SeqCst)
    })
    .await;
    assert!(!active.is_finished());
    let (new, q) = retirement::publish(&f, uploader.clone()).await;
    for n in 0..3 {
        select(&f, &root, &new, q, n).await;
    }
    let pending = i.binding(OwnerKind::Pending).unwrap().token();
    send(
        &f,
        &root,
        LedgerRequest::AdvanceMetadataReadGate {
            pending,
            expected: 0,
        },
    )
    .await
    .unwrap();
    send(&f, &root, LedgerRequest::CloseMetadataImageReads(pending))
        .await
        .unwrap();
    // Drive the finite initial catalog backfill through the production bounded
    // page implementation before testing the periodic per-image maintenance.
    let mut complete = false;
    for _ in 0..256 {
        let backend = f.backends[1].clone();
        let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
        complete = tokio::task::spawn_blocking(move || {
            let mut stats = crate::runtime::checkpoint_sweep::Stats::default();
            crate::runtime::checkpoint_sweep::build_index::pass(
                &backend,
                &ledger,
                &tokio_util::sync::CancellationToken::new(),
                Instant::now() + Duration::from_secs(5),
                &mut stats,
            )
            .unwrap();
            stats.image_publisher_index_catalog_complete
        })
        .await
        .unwrap();
        if complete {
            break;
        }
    }
    assert!(complete);
    f.hooks[1]
        .retention_image_publisher_census_fault
        .store(2, Ordering::SeqCst);
    // Route from a follower so this deliberately lost receipt traverses the
    // authenticated RPC hook; a leader-local ledger correctly bypasses RPC.
    let mut state = crate::runtime::checkpoint_sweep::Reconciler::default();
    for _ in 0..128 {
        let backend = f.backends[1].clone();
        let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
        let (next, result) = tokio::task::spawn_blocking(move || {
            let result = state.pass(
                &backend,
                &ledger,
                &tokio_util::sync::CancellationToken::new(),
                Instant::now() + Duration::from_secs(5),
                &mut crate::runtime::checkpoint_sweep::Stats::default(),
            );
            (state, result)
        })
        .await
        .unwrap();
        state = next;
        if f.hooks[1]
            .retention_image_publisher_census_fault
            .load(Ordering::SeqCst)
            == 1
        {
            assert!(result.is_err());
            break;
        }
        result.unwrap();
    }
    assert_eq!(
        f.hooks[1]
            .retention_image_publisher_census_fault
            .load(Ordering::SeqCst),
        1
    );
    assert!(!active.is_finished());
    assert!(matches!(
        metadata_image_write_gate::observe(
            f.backends[1]
                .node
                .meta_raft
                .store
                .engine()
                .snapshot()
                .unwrap()
                .as_ref(),
            &root,
            alias.pending_id()
        )
        .unwrap(),
        metadata_image_write_gate::Work::Closed(_)
    ));
    let puts = objects.puts.load(Ordering::SeqCst);
    let r = root.clone();
    let m = old.clone();
    let u = uploader.clone();
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    tokio::task::spawn_blocking(move || {
        let owners = CheckpointOwners::new(&r, &m, p + 101).unwrap();
        owners.before_io(&ledger).unwrap();
        let mut execution = owners.execution(&ledger).unwrap();
        let (result, report) = u
            .with_local_execution_report(|scope| execution.before_io_scoped(&ledger, scope))
            .unwrap();
        assert!(result.is_err());
        assert_eq!((report.issued_puts(), report.resolved_puts()), (0, 0));
        execution.observe_drain(report).unwrap();
        execution.after_local_drain(&ledger).unwrap();
    })
    .await
    .unwrap();
    assert_eq!(objects.puts.load(Ordering::SeqCst), puts);
    f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
    wait("Publisher census new metadata leader", || {
        f.meta[2].status().role == Role::Leader && f.meta[1].status().role != Role::Leader
    })
    .await;
    send(
        &f,
        &root,
        LedgerRequest::AdvanceImagePublisherClosure {
            pending,
            expected: 0,
        },
    )
    .await
    .unwrap();
    let maintenance = tasks(&f);
    wait("active Publisher remains a census blocker", || {
        matches!(
            observed(&f, &root, &i, 2),
            image_publisher_closure::Work::Complete(_)
        )
    })
    .await;
    stop(maintenance).await;
    let image_publisher_closure::Work::Complete(active_census) = observed(&f, &root, &i, 2) else {
        panic!()
    };
    assert_eq!(active_census.counts.active, 1);
    assert!(!active.is_finished());
    blocked.release();
    let (executor, counts) = tokio::time::timeout(Duration::from_secs(10), active)
        .await
        .unwrap()
        .unwrap();
    assert!(counts.0 > 0);
    assert_eq!(counts.1, 0);
    assert!(matches!(
        observed(&f, &root, &i, 2),
        image_publisher_closure::Work::Advance { restart: true, .. }
    ));
    let maintenance = tasks(&f);
    wait("retained actual nonzero Publisher report", || {
        matches!(
            observed(&f, &root, &i, 2),
            image_publisher_closure::Work::Complete(_)
        )
    })
    .await;
    assert_eq!(maintenance[2].snapshot().peak_in_flight, 1);
    stop(maintenance).await;
    let image_publisher_closure::Work::Complete(done) = observed(&f, &root, &i, 2) else {
        panic!()
    };
    assert_eq!(
        (
            done.counts.nonzero_put,
            done.counts.unresolved_puts,
            done.counts.active
        ),
        (1, counts.0, 0)
    );
    assert!(!observed(&f, &root, &i, 2).registered_zero_put_closed());
    let at = f.meta[2].driver_applied().unwrap();
    wait("image Publisher census replicated to all stores", || {
        f.meta
            .iter()
            .all(|d| d.driver_applied().is_some_and(|a| a.index >= at.index))
    })
    .await;
    let directory = f.shutdown_keep().await;
    for n in 1..=3 {
        let path = directory.join(format!("n{n}"));
        let guard = StoreGuard::lock(&path).unwrap();
        let mut storage = DiskRaftStorage::recover(&path.join("meta-raft")).unwrap();
        let bootstrap = BootstrapReader {
            directory: &path,
            root: &root,
            identity: StoreIdentity::for_voter(&root, NodeId(n)).unwrap(),
            uploader: &uploader,
        };
        let (e, _, anchor) = crate::bootstrap_retention::startup::open_metadata::<RoutedLedger>(
            &guard,
            &mut storage,
            &path.join("meta.wal"),
            &bootstrap,
            || panic!("authorized offline startup queried quorum"),
        )
        .unwrap();
        assert_eq!(anchor.unwrap().descriptor().manifest, new.encode().unwrap());
        let view = e.snapshot().unwrap();
        assert_eq!(
            image_publisher_closure::observe(view.as_ref(), &root, i.pending_id()).unwrap(),
            image_publisher_closure::Work::Complete(done.clone())
        );
        assert!(matches!(
            metadata_image_write_gate::observe(view.as_ref(), &root, alias.pending_id()).unwrap(),
            metadata_image_write_gate::Work::Closed(_)
        ));
        let report = kv9_meta::retention::checkpoint_admission::drain::checkpoint_drain(
            view.as_ref(),
            &root,
            alias.pending_id(),
            executor,
        )
        .unwrap()
        .unwrap();
        assert_eq!((report.issued, report.resolved), counts);
        for b in [
            i.binding(OwnerKind::Version).unwrap(),
            alias.binding(OwnerKind::Pending).unwrap(),
        ] {
            assert_eq!(
                kv9_meta::retention::retention_owner(view.as_ref(), &root, b.descriptor.id)
                    .unwrap()
                    .unwrap()
                    .phase,
                PinPhase::Published
            );
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}
