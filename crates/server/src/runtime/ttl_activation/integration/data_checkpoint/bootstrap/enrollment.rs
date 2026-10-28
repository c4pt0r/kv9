use super::*;
use crate::bootstrap_retention::startup::{open_metadata, StartupLedger};
use kv9_meta::retention::checkpoint_intent::CheckpointIntent;

async fn open(
    guard: Arc<StoreGuard>,
    root: RootDescriptor,
    uploader: Arc<RemoteUploader>,
    channels: Vec<tonic::transport::Channel>,
    token: &str,
) -> Result<()> {
    let handle = tokio::runtime::Handle::current();
    let token = token.to_owned();
    tokio::task::spawn_blocking(move || {
        let directory = guard.directory();
        let identity = StoreIdentity::for_voter(&root, NodeId(1))?;
        let mut storage = DiskRaftStorage::recover(&directory.join("meta-raft"))?;
        let reader = BootstrapReader { directory, root: &root, identity, uploader: &uploader };
        let (engine, _, anchor) = open_metadata(&guard, &mut storage, &directory.join("meta.wal"),
            &reader, || {
                Ok(StartupLedger::new(&root, identity, token, handle)?.with_channels(
                    root.voters.iter().zip(channels).map(|(v, c)| (v.addr, c))))
            })?;
        assert_eq!(anchor.as_ref().unwrap().descriptor().root, root);
        assert!(matches!(engine.applied_position()?, kv9_engine::DurableAppliedPosition::AppliedThrough(at) if at.index > 0));
        Ok(())
    }).await.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn legacy_selected_metadata_enrolls_through_live_quorum_then_reopens_offline_after_lost_receipts(
) {
    let mut f = Fixture::new().await;
    let root = f.backends[0].ttl_context().unwrap().root.clone();
    let identity = StoreIdentity::for_voter(&root, NodeId(1)).unwrap();
    let objects = Arc::new(Objects::default());
    let uploader = Arc::new(RemoteUploader::for_harness(objects.clone()));
    let (manifest, predecessor) = retirement::publish(&f, uploader.clone()).await;
    let tip = f.meta[1].driver_applied().unwrap().index;
    until(|| f.meta[0].driver_applied().is_some_and(|p| p.index >= tip)).await;
    // Explicit old-worker fixture: a valid selected checkpoint without a
    // bootstrap receipt/grant. Publication and Version use actual Raft apply.
    f.backends[0]
        .node
        .meta_raft
        .store
        .engine()
        .checkpoint_applied(&manifest)
        .unwrap();
    let receipt = f.path.join("n1/catalog.bootstrap-reads");
    assert!(!receipt.exists());
    let intent = CheckpointIntent::new(&root, &manifest, predecessor).unwrap();
    let version = intent.binding(OwnerKind::Version).unwrap();
    let grant = bootstrap_reader::binding(&root, &manifest, predecessor, identity).unwrap();
    // The live authenticated catalog must enforce typed admission itself.
    // A valid caller-side binding cannot make generic Acquire/Share eligible.
    let ledger = Arc::new(RoutedLedger::new(
        &f.backends[0],
        tokio::runtime::Handle::current(),
    ));
    for request in [
        LedgerRequest::Acquire(grant.clone()),
        LedgerRequest::Share {
            from: version.token(),
            to: grant.clone(),
        },
        LedgerRequest::Share {
            from: intent.binding(OwnerKind::Pending).unwrap().token(),
            to: grant.clone(),
        },
    ] {
        let ledger = ledger.clone();
        let bytes = kv9_meta::retention::encode_request(root.digest(), &request).unwrap();
        assert!(tokio::task::spawn_blocking(move || ledger.apply(bytes))
            .await
            .unwrap()
            .is_err());
    }
    f.meta[0].stop();
    f.data[0].stop();
    for owner in f.owners.drain(..2) {
        owner.join().unwrap();
    }
    let server = f.servers.remove(0);
    server.abort();
    let _ = server.await;
    let guard = Arc::new(f._guards.remove(0));
    let channels = f.public.clone();
    let puts = objects.puts.load(Ordering::SeqCst);
    let gets = objects.gets.load(Ordering::SeqCst);
    assert!(open(
        guard.clone(),
        root.clone(),
        uploader.clone(),
        channels.clone(),
        "wrong-token"
    )
    .await
    .is_err());
    assert!(!receipt.exists());
    assert_eq!(objects.gets.load(Ordering::SeqCst), gets);

    let leader = &f.hooks[1];
    let other_calls = f.hooks[2].retention_calls.load(Ordering::SeqCst);
    leader.retention_fail_after.store(
        leader.retention_commits.load(Ordering::SeqCst) + 1,
        Ordering::SeqCst,
    );
    assert!(open(
        guard.clone(),
        root.clone(),
        uploader.clone(),
        channels.clone(),
        "cluster-test"
    )
    .await
    .is_err());
    assert_eq!(retirement::owner(&f, &root, &grant), PinPhase::Held);
    let admission = |f: &Fixture| {
        let leader = f
            .meta
            .iter()
            .position(|d| d.status().role == Role::Leader)
            .unwrap();
        bootstrap_reader::admission::observe(
            f.backends[leader]
                .node
                .meta_raft
                .store
                .engine()
                .snapshot()
                .unwrap()
                .as_ref(),
            &root,
            grant.token(),
        )
        .unwrap()
        .unwrap()
    };
    let original = admission(&f);
    assert_eq!(
        original.pending,
        intent.binding(OwnerKind::Pending).unwrap().token()
    );
    assert_eq!(original.version, version.token());
    assert_eq!(original.node, identity.node_id);
    assert_eq!(original.store, identity.store_incarnation);
    assert_eq!(objects.gets.load(Ordering::SeqCst), gets);
    assert!(receipt.exists());
    let prospective = std::fs::read(&receipt).unwrap();
    assert_eq!(&prospective[..8], b"KV9MBR02");
    // Sole slot remains unauthorized.
    assert_eq!(prospective[122], 0);
    // Unknown writes must not even reach the other connected seed's handler.
    assert_eq!(
        f.hooks[2].retention_calls.load(Ordering::SeqCst),
        other_calls
    );
    assert_eq!(
        leader.retention_commits.load(Ordering::SeqCst),
        leader.retention_fail_after.load(Ordering::SeqCst)
    );
    // Resume on a new leader. Root seed 2 now returns a typed NotLeader and
    // the startup router reaches 3, reusing the original prospective identity.
    f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
    until(|| f.meta[2].status().role == Role::Leader).await;
    let leader = &f.hooks[2];
    leader.retention_fail_after.store(
        leader.retention_commits.load(Ordering::SeqCst) + 2,
        Ordering::SeqCst,
    );
    assert!(open(
        guard.clone(),
        root.clone(),
        uploader.clone(),
        channels.clone(),
        "cluster-test"
    )
    .await
    .is_err());
    assert_eq!(retirement::owner(&f, &root, &grant), PinPhase::Published);
    assert_eq!(admission(&f), original);
    assert_eq!(std::fs::read(&receipt).unwrap(), prospective);
    assert_eq!(objects.gets.load(Ordering::SeqCst), gets);
    leader.retention_fail_after.store(0, Ordering::SeqCst);
    open(
        guard.clone(),
        root.clone(),
        uploader.clone(),
        channels,
        "cluster-test",
    )
    .await
    .unwrap();
    assert!(objects.gets.load(Ordering::SeqCst) > gets);
    assert_eq!(objects.puts.load(Ordering::SeqCst), puts);
    assert_eq!(std::fs::read(&receipt).unwrap()[122], 1);
    assert_eq!(retirement::owner(&f, &root, &version), PinPhase::Published);
    assert_eq!(retirement::owner(&f, &root, &grant), PinPhase::Published);
    assert_eq!(admission(&f), original);
    let directory = f.shutdown_keep().await;
    let (reopened, _) = kv9_engine::WalEngine::open(directory.join("n3/meta.wal")).unwrap();
    assert_eq!(
        bootstrap_reader::admission::observe(
            reopened.snapshot().unwrap().as_ref(),
            &root,
            grant.token()
        )
        .unwrap(),
        Some(original)
    );
    drop(reopened);
    // With every original Raft owner and service stopped, no client can help.
    // The same production composition must take the durable offline path.
    let r = root.clone();
    let u = uploader.clone();
    let g = guard.clone();
    tokio::task::spawn_blocking(move || {
        for _ in 0..2 {
            let mut storage = DiskRaftStorage::recover(&g.directory().join("meta-raft")).unwrap();
            let reader = BootstrapReader {
                directory: g.directory(),
                root: &r,
                identity,
                uploader: &u,
            };
            let (engine, _, anchor) = open_metadata::<StartupLedger>(
                &g,
                &mut storage,
                &g.directory().join("meta.wal"),
                &reader,
                || panic!("offline recovery contacted quorum"),
            )
            .unwrap();
            assert_eq!(
                anchor.unwrap().descriptor().manifest,
                manifest.encode().unwrap()
            );
            drop(engine);
        }
    })
    .await
    .unwrap();
    assert_eq!(objects.puts.load(Ordering::SeqCst), puts);
    drop(guard);
    std::fs::remove_dir_all(directory).unwrap();
}
