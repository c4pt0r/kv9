use super::*;
use kv9_meta::retention::{
    bootstrap_closure, checkpoint_intent::CheckpointIntent, retention_owner,
};
use read_gate::{select, send, stop, tasks, wait};

fn owner(
    f: &Fixture,
    root: &RootDescriptor,
    id: OwnerId,
    leader: usize,
) -> Option<kv9_meta::retention::LedgerOwner> {
    retention_owner(
        f.backends[leader]
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
}
fn unentered(
    f: &Fixture,
    root: &RootDescriptor,
    b: &kv9_meta::retention::OwnerBinding,
    leader: usize,
) -> Option<bootstrap_reader::unentered::Unentered> {
    bootstrap_reader::unentered::observe(
        f.backends[leader]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap()
            .as_ref(),
        root,
        b.token(),
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn prospective_bootstrap_fence_distinguishes_refused_entry_from_lost_entry_receipt_across_failover_and_recovery(
) {
    let f = Fixture::new().await;
    let root = f.backends[0].ttl_context().unwrap().root.clone();
    let objects = Arc::new(Objects::default());
    let uploader = Arc::new(RemoteUploader::for_harness(objects.clone()));
    let (old, p) = retirement::publish(&f, uploader.clone()).await;
    let intent = CheckpointIntent::new(&root, &old, p).unwrap();
    let grants: Vec<_> = (1..=3)
        .map(|n| {
            bootstrap_reader::binding(
                &root,
                &old,
                p,
                StoreIdentity::for_voter(&root, NodeId(n)).unwrap(),
            )
            .unwrap()
        })
        .collect();
    // The prospective receipt is persisted before Begin. Refusal leaves no
    // catalog owner; an independently committed Begin with a lost reply is Held.
    f.hooks[1].retention_refuse_from.store(
        f.hooks[1].retention_calls.load(Ordering::SeqCst) + 1,
        Ordering::SeqCst,
    );
    assert!(selection::try_select(&f, 0, old.clone(), p).await.is_err());
    f.hooks[1].retention_refuse_from.store(0, Ordering::SeqCst);
    assert!(owner(&f, &root, grants[0].descriptor.id, 1).is_none());
    f.hooks[1].retention_fail_after.store(
        f.hooks[1].retention_commits.load(Ordering::SeqCst) + 1,
        Ordering::SeqCst,
    );
    assert!(selection::try_select(&f, 2, old.clone(), p).await.is_err());
    f.hooks[1].retention_fail_after.store(0, Ordering::SeqCst);
    assert_eq!(
        owner(&f, &root, grants[2].descriptor.id, 1).unwrap().phase,
        PinPhase::Held
    );
    select(&f, &root, &old, p, 1).await;
    for n in [0, 2] {
        assert!(f.backends[n]
            .node
            .meta_raft
            .store
            .engine()
            .selected_checkpoint()
            .unwrap()
            .is_none());
        assert!(f
            .path
            .join(format!("n{}/catalog.bootstrap-reads", n + 1))
            .exists());
    }
    let (new, q) = retirement::publish(&f, uploader.clone()).await;
    let gets = objects.gets.load(Ordering::SeqCst);
    let puts = objects.puts.load(Ordering::SeqCst);
    // Closing the prospective local slot alone cannot certify its retirement.
    f.hooks[0].bootstrap_refuse.store(true, Ordering::SeqCst);
    assert!(selection::try_select(&f, 0, new.clone(), q).await.is_err());
    assert!(owner(&f, &root, grants[0].descriptor.id, 1).is_none());
    f.hooks[0].bootstrap_refuse.store(false, Ordering::SeqCst);
    f.hooks[1].retention_fail_after.store(
        f.hooks[1].retention_commits.load(Ordering::SeqCst) + 1,
        Ordering::SeqCst,
    );
    assert!(selection::try_select(&f, 0, new.clone(), q).await.is_err());
    let fact = unentered(&f, &root, &grants[0], 1).unwrap();
    assert_eq!(fact.pending.owner, intent.pending_id());
    assert_eq!(
        fact.version,
        intent.binding(OwnerKind::Version).unwrap().token()
    );
    assert_eq!(
        owner(&f, &root, grants[0].descriptor.id, 1).unwrap().phase,
        PinPhase::Released
    );
    assert!(f.backends[0]
        .node
        .meta_raft
        .store
        .engine()
        .selected_checkpoint()
        .unwrap()
        .is_none());
    f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
    wait("unentered bootstrap new leader", || {
        f.meta[2].status().role == Role::Leader && f.meta[1].status().role != Role::Leader
    })
    .await;
    // An exact terminal retry consumes the durable debt without requerying the
    // source. It preserves the first fact instead of minting another revision.
    f.hooks[0].bootstrap_refuse.store(true, Ordering::SeqCst);
    let queries = f.hooks[0].bootstrap_reads.load(Ordering::SeqCst);
    let directory = f.path.join("n1");
    let r = root.clone();
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    tokio::task::spawn_blocking(move || {
        crate::bootstrap_retention::resume_retirement(&directory, &r, &ledger)
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(f.hooks[0].bootstrap_reads.load(Ordering::SeqCst), queries);
    assert_eq!(unentered(&f, &root, &grants[0], 2), Some(fact.clone()));
    f.hooks[0].bootstrap_refuse.store(false, Ordering::SeqCst);
    for n in 0..3 {
        select(&f, &root, &new, q, n).await;
    }
    assert!(unentered(&f, &root, &grants[2], 2).is_none());
    assert!(bootstrap_reader::admission::observe(
        f.backends[2]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap()
            .as_ref(),
        &root,
        grants[2].token()
    )
    .unwrap()
    .is_some());
    send(
        &f,
        &root,
        LedgerRequest::AdvanceMetadataReadGate {
            pending: intent.binding(OwnerKind::Pending).unwrap().token(),
            expected: 0,
        },
    )
    .await
    .unwrap();
    let maintenance = tasks(&f);
    wait("bootstrap terminal origins census", || {
        bootstrap_closure::observe(
            f.backends[2]
                .node
                .meta_raft
                .store
                .engine()
                .snapshot()
                .unwrap()
                .as_ref(),
            &root,
            intent.pending_id(),
        )
        .unwrap()
        .registered_grants_retired()
    })
    .await;
    stop(maintenance).await;
    let view = f.backends[2]
        .node
        .meta_raft
        .store
        .engine()
        .snapshot()
        .unwrap();
    let bootstrap_closure::Work::Complete(c) =
        bootstrap_closure::observe(view.as_ref(), &root, intent.pending_id()).unwrap()
    else {
        panic!()
    };
    assert_eq!(
        (c.counts.grants, c.counts.retired, c.counts.legacy),
        (3, 3, 0)
    );
    drop(view);
    assert_eq!(objects.gets.load(Ordering::SeqCst), gets);
    assert_eq!(objects.puts.load(Ordering::SeqCst), puts);
    let applied = f.meta[2].driver_applied().unwrap();
    wait("unentered origin replicated", || {
        f.meta
            .iter()
            .all(|d| d.driver_applied().is_some_and(|p| p.index >= applied.index))
    })
    .await;
    let directory = f.shutdown_keep().await;
    for n in 1..=3 {
        let path = directory.join(format!("n{n}"));
        let guard = StoreGuard::lock(&path).unwrap();
        let mut storage = DiskRaftStorage::recover(&path.join("meta-raft")).unwrap();
        let reader = BootstrapReader {
            directory: &path,
            root: &root,
            identity: StoreIdentity::for_voter(&root, NodeId(n)).unwrap(),
            uploader: &uploader,
        };
        let (engine, _, anchor) = kv9_meta::recovery::open_initial_checkpoint_engine_with_reader(
            &mut storage,
            path.join("meta.wal"),
            Some(&reader),
            &root,
        )
        .unwrap();
        assert_eq!(anchor.unwrap().descriptor().manifest, new.encode().unwrap());
        let view = engine.snapshot().unwrap();
        assert_eq!(
            bootstrap_reader::unentered::observe(view.as_ref(), &root, grants[0].token()).unwrap(),
            Some(fact.clone())
        );
        assert!(
            bootstrap_reader::unentered::observe(view.as_ref(), &root, grants[2].token())
                .unwrap()
                .is_none()
        );
        assert!(
            bootstrap_reader::admission::observe(view.as_ref(), &root, grants[2].token())
                .unwrap()
                .is_some()
        );
        assert!(
            bootstrap_closure::observe(view.as_ref(), &root, intent.pending_id())
                .unwrap()
                .registered_grants_retired()
        );
        assert_eq!(
            retention_owner(
                view.as_ref(),
                &root,
                intent.binding(OwnerKind::Version).unwrap().descriptor.id
            )
            .unwrap()
            .unwrap()
            .phase,
            PinPhase::Published
        );
        drop(view);
        drop(engine);
        drop(storage);
        drop(guard);
    }
    std::fs::remove_dir_all(directory).unwrap();
}
