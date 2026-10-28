use super::*;
use kv9_common::store_lifecycle::{RawTtlFormatFence, StoreGuard};
use kv9_common::{RootDescriptor, RootVoter};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize};

struct Fixture {
    path: PathBuf,
    _guard: StoreGuard,
    discovery: Arc<Discovery>,
    ticket: HistoryTicket,
}
impl Fixture {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("kv9-history-{}", StoreIncarnation::mint().unwrap()));
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
            b"history-test",
        )
        .unwrap();
        let identity = StoreIdentity::for_voter(&root, record.node_id).unwrap();
        guard.bind(&root, &identity).unwrap();
        guard.activate(&identity).unwrap();
        let fence = guard.raw_ttl_format_fence(&root, &identity).unwrap();
        Self {
            path,
            _guard: guard,
            discovery: Arc::new(Discovery {
                root: RootWireIdentity {
                    root_digest: root.digest(),
                    bootstrap_generation: root.bootstrap_generation,
                },
                fence: Some(fence),
                allowed: AtomicBool::new(true),
            }),
            ticket: HistoryTicket::new(identity, RegionId(100), 7).unwrap(),
        }
    }
    fn service(&self) -> RaftGrpcService {
        RaftGrpcService::new(
            NodeId(2),
            crate::work::RaftInbox::default(),
            self.discovery.clone(),
        )
    }
    fn request(&self) -> Request<pb::CheckpointHistoryRequest> {
        let mut request = Request::new(self.ticket.request(Duration::from_secs(1)));
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

#[derive(Default)]
struct Backend {
    bootstrap_enabled: AtomicBool,
    calls: AtomicUsize,
    selection_enabled: AtomicBool,
    gated: AtomicBool,
    unknown: AtomicBool,
    entered: tokio::sync::Notify,
    release: (std::sync::Mutex<bool>, std::sync::Condvar),
}
impl Backend {
    fn unblock(&self) {
        *self.release.0.lock().unwrap() = true;
        self.release.1.notify_all();
    }
    async fn entered(&self) {
        tokio::time::timeout(Duration::from_secs(1), self.entered.notified())
            .await
            .unwrap();
    }
}
impl RegistrationBackend for Backend {
    fn bootstrap_read_fence(
        &self,
        root: RootDigest,
        predecessor: u64,
        deadline: Instant,
    ) -> kv9_common::Result<Option<[u8; 32]>> {
        if !self.bootstrap_enabled.load(Ordering::SeqCst) {
            return Err(Error::NotImplemented("no durable fence observation"));
        }
        self.checkpoint_history(root, RegionId(100), predecessor, deadline)
    }
    fn checkpoint_selection(
        &self,
        root: RootDigest,
        region: RegionId,
        predecessor: u64,
        deadline: Instant,
    ) -> kv9_common::Result<Option<[u8; 32]>> {
        if !self.selection_enabled.load(Ordering::SeqCst) {
            return Err(Error::NotImplemented("legacy history-only backend"));
        }
        self.checkpoint_history(root, region, predecessor, deadline)
    }
    fn register(
        &self,
        _: NodeId,
        _: &str,
        _: ClusterId,
        _: &[u8],
        _: StoreIncarnation,
    ) -> std::result::Result<RegistrationReceipt, RegistrationError> {
        panic!("history RPC must not register a node")
    }
    fn checkpoint_history(
        &self,
        _: RootDigest,
        region: RegionId,
        predecessor: u64,
        _: Instant,
    ) -> kv9_common::Result<Option<[u8; 32]>> {
        assert!(matches!(region.0, 1 | 100));
        assert_eq!(predecessor, 7);
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.gated.load(Ordering::SeqCst) {
            self.entered.notify_one();
            let (lock, signal) = &self.release;
            let (released, _) = signal
                .wait_timeout_while(lock.lock().unwrap(), Duration::from_secs(5), |v| !*v)
                .unwrap();
            if !*released {
                return Err(invalid());
            }
        }
        Ok((!self.unknown.load(Ordering::SeqCst)).then_some([9; 32]))
    }
}

#[tokio::test]
async fn history_requires_authenticated_active_exact_store_and_bounded_query() {
    let f = Fixture::new();
    let backend = Arc::new(Backend::default());
    let service = f.service().with_registration(backend.clone());
    for caller in [None, Some(NodeId(0))] {
        let mut request = f.request();
        request.extensions_mut().remove::<NodeId>();
        if let Some(caller) = caller {
            request.extensions_mut().insert(caller);
        }
        assert_eq!(
            service
                .inspect_checkpoint_history_request(request)
                .await
                .unwrap_err()
                .code(),
            tonic::Code::Unauthenticated
        );
    }
    f.discovery.allowed.store(false, Ordering::SeqCst);
    assert_eq!(
        service
            .inspect_checkpoint_history_request(f.request())
            .await
            .unwrap_err()
            .code(),
        tonic::Code::FailedPrecondition
    );
    f.discovery.allowed.store(true, Ordering::SeqCst);
    let unfenced = RaftGrpcService::new(
        NodeId(2),
        crate::work::RaftInbox::default(),
        Arc::new(Discovery {
            root: f.discovery.root,
            fence: None,
            allowed: AtomicBool::new(true),
        }),
    );
    assert_eq!(
        unfenced
            .inspect_checkpoint_history_request(f.request())
            .await
            .unwrap_err()
            .code(),
        tonic::Code::FailedPrecondition
    );
    for field in 0..4 {
        let mut request = f.request();
        let v = request.get_mut();
        match field {
            0 => v.node_id += 1,
            1 => v.root_digest[0] ^= 1,
            2 => v.cluster_id.push(0),
            _ => v.store_incarnation[0] ^= 1,
        }
        assert_eq!(
            service
                .inspect_checkpoint_history_request(request)
                .await
                .unwrap_err()
                .code(),
            tonic::Code::FailedPrecondition
        );
    }
    for field in 0..8 {
        let mut request = f.request();
        let v = request.get_mut();
        match field {
            0 => v.challenge = vec![0; 16],
            1 => v.challenge.pop().map(|_| ()).unwrap(),
            2 => v.budget_micros = 0,
            3 => v.budget_micros = MAX_MICROS + 1,
            4 => v.region_id = 0,
            5 => v.region_id = 99,
            6 => v.predecessor = u64::MAX,
            _ => v.root_digest = vec![1; MAX_BYTES + 1],
        }
        assert_eq!(
            service
                .inspect_checkpoint_history_request(request)
                .await
                .unwrap_err()
                .code(),
            tonic::Code::InvalidArgument
        );
    }
    assert_eq!(
        f.service()
            .inspect_checkpoint_history_request(f.request())
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unimplemented
    );
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
    for unknown in [false, true] {
        backend.unknown.store(unknown, Ordering::SeqCst);
        let reply = service
            .inspect_checkpoint_history_request(f.request())
            .await
            .unwrap()
            .into_inner();
        assert_eq!(
            ConfirmedHistory::decode(&f.ticket, reply)
                .unwrap()
                .winner_change_id(),
            (!unknown).then_some([9; 32])
        );
    }
}

#[tokio::test]
async fn history_reply_must_correlate_every_identity_window_and_challenge_field() {
    let f = Fixture::new();
    let service = f.service().with_registration(Arc::new(Backend::default()));
    let reply = service
        .inspect_checkpoint_history_request(f.request())
        .await
        .unwrap()
        .into_inner();
    for field in 0..12 {
        use pb::checkpoint_history_response::Observation;
        let mut bad = reply.clone();
        match field {
            0 => bad.root_digest[0] ^= 1,
            1 => bad.cluster_id[0] ^= 1,
            2 => bad.node_id += 1,
            3 => bad.store_incarnation[0] ^= 1,
            4 => bad.region_id += 1,
            5 => bad.predecessor += 1,
            6 => bad.challenge[0] ^= 1,
            7 => bad.observation = None,
            8 => bad.observation = Some(Observation::Unknown(false)),
            9 => bad.observation = Some(Observation::WinnerChangeId(vec![9; 31])),
            10 => bad.observation = Some(Observation::WinnerChangeId(vec![9; 33])),
            _ => bad.observation = Some(Observation::WinnerChangeId(vec![9; MAX_BYTES + 1])),
        }
        assert!(
            ConfirmedHistory::decode(&f.ticket, bad).is_err(),
            "field {field}"
        );
    }
    let newer =
        HistoryTicket::new(f.ticket.identity, f.ticket.region, f.ticket.predecessor).unwrap();
    assert!(
        ConfirmedHistory::decode(&newer, reply).is_err(),
        "a prior query cannot answer a fresh challenge"
    );
}

#[tokio::test]
async fn selection_rpc_never_falls_back_to_history_and_rechecks_store_and_scope() {
    let f = Fixture::new();
    let backend = Arc::new(Backend::default());
    let service = f.service().with_registration(backend.clone());
    assert!(service
        .inspect_checkpoint_history_request(f.request())
        .await
        .is_ok());
    assert!(service
        .inspect_checkpoint_selection_request(f.request())
        .await
        .is_err());
    assert_eq!(
        backend.calls.load(Ordering::SeqCst),
        1,
        "selection cannot use the history method as a fallback"
    );
    backend.selection_enabled.store(true, Ordering::SeqCst);
    let mut unauthenticated = f.request();
    unauthenticated.extensions_mut().clear();
    assert_eq!(
        service
            .inspect_checkpoint_selection_request(unauthenticated)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unauthenticated
    );
    let mut metadata = f.request();
    metadata.get_mut().region_id = 2;
    assert_eq!(
        service
            .inspect_checkpoint_selection_request(metadata)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::InvalidArgument
    );
    f.discovery.allowed.store(false, Ordering::SeqCst);
    assert_eq!(
        service
            .inspect_checkpoint_selection_request(f.request())
            .await
            .unwrap_err()
            .code(),
        tonic::Code::FailedPrecondition
    );
    f.discovery.allowed.store(true, Ordering::SeqCst);
    for unknown in [false, true] {
        backend.unknown.store(unknown, Ordering::SeqCst);
        let reply = service
            .inspect_checkpoint_selection_request(f.request())
            .await
            .unwrap()
            .into_inner();
        let confirmed = ConfirmedSelection(ConfirmedHistory::decode(&f.ticket, reply).unwrap());
        assert_eq!(
            confirmed.selected_change_id(),
            (!unknown).then_some([9; 32])
        );
    }
}

async fn drained(service: &RaftGrpcService) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while service.checkpoint_history_capacity.available_permits() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn history_timeout_and_cancel_keep_actual_reader_slot_until_job_finishes() {
    let f = Fixture::new();
    for cancel in [false, true] {
        let backend = Arc::new(Backend::default());
        backend.gated.store(true, Ordering::SeqCst);
        let service = Arc::new(f.service().with_registration(backend.clone()));
        let mut request = f.request();
        if !cancel {
            request.get_mut().budget_micros = 100_000;
        }
        let s = service.clone();
        let call = tokio::spawn(async move { s.inspect_checkpoint_history_request(request).await });
        backend.entered().await;
        if cancel {
            call.abort();
            assert!(call.await.unwrap_err().is_cancelled());
        } else {
            assert_eq!(
                call.await.unwrap().unwrap_err().code(),
                tonic::Code::DeadlineExceeded
            );
        }
        assert_eq!(service.checkpoint_history_capacity.available_permits(), 0);
        assert_eq!(
            service
                .inspect_checkpoint_history_request(f.request())
                .await
                .unwrap_err()
                .code(),
            tonic::Code::ResourceExhausted
        );
        assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
        backend.unblock();
        drained(&service).await;
        let reply = service
            .inspect_checkpoint_history_request(f.request())
            .await
            .unwrap()
            .into_inner();
        assert_eq!(
            ConfirmedHistory::decode(&f.ticket, reply)
                .unwrap()
                .winner_change_id(),
            Some([9; 32])
        );
    }
}

#[tokio::test]
async fn history_authority_withdrawn_during_actual_read_invalidates_reply() {
    let f = Fixture::new();
    let backend = Arc::new(Backend::default());
    backend.gated.store(true, Ordering::SeqCst);
    let service = Arc::new(f.service().with_registration(backend.clone()));
    let (s, request) = (service.clone(), f.request());
    let call = tokio::spawn(async move { s.inspect_checkpoint_history_request(request).await });
    backend.entered().await;
    f.discovery.allowed.store(false, Ordering::SeqCst);
    backend.unblock();
    assert_eq!(
        call.await.unwrap().unwrap_err().code(),
        tonic::Code::FailedPrecondition
    );
    drained(&service).await;
}

#[tokio::test]
async fn bootstrap_fence_rpc_is_independent_authenticated_exact_store_metadata_only_and_unknown_safe(
) {
    let mut f = Fixture::new();
    let backend = Arc::new(Backend::default());
    backend.selection_enabled.store(true, Ordering::SeqCst);
    let service = f.service().with_registration(backend.clone());
    assert!(service
        .inspect_checkpoint_selection_request(f.request())
        .await
        .is_ok());
    assert!(service
        .inspect_bootstrap_read_fence_request(f.request())
        .await
        .is_err());
    f.ticket.region = kv9_common::META_REGION_0;
    assert!(service
        .inspect_bootstrap_read_fence_request(f.request())
        .await
        .is_err());
    assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
    backend.bootstrap_enabled.store(true, Ordering::SeqCst);
    let mut unauthenticated = f.request();
    unauthenticated.extensions_mut().clear();
    assert_eq!(
        service
            .inspect_bootstrap_read_fence_request(unauthenticated)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unauthenticated
    );
    let mut foreign = f.request();
    foreign.get_mut().store_incarnation = vec![8; 16];
    assert!(service
        .inspect_bootstrap_read_fence_request(foreign)
        .await
        .is_err());
    f.discovery.allowed.store(false, Ordering::SeqCst);
    assert!(service
        .inspect_bootstrap_read_fence_request(f.request())
        .await
        .is_err());
    f.discovery.allowed.store(true, Ordering::SeqCst);
    for unknown in [true, false] {
        backend.unknown.store(unknown, Ordering::SeqCst);
        let reply = service
            .inspect_bootstrap_read_fence_request(f.request())
            .await
            .unwrap()
            .into_inner();
        let observed =
            ConfirmedBootstrapFence(ConfirmedHistory::decode(&f.ticket, reply.clone()).unwrap());
        assert_eq!(observed.image_digest(), (!unknown).then_some([9; 32]));
        let next =
            HistoryTicket::new(f.ticket.identity, f.ticket.region, f.ticket.predecessor).unwrap();
        assert!(ConfirmedHistory::decode(&next, reply).is_err());
    }
}

#[tokio::test]
async fn metadata_selection_reply_is_bound_to_its_exact_metadata_ticket() {
    let f = Fixture::new();
    let backend = Arc::new(Backend::default());
    backend.selection_enabled.store(true, Ordering::SeqCst);
    let service = f.service().with_registration(backend);
    let ticket = HistoryTicket::new(f.ticket.identity, kv9_common::META_REGION_0, 7).unwrap();
    let mut request = Request::new(ticket.request(Duration::from_secs(1)));
    request.extensions_mut().insert(NodeId(1));
    let reply = service
        .inspect_checkpoint_selection_request(request)
        .await
        .unwrap()
        .into_inner();
    assert!(ConfirmedHistory::decode(&ticket, reply.clone())
        .unwrap()
        .winner_change_id()
        .is_some());
    assert!(ConfirmedHistory::decode(&f.ticket, reply.clone()).is_err());
    let mut wrong = reply;
    wrong.region_id = 100;
    assert!(ConfirmedHistory::decode(&ticket, wrong).is_err());
}
