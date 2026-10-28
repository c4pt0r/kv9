use super::*;
use crate::checkpoint_retention::CheckpointRead;
use crate::runtime::checkpoint_sweep::{Maintenance, Reconciler, Stats};
use kv9_meta::retention::{
    checkpoint_intent::CheckpointIntent, encode_request, metadata_read_gate as gate, reader_closure,
};

pub(super) async fn send(f: &Fixture, root: &RootDescriptor, request: LedgerRequest) -> Result<()> {
    let bytes = encode_request(root.digest(), &request)?;
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    tokio::task::spawn_blocking(move || ledger.apply(bytes))
        .await
        .unwrap()
}
pub(super) async fn select(
    f: &Fixture,
    root: &RootDescriptor,
    manifest: &CheckpointManifest,
    p: u64,
    n: usize,
) {
    super::selection::select(f, n, manifest.clone(), p).await;
    let id = StoreIdentity::for_voter(root, NodeId(n as u64 + 1)).unwrap();
    send(
        f,
        root,
        LedgerRequest::RecordCheckpointSelection {
            pending: CheckpointIntent::new(root, manifest, p)
                .unwrap()
                .binding(OwnerKind::Pending)
                .unwrap()
                .token(),
            source: id.node_id,
            store: id.store_incarnation,
        },
    )
    .await
    .unwrap();
}
fn observed(f: &Fixture, root: &RootDescriptor, pending: OwnerId, leader: usize) -> gate::Work {
    gate::observe(
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
fn readers(
    f: &Fixture,
    root: &RootDescriptor,
    pending: OwnerId,
    leader: usize,
) -> reader_closure::Work {
    reader_closure::observe(
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
pub(super) fn tasks(f: &Fixture) -> Vec<Maintenance> {
    f.backends
        .iter()
        .map(|b| Maintenance::start(b.clone(), &tokio::runtime::Handle::current()))
        .collect()
}
pub(super) async fn stop(tasks: Vec<Maintenance>) {
    for t in &tasks {
        t.cancel();
    }
    for t in tasks {
        t.drain().await;
    }
}
pub(super) fn read_scope(
    root: &RootDescriptor,
    manifest: &CheckpointManifest,
    p: u64,
    ledger: &RoutedLedger,
    uploader: &RemoteUploader,
) -> Result<()> {
    let mut reader = CheckpointRead::new(root, manifest, p, OwnerKind::Version, ledger)?;
    let (result, report) = uploader.with_local_read_report(|scope| {
        reader.before_io(ledger, scope)?;
        scope.restore(manifest)?;
        Ok(())
    })?;
    reader.observe_drain(report)?;
    reader.finish(ledger)?;
    result
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn metadata_read_gate_waits_for_every_store_survives_lost_receipt_and_drains_real_old_get() {
    let f = Fixture::new().await;
    let root = f.backends[0].ttl_context().unwrap().root.clone();
    let objects = Arc::new(Objects::default());
    let uploader = Arc::new(RemoteUploader::for_harness(objects.clone()));
    let (old, p) = retirement::publish(&f, uploader.clone()).await;
    for n in 0..3 {
        select(&f, &root, &old, p, n).await;
    }
    let intent = CheckpointIntent::new(&root, &old, p).unwrap();
    let pending = intent.pending_id();
    let request = LedgerRequest::AdvanceMetadataReadGate {
        pending: intent.binding(OwnerKind::Pending).unwrap().token(),
        expected: 0,
    };
    let (store, pause) =
        kv9_engine::MinioObjectStore::with_paused_memory_worker(Duration::from_millis(100))
            .unwrap();
    let r = root.clone();
    let m = old.clone();
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    let active = tokio::task::spawn_blocking(move || {
        read_scope(&r, &m, p, &ledger, &RemoteUploader::new(Arc::new(store)))
    });
    let pause = tokio::task::spawn_blocking(move || {
        assert!(pause.wait_for_closed_execution(Duration::from_secs(10)));
        pause
    })
    .await
    .unwrap();
    assert!(!active.is_finished());
    let (new, q) = retirement::publish(&f, uploader.clone()).await;
    for n in 0..2 {
        select(&f, &root, &new, q, n).await;
    }
    assert!(send(&f, &root, request.clone()).await.is_err());
    assert!(matches!(
        observed(&f, &root, pending, 1),
        gate::Work::WaitingForSelection(_)
    ));
    let maintenance = tasks(&f);
    eprintln!("read gate stage: waiting store");
    wait("waiting store", || {
        maintenance[1].snapshot().metadata_read_gate_waiting > 0
    })
    .await;
    stop(maintenance).await;
    f.hooks[1]
        .retention_read_gate_fault
        .store(2, Ordering::SeqCst);
    select(&f, &root, &new, q, 2).await;
    // Drive the production reconciler through another node's authenticated
    // route: the normal leader-local maintenance call bypasses the RPC hook.
    let mut state = Reconciler::default();
    for _ in 0..16 {
        let backend = f.backends[1].clone();
        let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
        let (next, result) = tokio::task::spawn_blocking(move || {
            let result = state.pass(
                &backend,
                &ledger,
                &tokio_util::sync::CancellationToken::new(),
                Instant::now() + Duration::from_secs(5),
                &mut Stats::default(),
            );
            (state, result)
        })
        .await
        .unwrap();
        state = next;
        if f.hooks[1].retention_read_gate_fault.load(Ordering::SeqCst) == 1 {
            assert!(result.is_err());
            break;
        }
        result.unwrap();
    }
    assert_eq!(
        f.hooks[1].retention_read_gate_fault.load(Ordering::SeqCst),
        1
    );
    assert!(matches!(
        observed(&f, &root, pending, 1),
        gate::Work::Closed(_)
    ));
    assert!(
        !active.is_finished(),
        "read admission closure cannot invent actual GET drain"
    );
    assert_eq!(
        retirement::owner(&f, &root, &intent.binding(OwnerKind::Version).unwrap()),
        PinPhase::Published
    );
    let puts = objects.puts.load(Ordering::SeqCst);
    let gets = objects.gets.load(Ordering::SeqCst);
    for h in &f.hooks {
        h.selection_refuse.store(true, Ordering::SeqCst);
    }
    f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
    eprintln!("read gate stage: new leader");
    wait("new leader", || {
        f.meta[2].status().role == Role::Leader && f.meta[1].status().role != Role::Leader
    })
    .await;
    send(&f, &root, request).await.unwrap();
    let maintenance = tasks(&f);
    eprintln!("read gate stage: active reader census");
    wait("active reader census", || {
        matches!(
            readers(&f, &root, pending, 2),
            reader_closure::Work::Complete(_)
        )
    })
    .await;
    assert_eq!(maintenance[2].snapshot().peak_in_flight, 1);
    stop(maintenance).await;
    let reader_closure::Work::Complete(blocked) = readers(&f, &root, pending, 2) else {
        panic!()
    };
    assert_eq!((blocked.counts.generations, blocked.counts.active), (1, 1));
    assert!(!active.is_finished());
    let id = StoreIdentity::for_voter(&root, NodeId(1)).unwrap();
    assert!(send(
        &f,
        &root,
        LedgerRequest::BeginBootstrapRead {
            pending: intent.binding(OwnerKind::Pending).unwrap().token(),
            source: id.node_id,
            store: id.store_incarnation
        }
    )
    .await
    .is_err());
    drop(pause);
    // The paused in-memory object store has no SST: the actual job fails, but
    // its read scope must still drain and finish under the retained old Version.
    assert!(tokio::time::timeout(Duration::from_secs(5), active)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    assert!(matches!(
        readers(&f, &root, pending, 2),
        reader_closure::Work::Advance { restart: true, .. }
    ));
    let maintenance = tasks(&f);
    eprintln!("read gate stage: drained reader census");
    wait("drained reader census", || {
        readers(&f, &root, pending, 2).registered_generations_closed()
    })
    .await;
    stop(maintenance).await;
    let r = root.clone();
    let m = old.clone();
    let o = objects.clone();
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    assert!(tokio::task::spawn_blocking(move || read_scope(
        &r,
        &m,
        p,
        &ledger,
        &RemoteUploader::for_harness(o)
    ))
    .await
    .unwrap()
    .is_err());
    assert_eq!(
        objects.gets.load(Ordering::SeqCst),
        gets,
        "new read rejected before object I/O"
    );
    assert_eq!(objects.puts.load(Ordering::SeqCst), puts);
    let maintenance = tasks(&f);
    eprintln!("read gate stage: unentered reader census");
    wait("unentered reader census", || {
        readers(&f, &root, pending, 2).registered_generations_closed()
    })
    .await;
    stop(maintenance).await;
    let reader_closure::Work::Complete(done) = readers(&f, &root, pending, 2) else {
        panic!()
    };
    assert_eq!(
        (
            done.counts.generations,
            done.counts.drained,
            done.counts.unentered
        ),
        (2, 1, 1)
    );
    let applied = f.meta[2].driver_applied().unwrap();
    wait("all replicas applied before offline recovery", || {
        f.meta
            .iter()
            .all(|d| d.driver_applied().is_some_and(|p| p.index >= applied.index))
    })
    .await;
    let directory = f.shutdown_keep().await;
    for node in 1..=3 {
        let path = directory.join(format!("n{node}"));
        let guard = StoreGuard::lock(&path).unwrap();
        let mut storage = DiskRaftStorage::recover(&path.join("meta-raft")).unwrap();
        let bootstrap = BootstrapReader {
            directory: &path,
            root: &root,
            identity: StoreIdentity::for_voter(&root, NodeId(node)).unwrap(),
            uploader: &uploader,
        };
        let (e, _, anchor) = kv9_meta::recovery::open_initial_checkpoint_engine_with_reader(
            &mut storage,
            path.join("meta.wal"),
            Some(&bootstrap),
            &root,
        )
        .unwrap();
        assert_eq!(anchor.unwrap().descriptor().manifest, new.encode().unwrap());
        let view = e.snapshot().unwrap();
        assert!(matches!(
            gate::observe(view.as_ref(), &root, pending).unwrap(),
            gate::Work::Closed(_)
        ));
        assert!(reader_closure::observe(view.as_ref(), &root, pending)
            .unwrap()
            .registered_generations_closed());
        assert_eq!(
            kv9_meta::retention::retention_owner(
                view.as_ref(),
                &root,
                intent.binding(OwnerKind::Version).unwrap().descriptor.id
            )
            .unwrap()
            .unwrap()
            .phase,
            PinPhase::Published
        );
        assert!(
            kv9_meta::retention::checkpoint_refusal::refusal(view.as_ref(), &root, pending)
                .unwrap()
                .is_none()
        );
        drop(view);
        drop(e);
        drop(storage);
        drop(guard);
    }
    std::fs::remove_dir_all(directory).unwrap();
}

pub(super) async fn wait(label: &str, mut ready: impl FnMut() -> bool) {
    let result = tokio::time::timeout(Duration::from_secs(20), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(result.is_ok(), "metadata read gate test timeout: {label}");
}
