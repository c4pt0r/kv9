//! Real HTTP/2 tests for the method boundary, including a legacy dispatcher
//! that does not recognize BatchDataRaft and ignores IDs on BatchRaft.

use super::*;
use crate::work::RaftInbox;
use kv9_common::RegionId;
use pb::kv9_raft_server::Kv9RaftServer;
use std::sync::atomic::AtomicBool;
use tonic::codegen::{http, Service};

const TOKEN: &str = "wire-fencing-test";

struct Discovery(Arc<AtomicBool>);
impl GrpcDiscoveryState for Discovery {
    fn answer(&self) -> (NodeId, bool, u64) {
        (NodeId(2), false, 0)
    }
    fn raft_receive_allowed(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

fn message(term: u64) -> Message {
    Message {
        from: 1,
        to: 2,
        term,
        ..Default::default()
    }
}

fn envelope(region_id: u64, term: u64) -> pb::RaftEnvelope {
    pb::RaftEnvelope {
        region_id,
        from_node: 1,
        to_node: 2,
        raft_message: message(term).write_to_bytes().unwrap(),
        epoch_conf_ver: 0,
        epoch_version: 0,
    }
}

fn batch(msgs: Vec<pb::RaftEnvelope>) -> pb::BatchRaftMessage {
    pb::BatchRaftMessage {
        msgs,
        flushed_unix_nanos: 0,
        root_digest: vec![0; 32],
    }
}

async fn call(
    address: SocketAddr,
    class: StreamClass,
    batch: pb::BatchRaftMessage,
) -> std::result::Result<Response<pb::Done>, Status> {
    let mut client = Kv9RaftClient::connect(format!("http://{address}"))
        .await
        .unwrap();
    let mut request = Request::new(tokio_stream::iter([batch]));
    attach_auth(&mut request, &Some(TOKEN.into()), NodeId(1));
    tokio::time::timeout(Duration::from_secs(3), async {
        match class {
            StreamClass::Metadata => client.batch_raft(request).await,
            StreamClass::Data => client.batch_data_raft(request).await,
        }
    })
    .await
    .expect("wire call must finish")
}

/// Removes the new method from the dispatcher when downgraded. Its unknown
/// method response is the generated tonic UNIMPLEMENTED response; it never
/// reaches a Raft handler. This emulates the old wire surface, not an old binary.
#[derive(Clone)]
struct LegacySurface<S> {
    service: S,
    modern: watch::Sender<bool>,
    rejected: Arc<AtomicU64>,
}

impl<S: tonic::server::NamedService> tonic::server::NamedService for LegacySurface<S> {
    const NAME: &'static str = S::NAME;
}

impl<S, B> Service<http::Request<B>> for LegacySurface<S>
where
    S: Service<http::Request<B>>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&mut self, mut request: http::Request<B>) -> Self::Future {
        if !*self.modern.borrow() && request.uri().path() == "/kv9.raft.v3.Kv9Raft/BatchDataRaft" {
            self.rejected.fetch_add(1, Ordering::SeqCst);
            *request.uri_mut() = "/kv9.raft.v3.Kv9Raft/UnknownLegacyMethod".parse().unwrap();
        }
        self.service.call(request)
    }
}

struct Endpoint {
    service: RaftGrpcService,
    modern: watch::Sender<bool>,
    legacy_received: Arc<Mutex<Vec<pb::RaftEnvelope>>>,
}

#[tonic::async_trait]
impl Kv9Raft for Endpoint {
    async fn batch_raft(
        &self,
        request: Request<Streaming<pb::BatchRaftMessage>>,
    ) -> std::result::Result<Response<pb::Done>, Status> {
        if *self.modern.borrow() {
            return self.service.batch_raft(request).await;
        }
        // Deliberately reproduce the unsafe old receiver: ignore region_id.
        let mut stream = request.into_inner();
        while let Some(batch) = stream.message().await? {
            self.legacy_received.lock().unwrap().extend(batch.msgs);
        }
        Ok(Response::new(pb::Done {}))
    }

    async fn batch_data_raft(
        &self,
        request: Request<Streaming<pb::BatchRaftMessage>>,
    ) -> std::result::Result<Response<pb::Done>, Status> {
        let mut version = self.modern.subscribe();
        if !*version.borrow_and_update() {
            return Err(Status::unimplemented("legacy endpoint"));
        }
        tokio::select! {
            result = self.service.batch_data_raft(request) => result,
            _ = version.changed() => Err(Status::unavailable("endpoint generation replaced")),
        }
    }

    async fn discover(
        &self,
        request: Request<pb::DiscoverRequest>,
    ) -> std::result::Result<Response<pb::DiscoverResponse>, Status> {
        self.service.discover(request).await
    }
    async fn register(
        &self,
        request: Request<pb::RegisterRequest>,
    ) -> std::result::Result<Response<pb::RegisterReceipt>, Status> {
        self.service.register(request).await
    }
    async fn confirm_endpoint(
        &self,
        request: Request<pb::ConfirmEndpointRequest>,
    ) -> std::result::Result<Response<pb::EndpointConfirmationReceipt>, Status> {
        self.service.confirm_endpoint(request).await
    }
}

struct Fixture {
    address: SocketAddr,
    modern: watch::Sender<bool>,
    rejected: Arc<AtomicU64>,
    legacy_received: Arc<Mutex<Vec<pb::RaftEnvelope>>>,
    metadata: RaftInbox,
    data: RaftInbox,
    allowed: Arc<AtomicBool>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

fn serve(runtime: &tokio::runtime::Runtime, modern: bool) -> Fixture {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let (modern, _) = watch::channel(modern);
    let rejected = Arc::new(AtomicU64::new(0));
    let legacy_received = Arc::new(Mutex::new(Vec::new()));
    let metadata = RaftInbox::default();
    let router = Arc::new(RegionInboxes::new(metadata.clone()));
    let data = router.register(RegionId(10)).unwrap();
    let allowed = Arc::new(AtomicBool::new(true));
    let service = Kv9RaftServer::with_interceptor(
        Endpoint {
            service: RaftGrpcService::new(NodeId(2), router, Arc::new(Discovery(allowed.clone()))),
            modern: modern.clone(),
            legacy_received: legacy_received.clone(),
        },
        cluster_token_interceptor(TOKEN.into()),
    );
    let surface = LegacySurface {
        service,
        modern: modern.clone(),
        rejected: rejected.clone(),
    };
    let server = runtime.spawn(async move {
        let listener = tokio::net::TcpListener::from_std(listener).unwrap();
        tonic::transport::Server::builder()
            .add_service(surface)
            .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    Fixture {
        address,
        modern,
        rejected,
        legacy_received,
        metadata,
        data,
        allowed,
        server,
    }
}

#[test]
fn group_wire_methods_reject_wrong_domains_before_any_batch_delivery() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let fixture = serve(&runtime, true);
    for (class, regions) in [
        (StreamClass::Metadata, vec![0, 10]),
        (StreamClass::Metadata, vec![0, 1]),
        (StreamClass::Data, vec![10, 0]),
        (StreamClass::Data, vec![10, 1]),
    ] {
        let result = runtime.block_on(call(
            fixture.address,
            class,
            batch(regions.into_iter().map(|r| envelope(r, 7)).collect()),
        ));
        assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
        assert!(
            fixture.metadata.drain().is_empty(),
            "wrong method delivered metadata"
        );
        assert!(
            fixture.data.drain().is_empty(),
            "wrong method partially delivered data"
        );
    }
    runtime
        .block_on(call(
            fixture.address,
            StreamClass::Metadata,
            batch(vec![envelope(0, 11)]),
        ))
        .unwrap();
    runtime
        .block_on(call(
            fixture.address,
            StreamClass::Data,
            batch(vec![envelope(10, 12)]),
        ))
        .unwrap();
    assert_eq!(fixture.metadata.drain()[0].term, 11);
    assert_eq!(fixture.data.drain()[0].term, 12);
}

#[test]
fn group_wire_data_preserves_root_authority_and_payload_identity_gates() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let fixture = serve(&runtime, true);
    let valid = batch(vec![envelope(10, 7)]);
    let mut wrong_root = valid.clone();
    wrong_root.root_digest = vec![1; 32];
    let mut wrong_sender = valid.clone();
    wrong_sender.msgs[0].from_node = 3;
    let mut wrong_payload_sender = valid.clone();
    let mut payload = message(7);
    payload.from = 3;
    wrong_payload_sender.msgs[0].raft_message = payload.write_to_bytes().unwrap();
    let mut wrong_payload_target = valid.clone();
    payload.from = 1;
    payload.to = 3;
    wrong_payload_target.msgs[0].raft_message = payload.write_to_bytes().unwrap();
    let mut wrong_target = valid.clone();
    wrong_target.msgs[0].to_node = 3;
    for (invalid, code) in [
        (wrong_root, tonic::Code::FailedPrecondition),
        (wrong_sender, tonic::Code::PermissionDenied),
        (wrong_payload_sender, tonic::Code::PermissionDenied),
        (wrong_payload_target, tonic::Code::InvalidArgument),
        (wrong_target, tonic::Code::InvalidArgument),
    ] {
        let result = runtime.block_on(call(fixture.address, StreamClass::Data, invalid));
        assert_eq!(result.unwrap_err().code(), code);
        assert!(fixture.data.drain().is_empty());
    }
    fixture.allowed.store(false, Ordering::SeqCst);
    let denied = runtime.block_on(call(fixture.address, StreamClass::Data, valid.clone()));
    assert_eq!(denied.unwrap_err().code(), tonic::Code::FailedPrecondition);
    assert!(fixture.data.drain().is_empty());
    fixture.allowed.store(true, Ordering::SeqCst);
    runtime
        .block_on(call(fixture.address, StreamClass::Data, valid))
        .unwrap();
    assert_eq!(fixture.data.drain()[0].term, 7);
    assert!(fixture.metadata.drain().is_empty());
}

#[test]
fn group_wire_legacy_dispatcher_refuses_data_method_but_ignores_legacy_region() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let fixture = serve(&runtime, false);
    let result = runtime.block_on(call(
        fixture.address,
        StreamClass::Data,
        batch(vec![envelope(10, 7)]),
    ));
    assert_eq!(result.unwrap_err().code(), tonic::Code::Unimplemented);
    assert_eq!(fixture.rejected.load(Ordering::SeqCst), 1);
    assert!(fixture.legacy_received.lock().unwrap().is_empty());
    // Positive hazard control: the same payload on the old method is consumed.
    runtime
        .block_on(call(
            fixture.address,
            StreamClass::Metadata,
            batch(vec![envelope(10, 8)]),
        ))
        .unwrap();
    assert_eq!(fixture.legacy_received.lock().unwrap()[0].region_id, 10);
}

fn eventually(mut condition: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(
            std::time::Instant::now() < deadline,
            "wire progress deadline elapsed"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn group_wire_sender_refuses_legacy_fallback_and_backs_off_unsupported_method() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let legacy = serve(&runtime, false);
    let sender = GrpcTransport::new(
        NodeId(1),
        Some(TOKEN.into()),
        runtime.handle().clone(),
        RootDigest::from_bytes([0; 32]),
    );
    let group = sender.register_group(RegionId(10)).unwrap();
    sender.register_peer(NodeId(2), legacy.address);
    let started = std::time::Instant::now();
    eventually(|| {
        group.send(NodeId(2), message(22));
        sender.send(NodeId(2), message(33));
        let received = legacy.legacy_received.lock().unwrap();
        assert!(
            received.iter().all(|e| e.region_id == 0),
            "sender leaked data through legacy RPC"
        );
        legacy.rejected.load(Ordering::SeqCst) >= 3 && !received.is_empty()
    });
    let attempts = legacy.rejected.load(Ordering::SeqCst);
    assert!(
        started.elapsed() + Duration::from_millis(10) >= RECONNECT_MIN * (attempts as u32 - 1),
        "unsupported method retried without its backoff budget"
    );
}

fn replacement(change_address: bool) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let first = serve(&runtime, true);
    let sender = GrpcTransport::new(
        NodeId(1),
        Some(TOKEN.into()),
        runtime.handle().clone(),
        RootDigest::from_bytes([0; 32]),
    );
    let group = sender.register_group(RegionId(10)).unwrap();
    sender.register_peer(NodeId(2), first.address);
    eventually(|| {
        group.send(NodeId(2), message(11));
        first.data.drain().iter().any(|m| m.term == 11)
    });
    let alternate = change_address.then(|| serve(&runtime, false));
    let legacy = alternate.as_ref().unwrap_or(&first);
    if change_address {
        sender.register_peer(NodeId(2), legacy.address);
    } else {
        // Same address, after successful group delivery. Close the active RPC
        // and remove its method from subsequent dispatch, as on downgrade.
        first.modern.send_replace(false);
    }
    eventually(|| {
        group.send(NodeId(2), message(22));
        sender.send(NodeId(2), message(33));
        legacy.rejected.load(Ordering::SeqCst) >= 3
            && legacy
                .legacy_received
                .lock()
                .unwrap()
                .iter()
                .any(|e| e.region_id == 0)
    });
    assert!(
        legacy
            .legacy_received
            .lock()
            .unwrap()
            .iter()
            .all(|e| e.region_id == 0),
        "data traffic fell back to the unsafe legacy method after replacement"
    );
    assert!(
        legacy.data.drain().iter().all(|m| m.term != 22),
        "legacy endpoint delivered data"
    );
    // Successful TCP connections plus immediate UNIMPLEMENTED must not spin.
    assert!(
        legacy.rejected.load(Ordering::SeqCst) <= 30,
        "unsupported RPC retries spun"
    );
    legacy.modern.send_replace(true);
    eventually(|| {
        group.send(NodeId(2), message(44));
        legacy.data.drain().iter().any(|m| m.term == 44)
    });
    assert!(legacy
        .legacy_received
        .lock()
        .unwrap()
        .iter()
        .all(|e| e.region_id == 0));
}

#[test]
fn group_wire_same_address_downgrade_never_falls_back_and_upgrade_recovers() {
    replacement(false);
}

#[test]
fn group_wire_route_replacement_never_falls_back_and_upgrade_recovers() {
    replacement(true);
}

#[test]
fn group_wire_data_saturation_reserves_metadata_queue_and_bounds_worker_count() {
    // No executor turns: deterministically fill queues before any draining.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let sender = GrpcTransport::new(
        NodeId(1),
        None,
        runtime.handle().clone(),
        RootDigest::from_bytes([0; 32]),
    );
    sender.register_peer(NodeId(2), "127.0.0.1:1".parse().unwrap());
    let group = sender.register_group(RegionId(10)).unwrap();
    for _ in 0..PEER_QUEUE + 1 {
        group.send(NodeId(2), message(11));
    }
    sender.send(NodeId(2), message(22));
    let peers = sender.peers.lock().unwrap();
    let peer = &peers[&2];
    let meta_task = peer.sender.as_ref().unwrap().task.abort_handle();
    let data_task = peer.data_sender.as_ref().unwrap().task.abort_handle();
    assert_ne!(meta_task.id(), data_task.id());
    assert_eq!(peer.data_sender.as_ref().unwrap().queue.capacity(), 0);
    assert_eq!(
        peer.sender.as_ref().unwrap().queue.capacity(),
        PEER_QUEUE - 1
    );
    let first_destination = peer.destination.clone();
    drop(peers);
    for region in 11..=100 {
        sender
            .register_group(RegionId(region))
            .unwrap()
            .send(NodeId(2), message(region));
    }
    sender.register_peer(NodeId(2), "127.0.0.1:2".parse().unwrap());
    sender.register_peer(NodeId(2), "127.0.0.1:1".parse().unwrap());
    let peers = sender.peers.lock().unwrap();
    let peer = &peers[&2];
    assert!(!Arc::ptr_eq(&first_destination, &peer.destination));
    for (slot, task) in [(&peer.sender, &meta_task), (&peer.data_sender, &data_task)] {
        let slot = slot.as_ref().unwrap();
        assert_eq!(
            slot.task.id(),
            task.id(),
            "extra group/route spawned another worker"
        );
        assert!(
            Arc::ptr_eq(&slot.destination.borrow(), &peer.destination),
            "worker retained an old endpoint generation"
        );
    }
    drop(peers);
    drop(group);
    drop(sender);
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(3), async {
            while !meta_task.is_finished() || !data_task.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("both stream workers must stop with their owner");
    });
}
