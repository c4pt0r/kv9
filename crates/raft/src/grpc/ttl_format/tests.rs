use super::*;
use kv9_common::store_lifecycle::{RawTtlFormatFence, StoreGuard};
use kv9_common::{RootDescriptor, RootVoter};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

struct Fixture {
    path: PathBuf,
    _guard: StoreGuard,
    root: RootDescriptor,
    fence: RawTtlFormatFence,
}
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "kv9-ttl-format-{}",
            StoreIncarnation::mint().unwrap()
        ));
        let mut guard = StoreGuard::lock(&path).unwrap();
        let record = guard.prepare(NodeId(2)).unwrap();
        let root = RootDescriptor::new(
            ClusterId::mint().unwrap(),
            BootstrapGeneration::mint().unwrap(),
            vec![RootVoter {
                node_id: record.node_id,
                addr: "127.0.0.1:20162".parse().unwrap(),
                store_incarnation: record.incarnation,
            }],
            b"ttl-format-test",
        )
        .unwrap();
        let identity = StoreIdentity::for_voter(&root, record.node_id).unwrap();
        guard.bind(&root, &identity).unwrap();
        guard.activate(&identity).unwrap();
        let fence = guard.raw_ttl_format_fence(&root, &identity).unwrap();
        Self {
            path,
            _guard: guard,
            root,
            fence,
        }
    }
    fn discovery(&self, fenced: bool, allowed: bool) -> Arc<Discovery> {
        Arc::new(Discovery {
            root: RootWireIdentity {
                root_digest: self.root.digest(),
                bootstrap_generation: self.root.bootstrap_generation,
            },
            fence: fenced.then_some(self.fence.clone()),
            allowed: AtomicBool::new(allowed),
        })
    }
    fn request(&self) -> Request<pb::RawTtlFormatRequest> {
        let id = self.fence.identity();
        let mut request = Request::new(pb::RawTtlFormatRequest {
            root_digest: id.root_digest.as_bytes().to_vec(),
            cluster_id: id.cluster_id.as_bytes().to_vec(),
            node_id: id.node_id.0,
            store_incarnation: id.store_incarnation.as_bytes().to_vec(),
        });
        request.extensions_mut().insert(NodeId(1));
        request
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
struct Discovery {
    root: RootWireIdentity,
    fence: Option<RawTtlFormatFence>,
    allowed: AtomicBool,
}
impl GrpcDiscoveryState for Discovery {
    fn answer(&self) -> (NodeId, bool, u64) {
        (NodeId(2), false, 0)
    }
    fn raft_receive_allowed(&self) -> bool {
        self.allowed.load(Ordering::SeqCst)
    }
    fn root_identity(&self) -> RootWireIdentity {
        self.root
    }
    fn raw_ttl_format_fence(&self) -> Option<RawTtlFormatFence> {
        self.fence.clone()
    }
}
fn service(discovery: Arc<Discovery>) -> RaftGrpcService {
    RaftGrpcService::new(NodeId(2), crate::work::RaftInbox::default(), discovery)
}

#[test]
fn ttl_format_confirmation_requires_durable_fence_and_live_local_authority() {
    let fixture = Fixture::new();
    for (fenced, allowed) in [(false, false), (true, false), (false, true)] {
        let result = service(fixture.discovery(fenced, allowed))
            .confirm_raw_ttl_format_request(fixture.request());
        assert_eq!(result.unwrap_err().code(), tonic::Code::FailedPrecondition);
    }
    let discovery = fixture.discovery(true, true);
    let server = service(discovery.clone());
    let receipt = server
        .confirm_raw_ttl_format_request(fixture.request())
        .unwrap()
        .into_inner();
    assert_eq!(
        ConfirmedRawTtlFormat::from_receipt(fixture.fence.identity(), receipt)
            .unwrap()
            .identity(),
        fixture.fence.identity()
    );
    discovery.allowed.store(false, Ordering::SeqCst);
    assert_eq!(
        server
            .confirm_raw_ttl_format_request(fixture.request())
            .unwrap_err()
            .code(),
        tonic::Code::FailedPrecondition
    );
}

#[test]
fn ttl_format_requests_and_replies_cannot_substitute_another_root_node_or_disk() {
    let fixture = Fixture::new();
    let server = service(fixture.discovery(true, true));
    let mut missing_auth = fixture.request();
    missing_auth.extensions_mut().remove::<NodeId>();
    assert_eq!(
        server
            .confirm_raw_ttl_format_request(missing_auth)
            .unwrap_err()
            .code(),
        tonic::Code::Unauthenticated
    );
    for field in 0..4 {
        let mut request = fixture.request();
        let value = request.get_mut();
        match field {
            0 => value.node_id += 1,
            1 => value.root_digest[0] ^= 1,
            2 => value.cluster_id.push(0),
            _ => value.store_incarnation[0] ^= 1,
        }
        assert_eq!(
            server
                .confirm_raw_ttl_format_request(request)
                .unwrap_err()
                .code(),
            tonic::Code::FailedPrecondition
        );
    }
    let receipt = server
        .confirm_raw_ttl_format_request(fixture.request())
        .unwrap()
        .into_inner();
    for field in 0..12 {
        let mut bad = receipt.clone();
        match field {
            0 => bad.node_id += 1,
            1 => bad.root_digest.push(0),
            2 => bad.cluster_id.clear(),
            3 => bad.store_incarnation[0] ^= 1,
            4 => bad.format_version = 0,
            5 => bad.format_version = 3,
            6 => bad.format_version = 4,
            7 => bad.format_version = 5,
            8 => bad.format_version = 6,
            9 => bad.format_version = 7,
            10 => bad.format_version = 8,
            _ => bad.format_version = RAW_TTL_FORMAT_VERSION + 1,
        }
        assert!(ConfirmedRawTtlFormat::from_receipt(fixture.fence.identity(), bad).is_err());
    }
}

#[tokio::test]
async fn ttl_v11_dispatcher_has_no_v1_through_v10_paths_even_with_valid_authentication() {
    use pb::kv9_raft_server::Kv9RaftServer;
    use tonic::codegen::{http, Service};
    use tonic::server::NamedService;
    let fixture = Fixture::new();
    assert_eq!(
        Kv9RaftServer::<RaftGrpcService>::NAME,
        "kv9.raft.v11.Kv9Raft"
    );
    let mut dispatcher = Kv9RaftServer::with_interceptor(
        service(fixture.discovery(true, true)),
        cluster_token_interceptor("test-token".into()),
    );
    for generation in [
        "kv9.raft",
        "kv9.raft.v2",
        "kv9.raft.v3",
        "kv9.raft.v4",
        "kv9.raft.v5",
        "kv9.raft.v6",
        "kv9.raft.v7",
        "kv9.raft.v8",
        "kv9.raft.v9",
        "kv9.raft.v10",
    ] {
        for method in [
            "Discover",
            "Register",
            "BatchRaft",
            "BatchDataRaft",
            "ConfirmEndpoint",
            "ConfirmRawTtlFormat",
            "ResolveRawTtlStores",
            "CheckpointRetention",
            "InspectLogPressure",
            "InspectCheckpointHistory",
            "InspectCheckpointSelection",
            "InspectBootstrapReadFence",
            "InspectDataReadFence",
            "InspectMigrationCapture",
            "FetchMigrationCapture",
            "ReplicateMigrationCapture",
        ] {
            let request = http::Request::builder()
                .uri(format!("/{generation}.Kv9Raft/{method}"))
                .header(CLUSTER_TOKEN_KEY, "test-token")
                .header(NODE_ID_KEY, "1")
                .body(tonic::body::Body::empty())
                .unwrap();
            let response = dispatcher.call(request).await.unwrap();
            assert_eq!(
                response.headers().get("grpc-status").unwrap(),
                "12",
                "legacy path {generation}/{method} is exposed"
            );
        }
    }
    // The current method exists and enters the real authentication boundary.
    let response = dispatcher
        .call(
            http::Request::builder()
                .uri("/kv9.raft.v11.Kv9Raft/ConfirmRawTtlFormat")
                .body(tonic::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.headers().get("grpc-status").unwrap(), "16");
}

mod data_reads_upgrade;
mod payload;
mod readers_upgrade;
mod store_history_upgrade;
mod upgrade;

#[tokio::test]
async fn ttl_format_client_rejects_invalid_scope_or_budget_before_connecting() {
    let fixture = Fixture::new();
    let good = fixture.fence.identity();
    for field in 0..5 {
        let mut identity = good;
        match field {
            0 => identity.node_id = NodeId(0),
            1 => identity.root_digest = RootDigest::from_bytes([0; 32]),
            2 => identity.cluster_id = ClusterId::from_bytes([0; 16]),
            3 => identity.store_incarnation = StoreIncarnation::from_bytes([0; 16]),
            _ => {}
        }
        let budget = if field == 4 {
            Duration::ZERO
        } else {
            Duration::from_secs(1)
        };
        assert!(matches!(
            grpc_confirm_raw_ttl_format(
                "127.0.0.1:20162".parse().unwrap(),
                identity,
                NodeId(1),
                None,
                budget
            )
            .await,
            Err(Error::Config(_))
        ));
    }
}
