use super::*;
use kv9_meta::retention::checkpoint_intent::CheckpointIntent;
use kv9_meta::retention::reader_index::{observe, readers, Work};
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
async fn global_reader_census_resumes_lost_receipt_on_new_leader_and_preserves_legacy_phases_in_wal(
) {
    let mut f = Fixture::new().await;
    let backend = f.backends[1].clone();
    let ledger = RoutedLedger::new(&f.backends[0], tokio::runtime::Handle::current());
    let (root, pending, operation, mut expected) = tokio::task::spawn_blocking(move || {
        let root = backend.ttl_context().unwrap().root.clone();
        let engine = backend.node.meta_raft.store.engine();
        let plan = settlement::planned(engine, &root, kv9_common::META_REGION_0);
        let intent = CheckpointIntent::new(&root, plan.manifest(), 0).unwrap();
        let pending = intent.binding(OwnerKind::Pending).unwrap();
        let apply = |r| {
            ledger
                .apply(encode_request(root.digest(), &r).unwrap())
                .unwrap()
        };
        apply(LedgerRequest::Initialize);
        apply(LedgerRequest::Register(pending.resources.clone()));
        apply(LedgerRequest::Acquire(pending.clone()));
        apply(LedgerRequest::Publish(pending.token()));
        let mut expected = Vec::new();
        for n in 1..=9 {
            let mut lifetime = ledger.local_lifetime().unwrap();
            lifetime.process = [n; 16];
            let b = kv9_meta::retention::checkpoint_reader::binding(&pending, lifetime).unwrap();
            apply(LedgerRequest::Share {
                from: pending.token(),
                to: b.clone(),
            });
            let phase = match n % 3 {
                1 => PinPhase::Held,
                2 => {
                    apply(LedgerRequest::Publish(b.token()));
                    PinPhase::Published
                }
                _ => {
                    apply(LedgerRequest::Publish(b.token()));
                    apply(LedgerRequest::FinishCheckpointRead {
                        successor: pending.token(),
                        reader: b.clone(),
                    });
                    PinPhase::Released
                }
            };
            expected.push((b.descriptor.id, phase));
        }
        // Explicit legacy V8 fixture replicated to all nodes: preserve every
        // phase and pin, encode legacy owner rows and remove new indexes and
        // generation evidence. This batch deliberately constructs old bytes.
        let view = engine.snapshot().unwrap();
        let mut batch = kv9_engine::WriteBatch::new();
        for row in view
            .iter(ColumnFamily::Default, &key(23), &key(26))
            .unwrap()
        {
            batch.delete(ColumnFamily::Default, row.unwrap().0);
        }
        // Image directory/count/aggregate records did not exist in V8.
        for row in view
            .iter(ColumnFamily::Default, &key(38), &key(42))
            .unwrap()
        {
            batch.delete(ColumnFamily::Default, row.unwrap().0);
        }
        for (id, _) in &expected {
            let owner = kv9_meta::retention::retention_owner(view.as_ref(), &root, *id)
                .unwrap()
                .unwrap();
            let mut k = key(3);
            k.extend_from_slice(id.as_bytes());
            batch.put(
                ColumnFamily::Default,
                k,
                kv9_meta::retention::encode_owner_observation(&owner).unwrap(),
            );
            let mut registered = owner;
            registered.binding.generation = std::num::NonZeroU64::new(1).unwrap();
            registered.phase = PinPhase::Held;
            let mut k = key(5);
            k.extend_from_slice(id.as_bytes());
            batch.put(
                ColumnFamily::Default,
                k,
                kv9_meta::retention::encode_owner_observation(&registered).unwrap(),
            );
        }
        let mut header = view.get(ColumnFamily::Default, &key(1)).unwrap().unwrap();
        header.truncate(header.len() - 32);
        header[..8].copy_from_slice(b"KV9LED08");
        header.extend_from_slice(RootDigest::sha256(&header).as_bytes());
        batch.put(ColumnFamily::Default, key(1), header);
        drop(view);
        let at = backend
            .driver
            .propose(&Command::from_batch(&batch))
            .unwrap();
        backend
            .driver
            .wait_applied(at, Duration::from_secs(3))
            .unwrap();
        assert!(matches!(
            observe(engine.snapshot().unwrap().as_ref(), &root).unwrap(),
            Work::Upgrade
        ));
        (
            root,
            pending.descriptor.id,
            pending.descriptor.operation,
            expected,
        )
    })
    .await
    .unwrap();
    expected.sort_by_key(|(id, _)| *id);
    assert_eq!(
        census(&f, 1, CancellationToken::new())
            .await
            .unwrap()
            .build_index_upgrades,
        1
    );
    // Build census has its own cursor. Complete it before dropping the first
    // committed Reader-page response, using the same actual maintenance pass.
    loop {
        let view = f.backends[1]
            .node
            .meta_raft
            .store
            .engine()
            .snapshot()
            .unwrap();
        if matches!(
            kv9_meta::retention::build_index::observe(view.as_ref(), &root).unwrap(),
            kv9_meta::retention::build_index::Work::Complete
        ) {
            break;
        }
        drop(view);
        census(&f, 1, CancellationToken::new()).await.unwrap();
    }
    f.hooks[1].retention_calls.store(0, Ordering::SeqCst);
    f.hooks[1].retention_commits.store(0, Ordering::SeqCst);
    f.hooks[1].retention_fail_after.store(1, Ordering::SeqCst);
    assert!(census(&f, 1, CancellationToken::new()).await.is_err());
    assert_eq!(f.hooks[1].retention_commits.load(Ordering::SeqCst), 1);
    assert!(
        matches!(observe(f.backends[1].node.meta_raft.store.engine().snapshot().unwrap().as_ref(), &root).unwrap(), Work::Page { progress, .. } if progress.sequence == 1)
    );
    f.hooks[1].retention_fail_after.store(0, Ordering::SeqCst);
    f.meta[1].peer().transfer_leader_for_tests(NodeId(3));
    until(|| f.meta[2].status().role == Role::Leader && f.meta[1].status().role != Role::Leader)
        .await;
    assert!(census(&f, 1, CancellationToken::new()).await.is_err());
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(census(&f, 2, cancel).await.is_err());

    // The actual maintenance lane, without an uploader, completes both passes
    // from the replicated cursor. No legacy phase is promoted into drain proof.
    let tasks: Vec<_> = f
        .backends
        .iter()
        .map(|b| Maintenance::start(b.clone(), &tokio::runtime::Handle::current()))
        .collect();
    until(|| tasks[2].snapshot().data_store_history_catalog_complete).await;
    assert_eq!(tasks[2].snapshot().peak_in_flight, 1);
    assert_eq!(tasks[0].snapshot().reader_index_pages, 0);
    assert_eq!(tasks[1].snapshot().reader_index_pages, 0);
    for task in &tasks {
        task.cancel();
    }
    for task in tasks {
        task.drain().await;
    }
    let stable_rows = owner_and_placement_rows(&f, 2);
    for _ in 0..3 {
        assert!(
            census(&f, 2, CancellationToken::new())
                .await
                .unwrap()
                .reader_index_catalog_complete
        );
    }
    assert_eq!(owner_and_placement_rows(&f, 2), stable_rows);
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
    assert!(matches!(
        observe(view.as_ref(), &root).unwrap(),
        Work::Complete
    ));
    let mut found = Vec::new();
    let mut after = None;
    loop {
        let page = readers(view.as_ref(), &root, operation, after).unwrap();
        for owner in page.owners {
            let mut k = key(3);
            k.extend_from_slice(owner.binding.descriptor.id.as_bytes());
            assert!(view
                .get(ColumnFamily::Default, &k)
                .unwrap()
                .unwrap()
                .starts_with(b"KV9OWN01"));
            found.push((owner.binding.descriptor.id, owner.phase));
        }
        match page.next {
            Some(next) => after = Some(next),
            None => break,
        }
    }
    assert_eq!(found, expected);
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
