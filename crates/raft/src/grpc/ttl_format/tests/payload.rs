//! Real generated V9 clients and authenticated HTTP/2 duplex transport.
use super::*;
use crate::grpc::migration_payload::{PayloadExport, MAX_CHUNK_BYTES};
use kv9_common::RegionId;
use std::io::{Cursor, Read};
use std::sync::{atomic::AtomicUsize, Condvar, Mutex};
use std::time::Instant;

#[derive(Default)]
struct Gate {
    entered: tokio::sync::Notify,
    drained: tokio::sync::Notify,
    release: (Mutex<bool>, Condvar),
}
impl Gate {
    fn block(&self) -> kv9_common::Result<()> {
        self.entered.notify_one();
        let (lock, signal) = &self.release;
        let (released, _) = signal
            .wait_timeout_while(lock.lock().unwrap(), Duration::from_secs(5), |v| !*v)
            .unwrap();
        if !*released {
            return Err(Error::MetaNotReady("test gate timeout".into()));
        }
        Ok(())
    }
    fn unblock(&self) {
        *self.release.0.lock().unwrap() = true;
        self.release.1.notify_all();
    }
}
struct Reader {
    gate: Arc<Gate>,
    first: bool,
    bytes: Cursor<Vec<u8>>,
}
impl Read for Reader {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if self.first {
            self.first = false;
            self.gate
                .block()
                .map_err(|e| std::io::Error::other(e.to_string()))?;
        }
        self.bytes.read(bytes)
    }
}
impl Drop for Reader {
    fn drop(&mut self) {
        self.gate.drained.notify_one();
    }
}
#[derive(Default)]
struct Backend {
    gate: Arc<Gate>,
    calls: AtomicUsize,
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
        panic!("payload RPC cannot register a node")
    }
    fn export_migration_capture(
        &self,
        _: RootDigest,
        region: RegionId,
        operation: [u8; 16],
        digest: RootDigest,
        _: Instant,
    ) -> kv9_common::Result<PayloadExport> {
        assert_eq!(
            (region.0, operation, *digest.as_bytes()),
            (100, [3; 16], [4; 32])
        );
        self.calls.fetch_add(1, Ordering::SeqCst);
        let length = MAX_CHUNK_BYTES * 2 + 17;
        Ok(PayloadExport {
            reader: Box::new(Reader {
                gate: self.gate.clone(),
                first: true,
                bytes: Cursor::new(vec![9; length]),
            }),
            length: length as u64,
        })
    }
    fn replicate_migration_capture(
        &self,
        _: RootDigest,
        _: RegionId,
        _: [u8; 16],
        _: RootDigest,
        _: StoreIdentity,
        _: Instant,
    ) -> kv9_common::Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let result = self.gate.block();
        self.gate.drained.notify_one();
        result
    }
}
fn query(f: &Fixture, copy: bool) -> pb::MigrationCaptureTransferRequest {
    let identity = f.fence.identity();
    pb::MigrationCaptureTransferRequest {
        store_identity: identity.canonical_bytes(),
        region_id: 100,
        operation_id: vec![3; 16],
        descriptor_digest: vec![4; 32],
        challenge: vec![5; 16],
        budget_micros: 3_000_000,
        source_store_identity: if copy {
            StoreIdentity {
                node_id: NodeId(1),
                ..identity
            }
            .canonical_bytes()
        } else {
            vec![]
        },
    }
}
fn auth(
    value: pb::MigrationCaptureTransferRequest,
) -> Request<pb::MigrationCaptureTransferRequest> {
    let mut request = Request::new(value);
    attach_auth(&mut request, &Some("payload-test".into()), NodeId(1));
    request
}
async fn notified(notify: &tokio::sync::Notify) {
    tokio::time::timeout(Duration::from_secs(3), notify.notified())
        .await
        .unwrap();
}
async fn drained(capacity: &tokio::sync::Semaphore) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while capacity.available_permits() != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cancelled_http2_fetch_keeps_actual_reader_and_capacity_until_drain() {
    let f = Fixture::new();
    let backend = Arc::new(Backend::default());
    let discovery = f.discovery(true, true);
    let service = service(discovery.clone()).with_registration(backend.clone());
    let capacity = service.checkpoint_history_capacity.clone();
    let (channel, server) =
        super::upgrade::connect(tonic::transport::Server::builder().add_service(
            pb::kv9_raft_server::Kv9RaftServer::with_interceptor(
                service,
                cluster_token_interceptor("payload-test".into()),
            ),
        ))
        .await;
    let mut client = Kv9RaftClient::new(channel);
    let stream = client
        .fetch_migration_capture(auth(query(&f, false)))
        .await
        .unwrap()
        .into_inner();
    notified(&backend.gate.entered).await;
    drop(stream);
    assert_eq!(capacity.available_permits(), 0);
    assert_eq!(
        client
            .fetch_migration_capture(auth(query(&f, false)))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::ResourceExhausted
    );
    assert_eq!(backend.calls.load(Ordering::SeqCst), 1);
    backend.gate.unblock();
    notified(&backend.gate.drained).await;
    drained(&capacity).await;
    let mut stream = client
        .fetch_migration_capture(auth(query(&f, false)))
        .await
        .unwrap()
        .into_inner();
    let mut offset = 0;
    let mut frames = 0;
    while let Some(frame) = stream.message().await.unwrap() {
        assert_eq!(frame.offset, offset);
        assert_eq!(frame.challenge, [5; 16]);
        assert_eq!(frame.descriptor_digest, [4; 32]);
        assert_eq!(frame.total_bytes, (MAX_CHUNK_BYTES * 2 + 17) as u64);
        assert!(frame.payload.len() <= MAX_CHUNK_BYTES);
        assert!(frame.payload.iter().all(|b| *b == 9));
        offset += frame.payload.len() as u64;
        assert_eq!(frame.last, offset == frame.total_bytes);
        frames += 1;
    }
    assert_eq!((frames, offset), (3, (MAX_CHUNK_BYTES * 2 + 17) as u64));
    drained(&capacity).await;
    discovery.allowed.store(false, Ordering::SeqCst);
    assert_eq!(
        client
            .fetch_migration_capture(auth(query(&f, false)))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::FailedPrecondition
    );
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn cancelled_or_expired_http2_copy_keeps_worker_capacity_until_actual_completion() {
    for deadline in [false, true] {
        let f = Fixture::new();
        let backend = Arc::new(Backend::default());
        let service = service(f.discovery(true, true)).with_registration(backend.clone());
        let capacity = service.checkpoint_history_capacity.clone();
        let (channel, server) =
            super::upgrade::connect(tonic::transport::Server::builder().add_service(
                pb::kv9_raft_server::Kv9RaftServer::with_interceptor(
                    service,
                    cluster_token_interceptor("payload-test".into()),
                ),
            ))
            .await;
        let mut client = Kv9RaftClient::new(channel);
        let mut copying_client = client.clone();
        let mut value = query(&f, true);
        if deadline {
            value.budget_micros = 50_000;
        }
        let task = tokio::spawn(async move {
            copying_client
                .replicate_migration_capture(auth(value))
                .await
        });
        notified(&backend.gate.entered).await;
        if deadline {
            assert_eq!(
                task.await.unwrap().unwrap_err().code(),
                tonic::Code::DeadlineExceeded
            );
        } else {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        }
        assert_eq!(capacity.available_permits(), 0);
        assert_eq!(
            client
                .fetch_migration_capture(auth(query(&f, false)))
                .await
                .unwrap_err()
                .code(),
            tonic::Code::ResourceExhausted
        );
        backend.gate.unblock();
        notified(&backend.gate.drained).await;
        drained(&capacity).await;
        let reply = client
            .replicate_migration_capture(auth(query(&f, true)))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(reply.store_identity, f.fence.identity().canonical_bytes());
        assert_eq!(reply.region_id, 100);
        assert_eq!(reply.operation_id, [3; 16]);
        assert_eq!(reply.challenge, [5; 16]);
        assert_eq!(reply.descriptor_digest, Some(vec![4; 32]));
        assert_eq!(backend.calls.load(Ordering::SeqCst), 2);
        server.abort();
        let _ = server.await;
    }
}

#[tokio::test]
async fn transfer_admission_refuses_bad_identity_auth_scope_and_bounds_before_io() {
    let f = Fixture::new();
    let backend = Arc::new(Backend::default());
    let service = service(f.discovery(true, true)).with_registration(backend.clone());
    assert_eq!(
        service
            .fetch_migration_payload(Request::new(query(&f, false)))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unauthenticated
    );
    for case in 0..11 {
        let mut value = query(&f, false);
        match case {
            0 => value.region_id = 99,
            1 => value.operation_id.fill(0),
            2 => value.descriptor_digest.fill(0),
            3 => value.challenge.fill(0),
            4 => value.budget_micros = 0,
            5 => value.budget_micros = 120_000_001,
            6 => value.source_store_identity = f.fence.identity().canonical_bytes(),
            7 => value.store_identity[0] ^= 1,
            8 => value.store_identity.push(0),
            9 => value.challenge.resize(600, 1),
            _ => value.operation_id.pop().map(|_| ()).unwrap(),
        }
        let mut request = Request::new(value);
        request.extensions_mut().insert(NodeId(1));
        assert!(
            service.fetch_migration_payload(request).await.is_err(),
            "case {case}"
        );
    }
    for case in 0..3 {
        let mut value = query(&f, true);
        match case {
            0 => value.source_store_identity = value.store_identity.clone(),
            1 => value.source_store_identity[0] ^= 1,
            _ => value.source_store_identity.clear(),
        }
        let mut request = Request::new(value);
        request.extensions_mut().insert(NodeId(1));
        assert!(service.replicate_migration_payload(request).await.is_err());
    }
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
    assert_eq!(service.checkpoint_history_capacity.available_permits(), 1);
}
