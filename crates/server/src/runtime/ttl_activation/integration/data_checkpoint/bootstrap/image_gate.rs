use super::*;
use crate::runtime::checkpoint_sweep::{Reconciler, Stats};
use kv9_meta::retention::{
    checkpoint_intent::CheckpointIntent, metadata_image_gate, reader_closure,
};
use read_gate::{read_scope, select, send, stop, tasks, wait};

async fn alias(
    f: &Fixture,
    root: &RootDescriptor,
    manifest: &CheckpointManifest,
    predecessor: u64,
) -> CheckpointIntent {
    // Exercise catalog aliases through actual Raft/RPC commands. These owner
    // aliases are not assertions of another committed ManifestChange window.
    let i = CheckpointIntent::new(root, manifest, predecessor).unwrap();
    let p = i.binding(OwnerKind::Pending).unwrap().token();
    let v = i.binding(OwnerKind::Version).unwrap();
    for request in [
        LedgerRequest::BeginCheckpoint(i.clone()),
        LedgerRequest::Share {
            from: p,
            to: v.clone(),
        },
        LedgerRequest::Publish(v.token()),
        LedgerRequest::QuiesceAfterTransfer {
            from: p,
            to: v.token(),
        },
        LedgerRequest::Release(p),
    ] {
        send(f, root, request).await.unwrap();
    }
    i
}
fn observed(
    f: &Fixture,
    root: &RootDescriptor,
    pending: OwnerId,
    leader: usize,
) -> metadata_image_gate::Work {
    metadata_image_gate::observe(
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
fn readers(f: &Fixture, root: &RootDescriptor, pending: OwnerId) -> reader_closure::Work {
    reader_closure::observe(
        f.backends[2]
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
async fn image_gate_blocks_future_alias_io_survives_lost_receipt_and_leader_change_while_real_get_drains(
) {
    let f = Fixture::new().await;
    let root = f.backends[0].ttl_context().unwrap().root.clone();
    let objects = Arc::new(Objects::default());
    let uploader = Arc::new(RemoteUploader::for_harness(objects.clone()));
    let (old, p) = retirement::publish(&f, uploader.clone()).await;
    for n in 0..3 {
        select(&f, &root, &old, p, n).await;
    }
    let original = CheckpointIntent::new(&root, &old, p).unwrap();
    let a = alias(&f, &root, &old, p + 100).await;
    let (store, pause) =
        kv9_engine::MinioObjectStore::with_paused_memory_worker(Duration::from_millis(100))
            .unwrap();
    let r = root.clone();
    let m = old.clone();
    let predecessor = a.predecessor();
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    let active = tokio::task::spawn_blocking(move || {
        read_scope(
            &r,
            &m,
            predecessor,
            &ledger,
            &RemoteUploader::new(Arc::new(store)),
        )
    });
    let pause = tokio::task::spawn_blocking(move || {
        assert!(pause.wait_for_closed_execution(Duration::from_secs(10)));
        pause
    })
    .await
    .unwrap();
    assert!(!active.is_finished());
    let (new, q) = retirement::publish(&f, uploader.clone()).await;
    for n in 0..3 {
        select(&f, &root, &new, q, n).await;
    }
    let pending = original.binding(OwnerKind::Pending).unwrap().token();
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
    assert!(matches!(
        observed(&f, &root, original.pending_id(), 1),
        metadata_image_gate::Work::Ready
    ));
    let gets = objects.gets.load(Ordering::SeqCst);
    let puts = objects.puts.load(Ordering::SeqCst);
    f.hooks[1]
        .retention_image_gate_fault
        .store(2, Ordering::SeqCst);
    let mut state = Reconciler::default();
    for _ in 0..32 {
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
        if f.hooks[1].retention_image_gate_fault.load(Ordering::SeqCst) == 1 {
            assert!(result.is_err());
            break;
        }
        result.unwrap();
    }
    assert_eq!(
        f.hooks[1].retention_image_gate_fault.load(Ordering::SeqCst),
        1
    );
    let closed = observed(&f, &root, original.pending_id(), 1);
    assert!(matches!(closed, metadata_image_gate::Work::Closed(_)));
    assert_eq!(observed(&f, &root, a.pending_id(), 1), closed);
    assert!(!active.is_finished());
    for h in &f.hooks {
        h.selection_refuse.store(true, Ordering::SeqCst);
    }
    f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
    wait("image gate new leader", || {
        f.meta[2].status().role == Role::Leader && f.meta[1].status().role != Role::Leader
    })
    .await;
    send(&f, &root, LedgerRequest::CloseMetadataImageReads(pending))
        .await
        .unwrap();
    send(
        &f,
        &root,
        LedgerRequest::CloseMetadataImageReads(a.binding(OwnerKind::Pending).unwrap().token()),
    )
    .await
    .unwrap();
    assert_eq!(observed(&f, &root, a.pending_id(), 2), closed);
    let maintenance = tasks(&f);
    wait("inherited alias reader stays active", || {
        matches!(
            readers(&f, &root, a.pending_id()),
            reader_closure::Work::Complete(_)
        ) && maintenance[2].snapshot().history_unknown > 0
    })
    .await;
    stop(maintenance).await;
    let reader_closure::Work::Complete(blocked) = readers(&f, &root, a.pending_id()) else {
        panic!()
    };
    assert_eq!((blocked.counts.generations, blocked.counts.active), (1, 1));
    assert!(!active.is_finished());
    let view = f.backends[2]
        .node
        .meta_raft
        .store
        .engine()
        .snapshot()
        .unwrap();
    assert!(
        matches!(
            kv9_meta::retention::checkpoint_admission::sweep::observe(
                view.as_ref(),
                &root,
                a.pending_id()
            )
            .unwrap(),
            kv9_meta::retention::checkpoint_admission::sweep::SweepObservation::Open
        ),
        "the read census must not infer publisher settlement for the alias"
    );
    assert!(
        kv9_meta::retention::checkpoint_refusal::refusal(view.as_ref(), &root, a.pending_id())
            .unwrap()
            .is_none()
    );
    drop(view);
    let b = alias(&f, &root, &old, p + 200).await;
    let id = StoreIdentity::for_voter(&root, NodeId(1)).unwrap();
    for i in [&a, &b] {
        assert!(send(
            &f,
            &root,
            LedgerRequest::BeginBootstrapRead {
                pending: i.binding(OwnerKind::Pending).unwrap().token(),
                source: id.node_id,
                store: id.store_incarnation
            }
        )
        .await
        .is_err());
        let r = root.clone();
        let m = old.clone();
        let predecessor = i.predecessor();
        let u = uploader.clone();
        // Another real store/process can attempt a fresh reader while n1's GET
        // is still active; reject by the image gate, not by generation reuse.
        let ledger = RoutedLedger::new(&f.backends[1], tokio::runtime::Handle::current());
        assert!(
            tokio::task::spawn_blocking(move || read_scope(&r, &m, predecessor, &ledger, &u))
                .await
                .unwrap()
                .is_err()
        );
    }
    assert_eq!(objects.gets.load(Ordering::SeqCst), gets);
    assert_eq!(objects.puts.load(Ordering::SeqCst), puts);
    drop(pause);
    assert!(tokio::time::timeout(Duration::from_secs(5), active)
        .await
        .unwrap()
        .unwrap()
        .is_err());
    let maintenance = tasks(&f);
    wait("actual alias GET drain", || {
        readers(&f, &root, a.pending_id()).registered_generations_closed()
            && readers(&f, &root, b.pending_id()).registered_generations_closed()
    })
    .await;
    assert_eq!(maintenance[2].snapshot().peak_in_flight, 1);
    stop(maintenance).await;
    assert_eq!(
        retirement::owner(&f, &root, &original.binding(OwnerKind::Version).unwrap()),
        PinPhase::Published
    );
    let at = f.meta[2].driver_applied().unwrap();
    wait("image gate all replicas before offline recovery", || {
        f.meta
            .iter()
            .all(|d| d.driver_applied().is_some_and(|p| p.index >= at.index))
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
        let (e, _, anchor) = kv9_meta::recovery::open_initial_checkpoint_engine_with_reader(
            &mut storage,
            path.join("meta.wal"),
            Some(&bootstrap),
            &root,
        )
        .unwrap();
        assert_eq!(anchor.unwrap().descriptor().manifest, new.encode().unwrap());
        let view = e.snapshot().unwrap();
        for i in [&original, &a, &b] {
            assert_eq!(
                metadata_image_gate::observe(view.as_ref(), &root, i.pending_id()).unwrap(),
                closed
            );
        }
        for i in [&a, &b] {
            assert!(
                reader_closure::observe(view.as_ref(), &root, i.pending_id())
                    .unwrap()
                    .registered_generations_closed()
            );
        }
        assert_eq!(
            kv9_meta::retention::retention_owner(
                view.as_ref(),
                &root,
                original.binding(OwnerKind::Version).unwrap().descriptor.id
            )
            .unwrap()
            .unwrap()
            .phase,
            PinPhase::Published
        );
        drop(view);
        drop(e);
        drop(storage);
        drop(guard);
    }
    std::fs::remove_dir_all(directory).unwrap();
}
