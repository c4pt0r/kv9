use super::*;
use kv9_common::{KeyspaceId, RegionId, TenantId};
fn binding() -> DataRange {
    DataRange {
        root: RootDigest::from_bytes([1; 32]),
        creation: RootDigest::from_bytes([2; 32]),
        region: RegionId(100),
        keyspace: KeyspaceId(100),
        tenant: TenantId(1),
        conf_ver: 1,
        version: 1,
        start: vec![],
        end: vec![],
        sealed: false,
    }
}
fn directory() -> RawTtlDirectory {
    RawTtlDirectory {
        cluster: ClusterId::from_bytes([3; 16]),
        root: binding().root,
        binding_digest: binding().digest(),
        stores: vec![RawTtlStore {
            node: NodeId(1),
            incarnation: StoreIncarnation::from_bytes([4; 16]),
            address: "127.0.0.1:21001".parse().unwrap(),
            generation: 0,
            previous_address: None,
        }],
    }
}
#[test]
fn directory_wire_checks_full_scope_members_and_endpoint_generations() {
    let b = binding();
    let ids = [NodeId(1)];
    let valid = directory().encode();
    assert_eq!(
        RawTtlDirectory::decode(valid.clone(), &b, &ids).unwrap(),
        directory()
    );
    for mode in 0..10 {
        let mut bad = valid.clone();
        match mode {
            0 => bad.cluster_id = vec![0; 16],
            1 => bad.root_digest[0] ^= 1,
            2 => bad.binding_digest[0] ^= 1,
            3 => bad.stores.clear(),
            4 => bad.stores.push(bad.stores[0].clone()),
            5 => bad.stores[0].node_id = 2,
            6 => bad.stores[0].store_incarnation = vec![0; 16],
            7 => bad.stores[0].address = "0.0.0.0:21001".into(),
            8 => bad.stores[0].generation = 1,
            9 => bad.stores[0].previous_address = Some("127.0.0.1:21000".into()),
            _ => unreachable!(),
        }
        assert!(
            RawTtlDirectory::decode(bad, &b, &ids).is_err(),
            "mode {mode}"
        );
    }
    for ids in [
        vec![],
        vec![NodeId(0)],
        vec![NodeId(1), NodeId(1)],
        vec![NodeId(2), NodeId(1)],
    ] {
        assert!(validate_query(&b, &ids).is_err());
    }
}
struct Discovery;
impl GrpcDiscoveryState for Discovery {
    fn answer(&self) -> (NodeId, bool, u64) {
        (NodeId(2), true, 0)
    }
    fn raft_receive_allowed(&self) -> bool {
        true
    }
    fn root_identity(&self) -> RootWireIdentity {
        RootWireIdentity {
            bootstrap_generation: BootstrapGeneration::from_bytes([5; 16]),
            root_digest: binding().root,
        }
    }
}
struct Backend;
impl RegistrationBackend for Backend {
    fn raw_ttl_directory(
        &self,
        binding: &DataRange,
        _: &[NodeId],
        _: Instant,
    ) -> kv9_common::Result<RawTtlDirectory> {
        let mut observation = directory();
        observation.root = binding.root;
        observation.binding_digest = binding.digest();
        Ok(observation)
    }
    fn register(
        &self,
        _: NodeId,
        _: &str,
        _: ClusterId,
        _: &[u8],
        _: StoreIncarnation,
    ) -> std::result::Result<RegistrationReceipt, RegistrationError> {
        Err(RegistrationError::Failed(invalid()))
    }
}
fn request() -> Request<pb::RawTtlStoresRequest> {
    let mut request = Request::new(pb::RawTtlStoresRequest {
        binding: binding().encode(),
        replicas: vec![1],
        budget_micros: 1_000_000,
    });
    request.extensions_mut().insert(NodeId(1));
    request
}
#[tokio::test]
async fn directory_rpc_requires_authenticated_node_and_bounded_canonical_query() {
    let service = RaftGrpcService::new(
        NodeId(2),
        crate::work::RaftInbox::default(),
        Arc::new(Discovery),
    )
    .with_registration(Arc::new(Backend));
    for mode in 0..6 {
        let mut r = request();
        match mode {
            0 => {
                r.extensions_mut().remove::<NodeId>();
            }
            1 => {
                r.extensions_mut().insert(NodeId(0));
            }
            2 => r.get_mut().replicas = vec![1, 1],
            3 => r.get_mut().budget_micros = 0,
            4 => r.get_mut().budget_micros = 30_000_001,
            5 => {
                let mut b = binding();
                b.root = RootDigest::from_bytes([8; 32]);
                r.get_mut().binding = b.encode();
            }
            _ => unreachable!(),
        }
        assert!(service.resolve_raw_ttl_stores(r).await.is_err());
    }
    let response = service
        .resolve_raw_ttl_stores(request())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        RawTtlDirectory::decode(response, &binding(), &[NodeId(1)]).unwrap(),
        directory()
    );
}

#[tokio::test]
async fn directory_request_accepts_the_maximum_canonical_range_binding() {
    let service = RaftGrpcService::new(
        NodeId(2),
        crate::work::RaftInbox::default(),
        Arc::new(Discovery),
    )
    .with_registration(Arc::new(Backend));
    let mut b = binding();
    b.start = vec![1; kv9_common::data_range::MAX_BOUND];
    b.end = vec![2; kv9_common::data_range::MAX_BOUND];
    let mut query = request();
    query.get_mut().binding = b.encode();
    assert!(query.get_ref().encoded_len() > 16 * 1024);
    let response = service
        .resolve_raw_ttl_stores(query)
        .await
        .unwrap()
        .into_inner();
    assert!(RawTtlDirectory::decode(response, &b, &[NodeId(1)]).is_ok());
}

struct HeldDirectory {
    entered: tokio::sync::mpsc::UnboundedSender<()>,
    released: std::sync::Mutex<bool>,
    wake: std::sync::Condvar,
}
impl RegistrationBackend for HeldDirectory {
    fn raw_ttl_directory(
        &self,
        _: &DataRange,
        _: &[NodeId],
        _: Instant,
    ) -> kv9_common::Result<RawTtlDirectory> {
        let _ = self.entered.send(());
        let (released, _) = self
            .wake
            .wait_timeout_while(self.released.lock().unwrap(), Duration::from_secs(5), |r| {
                !*r
            })
            .unwrap();
        if !*released {
            return Err(invalid());
        }
        Ok(directory())
    }
    fn register(
        &self,
        _: NodeId,
        _: &str,
        _: ClusterId,
        _: &[u8],
        _: StoreIncarnation,
    ) -> std::result::Result<RegistrationReceipt, RegistrationError> {
        Err(RegistrationError::Failed(invalid()))
    }
}

#[tokio::test]
async fn cancelled_directory_requests_keep_all_four_worker_permits_until_exit() {
    let (entered, mut started) = tokio::sync::mpsc::unbounded_channel();
    let backend = Arc::new(HeldDirectory {
        entered,
        released: std::sync::Mutex::new(false),
        wake: std::sync::Condvar::new(),
    });
    let service = Arc::new(
        RaftGrpcService::new(
            NodeId(2),
            crate::work::RaftInbox::default(),
            Arc::new(Discovery),
        )
        .with_registration(backend.clone()),
    );
    let mut requests = Vec::new();
    for _ in 0..4 {
        let service = service.clone();
        requests.push(tokio::spawn(async move {
            service.resolve_raw_ttl_stores(request()).await
        }));
    }
    for _ in 0..4 {
        tokio::time::timeout(Duration::from_secs(2), started.recv())
            .await
            .unwrap()
            .unwrap();
    }
    for task in requests {
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
    }
    assert_eq!(service.ttl_directory_capacity.available_permits(), 0);
    assert_eq!(
        service
            .resolve_raw_ttl_stores(request())
            .await
            .unwrap_err()
            .code(),
        tonic::Code::ResourceExhausted
    );
    *backend.released.lock().unwrap() = true;
    backend.wake.notify_all();
    tokio::time::timeout(Duration::from_secs(2), async {
        while service.ttl_directory_capacity.available_permits() != 4 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(service.resolve_raw_ttl_stores(request()).await.is_ok());
}
