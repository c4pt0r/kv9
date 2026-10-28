use super::*;
use kv9_common::log_budget::{headroom::DiskSpaceProbe, LogBudget};
use kv9_common::store_lifecycle::{RawTtlFormatFence, StoreGuard};
use kv9_common::{RootDescriptor, RootVoter};
use std::path::{Path, PathBuf};
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
            "kv9-pressure-{}",
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
            b"pressure-test",
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
    fn service(&self, allowed: bool, fenced: bool) -> RaftGrpcService {
        RaftGrpcService::new(
            NodeId(2),
            crate::work::RaftInbox::default(),
            Arc::new(Discovery {
                root: RootWireIdentity {
                    root_digest: self.root.digest(),
                    bootstrap_generation: self.root.bootstrap_generation,
                },
                fence: fenced.then_some(self.fence.clone()),
                allowed: AtomicBool::new(allowed),
            }),
        )
    }
    fn request(&self) -> Request<pb::LogPressureRequest> {
        let id = self.fence.identity();
        let mut request = Request::new(pb::LogPressureRequest {
            root_digest: id.root_digest.as_bytes().to_vec(),
            cluster_id: id.cluster_id.as_bytes().to_vec(),
            node_id: id.node_id.0,
            store_incarnation: id.store_incarnation.as_bytes().to_vec(),
            challenge: vec![3; 16],
            budget_micros: 1_000_000,
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

#[tokio::test]
async fn pressure_rpc_requires_authenticated_active_exact_store_and_valid_budget() {
    let f = Fixture::new();
    for (allowed, fenced) in [(false, false), (true, false), (false, true)] {
        assert_eq!(
            f.service(allowed, fenced)
                .inspect_log_pressure_request(f.request())
                .await
                .unwrap_err()
                .code(),
            tonic::Code::FailedPrecondition
        );
    }
    let server = f
        .service(true, true)
        .with_log_budget(Arc::new(LogBudget::new(100).unwrap()));
    let mut request = f.request();
    request.extensions_mut().remove::<NodeId>();
    assert_eq!(
        server
            .inspect_log_pressure_request(request)
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unauthenticated
    );
    for field in 0..4 {
        let mut request = f.request();
        match field {
            0 => request.get_mut().node_id += 1,
            1 => request.get_mut().root_digest[0] ^= 1,
            2 => request.get_mut().cluster_id.push(0),
            _ => request.get_mut().store_incarnation[0] ^= 1,
        }
        assert_eq!(
            server
                .inspect_log_pressure_request(request)
                .await
                .unwrap_err()
                .code(),
            tonic::Code::FailedPrecondition
        );
    }
    for field in 0..5 {
        let mut request = f.request();
        match field {
            0 => request.get_mut().challenge = vec![0; 16],
            1 => request.get_mut().challenge = vec![1; 15],
            2 => request.get_mut().budget_micros = 0,
            3 => request.get_mut().budget_micros = MAX_BUDGET_MICROS + 1,
            _ => request.get_mut().root_digest = vec![1; 513],
        }
        assert_eq!(
            server
                .inspect_log_pressure_request(request)
                .await
                .unwrap_err()
                .code(),
            tonic::Code::InvalidArgument
        );
    }
    assert_eq!(
        f.service(true, true)
            .inspect_log_pressure_request(f.request())
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unimplemented
    );
    let reply = server
        .inspect_log_pressure_request(f.request())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(reply.challenge, vec![3; 16]);
    assert_eq!(
        reply.observation,
        Some(pb::log_pressure_response::Observation::AvailableBytes(100))
    );
}

#[tokio::test]
async fn pressure_rpc_distinguishes_accounting_uncertainty_from_exhaustion() {
    let f = Fixture::new();
    let budget = Arc::new(LogBudget::new(100).unwrap());
    let debt = budget.track(Path::new("raft"), 100).unwrap();
    let service = f.service(true, true).with_log_budget(budget);
    let reply = service
        .inspect_log_pressure_request(f.request())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        reply.observation,
        Some(pb::log_pressure_response::Observation::AvailableBytes(0))
    );
    debt.mark_uncertain();
    let reply = service
        .inspect_log_pressure_request(f.request())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        reply.observation,
        Some(pb::log_pressure_response::Observation::Uncertain(true))
    );
    debt.reconcile(20).unwrap();
    let reply = service
        .inspect_log_pressure_request(f.request())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        reply.observation,
        Some(pb::log_pressure_response::Observation::AvailableBytes(80))
    );
}

#[derive(Debug)]
struct GatedProbe {
    entered: tokio::sync::Notify,
    release: (std::sync::Mutex<bool>, std::sync::Condvar),
}
impl GatedProbe {
    fn unblock(&self) {
        *self.release.0.lock().unwrap() = true;
        self.release.1.notify_all();
    }
}
impl DiskSpaceProbe for GatedProbe {
    fn available_bytes(&self) -> std::io::Result<u64> {
        self.entered.notify_one();
        let (lock, signal) = &self.release;
        let ready = lock.lock().unwrap();
        // A test failure must not strand the runtime's blocking pool forever.
        let (ready, _) = signal
            .wait_timeout_while(ready, Duration::from_secs(5), |ready| !*ready)
            .unwrap();
        if !*ready {
            return Err(std::io::Error::other("test gate timed out"));
        }
        Ok(100)
    }
    fn check_path(&self, _: &Path) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn pressure_rpc_timeout_keeps_actual_probe_slot_until_blocking_work_finishes() {
    let f = Fixture::new();
    let probe = Arc::new(GatedProbe {
        entered: tokio::sync::Notify::new(),
        release: Default::default(),
    });
    let budget = Arc::new(LogBudget::with_disk_headroom(100, 20, probe.clone()).unwrap());
    let service = Arc::new(f.service(true, true).with_log_budget(budget));
    let mut request = f.request();
    request.get_mut().budget_micros = 50_000;
    let s = service.clone();
    let call = tokio::spawn(async move { s.inspect_log_pressure_request(request).await });
    tokio::time::timeout(Duration::from_secs(1), probe.entered.notified())
        .await
        .unwrap();
    assert_eq!(
        call.await.unwrap().unwrap_err().code(),
        tonic::Code::DeadlineExceeded
    );
    assert_eq!(service.log_pressure_capacity.available_permits(), 0);
    assert_eq!(
        service
            .inspect_log_pressure_request(f.request())
            .await
            .unwrap_err()
            .code(),
        tonic::Code::ResourceExhausted
    );
    probe.unblock();
    tokio::time::timeout(Duration::from_secs(1), async {
        while service.log_pressure_capacity.available_permits() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let reply = service
        .inspect_log_pressure_request(f.request())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        reply.observation,
        Some(pb::log_pressure_response::Observation::AvailableBytes(80))
    );
}

#[tokio::test]
async fn pressure_rpc_rechecks_authority_after_probe_and_cancelled_call_keeps_slot() {
    let f = Fixture::new();
    let discovery = Arc::new(Discovery {
        root: RootWireIdentity {
            root_digest: f.root.digest(),
            bootstrap_generation: f.root.bootstrap_generation,
        },
        fence: Some(f.fence.clone()),
        allowed: AtomicBool::new(true),
    });
    let probe = Arc::new(GatedProbe {
        entered: tokio::sync::Notify::new(),
        release: Default::default(),
    });
    let budget = Arc::new(LogBudget::with_disk_headroom(100, 20, probe.clone()).unwrap());
    let service = Arc::new(
        RaftGrpcService::new(
            NodeId(2),
            crate::work::RaftInbox::default(),
            discovery.clone(),
        )
        .with_log_budget(budget),
    );
    let (s, request) = (service.clone(), f.request());
    let call = tokio::spawn(async move { s.inspect_log_pressure_request(request).await });
    tokio::time::timeout(Duration::from_secs(1), probe.entered.notified())
        .await
        .unwrap();
    discovery.allowed.store(false, Ordering::SeqCst);
    probe.unblock();
    assert_eq!(
        call.await.unwrap().unwrap_err().code(),
        tonic::Code::FailedPrecondition
    );

    discovery.allowed.store(true, Ordering::SeqCst);
    *probe.release.0.lock().unwrap() = false;
    let (s, request) = (service.clone(), f.request());
    let call = tokio::spawn(async move { s.inspect_log_pressure_request(request).await });
    tokio::time::timeout(Duration::from_secs(1), probe.entered.notified())
        .await
        .unwrap();
    call.abort();
    assert!(call.await.unwrap_err().is_cancelled());
    assert_eq!(service.log_pressure_capacity.available_permits(), 0);
    assert_eq!(
        service
            .inspect_log_pressure_request(f.request())
            .await
            .unwrap_err()
            .code(),
        tonic::Code::ResourceExhausted
    );
    probe.unblock();
    tokio::time::timeout(Duration::from_secs(1), async {
        while service.log_pressure_capacity.available_permits() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
