use super::*;
use prost::Message;
use proto::kv9_server::Kv9;
use std::time::Duration;

fn message() -> proto::RawBatchGetRequest {
    proto::RawBatchGetRequest {
        context: Some(request_context_message()),
        keys: vec![b"b".to_vec(), b"a".to_vec(), b"b".to_vec()],
    }
}

#[test]
fn completed_batch_read_bypasses_a_saturated_blocking_pool() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            entered_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        });
        entered_rx.await.unwrap();
        let backend = Arc::new(FakeBackend {
            raw_completed: true,
            ..Default::default()
        });
        let service = Kv9Grpc::with_limits(
            backend.clone(),
            PublicApiLimits {
                max_requests: 1,
                max_encoded_bytes: message().encoded_len(),
            },
        )
        .unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            service.raw_batch_get(authenticated(message())),
        )
        .await;
        release_tx.send(()).unwrap();
        blocker.await.unwrap();
        let values = result
            .expect("resident batch queued behind a blocked engine worker")
            .unwrap()
            .into_inner()
            .values;
        assert_eq!(values.len(), 3);
        assert!(values.iter().all(|value| !value.found));
        assert_eq!(backend.callers.lock().unwrap().len(), 1);
        let state = service.admission().snapshot();
        assert_eq!(
            (state.in_flight, state.running, state.encoded_bytes),
            (0, 0, 0)
        );
        assert_eq!(
            (
                state.raw_get_completed_inline,
                state.raw_get_blocking_submitted
            ),
            (0, 0)
        );
        assert_eq!(
            (
                state.raw_batch_get_completed_inline,
                state.raw_batch_get_blocking_submitted
            ),
            (1, 0)
        );
        assert!(state
            .status_lines()
            .contains("public_raw_batch_get_completed_inline=1\n"));
    });
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_batch_preparation_drops_the_wait_without_starting_engine_work() {
    let (entered_tx, mut entered_rx) = tokio::sync::mpsc::unbounded_channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let (dropped_tx, mut dropped_rx) = tokio::sync::mpsc::unbounded_channel();
    let backend = Arc::new(FakeBackend {
        raw_preparation_gate: Some(RawPreparationGate {
            entered: entered_tx,
            release: Mutex::new(Some(release_rx)),
            dropped: dropped_tx,
        }),
        ..Default::default()
    });
    let bytes = message().encoded_len();
    let service = Kv9Grpc::with_limits(
        backend.clone(),
        PublicApiLimits {
            max_requests: 1,
            max_encoded_bytes: bytes,
        },
    )
    .unwrap();
    let admission = service.admission();
    let pending =
        tokio::spawn(async move { service.raw_batch_get(authenticated(message())).await });
    tokio::time::timeout(Duration::from_secs(5), entered_rx.recv())
        .await
        .unwrap()
        .unwrap();
    let state = admission.snapshot();
    assert_eq!(
        (state.in_flight, state.running, state.encoded_bytes),
        (1, 1, bytes)
    );
    assert!(backend.callers.lock().unwrap().is_empty());
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(5), dropped_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(release_tx.send(()).is_err());
    assert!(backend.callers.lock().unwrap().is_empty());
    let state = admission.snapshot();
    assert_eq!(
        (state.in_flight, state.running, state.encoded_bytes),
        (0, 0, 0)
    );
    assert_eq!(
        (state.classes[0].completed, state.classes[0].backend_aborted),
        (0, 1)
    );
    assert_eq!(
        (
            state.raw_batch_get_completed_inline,
            state.raw_batch_get_blocking_submitted
        ),
        (0, 0)
    );
    assert!(admission.reserve(WorkClass::RawRead, bytes).is_ok());
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_blocking_batch_keeps_its_whole_request_reservation() {
    let (entered_tx, mut entered_rx) = tokio::sync::mpsc::unbounded_channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let backend = Arc::new(FakeBackend {
        raw_gate: Some((entered_tx, Mutex::new(release_rx))),
        ..Default::default()
    });
    let bytes = message().encoded_len();
    let service = Arc::new(
        Kv9Grpc::with_limits(
            backend.clone(),
            PublicApiLimits {
                max_requests: 1,
                max_encoded_bytes: bytes,
            },
        )
        .unwrap(),
    );
    let task_service = service.clone();
    let pending =
        tokio::spawn(async move { task_service.raw_batch_get(authenticated(message())).await });
    tokio::time::timeout(Duration::from_secs(5), entered_rx.recv())
        .await
        .unwrap()
        .unwrap();
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    let admission = service.admission();
    let state = admission.snapshot();
    let refused = service.raw_batch_get(authenticated(message())).await;
    // Always let the owned blocking job finish before assertions can unwind.
    release_tx.send(()).unwrap();
    assert_eq!(
        (state.in_flight, state.running, state.encoded_bytes),
        (1, 1, bytes)
    );
    assert_eq!(refused.unwrap_err().code(), Code::ResourceExhausted);
    tokio::time::timeout(Duration::from_secs(5), async {
        while admission.snapshot().in_flight != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(backend.callers.lock().unwrap().len(), 1);
    let state = admission.snapshot();
    assert_eq!(
        (
            state.raw_get_completed_inline,
            state.raw_get_blocking_submitted
        ),
        (0, 0)
    );
    assert_eq!(
        (
            state.raw_batch_get_completed_inline,
            state.raw_batch_get_blocking_submitted
        ),
        (0, 1)
    );
    assert_eq!(
        (state.classes[0].completed, state.classes[0].backend_errors),
        (1, 0)
    );
}
