use super::*;
use kv9_common::metrics::Outcome;
use proto::kv9_server::Kv9;
use std::time::Duration;

fn bounded(count: usize, bytes: usize) -> Kv9Grpc {
    Kv9Grpc::with_limits(
        Arc::new(FakeBackend::default()),
        PublicApiLimits {
            max_requests: count,
            max_encoded_bytes: bytes,
        },
    )
    .unwrap()
}

#[tokio::test]
async fn admission_covers_every_public_handler_before_preparation() {
    let service = bounded(1, 4096);
    let held = service.admission().reserve(WorkClass::RawRead, 1).unwrap();
    macro_rules! refused {
        ($method:ident, $request:ident) => {
            let status = service
                .clone()
                .$method(authenticated(proto::$request::default()))
                .await
                .unwrap_err();
            assert_eq!(
                admission_refusal(&status),
                Some("request_count"),
                stringify!($method)
            );
        };
    }
    refused!(raw_get, RawGetRequest);
    refused!(raw_batch_get, RawBatchGetRequest);
    refused!(raw_put, RawPutRequest);
    refused!(raw_batch_put, RawBatchPutRequest);
    refused!(raw_delete, RawDeleteRequest);
    refused!(raw_scan, RawScanRequest);
    refused!(raw_delete_range, RawDeleteRangeRequest);
    refused!(kv_begin, KvBeginRequest);
    refused!(kv_get, KvGetRequest);
    refused!(kv_batch_get, KvBatchGetRequest);
    refused!(kv_scan, KvScanRequest);
    refused!(kv_prewrite, KvPrewriteRequest);
    refused!(kv_commit, KvCommitRequest);
    refused!(kv_pessimistic_lock, KvPessimisticLockRequest);
    refused!(kv_pessimistic_rollback, KvPessimisticRollbackRequest);
    refused!(kv_resolve_lock, KvResolveLockRequest);
    refused!(kv_cleanup, KvCleanupRequest);
    refused!(kv_check_txn_status, KvCheckTxnStatusRequest);
    refused!(create_keyspace, CreateKeyspaceRequest);
    refused!(list_keyspaces, ListKeyspacesRequest);
    refused!(get_region, GetRegionRequest);
    refused!(split_region, SplitRegionRequest);
    refused!(cluster_info, ClusterInfoRequest);
    refused!(admit_node, AdmitNodeRequest);
    refused!(promote_node, PromoteNodeRequest);

    let auth = service
        .cluster_info(Request::new(proto::ClusterInfoRequest {}))
        .await
        .unwrap_err();
    assert_eq!(auth.code(), Code::Unauthenticated);
    assert_eq!(
        service
            .admission()
            .snapshot()
            .classes
            .iter()
            .map(|c| c.admitted)
            .sum::<u64>(),
        1
    );
    assert_eq!(
        service
            .admission()
            .snapshot()
            .classes
            .iter()
            .map(|c| c.refused_count)
            .sum::<u64>(),
        25
    );
    drop(held);
    assert!(service
        .cluster_info(authenticated(proto::ClusterInfoRequest {}))
        .await
        .is_ok());
}

#[tokio::test]
async fn admission_releases_validation_error_and_backend_error() {
    let service = bounded(1, 4096);
    let invalid = service
        .raw_get(authenticated(proto::RawGetRequest::default()))
        .await
        .unwrap_err();
    assert_eq!(invalid.code(), Code::InvalidArgument);
    assert_eq!(
        service.admission().snapshot().classes[0].released_before_execution,
        1
    );
    let failure = service
        .backend
        .call(
            service.admission().reserve(WorkClass::RawWrite, 3).unwrap(),
            |_| Err::<(), _>(Error::NotImplemented("controlled error")),
        )
        .await
        .unwrap_err();
    assert_eq!(failure.code(), Code::Unimplemented);
    let state = service.admission().snapshot();
    assert_eq!((state.in_flight, state.encoded_bytes), (0, 0));
    assert_eq!(
        (state.classes[1].completed, state.classes[1].backend_errors),
        (1, 1)
    );
    let metrics = service.admission().latency_snapshots();
    assert_eq!(
        metrics[0].latency.outcomes[Outcome::Released as usize].count,
        1
    );
    assert!(metrics[1].latency.outcomes.iter().all(|h| h.count == 0));
    assert_eq!(
        metrics[3].latency.outcomes[Outcome::Error as usize].count,
        1
    );
}

#[tokio::test]
async fn admission_releases_budget_on_backend_panic() {
    let service = bounded(1, 4096);
    let failure = service
        .backend
        .call(
            service.admission().reserve(WorkClass::RawWrite, 3).unwrap(),
            |_| -> kv9_common::Result<()> { panic!("controlled backend unwind") },
        )
        .await
        .unwrap_err();
    assert_eq!(failure.code(), Code::Internal);
    let state = service.admission().snapshot();
    assert_eq!(
        (state.in_flight, state.running, state.encoded_bytes),
        (0, 0, 0)
    );
    assert_eq!(state.classes[1].backend_aborted, 1);
    let metrics = service.admission().latency_snapshots();
    assert_eq!(
        metrics[3].latency.outcomes[Outcome::Aborted as usize].count,
        1
    );
    assert_eq!(
        metrics[3].latency.outcomes[Outcome::Success as usize].count,
        0
    );

    assert!(service
        .admission()
        .reserve(WorkClass::RawRead, 4096)
        .is_ok());
}

#[test]
fn admission_cancelled_running_and_queued_jobs_retain_their_reservations() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let service = bounded(2, 10);
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let first = service.clone();
        let held = first.admission().reserve(WorkClass::RawWrite, 6).unwrap();
        let running = tokio::spawn(async move {
            first
                .backend
                .call(held, move |_| {
                    entered_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                    Ok(())
                })
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), entered_rx)
            .await
            .unwrap()
            .unwrap();
        let (queued_tx, queued_rx) = tokio::sync::oneshot::channel();
        let second = service.clone();
        let held = second.admission().reserve(WorkClass::RawRead, 4).unwrap();
        let mut queued = Box::pin(second.backend.call(held, move |_| {
            queued_tx.send(()).unwrap();
            Ok(())
        }));
        // Poll exactly once: the job is submitted and waits on the only,
        // currently occupied blocking worker. Then drop the RPC future.
        std::future::poll_fn(|cx| {
            use std::future::Future;
            assert!(queued.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        running.abort();
        drop(queued);
        assert!(running.await.unwrap_err().is_cancelled());
        let state = service.admission().snapshot();
        let refused = service.admission().reserve(WorkClass::RawRead, 0);
        let before_release = service.admission().latency_snapshots();
        release_tx.send(()).unwrap(); // Release even when a source control is wrong.
        assert_eq!(
            (
                state.in_flight,
                state.queued,
                state.running,
                state.encoded_bytes
            ),
            (2, 1, 1, 10),
            "cancelled RPC released live backend capacity"
        );
        assert!(matches!(refused, Err(Refusal::RequestCount)));
        assert!(before_release[1]
            .latency
            .outcomes
            .iter()
            .all(|h| h.count == 0));
        assert!(
            before_release[3]
                .latency
                .outcomes
                .iter()
                .all(|h| h.count == 0),
            "RPC cancellation must not finish live backend timing"
        );
        assert_eq!(
            before_release[2].latency.outcomes[Outcome::Success as usize].count,
            1
        );
        assert_eq!(
            before_release[0].latency.outcomes[Outcome::Success as usize].count,
            0
        );

        tokio::time::timeout(Duration::from_secs(5), queued_rx)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while service.admission().snapshot().in_flight != 0
                || service.admission().latency_snapshots()[1].latency.outcomes
                    [Outcome::Success as usize]
                    .count
                    != 1
                || service.admission().latency_snapshots()[3].latency.outcomes
                    [Outcome::Success as usize]
                    .count
                    != 1
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            service
                .admission()
                .snapshot()
                .classes
                .iter()
                .map(|c| c.completed)
                .sum::<u64>(),
            2
        );
    });
}

#[test]
fn admission_marker_is_exclusive_and_fail_closed() {
    for reason in [
        Refusal::RequestCount,
        Refusal::EncodedBytes,
        Refusal::RequestTooLarge,
    ] {
        assert_eq!(
            admission_refusal(&admission_status(reason)),
            Some(reason.label())
        );
    }
    assert_eq!(
        admission_refusal(&Status::resource_exhausted("no marker")),
        None
    );
    for code in [Code::Unavailable, Code::FailedPrecondition] {
        let mut status = Status::new(code, "wrong code");
        status
            .metadata_mut()
            .insert(ADMISSION_REFUSED_KEY, "request_count".parse().unwrap());
        assert_eq!(admission_refusal(&status), None);
    }
    for key in [
        NOT_LEADER_KEY,
        LEADER_HINT_KEY,
        READ_UNCONFIRMED_KEY,
        PARTIAL_WRITE_KEY,
        COMMITTED_CHUNKS_KEY,
        LAST_APPLIED_TERM_KEY,
        LAST_APPLIED_INDEX_KEY,
    ] {
        let mut status = admission_status(Refusal::RequestCount);
        status.metadata_mut().insert(key, "1".parse().unwrap());
        assert_eq!(admission_refusal(&status), None, "mixed marker: {key}");
    }
    let mut duplicate = admission_status(Refusal::RequestCount);
    duplicate
        .metadata_mut()
        .append(ADMISSION_REFUSED_KEY, "request_count".parse().unwrap());
    assert_eq!(admission_refusal(&duplicate), None);
    let mut unknown = admission_status(Refusal::RequestCount);
    unknown
        .metadata_mut()
        .insert(ADMISSION_REFUSED_KEY, "future".parse().unwrap());
    assert_eq!(admission_refusal(&unknown), None);
}

async fn admission_wire_case(max_requests: usize, max_encoded_bytes: usize, reason: &str) {
    use kv9_raft::grpc::{self as raft, pb, GrpcDiscoveryState, RaftGrpcService};
    struct Discovery;
    impl GrpcDiscoveryState for Discovery {
        fn answer(&self) -> (NodeId, bool, u64) {
            (NodeId(1), false, 19)
        }
    }
    fn wire<T>(message: T) -> Request<T> {
        let mut request = Request::new(message);
        request
            .metadata_mut()
            .insert("authorization", "Bearer wire-secret".parse().unwrap());
        request
    }
    let (entered_tx, mut entered_rx) = tokio::sync::mpsc::unbounded_channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let backend = Arc::new(FakeBackend {
        callers: Mutex::new(Vec::new()),
        raw_gate: Some((entered_tx, Mutex::new(release_rx))),
    });
    let api = Kv9Grpc::with_limits(
        backend.clone(),
        PublicApiLimits {
            max_requests,
            max_encoded_bytes,
        },
    )
    .unwrap();
    let admission = api.admission();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let (inbox, _receiver) = tokio::sync::mpsc::unbounded_channel();
    let server = tonic::transport::Server::builder()
        .add_service(api.authenticated_service(Arc::new(
            TokenAuthenticator::new([("wire-secret", "wire-client")]).unwrap(),
        )))
        .add_service(pb::kv9_raft_server::Kv9RaftServer::with_interceptor(
            RaftGrpcService::new(NodeId(1), inbox, Arc::new(Discovery)),
            raft::cluster_token_interceptor("cluster-secret".into()),
        ))
        .serve_with_incoming_shutdown(
            tokio_stream::wrappers::TcpListenerStream::new(listener),
            async {
                let _ = shutdown_rx.await;
            },
        );
    let server_task = tokio::spawn(server);
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{address}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut client = proto::kv9_client::Kv9Client::new(channel.clone());
    let message = proto::RawGetRequest {
        context: Some(request_context_message()),
        key: vec![7; 32],
    };
    let mut first_client = client.clone();
    let first_request = message.clone();
    let pending = tokio::spawn(async move { first_client.raw_get(wire(first_request)).await });
    tokio::time::timeout(Duration::from_secs(5), entered_rx.recv())
        .await
        .unwrap()
        .unwrap();
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    let refusal = tokio::time::timeout(
        Duration::from_secs(5),
        client.raw_get(wire(message.clone())),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert_eq!(admission_refusal(&refusal), Some(reason));
    let unauthorized = client.raw_get(message).await.unwrap_err();
    assert_eq!(unauthorized.code(), Code::Unauthenticated);
    // The same listener and single async worker must still serve authenticated
    // Raft discovery while a public backend is held and its RPC is cancelled.
    let mut peer = pb::kv9_raft_client::Kv9RaftClient::new(channel);
    let mut discover = Request::new(pb::DiscoverRequest {
        from_node: 2,
        bootstrap_generation: vec![0; 16],
        root_digest: vec![0; 32],
        ..Default::default()
    });
    discover
        .metadata_mut()
        .insert(raft::CLUSTER_TOKEN_KEY, "cluster-secret".parse().unwrap());
    discover
        .metadata_mut()
        .insert(raft::NODE_ID_KEY, "2".parse().unwrap());
    let reply = tokio::time::timeout(Duration::from_secs(5), peer.discover(discover))
        .await
        .unwrap()
        .unwrap()
        .into_inner();
    assert!(!reply.initialized);
    assert_eq!(
        admission.snapshot().in_flight,
        1,
        "wire cancellation released live backend capacity"
    );
    assert_eq!(backend.callers.lock().unwrap().as_slice(), ["wire-client"]);
    release_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while admission.snapshot().in_flight != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(client
        .cluster_info(wire(proto::ClusterInfoRequest {}))
        .await
        .is_ok());
    assert_eq!(admission.snapshot().classes[0].completed, 1);
    shutdown_tx.send(()).unwrap();
    server_task.await.unwrap().unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn admission_real_wire_count_refusal_keeps_raft_discovery_live() {
    admission_wire_case(1, 4096, "request_count").await;
}

#[tokio::test(flavor = "current_thread")]
async fn admission_real_wire_aggregate_bytes_are_charged_by_encoded_length() {
    use prost::Message;
    let request = proto::RawGetRequest {
        context: Some(request_context_message()),
        key: vec![7; 32],
    };
    admission_wire_case(2, request.encoded_len(), "encoded_bytes").await;
}
