//! Actual frozen V7 generated dispatcher/client against V9 over HTTP/2 duplex.
//! The V7 backend is a counting fixture, not a complete prior server binary.
use super::*;
use std::sync::atomic::AtomicUsize;

mod v7 {
    tonic::include_proto!("kv9.raft.v7");
}
struct Old {
    identity: StoreIdentity,
    calls: Arc<AtomicUsize>,
}
#[tonic::async_trait]
impl v7::kv9_raft_server::Kv9Raft for Old {
    async fn confirm_raw_ttl_format(
        &self,
        _: Request<v7::RawTtlFormatRequest>,
    ) -> std::result::Result<Response<v7::RawTtlFormatReceipt>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let id = self.identity;
        Ok(Response::new(v7::RawTtlFormatReceipt {
            root_digest: id.root_digest.as_bytes().to_vec(),
            cluster_id: id.cluster_id.as_bytes().to_vec(),
            node_id: id.node_id.0,
            store_incarnation: id.store_incarnation.as_bytes().to_vec(),
            format_version: 7,
        }))
    }
    async fn batch_raft(
        &self,
        _: Request<tonic::Streaming<v7::BatchRaftMessage>>,
    ) -> std::result::Result<Response<v7::Done>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied("frozen V7 test backend reached"))
    }
    async fn batch_data_raft(
        &self,
        _: Request<tonic::Streaming<v7::BatchRaftMessage>>,
    ) -> std::result::Result<Response<v7::Done>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied("frozen V7 test backend reached"))
    }
    async fn discover(
        &self,
        _: Request<v7::DiscoverRequest>,
    ) -> std::result::Result<Response<v7::DiscoverResponse>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied("frozen V7 test backend reached"))
    }
    async fn register(
        &self,
        _: Request<v7::RegisterRequest>,
    ) -> std::result::Result<Response<v7::RegisterReceipt>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied("frozen V7 test backend reached"))
    }
    async fn confirm_endpoint(
        &self,
        _: Request<v7::ConfirmEndpointRequest>,
    ) -> std::result::Result<Response<v7::EndpointConfirmationReceipt>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied("frozen V7 test backend reached"))
    }
    async fn resolve_raw_ttl_stores(
        &self,
        _: Request<v7::RawTtlStoresRequest>,
    ) -> std::result::Result<Response<v7::RawTtlStoresResponse>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied("frozen V7 test backend reached"))
    }
    async fn checkpoint_retention(
        &self,
        _: Request<v7::CheckpointRetentionRequest>,
    ) -> std::result::Result<Response<v7::CheckpointRetentionResponse>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied("frozen V7 test backend reached"))
    }
    async fn inspect_log_pressure(
        &self,
        _: Request<v7::LogPressureRequest>,
    ) -> std::result::Result<Response<v7::LogPressureResponse>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied("frozen V7 test backend reached"))
    }
    async fn inspect_checkpoint_history(
        &self,
        _: Request<v7::CheckpointHistoryRequest>,
    ) -> std::result::Result<Response<v7::CheckpointHistoryResponse>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied("frozen V7 test backend reached"))
    }
    async fn inspect_checkpoint_selection(
        &self,
        _: Request<v7::CheckpointHistoryRequest>,
    ) -> std::result::Result<Response<v7::CheckpointHistoryResponse>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied("frozen V7 test backend reached"))
    }
    async fn inspect_migration_capture(
        &self,
        _: Request<v7::MigrationCaptureRequest>,
    ) -> std::result::Result<Response<v7::MigrationCaptureResponse>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied("frozen V7 test backend reached"))
    }
}

fn authenticated<T>(value: T) -> Request<T> {
    let mut request = Request::new(value);
    attach_auth(&mut request, &Some("upgrade-test".into()), NodeId(1));
    request
}
pub(super) async fn connect(
    router: tonic::transport::server::Router,
) -> (tonic::transport::Channel, tokio::task::JoinHandle<()>) {
    let (incoming, receive) = tokio::sync::mpsc::channel(4);
    let server = tokio::spawn(async move {
        router
            .serve_with_incoming(tokio_stream::wrappers::ReceiverStream::new(receive))
            .await
            .unwrap();
    });
    let channel = tonic::transport::Endpoint::from_static("http://upgrade.test")
        .connect_with_connector(tower::service_fn(move |_| {
            let incoming = incoming.clone();
            async move {
                let (client, server) = tokio::io::duplex(65536);
                incoming
                    .send(Ok::<_, std::io::Error>(server))
                    .await
                    .map_err(|_| std::io::Error::from(std::io::ErrorKind::BrokenPipe))?;
                Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(client))
            }
        }))
        .await
        .unwrap();
    (channel, server)
}

#[tokio::test]
async fn v7_and_v9_http2_clients_refuse_cross_generation_without_fallback() {
    let fixture = Fixture::new();
    let identity = fixture.fence.identity();
    let calls = Arc::new(AtomicUsize::new(0));
    let (old_channel, old_server) = connect(tonic::transport::Server::builder().add_service(
        v7::kv9_raft_server::Kv9RaftServer::with_interceptor(
            Old {
                identity,
                calls: calls.clone(),
            },
            cluster_token_interceptor("upgrade-test".into()),
        ),
    ))
    .await;
    let inbox = crate::work::RaftInbox::default();
    let (new_channel, new_server) = connect(tonic::transport::Server::builder().add_service(
        pb::kv9_raft_server::Kv9RaftServer::with_interceptor(
            RaftGrpcService::new(NodeId(2), inbox.clone(), fixture.discovery(true, true)),
            cluster_token_interceptor("upgrade-test".into()),
        ),
    ))
    .await;
    // Both intended paths really work; failures below cannot be attributed to
    // a dead server, invalid credentials or disconnected channel.
    let old_request = || v7::RawTtlFormatRequest {
        root_digest: identity.root_digest.as_bytes().to_vec(),
        cluster_id: identity.cluster_id.as_bytes().to_vec(),
        node_id: identity.node_id.0,
        store_incarnation: identity.store_incarnation.as_bytes().to_vec(),
    };
    let mut old_client = v7::kv9_raft_client::Kv9RaftClient::new(old_channel.clone());
    assert_eq!(
        old_client
            .confirm_raw_ttl_format(authenticated(old_request()))
            .await
            .unwrap()
            .into_inner()
            .format_version,
        7
    );
    assert_eq!(
        confirm_on_channel(
            new_channel.clone(),
            identity,
            NodeId(1),
            Some("upgrade-test".into()),
            Duration::from_secs(1)
        )
        .await
        .unwrap()
        .identity(),
        identity
    );
    let before = calls.load(Ordering::SeqCst);
    assert!(confirm_on_channel(
        old_channel.clone(),
        identity,
        NodeId(1),
        Some("upgrade-test".into()),
        Duration::from_secs(1)
    )
    .await
    .is_err());
    let query = crate::grpc::checkpoint_retention::LedgerQuery::CapturePlan(vec![1]);
    assert!(crate::grpc::checkpoint_retention::on_channel(
        old_channel.clone(),
        identity.root_digest,
        &query,
        NodeId(1),
        Some("upgrade-test".into()),
        Duration::from_secs(1)
    )
    .await
    .is_err());
    let mut modern = pb::kv9_raft_client::Kv9RaftClient::new(old_channel);
    assert_eq!(
        modern
            .register(authenticated(pb::RegisterRequest::default()))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unimplemented
    );
    assert_eq!(
        modern
            .batch_raft(authenticated(tokio_stream::iter(vec![
                pb::BatchRaftMessage::default()
            ])))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unimplemented
    );
    assert_eq!(
        modern
            .batch_data_raft(authenticated(tokio_stream::iter(vec![
                pb::BatchRaftMessage::default()
            ])))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unimplemented
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        before,
        "V9 request entered V7 backend or fell back to a V7 method"
    );
    let mut legacy = v7::kv9_raft_client::Kv9RaftClient::new(new_channel);
    assert_eq!(
        legacy
            .confirm_raw_ttl_format(authenticated(old_request()))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unimplemented
    );
    assert_eq!(
        legacy
            .register(authenticated(v7::RegisterRequest::default()))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unimplemented
    );
    assert_eq!(
        legacy
            .checkpoint_retention(authenticated(v7::CheckpointRetentionRequest::default()))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unimplemented
    );
    assert_eq!(
        legacy
            .batch_raft(authenticated(tokio_stream::iter(vec![
                v7::BatchRaftMessage::default()
            ])))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unimplemented
    );
    assert_eq!(
        legacy
            .batch_data_raft(authenticated(tokio_stream::iter(vec![
                v7::BatchRaftMessage::default()
            ])))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unimplemented
    );
    assert!(
        inbox.drain().is_empty(),
        "old wire traffic reached a current Raft owner"
    );
    for server in [old_server, new_server] {
        server.abort();
        let _ = server.await;
    }
}
