use super::*;
use kv9_common::data_read_fence::{DataReadFences, ReadImage};
use kv9_common::store_lifecycle::{RawTtlFormatFence, StoreGuard};
use kv9_common::{AppliedPosition, RootDescriptor, RootVoter};
use std::sync::atomic::{AtomicBool, AtomicUsize};

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
    floors: Arc<DataReadFences>,
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
    fn data_read_fence(
        &self,
        root: RootDigest,
        region: RegionId,
        creation: RootDigest,
        _: Instant,
    ) -> kv9_common::Result<Option<ReadFloor>> {
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
        let floor = self.floors.floor(region)?;
        if root != self.floors.identity().root_digest
            || floor.is_some_and(|f| f.creation != creation)
        {
            return Err(invalid());
        }
        Ok(floor)
    }
}
struct Fixture {
    path: std::path::PathBuf,
    _guard: StoreGuard,
    discovery: Arc<Discovery>,
    backend: Arc<Backend>,
    ticket: FenceTicket,
}
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "kv9-floor-rpc-{}",
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
            b"floor-rpc-test",
        )
        .unwrap();
        let identity = StoreIdentity::for_voter(&root, record.node_id).unwrap();
        guard.bind(&root, &identity).unwrap();
        guard.activate(&identity).unwrap();
        let fence = guard.raw_ttl_format_fence(&root, &identity).unwrap();
        let backend = Arc::new(Backend {
            floors: fence.data_read_fences(),
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
            ticket: FenceTicket::new(identity, RegionId(100), RootDigest::from_bytes([7; 32]))
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
    fn request(&self, budget: Duration) -> Request<pb::DataReadFenceRequest> {
        let mut r = Request::new(self.ticket.request(budget));
        r.extensions_mut().insert(NodeId(1));
        r
    }
    fn floor(&self) -> ReadFloor {
        ReadFloor {
            identity: self.ticket.identity,
            creation: self.ticket.creation,
            generation: 8,
            image: ReadImage {
                region: RegionId(100),
                digest: RootDigest::from_bytes([9; 32]),
                cut: AppliedPosition { term: 2, index: 99 },
                conf_ver: 1,
                version: 1,
            },
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        *self.backend.release.0.lock().unwrap() = true;
        self.backend.release.1.notify_all();
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[tokio::test]
async fn data_fence_rpc_requires_exact_authenticated_store_and_correlates_canonical_floor() {
    let f = Fixture::new();
    let service = f.service();
    let budget = Duration::from_secs(1);
    let mut unauth = f.request(budget);
    unauth.extensions_mut().remove::<NodeId>();
    assert_eq!(
        service
            .inspect_data_read_fence_request(unauth)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unauthenticated
    );
    for field in 0..5 {
        let mut r = f.request(budget);
        let v = r.get_mut();
        match field {
            0 => v.region_id = 1,
            1 => v.creation.fill(0),
            2 => v.challenge.clear(),
            3 => v.budget_micros = MAX_MICROS + 1,
            _ => v.node_id = 9,
        }
        assert!(service.inspect_data_read_fence_request(r).await.is_err());
    }
    assert_eq!(f.backend.calls.load(Ordering::SeqCst), 0);
    let query = f.ticket.request(budget);
    let unknown = service
        .inspect_data_read_fence_request(f.request(budget))
        .await
        .unwrap()
        .into_inner();
    assert!(ConfirmedDataReadFence::decode(&f.ticket, &query, unknown)
        .unwrap()
        .floor()
        .is_none());
    f.backend.floors.advance(f.floor()).unwrap();
    let reply = service
        .inspect_data_read_fence_request(f.request(budget))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        ConfirmedDataReadFence::decode(&f.ticket, &query, reply.clone())
            .unwrap()
            .floor(),
        Some(f.floor())
    );
    for field in 0..8 {
        let mut wrong = reply.clone();
        let q = wrong.query.as_mut().unwrap();
        match field {
            0 => q.root_digest[0] ^= 1,
            1 => q.cluster_id[0] ^= 1,
            2 => q.node_id += 1,
            3 => q.store_incarnation[0] ^= 1,
            4 => q.region_id += 1,
            5 => q.creation[0] ^= 1,
            6 => q.challenge[0] ^= 1,
            _ => q.budget_micros += 1,
        }
        assert!(ConfirmedDataReadFence::decode(&f.ticket, &query, wrong).is_err());
    }
    let mut wrong = reply.clone();
    let mut floor = f.floor();
    floor.creation = RootDigest::from_bytes([3; 32]);
    wrong.observation = Some(pb::data_read_fence_response::Observation::Floor(
        floor.encode().unwrap(),
    ));
    assert!(ConfirmedDataReadFence::decode(&f.ticket, &query, wrong).is_err());
    let mut wrong = reply;
    wrong.observation = Some(pb::data_read_fence_response::Observation::Unknown(false));
    assert!(ConfirmedDataReadFence::decode(&f.ticket, &query, wrong).is_err());
    std::fs::write(
        f.path
            .join(kv9_common::data_read_fence::DIRECTORY)
            .join("100"),
        vec![0; 224],
    )
    .unwrap();
    assert!(service
        .inspect_data_read_fence_request(f.request(budget))
        .await
        .is_err());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn data_fence_timeout_retains_actual_read_permit_and_rechecks_authority() {
    let f = Fixture::new();
    let service = Arc::new(f.service());
    f.backend.gated.store(true, Ordering::SeqCst);
    let request = f.request(Duration::from_millis(50));
    let s = service.clone();
    let job = tokio::spawn(async move { s.inspect_data_read_fence_request(request).await });
    tokio::time::timeout(Duration::from_secs(1), f.backend.entered.notified())
        .await
        .unwrap();
    assert_eq!(
        job.await.unwrap().unwrap_err().code(),
        tonic::Code::DeadlineExceeded
    );
    assert_eq!(
        service
            .inspect_data_read_fence_request(f.request(Duration::from_secs(1)))
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
    *f.backend.release.0.lock().unwrap() = false;
    let request = f.request(Duration::from_secs(1));
    let s = service.clone();
    let job = tokio::spawn(async move { s.inspect_data_read_fence_request(request).await });
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
