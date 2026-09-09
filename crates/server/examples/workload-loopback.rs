//! Bounded loopback calibration endpoint. No Raft, WAL, object store or durability.
//! Synthetic receipts identify serialized in-memory effects only.
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kv9_server::proto;
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status};

#[derive(Default)]
struct Data {
    values: BTreeMap<Vec<u8>, Vec<u8>>,
    index: u64,
}

#[derive(Default)]
struct State {
    data: Mutex<Data>,
    requests: [AtomicU64; 3],
    connections: AtomicU64,
}

#[derive(Clone)]
struct Service(Arc<State>);

fn context(context: &Option<proto::RequestContext>, key: &[u8]) -> Result<(), Status> {
    if key.len() > 128
        || !context.as_ref().is_some_and(|c| {
            c.keyspace_id == 1
                && c.region_epoch
                    .as_ref()
                    .is_some_and(|e| e.conf_ver == 1 && e.version == 1)
        })
    {
        return Err(Status::invalid_argument(
            "calibration requires keyspace 1, epoch 1/1 and bounded keys",
        ));
    }
    Ok(())
}

impl Service {
    fn write(
        &self,
        key: Vec<u8>,
        value: Option<Vec<u8>>,
    ) -> Result<Response<proto::RawWriteResponse>, Status> {
        if value.as_ref().is_some_and(|v| v.len() > 8192) {
            return Err(Status::invalid_argument(
                "calibration value exceeds 8192 bytes",
            ));
        }
        let mut data = self.0.data.lock().unwrap();
        if value.is_some() && !data.values.contains_key(&key) && data.values.len() >= 4096 {
            return Err(Status::resource_exhausted(
                "calibration dataset exceeds 4096 keys",
            ));
        }
        if let Some(value) = value {
            data.values.insert(key, value);
        } else {
            data.values.remove(&key);
        }
        data.index += 1;
        Ok(Response::new(proto::RawWriteResponse {
            applied_term: 1,
            applied_index: data.index,
        }))
    }
}

macro_rules! service {
    ($( $name:ident : $request:ident => $response:ident ),* $(,)?) => {
        #[tonic::async_trait]
        impl proto::kv9_server::Kv9 for Service {
            async fn raw_get(&self, request: Request<proto::RawGetRequest>) -> Result<Response<proto::RawGetResponse>, Status> {
                self.0.requests[0].fetch_add(1, Ordering::Relaxed);
                let r = request.into_inner();
                context(&r.context, &r.key)?;
                let value = self.0.data.lock().unwrap().values.get(&r.key).cloned();
                Ok(Response::new(proto::RawGetResponse { value: Some(proto::OptionalValue {
                    found: value.is_some(), value: value.unwrap_or_default(),
                }) }))
            }
            async fn raw_put(&self, request: Request<proto::RawPutRequest>) -> Result<Response<proto::RawWriteResponse>, Status> {
                self.0.requests[1].fetch_add(1, Ordering::Relaxed);
                let r = request.into_inner();
                context(&r.context, &r.key)?;
                self.write(r.key, Some(r.value))
            }
            async fn raw_delete(&self, request: Request<proto::RawDeleteRequest>) -> Result<Response<proto::RawWriteResponse>, Status> {
                self.0.requests[2].fetch_add(1, Ordering::Relaxed);
                let r = request.into_inner();
                context(&r.context, &r.key)?;
                self.write(r.key, None)
            }
            $(async fn $name(&self, _: Request<proto::$request>) -> Result<Response<proto::$response>, Status> {
                Err(Status::unimplemented("calibration supports point operations only"))
            })*
        }
    }
}

service! {
    raw_batch_get: RawBatchGetRequest => RawBatchGetResponse,
    raw_batch_put: RawBatchPutRequest => RawWriteResponse,
    raw_scan: RawScanRequest => ScanResponse,
    raw_delete_range: RawDeleteRangeRequest => RawDeleteRangeResponse,
    kv_begin: KvBeginRequest => KvBeginResponse,
    kv_get: KvGetRequest => KvGetResponse,
    kv_batch_get: KvBatchGetRequest => KvBatchGetResponse,
    kv_scan: KvScanRequest => ScanResponse,
    kv_prewrite: KvPrewriteRequest => Empty,
    kv_commit: KvCommitRequest => Empty,
    kv_pessimistic_lock: KvPessimisticLockRequest => Empty,
    kv_pessimistic_rollback: KvPessimisticRollbackRequest => Empty,
    kv_resolve_lock: KvResolveLockRequest => Empty,
    kv_cleanup: KvCleanupRequest => Empty,
    kv_check_txn_status: KvCheckTxnStatusRequest => KvCheckTxnStatusResponse,
    create_keyspace: CreateKeyspaceRequest => CreateKeyspaceResponse,
    list_keyspaces: ListKeyspacesRequest => ListKeyspacesResponse,
    get_region: GetRegionRequest => GetRegionResponse,
    split_region: SplitRegionRequest => Empty,
    cluster_info: ClusterInfoRequest => ClusterInfoResponse,
    admit_node: AdmitNodeRequest => MembershipChangeResponse,
    promote_node: PromoteNodeRequest => MembershipChangeResponse,
}

fn publish(path: &Path, value: serde_json::Value) -> std::io::Result<()> {
    let temporary = path.with_extension("tmp");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(&serde_json::to_vec(&value)?)?;
    file.sync_all()?;
    std::fs::rename(temporary, path)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 {
        return Err("usage: workload-loopback READY_FILE STOP_FILE SUMMARY_FILE".into());
    }
    let token = std::env::var("KV9_CLIENT_TOKEN")?;
    if !(16..=256).contains(&token.len()) {
        return Err("invalid calibration token length".into());
    }
    let expected = format!("Bearer {token}");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let state = Arc::new(State::default());
    let connections = state.clone();
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener).map(move |r| {
        if r.is_ok() {
            connections.connections.fetch_add(1, Ordering::Relaxed);
        }
        r
    });
    let service = proto::kv9_server::Kv9Server::with_interceptor(
        Service(state.clone()),
        move |r: Request<()>| {
            if r.metadata()
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                != Some(expected.as_str())
            {
                return Err(Status::unauthenticated("invalid calibration credential"));
            }
            Ok(r)
        },
    );
    publish(
        Path::new(&args[0]),
        serde_json::json!({
            "version": 1, "kind": "in_memory_loopback_calibration", "address": address.to_string(),
            "pid": std::process::id(), "runtime_threads": 2, "max_keys": 4096,
            "max_key_bytes": 128, "max_value_bytes": 8192, "durability": false,
        }),
    )?;
    let stop = args[1].clone();
    tonic::transport::Server::builder()
        .concurrency_limit_per_connection(64)
        .add_service(service)
        .serve_with_incoming_shutdown(incoming, async move {
            let end = Instant::now() + Duration::from_secs(900);
            while !Path::new(&stop).exists() && Instant::now() < end {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await?;
    let data = state.data.lock().unwrap();
    publish(
        Path::new(&args[2]),
        serde_json::json!({
            "version": 1, "kind": "in_memory_loopback_calibration", "durability": false,
            "pid": std::process::id(), "requests": state.requests.each_ref().map(|n| n.load(Ordering::Relaxed)),
            "connections": state.connections.load(Ordering::Relaxed),
            "keys": data.values.len(), "synthetic_write_index": data.index,
        }),
    )?;
    Ok(())
}
