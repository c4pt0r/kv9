use super::*;
use crate::checkpoint::{FlushJournal, FlushScope, RemoteUploader};
use crate::{ColumnFamily, Engine, ReplicatedEngine, WalEngine, WriteBatch};
use kv9_common::AppliedPosition;
use s3_client::memory::InMemory;

#[test]
fn actual_put_replies_resolve_writes_even_when_followup_comparison_fails() {
    use crate::object_execution::{DrainOnExit, Execution};
    let scope = Arc::new(Execution::default());
    let drain = DrainOnExit(scope.clone());
    let store = store(
        Arc::new(InMemory::new()),
        Arc::new(ObjectIoBudget::new(u64::MAX, 1024).unwrap()),
        Duration::from_secs(2),
    );
    let key = ObjectKey::new("drain/answered").unwrap();
    store
        .put_in_execution(&key, b"value", Some(&scope))
        .unwrap();
    store
        .put_in_execution(&key, b"value", Some(&scope))
        .unwrap();
    assert!(matches!(
        store.put_in_execution(&key, b"other", Some(&scope)),
        Err(Error::ObjectContentMismatch { .. })
    ));
    drop(drain);
    let report = scope.report().unwrap();
    assert_eq!((report.issued_puts(), report.resolved_puts()), (3, 3));
}

#[test]
fn put_admission_budget_timeout_happens_before_any_possible_write() {
    use crate::object_execution::{DrainOnExit, Execution};
    let scope = Arc::new(Execution::default());
    let drain = DrainOnExit(scope.clone());
    let store = store(
        Arc::new(InMemory::new()),
        Arc::new(ObjectIoBudget::new(1, 4).unwrap()),
        Duration::from_millis(40),
    );
    store
        .put_in_execution(
            &ObjectKey::new("budget/first").unwrap(),
            b"data",
            Some(&scope),
        )
        .unwrap();
    assert!(store
        .put_in_execution(
            &ObjectKey::new("budget/next").unwrap(),
            b"data",
            Some(&scope)
        )
        .is_err());
    drop(drain);
    let report = scope.report().unwrap();
    assert_eq!((report.issued_puts(), report.resolved_puts()), (1, 1));
}

#[tokio::test]
async fn timed_out_sdk_future_remains_unknown_after_a_successful_retry_in_the_same_scope() {
    use crate::object_execution::{DrainOnExit, Execution};
    use s3_client::throttle::{ThrottleConfig, ThrottledStore};
    let scope = Arc::new(Execution::default());
    let drain = DrainOnExit(scope.clone());
    let lease = scope.admit().unwrap();
    let budget = ObjectIoBudget::new(u64::MAX, 1024).unwrap();
    let buffers = Arc::new(BufferBudget::default());
    let slow = ThrottledStore::new(
        InMemory::new(),
        ThrottleConfig {
            wait_put_per_call: Duration::from_secs(5),
            ..Default::default()
        },
    );
    assert!(tokio::time::timeout(
        Duration::from_millis(5),
        do_put(
            &slow,
            "unknown/first",
            Bytes::from_static(b"value"),
            Some(&budget),
            &buffers,
            Some(&lease)
        )
    )
    .await
    .is_err());
    do_put(
        &InMemory::new(),
        "unknown/next",
        Bytes::from_static(b"value"),
        Some(&budget),
        &buffers,
        Some(&lease),
    )
    .await
    .unwrap();
    drop(lease);
    drop(drain);
    let report = scope.report().unwrap();
    assert_eq!((report.issued_puts(), report.resolved_puts()), (2, 1));
}

#[test]
fn budgeted_client_never_retries_beneath_payload_admission() {
    let ceiling = Duration::from_secs(24);
    assert_eq!(retry_policy(ceiling, true).max_retries, 0);
    assert_eq!(retry_policy(ceiling, true).retry_timeout, ceiling);
    assert_eq!(
        retry_policy(ceiling, false).max_retries,
        RetryConfig::default().max_retries
    );
}

// Run the production worker loop, deadline and payload operations with the
// upstream in-memory transport. This is not MinIO durability/network evidence.
fn store(
    client: Arc<InMemory>,
    budget: Arc<ObjectIoBudget>,
    deadline: Duration,
) -> MinioObjectStore {
    store_with_buffers(client, budget, Arc::default(), deadline)
}
pub(super) fn store_with_buffers(
    client: Arc<InMemory>,
    budget: Arc<ObjectIoBudget>,
    buffers: Arc<BufferBudget>,
    deadline: Duration,
) -> MinioObjectStore {
    let (tx, rx) = sync_channel(JOB_QUEUE_DEPTH);
    let (ready, receive) = sync_channel(1);
    let io = budget.clone();
    let worker_buffers = buffers.clone();
    let worker = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        drive_jobs(
            client.as_ref(),
            Bounded { runtime },
            configured_budget(deadline),
            Some(&io),
            &worker_buffers,
            rx,
            ready,
        );
    });
    receive.recv().unwrap().unwrap();
    MinioObjectStore {
        test_gate: None,
        test_control: None,
        io_budget: Some(budget),
        buffer_budget: buffers,
        tx: Some(tx),
        worker: Some(worker),
        op_deadline: deadline,
        endpoint: "in-memory test transport".into(),
        bucket: "test".into(),
    }
}

#[test]
fn worker_charges_conditional_put_comparison_and_bounded_reads_without_refunding_errors() {
    let budget = Arc::new(ObjectIoBudget::new(u64::MAX, 1024).unwrap());
    let store = store(
        Arc::new(InMemory::new()),
        budget.clone(),
        Duration::from_secs(2),
    );
    let key = ObjectKey::new("sst/key").unwrap();
    store.put(&key, &[7; 64]).unwrap();
    store.put(&key, &[7; 64]).unwrap(); // The existing-object GET is also charged.
    assert_eq!(store.get_bounded(&key, 64).unwrap(), Some(vec![7; 64]));
    assert_eq!(store.get(&key).unwrap(), Some(vec![7; 64])); // HEAD then bounded GET.
    assert!(matches!(
        store.put(&key, &[8; 4]),
        Err(Error::ObjectContentMismatch { .. })
    ));
    assert!(store.get_bounded(&key, 4).is_err());
    let stats = budget.snapshot();
    assert_eq!(stats.admitted_put_bytes, 132);
    assert_eq!(stats.admitted_get_bytes, 64 * 3 + 4 * 2);
    assert_eq!(stats.admitted_operations, 8);
    assert_eq!(stats.waiting_operations, 0);
    assert_eq!(store.get_bounded(&key, 64).unwrap(), Some(vec![7; 64]));
}

#[test]
fn distinct_object_workers_share_one_payload_rate_and_burst() {
    let budget = Arc::new(ObjectIoBudget::new(1000, 64).unwrap());
    let client = Arc::new(InMemory::new());
    let stores = [
        store(client.clone(), budget.clone(), Duration::from_secs(2)),
        store(client, budget.clone(), Duration::from_secs(2)),
    ];
    let start = Instant::now();
    let jobs: Vec<_> = stores
        .into_iter()
        .enumerate()
        .map(|(worker, store)| {
            std::thread::spawn(move || {
                for n in 0..4 {
                    store
                        .put(
                            &ObjectKey::new(format!("sst/{worker}/{n}")).unwrap(),
                            &[7; 64],
                        )
                        .unwrap();
                }
            })
        })
        .collect();
    for job in jobs {
        job.join().unwrap();
    }
    let stats = budget.snapshot();
    assert_eq!(stats.admitted_put_bytes, 512);
    assert_eq!(stats.admitted_get_bytes, 0);
    assert_eq!(stats.admitted_operations, 8);
    assert!(start.elapsed() >= Duration::from_millis(448));
    assert_eq!(stats.waiting_operations, 0);
    assert_eq!(stats.cancelled_before_admission, 0);
}

#[tokio::test]
async fn bounded_collection_rejects_headers_and_stream_overflow_before_polling_the_tail() {
    use futures_util::StreamExt;
    let client = InMemory::new();
    let path = S3Path::from("sst/key");
    client.put(&path, vec![0; 100].into()).await.unwrap();
    let mut response = client.get(&path).await.unwrap();
    response.meta.size = 101;
    response.payload =
        s3_client::GetResultPayload::Stream(Box::pin(futures_util::stream::once(async {
            panic!("oversize headers must reject without polling body");
            #[allow(unreachable_code)]
            Ok(Vec::<u8>::new().into())
        })));
    assert!(matches!(
        collect_body(response, Some(100), &Arc::default())
            .await
            .unwrap(),
        Body::TooLarge
    ));
    let mut response = client.get(&path).await.unwrap();
    let chunks = futures_util::stream::iter([Ok(vec![1; 60].into()), Ok(vec![2; 60].into())]);
    let tail = futures_util::stream::once(async {
        panic!("overflow must stop before later chunks");
        #[allow(unreachable_code)]
        Ok(Vec::<u8>::new().into())
    });
    response.payload = s3_client::GetResultPayload::Stream(Box::pin(chunks.chain(tail)));
    assert!(matches!(
        collect_body(response, Some(100), &Arc::default())
            .await
            .unwrap(),
        Body::TooLarge
    ));
    let mut response = client.get(&path).await.unwrap();
    response.payload = s3_client::GetResultPayload::Stream(Box::pin(futures_util::stream::iter([
        Ok(vec![0; 20].into()),
    ])));
    assert!(collect_body(response, Some(100), &Arc::default())
        .await
        .is_err());
}

#[test]
fn throttled_pending_retry_keeps_identity_and_finishes_with_the_same_budget_after_reopen() {
    let path =
        std::env::temp_dir().join(format!("kv9-object-budget-pending-{}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    let engine = WalEngine::open(path.join("data.wal")).unwrap().0;
    let mut batch = WriteBatch::new();
    for cf in ColumnFamily::ALL {
        batch.put(cf, b"key".to_vec(), vec![9; 128]);
    }
    engine
        .write_applied(batch, AppliedPosition { term: 1, index: 4 })
        .unwrap();
    let plan = RemoteUploader::plan(
        engine
            .freeze(FlushScope {
                cluster: "budget-test".into(),
                region: 100,
                conf_ver: 1,
                version: 1,
            })
            .unwrap(),
    )
    .unwrap();
    let manifest = plan.manifest().clone();
    let burst = manifest.files.iter().map(|f| f.size).max().unwrap();
    let budget = Arc::new(ObjectIoBudget::new(1000, burst).unwrap());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    // Exhaust credit before the short-deadline attempt: its actual worker
    // times out waiting for admission, without receiving a prepared capability.
    runtime
        .block_on(budget.admit(Direction::Put, burst))
        .unwrap();
    let client = Arc::new(InMemory::new());
    let mut journal = FlushJournal::new(path.join("data.pending"));
    journal.stage_planned(&plan, 7).unwrap();
    let original = std::fs::read(path.join("data.pending")).unwrap();
    let uploader = RemoteUploader::new(Arc::new(store(
        client.clone(),
        budget.clone(),
        Duration::from_millis(10),
    )));
    assert!(journal.load().unwrap().unwrap().recover(&uploader).is_err());
    assert_eq!(std::fs::read(path.join("data.pending")).unwrap(), original);
    drop(uploader);
    drop(journal);
    let mut journal = FlushJournal::new(path.join("data.pending"));
    let uploader = RemoteUploader::new(Arc::new(store(
        client,
        budget.clone(),
        Duration::from_secs(2),
    )));
    let pending = journal.load().unwrap().unwrap();
    assert_eq!(pending.manifest(), &manifest);
    assert_eq!(pending.expected_generation(), 7);
    let (prepared, generation) = pending.recover(&uploader).unwrap();
    journal.stage(&prepared, generation).unwrap();
    assert_eq!(prepared.into_manifest(), manifest);
    assert!(!journal.load().unwrap().unwrap().requires_upload());
    let restored = uploader.restore(&manifest).unwrap();
    for cf in ColumnFamily::ALL {
        assert_eq!(restored.get(cf, b"key").unwrap(), Some(vec![9; 128]));
    }
    assert!(budget.snapshot().cancelled_before_admission >= 1);
    assert_eq!(budget.snapshot().waiting_operations, 0);
    drop(uploader);
    drop(engine);
    std::fs::remove_dir_all(path).unwrap();
}
