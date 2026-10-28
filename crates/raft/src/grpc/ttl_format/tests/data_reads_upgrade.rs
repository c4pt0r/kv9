//! Actual frozen V9 generated dispatcher/client against V11 over HTTP/2 duplex.
//! The V9 backend is a counting fixture, not a complete prior server binary.
use super::*;
use std::sync::atomic::AtomicUsize;

mod v9 {
    tonic::include_proto!("kv9.raft.v9");
}
struct Old {
    identity: StoreIdentity,
    calls: Arc<AtomicUsize>,
}
#[tonic::async_trait]
impl v9::kv9_raft_server::Kv9Raft for Old {
    async fn confirm_raw_ttl_format(
        &self,
        _: Request<v9::RawTtlFormatRequest>,
    ) -> std::result::Result<Response<v9::RawTtlFormatReceipt>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let id = self.identity;
        Ok(Response::new(v9::RawTtlFormatReceipt {
            root_digest: id.root_digest.as_bytes().to_vec(),
            cluster_id: id.cluster_id.as_bytes().to_vec(),
            node_id: id.node_id.0,
            store_incarnation: id.store_incarnation.as_bytes().to_vec(),
            format_version: 9,
        }))
    }
    async fn batch_raft(
        &self,
        _: Request<tonic::Streaming<v9::BatchRaftMessage>>,
    ) -> std::result::Result<Response<v9::Done>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied("frozen V9 test backend reached"))
    }
    async fn batch_data_raft(
        &self,
        _: Request<tonic::Streaming<v9::BatchRaftMessage>>,
    ) -> std::result::Result<Response<v9::Done>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied("frozen V9 test backend reached"))
    }
    async fn discover(
        &self,
        _: Request<v9::DiscoverRequest>,
    ) -> std::result::Result<Response<v9::DiscoverResponse>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied("frozen V9 test backend reached"))
    }
    async fn register(
        &self,
        _: Request<v9::RegisterRequest>,
    ) -> std::result::Result<Response<v9::RegisterReceipt>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied("frozen V9 test backend reached"))
    }
    async fn confirm_endpoint(
        &self,
        _: Request<v9::ConfirmEndpointRequest>,
    ) -> std::result::Result<Response<v9::EndpointConfirmationReceipt>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied("frozen V9 test backend reached"))
    }
    async fn resolve_raw_ttl_stores(
        &self,
        _: Request<v9::RawTtlStoresRequest>,
    ) -> std::result::Result<Response<v9::RawTtlStoresResponse>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied("frozen V9 test backend reached"))
    }
    async fn checkpoint_retention(
        &self,
        _: Request<v9::CheckpointRetentionRequest>,
    ) -> std::result::Result<Response<v9::CheckpointRetentionResponse>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied("frozen V9 test backend reached"))
    }
    async fn inspect_log_pressure(
        &self,
        _: Request<v9::LogPressureRequest>,
    ) -> std::result::Result<Response<v9::LogPressureResponse>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied("frozen V9 test backend reached"))
    }
    async fn inspect_checkpoint_history(
        &self,
        _: Request<v9::CheckpointHistoryRequest>,
    ) -> std::result::Result<Response<v9::CheckpointHistoryResponse>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied("frozen V9 test backend reached"))
    }
    async fn inspect_checkpoint_selection(
        &self,
        _: Request<v9::CheckpointHistoryRequest>,
    ) -> std::result::Result<Response<v9::CheckpointHistoryResponse>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied("frozen V9 test backend reached"))
    }
    async fn inspect_migration_capture(
        &self,
        _: Request<v9::MigrationCaptureRequest>,
    ) -> std::result::Result<Response<v9::MigrationCaptureResponse>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied("frozen V9 test backend reached"))
    }
    async fn inspect_bootstrap_read_fence(
        &self,
        _: Request<v9::CheckpointHistoryRequest>,
    ) -> std::result::Result<Response<v9::CheckpointHistoryResponse>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied("frozen V9 test backend reached"))
    }
    type FetchMigrationCaptureStream = std::pin::Pin<
        Box<
            dyn tokio_stream::Stream<Item = std::result::Result<v9::MigrationCaptureChunk, Status>>
                + Send,
        >,
    >;
    async fn fetch_migration_capture(
        &self,
        _: Request<v9::MigrationCaptureTransferRequest>,
    ) -> std::result::Result<Response<Self::FetchMigrationCaptureStream>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied("frozen V9 test backend reached"))
    }
    async fn replicate_migration_capture(
        &self,
        _: Request<v9::MigrationCaptureTransferRequest>,
    ) -> std::result::Result<Response<v9::MigrationCaptureResponse>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied("frozen V9 test backend reached"))
    }
}

fn authenticated<T>(value: T) -> Request<T> {
    let mut request = Request::new(value);
    attach_auth(&mut request, &Some("upgrade-test".into()), NodeId(1));
    request
}
use super::upgrade::connect;

#[tokio::test]
async fn v9_and_v11_http2_clients_refuse_cross_generation_without_fallback() {
    let fixture = Fixture::new();
    let identity = fixture.fence.identity();
    let calls = Arc::new(AtomicUsize::new(0));
    let (old_channel, old_server) = connect(tonic::transport::Server::builder().add_service(
        v9::kv9_raft_server::Kv9RaftServer::with_interceptor(
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
    let old_request = || v9::RawTtlFormatRequest {
        root_digest: identity.root_digest.as_bytes().to_vec(),
        cluster_id: identity.cluster_id.as_bytes().to_vec(),
        node_id: identity.node_id.0,
        store_incarnation: identity.store_incarnation.as_bytes().to_vec(),
    };
    let mut old_client = v9::kv9_raft_client::Kv9RaftClient::new(old_channel.clone());
    assert_eq!(
        old_client
            .confirm_raw_ttl_format(authenticated(old_request()))
            .await
            .unwrap()
            .into_inner()
            .format_version,
        9
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
    let query = crate::grpc::checkpoint_retention::LedgerQuery::Apply(vec![1]);
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
        modern
            .inspect_bootstrap_read_fence(authenticated(pb::CheckpointHistoryRequest::default()))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unimplemented
    );
    assert_eq!(
        modern
            .inspect_checkpoint_selection(authenticated(pb::CheckpointHistoryRequest::default()))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unimplemented
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        before,
        "V11 request entered V9 backend or fell back to a V9 method"
    );
    let mut legacy = v9::kv9_raft_client::Kv9RaftClient::new(new_channel);
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
            .register(authenticated(v9::RegisterRequest::default()))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unimplemented
    );
    assert_eq!(
        legacy
            .checkpoint_retention(authenticated(v9::CheckpointRetentionRequest::default()))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unimplemented
    );
    assert_eq!(
        legacy
            .batch_raft(authenticated(tokio_stream::iter(vec![
                v9::BatchRaftMessage::default()
            ])))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unimplemented
    );
    assert_eq!(
        legacy
            .batch_data_raft(authenticated(tokio_stream::iter(vec![
                v9::BatchRaftMessage::default()
            ])))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unimplemented
    );
    assert_eq!(
        legacy
            .inspect_bootstrap_read_fence(authenticated(v9::CheckpointHistoryRequest::default()))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unimplemented
    );
    assert_eq!(
        legacy
            .inspect_checkpoint_selection(authenticated(v9::CheckpointHistoryRequest::default()))
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
