//! Instrumented point backend shared by unary/stream transport controls.
use crate::api::{
    AdminApi, ClusterInfo, CreateKeyspaceResult, DeleteRangeReceipt, RawApi, RawReadJob,
    RawReadPreparation, RawWrite, RawWritePreparation, RegionLocation, RequestContext, TxnApi,
};
use crate::grpc::{proto, Authenticator, TokenAuthenticator};
use kv9_common::{
    AppliedPosition, Error, Keyspace, KeyspaceId, NodeId, RegionId, Result as ApiResult, TenantId,
    TxnGroupId, UserKey, Value,
};
use kv9_txn::{QualifiedKey, TxnDescriptor, TxnStatus};
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
    pub(crate) values: Mutex<HashMap<UserKey, Value>>,
    pub(crate) calls: Mutex<Vec<RequestContext>>,
    pub(crate) write_gate: Option<WriteGate>,
    pub(crate) read_gates: HashMap<UserKey, Arc<ReadGate>>,
}

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
    unsupported! {
        raw_scan(ctx: &RequestContext, start: &[u8], end: &[u8], limit: usize) -> Vec<(UserKey, Value)>;
        raw_delete_range(ctx: &RequestContext, start: &[u8], end: &[u8]) -> DeleteRangeReceipt;
    }
}

impl TxnApi for Backend {
    unsupported! {
        kv_begin(ctx: &RequestContext, primary: QualifiedKey) -> TxnDescriptor;
        kv_get(ctx: &RequestContext, key: &[u8], transaction: &TxnDescriptor) -> Option<Value>;
        kv_batch_get(ctx: &RequestContext, keys: &[UserKey], transaction: &TxnDescriptor) -> Vec<Option<Value>>;
        kv_scan(ctx: &RequestContext, start: &[u8], end: &[u8], limit: usize, transaction: &TxnDescriptor) -> Vec<(UserKey, Value)>;
        kv_prewrite(ctx: &RequestContext, mutations: &[(UserKey, Option<Value>)], transaction: &TxnDescriptor) -> ();
        kv_commit(ctx: &RequestContext, keys: &[UserKey], transaction: &TxnDescriptor) -> ();
        kv_pessimistic_lock(ctx: &RequestContext, keys: &[UserKey], transaction: &TxnDescriptor) -> ();
        kv_pessimistic_rollback(ctx: &RequestContext, keys: &[UserKey], transaction: &TxnDescriptor) -> ();
        kv_resolve_lock(ctx: &RequestContext, transaction: &TxnDescriptor) -> ();
        kv_cleanup(ctx: &RequestContext, key: &[u8], transaction: &TxnDescriptor) -> ();
        kv_check_txn_status(ctx: &RequestContext, transaction: &TxnDescriptor) -> TxnStatus;
    }
}
impl AdminApi for Backend {
    unsupported! {
        create_keyspace(caller: &str, name: &str, tenant: TenantId, api_type: kv9_common::ApiType, txn_group: TxnGroupId) -> CreateKeyspaceResult;
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
