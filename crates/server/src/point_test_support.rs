//! Instrumented point backend shared by unary/stream transport controls.
use crate::api::{
    AdminApi, ClusterInfo, CreateKeyspaceResult, DeleteRangeReceipt, RawApi, RawReadJob,
    RawReadPreparation, RawWrite, RawWritePreparation, RegionLocation, RequestContext,
};
use crate::grpc::{proto, Authenticator, TokenAuthenticator};
use kv9_common::{
    AppliedPosition, Error, Keyspace, KeyspaceId, NodeId, RegionId, Result as ApiResult, TenantId,
    UserKey, Value,
};
use kv9_raw::RawMutation;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    sync::{mpsc, oneshot},
    time::Instant,
};
use tonic::Request;

pub(crate) const APPLIED: AppliedPosition = AppliedPosition { term: 7, index: 19 };

pub(crate) struct WriteGate {
    pub(crate) entered: mpsc::UnboundedSender<()>,
    pub(crate) release: Mutex<Option<oneshot::Receiver<()>>>,
}

pub(crate) struct ReadDropGate {
    pub(crate) entered: mpsc::UnboundedSender<()>,
    pub(crate) finished: mpsc::UnboundedSender<bool>,
    pub(crate) release: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
}

pub(crate) struct ReadGate {
    // false releases a normal read; true injects a panic in async preparation.
    pub(crate) entered: mpsc::UnboundedSender<oneshot::Sender<bool>>,
    pub(crate) dropping: Option<ReadDropGate>,
}

struct ReadWaitGuard(Arc<ReadGate>);

impl Drop for ReadWaitGuard {
    fn drop(&mut self) {
        if let Some(gate) = &self.0.dropping {
            // A failed test may already have dropped its observers. Never panic
            // in a destructor during unwind; the test checks the explicit result.
            let release = gate.release.lock().ok().and_then(|mut slot| slot.take());
            let observed = gate.entered.send(()).is_ok();
            let released =
                release.is_some_and(|release| release.recv_timeout(Duration::from_secs(5)).is_ok());
            let _ = gate.finished.send(observed && released);
        }
    }
}

#[derive(Default)]
pub(crate) struct Backend {
    pub(crate) route_lookup: Option<Arc<RouteLookup>>,
    pub(crate) route_target: Option<Arc<RouteTarget>>,
    pub(crate) activate_prepare: Option<Arc<ActivationPrepare>>,
    pub(crate) ttl_prepare: Option<Arc<TtlPrepare>>,
    pub(crate) scoped: Option<(kv9_common::data_range::DataRange, Arc<dyn RawApi>)>,
    pub(crate) values: Mutex<HashMap<UserKey, Value>>,
    pub(crate) calls: Mutex<Vec<RequestContext>>,
    pub(crate) write_gate: Option<WriteGate>,
    pub(crate) read_gates: HashMap<UserKey, Arc<ReadGate>>,
}
pub(crate) type RouteLookup = dyn Fn(kv9_common::RootDigest, TenantId, KeyspaceId, &[u8]) -> ApiResult<crate::api::RawRouteLookup>
    + Send
    + Sync;
pub(crate) type RouteTarget =
    dyn Fn(&kv9_common::data_range::DataRange) -> ApiResult<Arc<dyn RawApi>> + Send + Sync;
pub(crate) type TtlPrepare = dyn Fn(RequestContext, crate::raw_ttl::Operation, std::time::Instant) -> crate::raw_ttl::Preparation
    + Send
    + Sync;
pub(crate) type ActivationPrepare = dyn Fn(
        String,
        kv9_common::data_range::DataRange,
        std::time::Instant,
    ) -> crate::api::OperationPreparation<AppliedPosition>
    + Send
    + Sync;

impl Backend {
    fn observe(&self, ctx: &RequestContext, key: &[u8]) -> ApiResult<()> {
        self.calls.lock().unwrap().push(ctx.clone());
        if key == b"not-leader" {
            return Err(Error::NotLeader {
                leader: Some(NodeId(3)),
            });
        }
        Ok(())
    }

    fn write(&self, ctx: &RequestContext, op: RawWrite) -> ApiResult<AppliedPosition> {
        match op {
            RawWrite::Put { key, value } => self.raw_put(ctx, key, value),
            RawWrite::Delete { key } => self.raw_delete(ctx, &key),
            RawWrite::BatchPut(pairs) => self.raw_batch_put(ctx, &pairs),
            RawWrite::Batch(mutations) => self.raw_batch(ctx, &mutations),
        }
    }
}

// These APIs are deliberately outside the point adapter. Any accidental call
// returns a typed failure rather than silently succeeding in the fixture.
macro_rules! unsupported {
    ($($name:ident($($arg:ident: $ty:ty),*) -> $ret:ty;)*) => {$ (
        fn $name(&self, $($arg: $ty),*) -> ApiResult<$ret> {
            $(let _ = $arg;)*
            Err(Error::NotImplemented(stringify!($name)))
        }
    )*};
}

impl RawApi for Backend {
    fn prepare_raw_ttl(
        self: Arc<Self>,
        ctx: RequestContext,
        op: crate::raw_ttl::Operation,
        deadline: std::time::Instant,
    ) -> crate::raw_ttl::Preparation {
        match &self.ttl_prepare {
            Some(prepare) => prepare(ctx, op, deadline),
            None => Box::new(|| Err(Error::NotImplemented("Raw TTL protocol"))),
        }
    }
    fn routed_target(
        &self,
        scope: &kv9_common::data_range::DataRange,
    ) -> ApiResult<Arc<dyn RawApi>> {
        if let Some(resolve) = &self.route_target {
            return resolve(scope);
        }
        self.scoped
            .as_ref()
            .filter(|(r, _)| r == scope)
            .map(|(_, target)| target.clone())
            .ok_or(Error::StaleEpoch {
                region: scope.region,
            })
    }
    fn prepare_raw_get(
        self: Arc<Self>,
        ctx: RequestContext,
        key: UserKey,
    ) -> RawReadPreparation<Option<Value>> {
        Box::pin(async move {
            if let Some(gate) = self.read_gates.get(&key) {
                let _guard = ReadWaitGuard(gate.clone());
                let (release, receiver) = oneshot::channel();
                gate.entered
                    .send(release)
                    .map_err(|_| Error::NotImplemented("test read observer dropped"))?;
                assert!(
                    !receiver
                        .await
                        .map_err(|_| Error::NotImplemented("test read control dropped"))?,
                    "controlled async read preparation panic"
                );
            }
            Ok(RawReadJob::Blocking(Box::new(move || {
                self.raw_get(&ctx, &key)
            })))
        })
    }

    fn prepare_raw_write(
        self: Arc<Self>,
        ctx: RequestContext,
        operation: RawWrite,
    ) -> RawWritePreparation {
        Box::new(move || {
            let release = self.write_gate.as_ref().map(|gate| {
                gate.entered.send(()).unwrap();
                gate.release.lock().unwrap().take().expect("one held write")
            });
            Ok(Box::pin(async move {
                if let Some(release) = release {
                    release.await.expect("test must settle its held write");
                }
                self.write(&ctx, operation)
            }))
        })
    }

    fn raw_get(&self, ctx: &RequestContext, key: &[u8]) -> ApiResult<Option<Value>> {
        self.observe(ctx, key)?;
        Ok(self.values.lock().unwrap().get(key).cloned())
    }
    fn raw_put(
        &self,
        ctx: &RequestContext,
        key: UserKey,
        value: Value,
    ) -> ApiResult<AppliedPosition> {
        self.observe(ctx, &key)?;
        self.values.lock().unwrap().insert(key, value);
        Ok(APPLIED)
    }
    fn raw_delete(&self, ctx: &RequestContext, key: &[u8]) -> ApiResult<AppliedPosition> {
        self.observe(ctx, key)?;
        self.values.lock().unwrap().remove(key);
        Ok(APPLIED)
    }
    fn raw_batch_get(
        &self,
        ctx: &RequestContext,
        keys: &[UserKey],
    ) -> ApiResult<Vec<Option<Value>>> {
        self.observe(ctx, keys.first().map(Vec::as_slice).unwrap_or_default())?;
        let values = self.values.lock().unwrap();
        Ok(keys.iter().map(|key| values.get(key).cloned()).collect())
    }
    fn raw_batch_put(
        &self,
        ctx: &RequestContext,
        pairs: &[(UserKey, Value)],
    ) -> ApiResult<AppliedPosition> {
        self.observe(
            ctx,
            pairs
                .first()
                .map(|(key, _)| key.as_slice())
                .unwrap_or_default(),
        )?;
        let mut values = self.values.lock().unwrap();
        for (key, value) in pairs {
            values.insert(key.clone(), value.clone());
        }
        Ok(APPLIED)
    }
    fn raw_batch(
        &self,
        ctx: &RequestContext,
        mutations: &[RawMutation],
    ) -> ApiResult<AppliedPosition> {
        self.observe(
            ctx,
            mutations.first().map(RawMutation::key).unwrap_or_default(),
        )?;
        let mut values = self.values.lock().unwrap();
        for mutation in mutations {
            match mutation {
                RawMutation::Put { key, value } => {
                    values.insert(key.clone(), value.clone());
                }
                RawMutation::Delete { key } => {
                    values.remove(key);
                }
            }
        }
        Ok(APPLIED)
    }
    unsupported! {
        raw_scan(ctx: &RequestContext, start: &[u8], end: &[u8], limit: usize) -> Vec<(UserKey, Value)>;
        raw_delete_range(ctx: &RequestContext, start: &[u8], end: &[u8]) -> DeleteRangeReceipt;
    }
}

impl AdminApi for Backend {
    fn lookup_raw_route(
        &self,
        root: kv9_common::RootDigest,
        tenant: TenantId,
        keyspace: KeyspaceId,
        key: &[u8],
    ) -> ApiResult<crate::api::RawRouteLookup> {
        self.route_lookup
            .as_ref()
            .ok_or(Error::NotImplemented("test route lookup"))?(root, tenant, keyspace, key)
    }
    fn prepare_raw_ttl_activation(
        self: Arc<Self>,
        caller: String,
        binding: kv9_common::data_range::DataRange,
        deadline: std::time::Instant,
    ) -> crate::api::OperationPreparation<AppliedPosition> {
        match &self.activate_prepare {
            Some(prepare) => prepare(caller, binding, deadline),
            None => Box::new(|| Err(Error::NotImplemented("Raw TTL activation"))),
        }
    }
    unsupported! {
        create_keyspace(caller: &str, name: &str, tenant: TenantId, api_type: kv9_common::ApiType) -> CreateKeyspaceResult;
        list_keyspaces(caller: &str) -> Vec<Keyspace>;
        get_region(caller: &str, keyspace: KeyspaceId, key: &[u8]) -> RegionLocation;
        split_region(caller: &str, region: RegionId, split_key: UserKey) -> ();
        cluster_info(caller: &str) -> ClusterInfo;
    }
}

pub(crate) fn authenticator() -> Arc<dyn Authenticator> {
    Arc::new(TokenAuthenticator::new([("secret", "alice")]).unwrap())
}
pub(crate) fn context_message() -> proto::RequestContext {
    proto::RequestContext {
        keyspace_id: 7,
        region_epoch: Some(proto::RegionEpoch {
            conf_ver: 11,
            version: 13,
        }),
    }
}
pub(crate) fn get(key: &[u8]) -> proto::RawGetRequest {
    proto::RawGetRequest {
        context: Some(context_message()),
        key: key.to_vec(),
    }
}
pub(crate) fn put(key: &[u8], value: &[u8]) -> proto::RawPutRequest {
    proto::RawPutRequest {
        context: Some(context_message()),
        key: key.to_vec(),
        value: value.to_vec(),
    }
}
pub(crate) fn request<T>(message: T) -> Request<T> {
    let mut request = Request::new(message);
    request
        .metadata_mut()
        .insert("authorization", "Bearer secret".parse().unwrap());
    request
}
pub(crate) fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

/// Actual tonic HTTP/2 framing over Tokio duplex I/O. No socket bind, TCP,
/// network failure simulation or deployment acceptance is implied.
pub(crate) async fn memory_channel(
    api: crate::grpc::Kv9Grpc,
) -> (tonic::transport::Channel, tokio::task::JoinHandle<()>) {
    let (incoming, receive) = mpsc::channel(8);
    let auth = authenticator();
    let service = proto::kv9_server::Kv9Server::with_interceptor(
        api.clone(),
        crate::grpc::AuthInterceptor::new(auth.clone()),
    );
    let stream =
        crate::point_stream::service(api, auth, tokio_util::sync::CancellationToken::new());
    let server = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(service)
            .add_service(stream)
            .serve_with_incoming(tokio_stream::wrappers::ReceiverStream::new(receive))
            .await
            .unwrap();
    });
    let channel = tonic::transport::Endpoint::from_static("http://memory.test")
        .connect_with_connector(tower::service_fn(move |_| {
            let incoming = incoming.clone();
            async move {
                let (client, server) = tokio::io::duplex(64 * 1024);
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
