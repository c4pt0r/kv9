//! Adapter tests use an instrumented in-memory backend, not a Raft cluster.
//! They establish wire/handler equivalence and reservation ownership only.

use super::*;
use crate::admission::PublicApiLimits;
use crate::api::{
    AdminApi, ClusterInfo, CreateKeyspaceResult, DeleteRangeReceipt, RawApi, RawWrite,
    RawWritePreparation, RegionLocation, RequestContext, TxnApi,
};
use crate::grpc::{
    AuthContext, AuthKind, TokenAuthenticator, ADMISSION_REFUSED_KEY, LEADER_HINT_KEY,
    NOT_LEADER_KEY,
};
use kv9_common::{
    AppliedPosition, Error, Keyspace, KeyspaceId, NodeId, RegionId, Result as ApiResult, TenantId,
    TxnGroupId, UserKey, Value,
};
use kv9_txn::{QualifiedKey, TxnDescriptor, TxnStatus};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tonic::metadata::MetadataValue;
use tonic::Code;

pub(super) const APPLIED: AppliedPosition = AppliedPosition { term: 7, index: 19 };

pub(super) struct WriteGate {
    pub(super) entered: mpsc::UnboundedSender<()>,
    pub(super) release: Mutex<Option<oneshot::Receiver<()>>>,
}

#[derive(Default)]
pub(super) struct Backend {
    pub(super) values: Mutex<HashMap<UserKey, Value>>,
    pub(super) calls: Mutex<Vec<RequestContext>>,
    pub(super) write_gate: Option<WriteGate>,
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
            RawWrite::BatchPut(_) => panic!("point adapter must not dispatch a batch"),
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
    unsupported! {
        raw_batch_get(ctx: &RequestContext, keys: &[UserKey]) -> Vec<Option<Value>>;
        raw_batch_put(ctx: &RequestContext, pairs: &[(UserKey, Value)]) -> AppliedPosition;
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

pub(super) fn authenticator() -> Arc<dyn Authenticator> {
    Arc::new(TokenAuthenticator::new([("secret", "alice")]).unwrap())
}
pub(super) fn context_message() -> proto::RequestContext {
    proto::RequestContext {
        keyspace_id: 7,
        region_epoch: Some(proto::RegionEpoch {
            conf_ver: 11,
            version: 13,
        }),
    }
}
pub(super) fn get(key: &[u8]) -> proto::RawGetRequest {
    proto::RawGetRequest {
        context: Some(context_message()),
        key: key.to_vec(),
    }
}
pub(super) fn put(key: &[u8], value: &[u8]) -> proto::RawPutRequest {
    proto::RawPutRequest {
        context: Some(context_message()),
        key: key.to_vec(),
        value: value.to_vec(),
    }
}
pub(super) fn request<T>(message: T) -> Request<T> {
    let mut request = Request::new(message);
    request
        .metadata_mut()
        .insert("authorization", "Bearer secret".parse().unwrap());
    request
}
fn direct_request<T>(message: T) -> Request<T> {
    let mut request = request(message);
    request.extensions_mut().insert(AuthContext {
        principal: Arc::from("alice"),
        node_id: None,
        auth_kind: AuthKind::Client,
    });
    request
}
pub(super) fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

pub(super) fn control_metadata() -> MetadataMap {
    let mut metadata = MetadataMap::new();
    metadata.append(NOT_LEADER_KEY, "true".parse().unwrap());
    metadata.append(LEADER_HINT_KEY, "3".parse().unwrap());
    metadata.append(LEADER_HINT_KEY, "4".parse().unwrap());
    metadata.append_bin("control-bin", MetadataValue::from_bytes(&[0, 0xff, b'\n']));
    metadata.append_bin("control-bin", MetadataValue::from_bytes(&[0x80, 0]));
    metadata
}
pub(super) fn assert_control_metadata(metadata: &MetadataMap) {
    assert_eq!(metadata.get(NOT_LEADER_KEY).unwrap(), "true");
    assert_eq!(
        metadata
            .get_all(LEADER_HINT_KEY)
            .iter()
            .map(|v| v.to_str().unwrap())
            .collect::<Vec<_>>(),
        ["3", "4"]
    );
    assert_eq!(
        metadata
            .get_all_bin("control-bin")
            .iter()
            .map(|v| v.to_bytes().unwrap().to_vec())
            .collect::<Vec<_>>(),
        [vec![0, 0xff, b'\n'], vec![0x80, 0]]
    );
}

#[test]
fn protobuf_success_preserves_presence_empty_value_and_duplicate_binary_metadata() {
    for value in [
        None,
        Some(proto::OptionalValue {
            found: false,
            value: vec![],
        }),
        Some(proto::OptionalValue {
            found: true,
            value: vec![],
        }),
        Some(proto::OptionalValue {
            found: true,
            value: vec![0, 0xff, 0x80],
        }),
    ] {
        let expected = proto::RawGetResponse { value };
        let mut response = Response::new(expected.clone());
        *response.metadata_mut() = control_metadata();
        let wire = WireReply::encode(Ok(response));
        let actual: Response<proto::RawGetResponse> = wire.decode().unwrap();
        assert_eq!(actual.get_ref(), &expected);
        assert_control_metadata(actual.metadata());
    }
}

#[test]
fn typed_status_round_trip_preserves_details_and_all_control_metadata() {
    for code in [
        Code::FailedPrecondition,
        Code::ResourceExhausted,
        Code::DeadlineExceeded,
        Code::Unavailable,
    ] {
        let status = Status::with_details_and_metadata(
            code,
            "typed refusal",
            vec![0, 0xff, 0x80].into(),
            control_metadata(),
        );
        let actual = WireReply::error(status)
            .decode::<proto::RawWriteResponse>()
            .unwrap_err();
        assert_eq!(actual.code(), code);
        assert_eq!(actual.message(), "typed refusal");
        assert_eq!(actual.details(), &[0, 0xff, 0x80]);
        assert_control_metadata(actual.metadata());
    }
}

#[test]
fn malformed_reply_is_never_decoded_as_success_or_a_retryable_typed_refusal() {
    fn success() -> WireReply {
        WireReply::encode(Ok(Response::new(proto::RawWriteResponse {
            applied_term: 7,
            applied_index: 19,
        })))
    }
    let mut cases = Vec::new();
    for code in [-1, 17] {
        let mut w = success();
        w.code = code;
        cases.push(w);
    }
    let mut w = success();
    w.message = "unexpected error".into();
    cases.push(w);
    let mut w = success();
    w.details = vec![1];
    cases.push(w);
    let mut w = success();
    w.payload = vec![0xff];
    cases.push(w);
    let mut w = success();
    w.payload.resize(crate::client::MAX_MESSAGE_BYTES + 1, 0);
    cases.push(w);
    let mut w = success();
    w.code = Code::FailedPrecondition as i32;
    cases.push(w);
    for metadata in [
        WireMetadata {
            key: "bad key".into(),
            value: b"true".to_vec(),
            binary: false,
        },
        WireMetadata {
            key: NOT_LEADER_KEY.into(),
            value: b"true\n".to_vec(),
            binary: false,
        },
        WireMetadata {
            key: "control".into(),
            value: vec![0xff],
            binary: true,
        },
        WireMetadata {
            key: "control-bin".into(),
            value: b"x".to_vec(),
            binary: false,
        },
    ] {
        let mut w = success();
        w.metadata.push(metadata);
        cases.push(w);
    }
    for wire in cases {
        let status = wire.decode::<proto::RawWriteResponse>().unwrap_err();
        assert_eq!(status.code(), Code::DataLoss);
        assert!(status.metadata().get(NOT_LEADER_KEY).is_none());
    }
}

#[tokio::test]
async fn authentication_and_request_decoding_fail_before_backend_execution() {
    let backend = Arc::new(Backend::default());
    let handler = Handler {
        api: Kv9Grpc::new(backend.clone()),
        authenticator: authenticator(),
    };
    for authorization in ["", "Bearer wrong", "Basic secret", "Bearer secret\n"] {
        assert_eq!(
            handler
                .request::<proto::RawGetRequest>(authorization, &get(b"key").encode_to_vec())
                .unwrap_err()
                .code(),
            Code::Unauthenticated
        );
    }
    assert_eq!(
        handler
            .request::<proto::RawGetRequest>("Bearer secret", &[0xff])
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    assert_eq!(
        handler
            .request::<proto::RawGetRequest>(&"x".repeat(4104), &[])
            .unwrap_err()
            .code(),
        Code::ResourceExhausted
    );
    assert_eq!(
        handler
            .request::<proto::RawGetRequest>(
                "Bearer secret",
                &vec![0; crate::client::MAX_MESSAGE_BYTES + 1]
            )
            .unwrap_err()
            .code(),
        Code::ResourceExhausted
    );
    let decoded = handler
        .request::<proto::RawGetRequest>("Bearer secret", &get(b"key").encode_to_vec())
        .unwrap();
    assert_eq!(
        decoded
            .extensions()
            .get::<AuthContext>()
            .unwrap()
            .principal
            .as_ref(),
        "alice"
    );
    assert_eq!(decoded.into_inner(), get(b"key"));
    assert!(backend.calls.lock().unwrap().is_empty());
}

async fn server(api: Kv9Grpc) -> (ExperimentalServer, Arc<ExperimentClient>) {
    // Production start deliberately requires a nonzero loopback address.
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = socket.local_addr().unwrap();
    drop(socket);
    let server = start(address, api, authenticator()).await.unwrap();
    (server, Arc::new(ExperimentClient::new(address, 8)))
}
async fn stop(mut server: ExperimentalServer) {
    server.task.abort();
    assert!((&mut server.task).await.unwrap_err().is_cancelled());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_tcp_point_dispatch_matches_shared_authenticated_handler_semantics() {
    let backend = Arc::new(Backend::default());
    let api = Kv9Grpc::new(backend.clone());
    let (server, client) = server(api.clone()).await;
    let bytes = [0, 0xff, 0x80, 0];
    let expected = api
        .raw_put(direct_request(put(b"key", &bytes)))
        .await
        .unwrap()
        .into_inner();
    let actual = client
        .raw_put(request(put(b"key", &bytes)), deadline())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(actual, expected);
    assert_eq!(
        (actual.applied_term, actual.applied_index),
        (APPLIED.term, APPLIED.index)
    );
    let expected = api
        .raw_get(direct_request(get(b"key")))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        client
            .raw_get(request(get(b"key")), deadline())
            .await
            .unwrap()
            .into_inner(),
        expected
    );
    let delete = proto::RawDeleteRequest {
        context: Some(context_message()),
        key: b"key".to_vec(),
    };
    assert_eq!(
        client
            .raw_delete(request(delete.clone()), deadline())
            .await
            .unwrap()
            .into_inner(),
        api.raw_delete(direct_request(delete))
            .await
            .unwrap()
            .into_inner()
    );
    let missing = client
        .raw_get(request(get(b"key")), deadline())
        .await
        .unwrap()
        .into_inner()
        .value
        .unwrap();
    assert!(!missing.found && missing.value.is_empty());
    let direct = api
        .raw_get(direct_request(get(b"not-leader")))
        .await
        .unwrap_err();
    let wire = client
        .raw_get(request(get(b"not-leader")), deadline())
        .await
        .unwrap_err();
    assert_eq!(wire.code(), direct.code());
    assert_eq!(wire.message(), direct.message());
    assert_eq!(
        wire.metadata().get(NOT_LEADER_KEY),
        direct.metadata().get(NOT_LEADER_KEY)
    );
    assert_eq!(
        wire.metadata().get(LEADER_HINT_KEY),
        direct.metadata().get(LEADER_HINT_KEY)
    );
    assert_eq!(wire.metadata().get(LEADER_HINT_KEY).unwrap(), "3");
    let count = backend.calls.lock().unwrap().len();
    let mut invalid = request(get(b"key"));
    invalid
        .metadata_mut()
        .insert("authorization", "Bearer wrong".parse().unwrap());
    assert_eq!(
        client
            .raw_get(invalid, deadline())
            .await
            .unwrap_err()
            .code(),
        Code::Unauthenticated
    );
    {
        let calls = backend.calls.lock().unwrap();
        assert_eq!(calls.len(), count);
        assert!(calls.iter().all(|ctx| ctx.origin.label() == "alice"
            && ctx.keyspace == KeyspaceId(7)
            && ctx.region_epoch.conf_ver == 11
            && ctx.region_epoch.version == 13));
    }
    stop(server).await;
}

async fn interrupted_write_keeps_ownership(use_deadline: bool) {
    let (entered_tx, mut entered) = mpsc::unbounded_channel();
    let (release, receiver) = oneshot::channel();
    let backend = Arc::new(Backend {
        write_gate: Some(WriteGate {
            entered: entered_tx,
            release: Mutex::new(Some(receiver)),
        }),
        ..Backend::default()
    });
    let api = Kv9Grpc::with_limits(
        backend.clone(),
        PublicApiLimits {
            max_requests: 1,
            max_encoded_bytes: 4096,
        },
    )
    .unwrap();
    let admission = api.admission();
    let (server, client) = server(api).await;
    let writer = client.clone();
    let at = if use_deadline {
        Instant::now() + Duration::from_millis(500)
    } else {
        deadline()
    };
    let task =
        tokio::spawn(async move { writer.raw_put(request(put(b"held", b"value")), at).await });
    tokio::time::timeout(Duration::from_secs(3), entered.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(admission.snapshot().in_flight, 1);
    assert!(backend.values.lock().unwrap().is_empty());
    if use_deadline {
        let status = tokio::time::timeout(Duration::from_secs(3), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(status.code(), Code::Unavailable);
        assert!(status.metadata().get(NOT_LEADER_KEY).is_none());
    } else {
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
    }
    let refused = client
        .raw_get(request(get(b"held")), deadline())
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::ResourceExhausted);
    assert!(refused.metadata().get(ADMISSION_REFUSED_KEY).is_some());
    assert_eq!(
        admission.snapshot().in_flight,
        1,
        "live detached write retains admission after client interruption"
    );
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while admission.snapshot().in_flight != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        backend
            .values
            .lock()
            .unwrap()
            .get(b"held".as_slice())
            .unwrap(),
        b"value"
    );
    assert_eq!(admission.snapshot().encoded_bytes, 0);
    assert_eq!(admission.snapshot().running, 0);
    stop(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_tcp_client_cancellation_does_not_release_live_write_reservation() {
    interrupted_write_keeps_ownership(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_tcp_deadline_is_unconfirmed_while_detached_write_still_owns_capacity() {
    interrupted_write_keeps_ownership(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconnects_new_calls_after_restart_without_replaying_unknown_write() {
    let (entered_tx, mut entered) = mpsc::unbounded_channel();
    let (release, receiver) = oneshot::channel();
    let backend = Arc::new(Backend {
        write_gate: Some(WriteGate {
            entered: entered_tx,
            release: Mutex::new(Some(receiver)),
        }),
        ..Backend::default()
    });
    let api = Kv9Grpc::new(backend.clone());
    let (listener, client) = server(api.clone()).await;
    let writer = client.clone();
    let write = tokio::spawn(async move {
        writer
            .raw_put(request(put(b"once", b"retained")), deadline())
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), entered.recv())
        .await
        .unwrap()
        .unwrap();
    let old = client.live_channel().unwrap().unwrap();
    stop(listener).await;
    let unknown = tokio::time::timeout(Duration::from_secs(3), write)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(unknown.code(), Code::Unavailable);
    assert!(unknown.metadata().get(NOT_LEADER_KEY).is_none());
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while !old.dispatch.is_finished() || backend.calls.lock().unwrap().len() != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let listener = start(client.address, api, authenticator()).await.unwrap();
    let mut readers = JoinSet::new();
    for _ in 0..8 {
        let reader = client.clone();
        readers.spawn(async move { reader.raw_get(request(get(b"once")), deadline()).await });
    }
    while let Some(result) = readers.join_next().await {
        let response = result.unwrap().unwrap().into_inner().value.unwrap();
        assert!(response.found);
        assert_eq!(response.value, b"retained");
    }
    let new = client.live_channel().unwrap().unwrap();
    assert!(!Arc::ptr_eq(&old, &new));
    assert_eq!(
        backend.calls.lock().unwrap().len(),
        9,
        "one write and eight new reads, no replay"
    );
    stop(listener).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_connection_deadline_cannot_stop_the_listener() {
    use tarpc::tokio_serde::Serializer;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    // This mirrors only the pinned wire envelope, replacing its remaining-time
    // field with an out-of-range value. It addresses only our owned listener.
    #[derive(Serialize)]
    struct TestContext {
        deadline: Duration,
        trace_context: tarpc::trace::Context,
    }
    #[derive(Serialize)]
    struct TestRequest {
        context: TestContext,
        id: u64,
        message: PointRpcRequest,
    }
    #[derive(Serialize)]
    enum TestMessage {
        Request(TestRequest),
    }
    let backend = Arc::new(Backend::default());
    let (listener, client) = server(Kv9Grpc::new(backend.clone())).await;
    let malformed = TestMessage::Request(TestRequest {
        context: TestContext {
            deadline: Duration::MAX,
            trace_context: context::current().trace_context,
        },
        id: 1,
        message: PointRpcRequest::Point {
            operation: 0,
            request: WireRequest {
                authorization: "Bearer secret".into(),
                payload: get(b"key").encode_to_vec(),
            },
        },
    });
    let mut codec = Bincode::<(), TestMessage>::default();
    let bytes = std::pin::Pin::new(&mut codec)
        .serialize(&malformed)
        .unwrap();
    let mut stream = TcpStream::connect(client.address).await.unwrap();
    stream
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .await
        .unwrap();
    stream.write_all(&bytes).await.unwrap();
    let mut byte = [0];
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), stream.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert!(backend.calls.lock().unwrap().is_empty());
    let response = client
        .raw_get(request(get(b"key")), deadline())
        .await
        .unwrap();
    assert!(!response.into_inner().value.unwrap().found);
    assert!(!listener.task.is_finished());
    stop(listener).await;
}

#[test]
fn wire_debug_does_not_disclose_authorization_or_request_response_payloads() {
    let request = WireRequest {
        authorization: "Bearer distinct-private-token".into(),
        payload: b"distinct-private-request".to_vec(),
    };
    let reply = WireReply {
        code: 0,
        message: "distinct-private-message".into(),
        details: b"distinct-private-details".to_vec(),
        metadata: vec![WireMetadata {
            key: "private-metadata".into(),
            value: b"distinct-private-metadata".to_vec(),
            binary: false,
        }],
        payload: b"distinct-private-reply".to_vec(),
    };
    for debug in [format!("{request:?}"), format!("{reply:?}")] {
        assert!(!debug.contains("distinct-private"));
        assert!(!debug.contains("Bearer"));
        assert!(!debug.contains("private-metadata"));
        assert!(debug.contains("payload_bytes"));
    }
}
