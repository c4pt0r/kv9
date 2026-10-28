use super::*;
use kv9_meta::retention::{
    bootstrap_index::{self, Work},
    bootstrap_reader,
    checkpoint_intent::CheckpointIntent,
    LedgerOwner,
};
use kv9_raft::Command;
const PREFIX: &[u8] = b"\0kv9\0retention_v1\0";
fn key(tag: u8) -> Vec<u8> {
    let mut k = PREFIX.to_vec();
    k.push(tag);
    k
}
async fn census(f: &Fixture, leader: usize, cancel: CancellationToken) -> Result<Stats> {
    let backend = f.backends[leader].clone();
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    tokio::task::spawn_blocking(move || {
        let mut stats = Stats::default();
        crate::runtime::checkpoint_sweep::build_index::pass(
            &backend,
            &ledger,
            &cancel,
            Instant::now() + Duration::from_secs(5),
            &mut stats,
        )?;
        Ok(stats)
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bootstrap_inventory_resumes_lost_page_across_leader_change_and_wal_with_legacy_unknowns() {
    let f = Fixture::new().await;
    let backend = f.backends[1].clone();
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    let (root, version, managed, origin, mut expected) = tokio::task::spawn_blocking(move || {
        let root = backend.ttl_context().unwrap().root.clone();
        let engine = backend.node.meta_raft.store.engine();
        let image = settlement::planned(engine, &root, kv9_common::META_REGION_0);
        let intent = CheckpointIntent::new(&root, image.manifest(), 0).unwrap();
        let pending = intent.binding(OwnerKind::Pending).unwrap();
        let version = intent.binding(OwnerKind::Version).unwrap();
        let apply = |request| {
            ledger
                .apply(encode_request(root.digest(), &request).unwrap())
                .unwrap_or_else(|e| panic!("{request:?}: {e}"))
        };
        apply(LedgerRequest::BeginCheckpoint(intent));
        apply(LedgerRequest::Share {
            from: pending.token(),
            to: version.clone(),
        });
        apply(LedgerRequest::Publish(version.token()));
        let identity = StoreIdentity::for_voter(&root, NodeId(1)).unwrap();
        let managed = bootstrap_reader::binding(&root, image.manifest(), 0, identity).unwrap();
        apply(LedgerRequest::BeginBootstrapRead {
            pending: pending.token(),
            source: identity.node_id,
            store: identity.store_incarnation,
        });
        apply(LedgerRequest::Publish(managed.token()));
        let origin = bootstrap_reader::admission::observe(
            engine.snapshot().unwrap().as_ref(),
            &root,
            managed.token(),
        )
        .unwrap()
        .unwrap();
        let mut expected = Vec::new();
        for n in 1..=9 {
            let mut legacy = version.clone();
            legacy.descriptor.kind = OwnerKind::Legacy;
            legacy.descriptor.id = OwnerId::new([n; 16]).unwrap();
            legacy.descriptor.operation = [n; 32];
            apply(LedgerRequest::Acquire(legacy.clone()));
            let phase = match n % 3 {
                0 => PinPhase::Held,
                1 => {
                    apply(LedgerRequest::Publish(legacy.token()));
                    PinPhase::Published
                }
                _ => {
                    apply(LedgerRequest::Publish(legacy.token()));
                    apply(LedgerRequest::QuiesceAfterTransfer {
                        from: legacy.token(),
                        to: version.token(),
                    });
                    apply(LedgerRequest::Release(legacy.token()));
                    PinPhase::Released
                }
            };
            legacy.descriptor.kind = OwnerKind::BootstrapReader;
            expected.push(LedgerOwner {
                binding: legacy,
                phase,
            });
        }
        // Replicate explicit old V12 catalog bytes. Legacy bindings retain pins
        // and phases; the genuinely admitted grant retains its V12 origin.
        let view = engine.snapshot().unwrap();
        let mut batch = kv9_engine::WriteBatch::new();
        for owner in &expected {
            let mut k = key(3);
            k.extend_from_slice(owner.binding.descriptor.id.as_bytes());
            batch.put(
                ColumnFamily::Default,
                k,
                kv9_meta::retention::encode_owner_observation(owner).unwrap(),
            );
            let mut registration = owner.clone();
            registration.phase = PinPhase::Held;
            let mut k = key(5);
            k.extend_from_slice(owner.binding.descriptor.id.as_bytes());
            batch.put(
                ColumnFamily::Default,
                k,
                kv9_meta::retention::encode_owner_observation(&registration).unwrap(),
            );
        }
        // V12 predates both the reverse index and the image closure stamps.
        for (start, end) in [(29, 31), (33, 35)] {
            for row in view
                .iter(ColumnFamily::Default, &key(start), &key(end))
                .unwrap()
            {
                batch.delete(ColumnFamily::Default, row.unwrap().0);
            }
        }
        let mut h = view.get(ColumnFamily::Default, &key(1)).unwrap().unwrap();
        h.truncate(h.len() - 32);
        h[..8].copy_from_slice(b"KV9LED12");
        h.extend_from_slice(RootDigest::sha256(&h).as_bytes());
        batch.put(ColumnFamily::Default, key(1), h);
        drop(view);
        let at = backend
            .driver
            .propose(&Command::from_batch(&batch))
            .unwrap();
        backend
            .driver
            .wait_applied(at, Duration::from_secs(3))
            .unwrap();
        (root, version, managed, origin, expected)
    })
    .await
    .unwrap();
    assert_eq!(
        census(&f, 1, CancellationToken::new())
            .await
            .unwrap()
            .build_index_upgrades,
        1
    );
    // The shared lane finishes Build and process Reader coverage first.
    loop {
        let view = f.backends[1]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap();
        if matches!(
            kv9_meta::retention::reader_index::observe(view.as_ref(), &root).unwrap(),
            kv9_meta::retention::reader_index::Work::Complete
        ) {
            break;
        }
        drop(view);
        census(&f, 1, CancellationToken::new()).await.unwrap();
    }
    let view = f.backends[1]
        .node
        .meta_raft
        .store
        .engine()
        .snapshot()
        .unwrap();
    let Work::Page { progress, next, .. } = bootstrap_index::observe(view.as_ref(), &root).unwrap()
    else {
        panic!()
    };
    assert_eq!(progress.sequence, 0);
    let original_request = progress.request(next);
    drop(view);
    f.hooks[1].retention_calls.store(0, Ordering::SeqCst);
    f.hooks[1].retention_commits.store(0, Ordering::SeqCst);
    f.hooks[1].retention_fail_after.store(1, Ordering::SeqCst);
    assert!(census(&f, 1, CancellationToken::new()).await.is_err());
    assert_eq!(f.hooks[1].retention_commits.load(Ordering::SeqCst), 1);
    f.hooks[1].retention_fail_after.store(0, Ordering::SeqCst);
    f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
    until(|| f.meta[2].status().role == Role::Leader).await;
    assert!(census(&f, 1, CancellationToken::new()).await.is_err());
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(census(&f, 2, cancelled).await.is_err());
    let view = f.backends[2]
        .node
        .meta_raft
        .store
        .engine()
        .snapshot()
        .unwrap();
    assert!(
        !kv9_meta::retention::plan_retention(view.as_ref(), &root, &original_request)
            .unwrap()
            .changed()
    );
    drop(view);
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    let bytes = encode_request(root.digest(), &original_request).unwrap();
    tokio::task::spawn_blocking(move || ledger.apply(bytes))
        .await
        .unwrap()
        .unwrap();
    let tasks: Vec<_> = f
        .backends
        .iter()
        .map(|b| Maintenance::start(b.clone(), &tokio::runtime::Handle::current()))
        .collect();
    until(|| tasks[2].snapshot().data_store_history_catalog_complete).await;
    assert_eq!(tasks[2].snapshot().peak_in_flight, 1);
    assert_eq!(tasks[0].snapshot().bootstrap_index_pages, 0);
    assert_eq!(tasks[1].snapshot().bootstrap_index_pages, 0);
    for task in &tasks {
        task.cancel();
    }
    for task in tasks {
        task.drain().await;
    }
    let stable_rows = owner_and_placement_rows(&f, 2);
    assert!(
        census(&f, 2, CancellationToken::new())
            .await
            .unwrap()
            .bootstrap_index_catalog_complete
    );
    assert_eq!(owner_and_placement_rows(&f, 2), stable_rows);
    let directory = f.shutdown_keep().await;
    let (engine, _) = WalEngine::open(directory.join("n3/meta.wal")).unwrap();
    let view = engine.snapshot().unwrap();
    assert!(matches!(
        bootstrap_index::observe(view.as_ref(), &root).unwrap(),
        Work::Complete
    ));
    let mut found = Vec::new();
    let mut after = None;
    let mut managed_count = 0;
    loop {
        let page = bootstrap_index::catalog(view.as_ref(), &root, after).unwrap();
        for g in page.grants {
            if g.owner.binding == managed {
                assert_eq!(g.admission, Some(origin.clone()));
                assert_eq!(g.owner.phase, PinPhase::Published);
                managed_count += 1;
            } else {
                assert!(g.admission.is_none());
                found.push(g.owner);
            }
        }
        match page.next {
            Some(next) => after = Some(next),
            None => break,
        }
    }
    found.sort_by_key(|o| o.binding.descriptor.id);
    expected.sort_by_key(|o| o.binding.descriptor.id);
    assert_eq!(found, expected);
    assert_eq!(managed_count, 1);
    assert_eq!(
        kv9_meta::retention::retention_owner(view.as_ref(), &root, version.descriptor.id)
            .unwrap()
            .unwrap()
            .phase,
        PinPhase::Published
    );
    drop(view);
    drop(engine);
    std::fs::remove_dir_all(directory).unwrap();
}
