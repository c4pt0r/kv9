use super::budget_tests::store_with_buffers;
use super::*;
use s3_client::memory::InMemory;
use std::sync::Mutex;

fn io() -> Arc<ObjectIoBudget> {
    Arc::new(ObjectIoBudget::new(u64::MAX, 1024).unwrap())
}

#[test]
fn sdk_and_get_owners_keep_charge_until_the_last_shared_reference_is_gone() {
    let buffers = Arc::new(BufferBudget::new(192).unwrap());
    let client = Arc::new(InMemory::new());
    let store = store_with_buffers(client, io(), buffers.clone(), Duration::from_secs(2));
    let key = ObjectKey::new("sst/retained").unwrap();
    store.put(&key, &[7; 64]).unwrap();
    // Upstream InMemory intentionally retains the exact SDK payload Bytes.
    // This proves reservation lifetime beyond the completed PUT future.
    assert_eq!(buffers.snapshot().retained_bytes, 64);
    store.put(&key, &[7; 64]).unwrap(); // Includes a separately charged comparison GET.
    assert_eq!(buffers.snapshot().peak_bytes, 192);
    assert_eq!(buffers.snapshot().retained_bytes, 64);
    let bytes = store.get_shared_bounded(&key, 64).unwrap().unwrap();
    let clone = bytes.clone();
    let slice = clone.slice(1..2);
    assert_eq!(buffers.snapshot().retained_bytes, 128);
    drop((bytes, clone));
    assert_eq!(buffers.snapshot().retained_bytes, 128);
    store.delete(&key).unwrap();
    assert_eq!(buffers.snapshot().retained_bytes, 64);
    assert_eq!(slice.as_ref(), &[7]);
    drop(slice);
    assert_eq!(buffers.snapshot().retained_bytes, 0);
    assert_eq!(buffers.snapshot().active_buffers, 0);
}

#[test]
fn distinct_workers_share_buffer_admission_and_refuse_before_a_put_is_queued() {
    let buffers = Arc::new(BufferBudget::new(128).unwrap());
    let a = store_with_buffers(
        Arc::new(InMemory::new()),
        io(),
        buffers.clone(),
        Duration::from_secs(2),
    );
    let b = store_with_buffers(
        Arc::new(InMemory::new()),
        io(),
        buffers.clone(),
        Duration::from_secs(2),
    );
    let key = ObjectKey::new("sst/shared").unwrap();
    a.put(&key, &[1; 64]).unwrap();
    b.put(&key, &[2; 64]).unwrap();
    let other = ObjectKey::new("sst/other").unwrap();
    let (source, result) = a.put_traced(&other, &[3; 64]);
    assert_eq!(source, TestCompletion::AdmissionRefused);
    assert!(result.is_err());
    assert_eq!(a.list("sst/").unwrap(), std::slice::from_ref(&key));
    assert_eq!(buffers.snapshot().retained_bytes, 128);
    b.delete(&key).unwrap();
    a.put(&other, &[3; 64]).unwrap();
    a.delete(&other).unwrap();
    a.delete(&key).unwrap();
    assert_eq!(buffers.snapshot().retained_bytes, 0);
    assert_eq!(buffers.snapshot().peak_bytes, 128);
    assert_eq!(buffers.snapshot().refused_buffers, 1);
}

#[tokio::test]
async fn refused_and_invalid_gets_release_reserved_buffers_without_polling_excess_data() {
    let client = InMemory::new();
    let path = S3Path::from("sst/bounded");
    client.put(&path, vec![0; 32].into()).await.unwrap();
    let buffers = Arc::new(BufferBudget::new(32).unwrap());
    let held = buffers.copy(&[7; 16]).unwrap();
    let mut response = client.get(&path).await.unwrap();
    response.payload =
        s3_client::GetResultPayload::Stream(Box::pin(futures_util::stream::once(async {
            panic!("refused allocation must not poll body");
            #[allow(unreachable_code)]
            Ok(Vec::<u8>::new().into())
        })));
    assert!(collect_body(response, Some(32), &buffers).await.is_err());
    assert_eq!(buffers.snapshot().retained_bytes, 16);
    drop(held);
    let mut response = client.get(&path).await.unwrap();
    response.payload = s3_client::GetResultPayload::Stream(Box::pin(futures_util::stream::iter([
        Ok(vec![0; 31].into()),
    ])));
    assert!(collect_body(response, Some(32), &buffers).await.is_err());
    assert_eq!(buffers.snapshot().retained_bytes, 0);
    let response = client.get(&path).await.unwrap();
    assert!(matches!(
        collect_body(response, Some(31), &buffers).await.unwrap(),
        Body::TooLarge
    ));
    assert_eq!(buffers.snapshot().retained_bytes, 0);
    assert!(BufferBudget::new(0).is_err());
}

#[test]
fn actual_worker_keeps_timed_out_queued_payload_until_it_declines_the_expired_job() {
    let (stamped_tx, stamped) = sync_channel(1);
    let (stamp_release, stamp_rx) = sync_channel(1);
    let (arrived_tx, arrived) = sync_channel(1);
    let (worker_release, worker_rx) = sync_channel(1);
    let (wait_tx, _wait) = sync_channel(1);
    let control = Arc::new(DeadlineControl {
        origin: Instant::now(),
        elapsed_ns: AtomicU64::new(0),
        stamped: stamped_tx,
        stamp_release: Mutex::new(stamp_rx),
        worker_arrived: arrived_tx,
        worker_release: Mutex::new(worker_rx),
        caller_wait: wait_tx,
    });
    let buffers = Arc::new(BufferBudget::new(64).unwrap());
    let mut store = MinioObjectStore::connect_with_budgets(
        MinioConfig {
            endpoint: "http://127.0.0.1:1".into(),
            bucket: "memory-probe".into(),
            access_key: "unused-fixture".into(),
            secret_key: "unused-fixture".into(),
            op_deadline: Duration::from_secs(60),
        },
        io(),
        buffers.clone(),
    )
    .unwrap();
    store.test_control = Some(control.clone());
    let store = Arc::new(store);
    struct Release(SyncSender<()>);
    impl Drop for Release {
        fn drop(&mut self) {
            let _ = self.0.try_send(());
        }
    }
    let release = Release(worker_release);
    let caller = store.clone();
    let thread = std::thread::spawn(move || {
        caller.put_traced(&ObjectKey::new("sst/queued").unwrap(), &[7; 64])
    });
    stamped.recv_timeout(Duration::from_secs(5)).unwrap();
    control
        .elapsed_ns
        .store(Duration::from_secs(61).as_nanos() as u64, Ordering::SeqCst);
    stamp_release.send(()).unwrap();
    arrived.recv_timeout(Duration::from_secs(5)).unwrap();
    let (source, result) = thread.join().unwrap();
    assert_eq!(source, TestCompletion::OuterFallback);
    assert!(result.is_err());
    assert_eq!(buffers.snapshot().retained_bytes, 64);
    let (source, result) = store.put_traced(&ObjectKey::new("sst/refused").unwrap(), &[1; 1]);
    assert_eq!(source, TestCompletion::AdmissionRefused);
    assert!(result.is_err());
    drop(release);
    let until = Instant::now() + Duration::from_secs(5);
    while buffers.snapshot().retained_bytes != 0 {
        assert!(
            Instant::now() < until,
            "expired worker must drop its payload"
        );
        std::thread::yield_now();
    }
    assert_eq!(buffers.snapshot().active_buffers, 0);
}
