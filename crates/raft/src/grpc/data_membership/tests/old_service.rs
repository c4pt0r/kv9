//! Frozen V11 dispatcher without membership method, not a previous binary.
use super::*;
use std::sync::atomic::AtomicUsize;

mod old_v11 {
    include!(concat!(env!("OUT_DIR"), "/pre_membership/kv9.raft.v11.rs"));
}
struct Old {
    identity: StoreIdentity,
    calls: Arc<AtomicUsize>,
}
#[tonic::async_trait]
impl old_v11::kv9_raft_server::Kv9Raft for Old {
    async fn confirm_raw_ttl_format(
        &self,
        _: Request<old_v11::RawTtlFormatRequest>,
    ) -> std::result::Result<Response<old_v11::RawTtlFormatReceipt>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let id = self.identity;
        Ok(Response::new(old_v11::RawTtlFormatReceipt {
            root_digest: id.root_digest.as_bytes().to_vec(),
            cluster_id: id.cluster_id.as_bytes().to_vec(),
            node_id: id.node_id.0,
            store_incarnation: id.store_incarnation.as_bytes().to_vec(),
            format_version: 11,
        }))
    }
    async fn batch_raft(
        &self,
        _: Request<tonic::Streaming<old_v11::BatchRaftMessage>>,
    ) -> std::result::Result<Response<old_v11::Done>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied(
            "frozen old V11 test backend reached",
        ))
    }
    async fn batch_data_raft(
        &self,
        _: Request<tonic::Streaming<old_v11::BatchRaftMessage>>,
    ) -> std::result::Result<Response<old_v11::Done>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied(
            "frozen old V11 test backend reached",
        ))
    }
    async fn discover(
        &self,
        _: Request<old_v11::DiscoverRequest>,
    ) -> std::result::Result<Response<old_v11::DiscoverResponse>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied(
            "frozen old V11 test backend reached",
        ))
    }
    async fn register(
        &self,
        _: Request<old_v11::RegisterRequest>,
    ) -> std::result::Result<Response<old_v11::RegisterReceipt>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied(
            "frozen old V11 test backend reached",
        ))
    }
    async fn confirm_endpoint(
        &self,
        _: Request<old_v11::ConfirmEndpointRequest>,
    ) -> std::result::Result<Response<old_v11::EndpointConfirmationReceipt>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied(
            "frozen old V11 test backend reached",
        ))
    }
    async fn resolve_raw_ttl_stores(
        &self,
        _: Request<old_v11::RawTtlStoresRequest>,
    ) -> std::result::Result<Response<old_v11::RawTtlStoresResponse>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied(
            "frozen old V11 test backend reached",
        ))
    }
    async fn checkpoint_retention(
        &self,
        _: Request<old_v11::CheckpointRetentionRequest>,
    ) -> std::result::Result<Response<old_v11::CheckpointRetentionResponse>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied(
            "frozen old V11 test backend reached",
        ))
    }
    async fn inspect_log_pressure(
        &self,
        _: Request<old_v11::LogPressureRequest>,
    ) -> std::result::Result<Response<old_v11::LogPressureResponse>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied(
            "frozen old V11 test backend reached",
        ))
    }
    async fn inspect_checkpoint_history(
        &self,
        _: Request<old_v11::CheckpointHistoryRequest>,
    ) -> std::result::Result<Response<old_v11::CheckpointHistoryResponse>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied(
            "frozen old V11 test backend reached",
        ))
    }
    async fn inspect_checkpoint_selection(
        &self,
        _: Request<old_v11::CheckpointHistoryRequest>,
    ) -> std::result::Result<Response<old_v11::CheckpointHistoryResponse>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied(
            "frozen old V11 test backend reached",
        ))
    }
    async fn inspect_migration_capture(
        &self,
        _: Request<old_v11::MigrationCaptureRequest>,
    ) -> std::result::Result<Response<old_v11::MigrationCaptureResponse>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied(
            "frozen old V11 test backend reached",
        ))
    }
    async fn inspect_bootstrap_read_fence(
        &self,
        _: Request<old_v11::CheckpointHistoryRequest>,
    ) -> std::result::Result<Response<old_v11::CheckpointHistoryResponse>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied(
            "frozen old V11 test backend reached",
        ))
    }
    async fn inspect_data_read_fence(
        &self,
        _: Request<old_v11::DataReadFenceRequest>,
    ) -> std::result::Result<Response<old_v11::DataReadFenceResponse>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied(
            "frozen old V11 test backend reached",
        ))
    }
    type FetchMigrationCaptureStream = std::pin::Pin<
        Box<
            dyn tokio_stream::Stream<
                    Item = std::result::Result<old_v11::MigrationCaptureChunk, Status>,
                > + Send,
        >,
    >;
    async fn fetch_migration_capture(
        &self,
        _: Request<old_v11::MigrationCaptureTransferRequest>,
    ) -> std::result::Result<Response<Self::FetchMigrationCaptureStream>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied(
            "frozen old V11 test backend reached",
        ))
    }
    async fn replicate_migration_capture(
        &self,
        _: Request<old_v11::MigrationCaptureTransferRequest>,
    ) -> std::result::Result<Response<old_v11::MigrationCaptureResponse>, Status> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(Status::permission_denied(
            "frozen old V11 test backend reached",
        ))
    }
}

fn authenticated<T>(value: T) -> Request<T> {
    let mut request = Request::new(value);
    attach_auth(&mut request, &Some("upgrade-test".into()), NodeId(1));
    request
}
pub(crate) async fn connect(
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

pub(crate) async fn serve_old_origin_fixture(
    identity: StoreIdentity,
    calls: Arc<AtomicUsize>,
) -> (tonic::transport::Channel, tokio::task::JoinHandle<()>) {
    connect(tonic::transport::Server::builder().add_service(
        old_v11::kv9_raft_server::Kv9RaftServer::with_interceptor(
            Old { identity, calls },
            cluster_token_interceptor("upgrade-test".into()),
        ),
    ))
    .await
}

#[tokio::test]
async fn pre_membership_v11_service_refuses_new_method_without_fallback() {
    let fixture = Fixture::new();
    let identity = fixture.ticket.identity();
    let calls = Arc::new(AtomicUsize::new(0));
    let (old_channel, old_server) = connect(tonic::transport::Server::builder().add_service(
        old_v11::kv9_raft_server::Kv9RaftServer::with_interceptor(
            Old {
                identity,
                calls: calls.clone(),
            },
            cluster_token_interceptor("upgrade-test".into()),
        ),
    ))
    .await;
    // Same V11 format confirmation works. Missing observation must stay unknown.
    assert_eq!(
        crate::grpc::ttl_format::confirm_on_channel(
            old_channel.clone(),
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
    let mut client = Kv9RaftClient::new(old_channel.clone());
    assert_eq!(
        client
            .inspect_data_membership(authenticated(
                fixture.ticket.request(Duration::from_secs(1))
            ))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unimplemented
    );
    assert!(inspect_on_channel(
        old_channel,
        &fixture.ticket,
        NodeId(1),
        Some("upgrade-test".into()),
        Duration::from_secs(1)
    )
    .await
    .is_err());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        before,
        "membership query fell back to another method"
    );
    let (channel, server) = connect(tonic::transport::Server::builder().add_service(
        pb::kv9_raft_server::Kv9RaftServer::with_interceptor(
            fixture.service(),
            cluster_token_interceptor("upgrade-test".into()),
        ),
    ))
    .await;
    assert!(inspect_on_channel(
        channel.clone(),
        &fixture.ticket,
        NodeId(1),
        Some("wrong".into()),
        Duration::from_secs(1)
    )
    .await
    .is_err());
    assert_eq!(fixture.backend.calls.load(Ordering::SeqCst), 0);
    let first = inspect_on_channel(
        channel.clone(),
        &fixture.ticket,
        NodeId(1),
        Some("upgrade-test".into()),
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    let next = first.next_ticket().unwrap().unwrap();
    let last = inspect_on_channel(
        channel,
        &next,
        NodeId(1),
        Some("upgrade-test".into()),
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    assert_eq!(last.page().unwrap().members[0].node, NodeId(5));
    assert!(last.next_ticket().unwrap().is_none());
    for job in [old_server, server] {
        job.abort();
        let _ = job.await;
    }
}
