//! Runtime batch-read preparation contracts. These use real three-voter
//! runtimes, but do not claim public-RPC cancellation or fault-matrix coverage.

use super::*;
use crate::api::RawReadJob;

fn batch_context(backend: &RuntimeBackend, name: &str) -> RequestContext {
    let created = backend
        .create_keyspace(
            "acceptance",
            name,
            TenantId::DEFAULT,
            ApiType::Raw,
            TxnGroupId(0),
        )
        .unwrap();
    let location = backend
        .get_region("acceptance", created.keyspace, b"alpha")
        .unwrap();
    RequestContext {
        keyspace: created.keyspace,
        region_epoch: location.epoch,
        origin: crate::api::RequestOrigin::from_transport("acceptance"),
    }
}

fn ordered_keys() -> Vec<UserKey> {
    [
        b"missing".as_slice(),
        b"empty",
        b"alpha",
        b"alpha",
        b"empty",
    ]
    .into_iter()
    .map(<[u8]>::to_vec)
    .collect()
}

fn before_values() -> Vec<Option<Value>> {
    vec![
        None,
        Some(Vec::new()),
        Some(b"before".to_vec()),
        Some(b"before".to_vec()),
        Some(Vec::new()),
    ]
}

fn after_values() -> Vec<Option<Value>> {
    vec![
        Some(b"created".to_vec()),
        Some(b"nonempty".to_vec()),
        Some(b"after".to_vec()),
        Some(b"after".to_vec()),
        Some(b"nonempty".to_vec()),
    ]
}

fn seed_batch(backend: &RuntimeBackend, ctx: &RequestContext) {
    backend
        .raw_batch_put(
            ctx,
            &[
                (b"alpha".to_vec(), b"before".to_vec()),
                (b"empty".to_vec(), Vec::new()),
            ],
        )
        .unwrap();
}

fn replacement_batch() -> Vec<(UserKey, Value)> {
    vec![
        (b"alpha".to_vec(), b"after".to_vec()),
        (b"empty".to_vec(), b"nonempty".to_vec()),
        (b"missing".to_vec(), b"created".to_vec()),
    ]
}

fn assert_batch_reads_drained(backend: &RuntimeBackend) {
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        let state = backend.driver.async_read_snapshot();
        assert!(!state.stopped, "batch read stopped the async registry");
        if state.in_flight == 0 && state.queued == 0 && state.active == 0 {
            break;
        }
        assert!(
            Instant::now() < until,
            "async batch reservation did not drain"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn async_batch_preserves_input_order_duplicates_missing_and_empty() {
    let (rts, root, _, base) = serving_trio("async-batch-order");
    let leader = cluster_leader(&rts).unwrap();
    let backend = Arc::new(backend_view(&rts[leader], &root));
    let ctx = batch_context(&backend, "async-batch-order");
    seed_batch(&backend, &ctx);
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let before = backend.driver.read_barriers_minted();
    let result = executor
        .block_on(backend.clone().prepare_raw_batch_get(ctx, ordered_keys()))
        .unwrap()
        .run()
        .unwrap();
    assert_eq!(
        result,
        before_values(),
        "batch read reordered or collapsed items"
    );
    assert_eq!(
        backend.driver.read_barriers_minted() - before,
        1,
        "one batch must establish one quorum barrier, not one per key"
    );
    assert!(
        backend.driver.async_read_snapshot().peak > 0,
        "batch preparation bypassed the asynchronous read registry"
    );
    assert_batch_reads_drained(&backend);
    drop(backend);
    drop(rts);
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn resident_batch_keeps_its_authorized_values_after_both_epoch_changes() {
    batch_epoch_case(false);
}

#[test]
fn contended_batch_checks_both_epoch_halves_when_the_job_executes() {
    batch_epoch_case(true);
}

fn batch_epoch_case(force_blocking: bool) {
    use kv9_meta::codec::{memcmp_uint, ColumnValue};
    use kv9_meta::schema::{ColumnId, REGIONS_DESC};

    let tag = if force_blocking {
        "async-batch-blocking-epoch"
    } else {
        "async-batch-resident-epoch"
    };
    let (rts, root, _, base) = serving_trio(tag);
    let leader = cluster_leader(&rts).unwrap();
    let backend = Arc::new(backend_view(&rts[leader], &root));
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    for (column, name) in [(5, "batch-conf-epoch"), (6, "batch-version-epoch")] {
        let ctx = batch_context(&backend, name);
        seed_batch(&backend, &ctx);
        let barrier = executor.block_on(backend.driver.read_barrier_async(READ_BARRIER_DEADLINE));
        // Contend only after quorum completion; never block the Raft owner
        // while waiting for that quorum. No view exists in this fallback yet.
        let held = force_blocking.then(|| backend.node.meta.lock().unwrap());
        let mut job = backend
            .clone()
            .finish_prepared_batch_get(ctx.clone(), ordered_keys(), barrier)
            .unwrap();
        drop(held);
        if force_blocking {
            assert!(
                matches!(job, RawReadJob::Blocking(_)),
                "lifecycle contention must retain an unexecuted batch job"
            );
        } else {
            // Publication can transiently contend. Retry only unmutating read
            // preparations, bounded in time, until the actual resident branch
            // is observed; no unknown write is retried.
            let until = Instant::now() + Duration::from_secs(5);
            while matches!(job, RawReadJob::Blocking(_)) {
                assert!(
                    Instant::now() < until,
                    "resident batch never completed inline"
                );
                job = executor
                    .block_on(
                        backend
                            .clone()
                            .prepare_raw_batch_get(ctx.clone(), ordered_keys()),
                    )
                    .unwrap();
            }
        }
        assert!(backend.driver.async_read_snapshot().peak > 0);

        let region = Tables::new(&backend.node.meta_raft.store)
            .region_for_key(ctx.keyspace, b"alpha")
            .unwrap()
            .unwrap();
        let mut next_ctx = ctx.clone();
        let next = if column == 5 {
            next_ctx.region_epoch.conf_ver += 1;
            next_ctx.region_epoch.conf_ver
        } else {
            next_ctx.region_epoch.version += 1;
            next_ctx.region_epoch.version
        };
        let term = backend.prepare_catalog().unwrap();
        let mut change = backend.node.meta_raft.store.begin().unwrap();
        change
            .update(
                &REGIONS_DESC,
                &[memcmp_uint(region.id.0)],
                vec![(ColumnId(column), ColumnValue::Uint(next))],
            )
            .unwrap();
        backend
            .commit_catalog(&Command::from_batch(&change.into_batch()), term)
            .unwrap();
        backend
            .raw_batch_put(&next_ctx, &replacement_batch())
            .unwrap();

        let result = job.run();
        if force_blocking {
            assert!(
                matches!(result, Err(Error::StaleEpoch { region: id }) if id == region.id),
                "deferred batch used obsolete epoch authorization: {result:?}"
            );
        } else {
            assert_eq!(
                result.unwrap(),
                before_values(),
                "completed batch was reevaluated or combined different versions"
            );
        }
        let stale = executor
            .block_on(backend.clone().prepare_raw_batch_get(ctx, ordered_keys()))
            .and_then(RawReadJob::run);
        assert!(
            matches!(stale, Err(Error::StaleEpoch { region: id }) if id == region.id),
            "fresh batch accepted the old epoch: {stale:?}"
        );
        let current = executor
            .block_on(
                backend
                    .clone()
                    .prepare_raw_batch_get(next_ctx, ordered_keys()),
            )
            .unwrap()
            .run()
            .unwrap();
        assert_eq!(
            current,
            after_values(),
            "fresh batch missed the committed replacement"
        );
    }
    assert_batch_reads_drained(&backend);
    drop(backend);
    drop(rts);
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn async_batch_waits_for_the_whole_committed_but_unapplied_write() {
    struct ResumeApply(Arc<NodeDriver<DiskRaftStorage, WalEngine>>);
    impl Drop for ResumeApply {
        fn drop(&mut self) {
            self.0.pause_apply(false);
        }
    }

    let (rts, root, _, base) = serving_trio("async-batch-committed-apply");
    let leader = cluster_leader(&rts).unwrap();
    let backend = Arc::new(backend_view(&rts[leader], &root));
    let ctx = batch_context(&backend, "async-batch-committed-apply");
    seed_batch(&backend, &ctx);
    let driver = backend.driver.clone();
    driver.pause_apply(true);
    std::thread::scope(|scope| {
        // Dropped before the scope joins on a failed assertion, so a failed
        // precondition cannot leave scoped workers waiting on frozen apply.
        let resume = ResumeApply(driver.clone());
        let put_backend = backend.clone();
        let put_ctx = ctx.clone();
        let put = scope.spawn(move || put_backend.raw_batch_put(&put_ctx, &replacement_batch()));
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            let status = driver.status();
            let applied = driver.driver_applied().map_or(0, |at| at.index);
            if status.raft_committed > applied {
                break;
            }
            assert!(status.fatal.is_none(), "driver failed before the apply cut");
            assert!(
                Instant::now() < until,
                "batch did not commit while apply was paused"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        let mints_before = driver.read_barriers_minted();
        let get_backend = backend.clone();
        let get = scope.spawn(move || {
            let executor = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .unwrap();
            executor
                .block_on(get_backend.prepare_raw_batch_get(ctx, ordered_keys()))?
                .run()
        });
        let until = Instant::now() + Duration::from_secs(2);
        while driver.read_barriers_minted() == mints_before
            || driver.async_read_snapshot().active == 0
        {
            assert!(
                !get.is_finished(),
                "batch completed before its paused apply cut"
            );
            assert!(
                Instant::now() < until,
                "batch never entered asynchronous preparation"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!get.is_finished(), "batch served a pre-apply snapshot");
        let committed_while_paused = driver.status().raft_committed;
        let applied_while_paused = driver.driver_applied().unwrap().index;
        assert!(committed_while_paused > applied_while_paused);
        drop(resume);
        let receipt = put.join().unwrap().unwrap();
        assert!(
            applied_while_paused < receipt.index && receipt.index <= committed_while_paused,
            "the actual batch write was not committed-but-unapplied at the read cut"
        );
        assert_eq!(
            get.join().unwrap().unwrap(),
            after_values(),
            "prepared batch exposed the old state or a partial atomic write"
        );
        assert_eq!(driver.read_barriers_minted() - mints_before, 1);
    });
    assert_batch_reads_drained(&backend);
    drop(backend);
    drop(rts);
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn oversized_resident_batch_moves_its_captured_authorized_view_into_the_job() {
    use kv9_meta::codec::{memcmp_uint, ColumnValue};
    use kv9_meta::schema::{ColumnId, REGIONS_DESC};

    let (rts, root, _, base) = serving_trio("async-batch-byte-budget");
    let leader = cluster_leader(&rts).unwrap();
    let backend = Arc::new(backend_view(&rts[leader], &root));
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    for (name, value_bytes, duplicates) in [
        ("duplicate-byte-budget", 65_536, 17),
        ("single-value-byte-budget", 1_048_577, 1),
    ] {
        let ctx = batch_context(&backend, name);
        let original = vec![b'v'; value_bytes];
        backend
            .raw_batch_put(
                &ctx,
                &[
                    (b"wide".to_vec(), original.clone()),
                    (b"empty".to_vec(), Vec::new()),
                ],
            )
            .unwrap();
        let mut keys = vec![b"missing".to_vec(), b"empty".to_vec()];
        keys.extend(std::iter::repeat_n(b"wide".to_vec(), duplicates));
        keys.push(b"missing".to_vec());
        let mut expected = vec![None, Some(Vec::new())];
        expected.extend(std::iter::repeat_n(Some(original), duplicates));
        expected.push(None);
        assert!(
            keys.len() <= 256,
            "the byte limit, not key count, selects this fallback"
        );

        backend.ensure_serving().unwrap();
        let mints_before = backend.driver.read_barriers_minted();
        let mut barrier = executor
            .block_on(backend.driver.read_barrier_async(READ_BARRIER_DEADLINE))
            .unwrap();
        // Acquire the actual resident view deterministically. A failed try
        // returns the SAME unconsumed credential, so retrying cannot mint a
        // second barrier or accidentally select lifecycle-contention fallback.
        let until = Instant::now() + Duration::from_secs(5);
        let view = loop {
            barrier = match backend.try_established_resident_view(barrier) {
                Ok(view) => break view,
                Err(unconsumed) => unconsumed,
            };
            assert!(
                Instant::now() < until,
                "resident view never became available"
            );
            std::thread::sleep(Duration::from_millis(1));
        };
        let job = backend
            .clone()
            .finish_resident_batch_get(ctx.clone(), keys.clone(), view)
            .unwrap();
        assert!(
            matches!(job, RawReadJob::Blocking(_)),
            "large or repeated values must defer materialization before copying the full batch"
        );
        assert_eq!(backend.driver.read_barriers_minted() - mints_before, 1);

        // This job already owns its authorized snapshot. Unlike the earlier
        // lifecycle-contention regression, later epochs must not invalidate it
        // or make it serve new values by recapturing a view at execution time.
        let region = Tables::new(&backend.node.meta_raft.store)
            .region_for_key(ctx.keyspace, b"wide")
            .unwrap()
            .unwrap();
        let mut next_ctx = ctx.clone();
        next_ctx.region_epoch.version += 1;
        let term = backend.prepare_catalog().unwrap();
        let mut change = backend.node.meta_raft.store.begin().unwrap();
        change
            .update(
                &REGIONS_DESC,
                &[memcmp_uint(region.id.0)],
                vec![(
                    ColumnId(6),
                    ColumnValue::Uint(next_ctx.region_epoch.version),
                )],
            )
            .unwrap();
        backend
            .commit_catalog(&Command::from_batch(&change.into_batch()), term)
            .unwrap();
        backend
            .raw_batch_put(
                &next_ctx,
                &[
                    (b"wide".to_vec(), b"replacement".to_vec()),
                    (b"empty".to_vec(), b"now-present".to_vec()),
                    (b"missing".to_vec(), b"created-later".to_vec()),
                ],
            )
            .unwrap();
        let mints_before_execution = backend.driver.read_barriers_minted();
        let returned = job.run().unwrap();
        assert_eq!(
            returned.len(),
            expected.len(),
            "captured batch cardinality changed"
        );
        for (index, (actual, wanted)) in returned.iter().zip(&expected).enumerate() {
            // A regression should identify its slot without printing megabytes
            // of repeated test data into a failed-test log.
            assert!(
                actual == wanted,
                "captured byte-budget fallback lost its old ordered view at slot {index}"
            );
        }
        assert_eq!(
            backend.driver.read_barriers_minted(),
            mints_before_execution,
            "captured fallback minted a second quorum credential"
        );
        let stale = executor
            .block_on(backend.clone().prepare_raw_batch_get(ctx, keys.clone()))
            .and_then(RawReadJob::run);
        assert!(
            matches!(stale, Err(Error::StaleEpoch { region: id }) if id == region.id),
            "control: a new read must reject the obsolete epoch: {stale:?}"
        );
        let current = executor
            .block_on(backend.clone().prepare_raw_batch_get(next_ctx, keys))
            .unwrap()
            .run()
            .unwrap();
        let mut next_expected = vec![
            Some(b"created-later".to_vec()),
            Some(b"now-present".to_vec()),
        ];
        next_expected.extend(std::iter::repeat_n(
            Some(b"replacement".to_vec()),
            duplicates,
        ));
        next_expected.push(Some(b"created-later".to_vec()));
        assert_eq!(
            current, next_expected,
            "control: new epoch did not see the replacement"
        );
    }
    assert_batch_reads_drained(&backend);
    drop(backend);
    drop(rts);
    fs::remove_dir_all(base).unwrap();
}
