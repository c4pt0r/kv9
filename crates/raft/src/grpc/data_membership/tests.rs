use super::*;
use kv9_common::store_lifecycle::{RawTtlFormatFence, StoreGuard};
use kv9_common::{RootDescriptor, RootVoter};
use std::sync::atomic::{AtomicBool, AtomicUsize};

pub(crate) mod old_service;

#[tokio::test]
async fn membership_rpc_binds_exact_store_and_correlates_every_query_field() {
    let f = Fixture::new();
    let service = f.service();
    let budget = Duration::from_secs(1);
    let mut unauth = f.request(budget);
    unauth.extensions_mut().remove::<NodeId>();
    assert_eq!(
        service
            .inspect_data_membership_request(unauth)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unauthenticated
    );
    for field in 0..10 {
        let mut r = f.request(budget);
        let q = r.get_mut();
        let s = q.scope.as_mut().unwrap();
        match field {
            0 => s.region_id = 1,
            1 => s.creation.fill(0),
            2 => s.challenge.clear(),
            3 => s.budget_micros = MAX_MICROS + 1,
            4 => s.node_id = 9,
            5 => s.store_incarnation[0] ^= 1,
            6 => s.root_digest[0] ^= 1,
            7 => q.version = 0,
            8 => q.seen = 1,
            _ => q.cursor = vec![1; 1025],
        }
        assert!(service.inspect_data_membership_request(r).await.is_err());
    }
    assert_eq!(f.backend.calls.load(Ordering::SeqCst), 0);
    let query = f.ticket.request(budget);
    let reply = service
        .inspect_data_membership_request(f.request(budget))
        .await
        .unwrap()
        .into_inner();
    let page = ConfirmedMembershipPage::decode(&f.ticket, &query, reply.clone()).unwrap();
    assert_eq!(page.page().unwrap().members.len(), 4);
    for field in 0..11 {
        let mut wrong = reply.clone();
        let q = wrong.query.as_mut().unwrap();
        let s = q.scope.as_mut().unwrap();
        match field {
            0 => s.root_digest[0] ^= 1,
            1 => s.cluster_id[0] ^= 1,
            2 => s.node_id += 1,
            3 => s.store_incarnation[0] ^= 1,
            4 => s.region_id += 1,
            5 => s.creation[0] ^= 1,
            6 => s.challenge[0] ^= 1,
            7 => s.budget_micros += 1,
            8 => q.version += 1,
            9 => q.seen += 1,
            _ => q.cursor = vec![1],
        }
        assert!(ConfirmedMembershipPage::decode(&f.ticket, &query, wrong).is_err());
    }
    let next = page.next_ticket().unwrap().unwrap();
    assert_ne!(
        next.request(budget).scope.unwrap().challenge,
        query.scope.unwrap().challenge
    );
    assert_eq!(next.seen(), 4);
    let mut request = Request::new(next.request(budget));
    request.extensions_mut().insert(NodeId(1));
    let reply = service
        .inspect_data_membership_request(request)
        .await
        .unwrap()
        .into_inner();
    let last =
        ConfirmedMembershipPage::decode(&next, &next.request(budget), reply.clone()).unwrap();
    assert_eq!(last.page().unwrap().members[0].node, NodeId(5));
    assert!(last.next_ticket().unwrap().is_none());
    // A terminal fragment cannot be accepted as the start of a complete walk.
    let mut wrong = reply;
    wrong.query = Some(f.ticket.request(budget));
    assert!(ConfirmedMembershipPage::decode(&f.ticket, &f.ticket.request(budget), wrong).is_err());
}

#[tokio::test]
async fn membership_unknown_and_malformed_backend_page_never_finish_a_census() {
    let f = Fixture::new();
    let service = f.service();
    let budget = Duration::from_secs(1);
    f.backend.unknown.store(true, Ordering::SeqCst);
    let reply = service
        .inspect_data_membership_request(f.request(budget))
        .await
        .unwrap()
        .into_inner();
    let unknown =
        ConfirmedMembershipPage::decode(&f.ticket, &f.ticket.request(budget), reply.clone())
            .unwrap();
    assert!(unknown.page().is_none());
    assert!(unknown.next_ticket().is_err());
    for observation in [
        None,
        Some(pb::data_membership_response::Observation::Unknown(false)),
        Some(pb::data_membership_response::Observation::Page(vec![
            0;
            112
        ])),
    ] {
        let mut wrong = reply.clone();
        wrong.observation = observation;
        assert!(
            ConfirmedMembershipPage::decode(&f.ticket, &f.ticket.request(budget), wrong).is_err()
        );
    }
    f.backend.unknown.store(false, Ordering::SeqCst);
    f.backend.malformed.store(true, Ordering::SeqCst);
    assert!(service
        .inspect_data_membership_request(f.request(budget))
        .await
        .is_err());
}

struct Discovery {
    root: RootWireIdentity,
    fence: RawTtlFormatFence,
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
        Some(self.fence.clone())
    }
}
struct Backend {
    storage: crate::storage::DiskRaftStorage,
    unknown: AtomicBool,
    malformed: AtomicBool,
    calls: AtomicUsize,
    gated: AtomicBool,
    entered: tokio::sync::Notify,
    release: (std::sync::Mutex<bool>, std::sync::Condvar),
}
impl RegistrationBackend for Backend {
    fn register(
        &self,
        _: NodeId,
        _: &str,
        _: ClusterId,
        _: &[u8],
        _: StoreIncarnation,
    ) -> std::result::Result<RegistrationReceipt, RegistrationError> {
        panic!("not registration")
    }
    fn data_membership(
        &self,
        _: RootDigest,
        _: RegionId,
        _: RootDigest,
        cursor: Option<MembershipCursor>,
        _: Instant,
    ) -> kv9_common::Result<Option<MembershipPage>> {
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
        if self.unknown.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let mut page = self.storage.membership_history_page(cursor)?;
        if self.malformed.load(Ordering::SeqCst) {
            page.next = None;
        }
        Ok(Some(page))
    }
}
struct Fixture {
    path: std::path::PathBuf,
    _guard: StoreGuard,
    discovery: Arc<Discovery>,
    backend: Arc<Backend>,
    ticket: MembershipTicket,
}
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "kv9-membership-rpc-{}",
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
            b"membership-rpc-test",
        )
        .unwrap();
        let identity = StoreIdentity::for_voter(&root, record.node_id).unwrap();
        guard.bind(&root, &identity).unwrap();
        guard.activate(&identity).unwrap();
        let fence = guard.raw_ttl_format_fence(&root, &identity).unwrap();
        let backend = Arc::new(Backend {
            storage: crate::storage::DiskRaftStorage::open(&path.join("raft"), &[1, 2, 3, 4, 5])
                .unwrap()
                .0,
            unknown: AtomicBool::new(false),
            malformed: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
            gated: AtomicBool::new(false),
            entered: tokio::sync::Notify::new(),
            release: (std::sync::Mutex::new(false), std::sync::Condvar::new()),
        });
        Self {
            path,
            _guard: guard,
            discovery: Arc::new(Discovery {
                root: RootWireIdentity {
                    root_digest: root.digest(),
                    bootstrap_generation: root.bootstrap_generation,
                },
                fence,
                allowed: AtomicBool::new(true),
            }),
            backend,
            ticket: MembershipTicket::new(identity, RegionId(100), RootDigest::from_bytes([7; 32]))
                .unwrap(),
        }
    }
    fn service(&self) -> RaftGrpcService {
        RaftGrpcService::new(
            NodeId(2),
            crate::work::RaftInbox::default(),
            self.discovery.clone(),
        )
        .with_registration(self.backend.clone())
    }
    fn request(&self, budget: Duration) -> Request<pb::DataMembershipRequest> {
        let mut r = Request::new(self.ticket.request(budget));
        r.extensions_mut().insert(NodeId(1));
        r
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        *self.backend.release.0.lock().unwrap() = true;
        self.backend.release.1.notify_all();
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn membership_timeout_retains_actual_read_permit_and_rechecks_authority() {
    let f = Fixture::new();
    let service = Arc::new(f.service());
    f.backend.gated.store(true, Ordering::SeqCst);
    let request = f.request(Duration::from_millis(50));
    let s = service.clone();
    let job = tokio::spawn(async move { s.inspect_data_membership_request(request).await });
    tokio::time::timeout(Duration::from_secs(1), f.backend.entered.notified())
        .await
        .unwrap();
    assert_eq!(
        job.await.unwrap().unwrap_err().code(),
        tonic::Code::DeadlineExceeded
    );
    assert_eq!(
        service
            .inspect_data_membership_request(f.request(Duration::from_secs(1)))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::ResourceExhausted
    );
    let mut floor = Request::new(f.ticket.scope.request(Duration::from_secs(1)));
    floor.extensions_mut().insert(NodeId(1));
    assert_eq!(
        service
            .inspect_data_read_fence_request(floor)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::ResourceExhausted,
        "observation RPCs must share the actual worker budget"
    );
    *f.backend.release.0.lock().unwrap() = true;
    f.backend.release.1.notify_all();
    tokio::time::timeout(Duration::from_secs(1), async {
        while service.checkpoint_history_capacity.available_permits() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    *f.backend.release.0.lock().unwrap() = false;
    let request = f.request(Duration::from_secs(1));
    let s = service.clone();
    let job = tokio::spawn(async move { s.inspect_data_membership_request(request).await });
    tokio::time::timeout(Duration::from_secs(1), f.backend.entered.notified())
        .await
        .unwrap();
    f.discovery.allowed.store(false, Ordering::SeqCst);
    *f.backend.release.0.lock().unwrap() = true;
    f.backend.release.1.notify_all();
    assert_eq!(
        job.await.unwrap().unwrap_err().code(),
        tonic::Code::FailedPrecondition
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn membership_cancel_does_not_release_the_running_reader() {
    let f = Fixture::new();
    let service = Arc::new(f.service());
    f.backend.gated.store(true, Ordering::SeqCst);
    let request = f.request(Duration::from_secs(1));
    let s = service.clone();
    let job = tokio::spawn(async move { s.inspect_data_membership_request(request).await });
    tokio::time::timeout(Duration::from_secs(1), f.backend.entered.notified())
        .await
        .unwrap();
    job.abort();
    assert!(job.await.unwrap_err().is_cancelled());
    assert_eq!(service.checkpoint_history_capacity.available_permits(), 0);
    assert_eq!(
        service
            .inspect_data_membership_request(f.request(Duration::from_secs(1)))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::ResourceExhausted
    );
    *f.backend.release.0.lock().unwrap() = true;
    f.backend.release.1.notify_all();
    tokio::time::timeout(Duration::from_secs(1), async {
        while service.checkpoint_history_capacity.available_permits() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(service
        .inspect_data_membership_request(f.request(Duration::from_secs(1)))
        .await
        .is_ok());
}
