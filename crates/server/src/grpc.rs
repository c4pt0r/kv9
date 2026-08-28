//! Public gRPC transport for the synchronous kv9 API surface.
//!
//! The transport deliberately owns only a [`BlockingBackend`]. Every call into the
//! synchronous node is therefore made through [`tokio::task::spawn_blocking`]; an
//! async handler cannot accidentally block a tonic worker by calling the node
//! directly.

use std::{collections::HashMap, sync::Arc};

use kv9_common::{ApiType, Error, KeyspaceId, NodeId, RegionId, TenantId, TimeStamp, TxnGroupId};
use kv9_region::RegionEpoch;
use tonic::{metadata::MetadataMap, service::Interceptor, Request, Response, Status};

use crate::api::{AdminApi, RawApi, RequestContext, TxnApi};

pub mod proto {
    tonic::include_proto!("kv9.v1");
}

const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

/// The kind of authenticated identity attached to a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthKind {
    Client,
    Node,
}

/// Trusted identity created by an interceptor, never decoded from a request body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthContext {
    pub principal: Arc<str>,
    pub node_id: Option<NodeId>,
    pub auth_kind: AuthKind,
}

/// Authentication seam. A later mTLS implementation can replace the credential
/// source without changing protobuf messages or handlers.
pub trait Authenticator: Send + Sync + 'static {
    fn authenticate(&self, metadata: &MetadataMap) -> Result<AuthContext, Status>;
}

/// Bearer-token authenticator for the first deployment phase.
///
/// Threat boundary: plaintext tokens reject unauthorized processes but do not
/// resist sniffing or a man-in-the-middle. TLS is therefore a hard gate before
/// cross-host deployment. This is also separate from the voter fingerprint,
/// which detects configuration accidents rather than authenticating a caller.
#[derive(Clone)]
pub struct TokenAuthenticator {
    principals: Arc<HashMap<String, Arc<str>>>,
}

impl TokenAuthenticator {
    pub fn new<I, T, P>(tokens: I) -> Result<Self, Error>
    where
        I: IntoIterator<Item = (T, P)>,
        T: Into<String>,
        P: Into<String>,
    {
        let mut principals = HashMap::new();
        for (token, principal) in tokens {
            let token = token.into();
            let principal = principal.into();
            if token.is_empty() || principal.is_empty() {
                return Err(Error::Config(
                    "authentication token and principal must be non-empty".into(),
                ));
            }
            if principals
                .insert(token, Arc::<str>::from(principal))
                .is_some()
            {
                return Err(Error::Config("duplicate authentication token".into()));
            }
        }
        if principals.is_empty() {
            return Err(Error::Config(
                "at least one client authentication token is required".into(),
            ));
        }
        Ok(Self {
            principals: Arc::new(principals),
        })
    }
}

impl Authenticator for TokenAuthenticator {
    fn authenticate(&self, metadata: &MetadataMap) -> Result<AuthContext, Status> {
        let header = metadata
            .get("authorization")
            .ok_or_else(|| Status::unauthenticated("missing bearer token"))?
            .to_str()
            .map_err(|_| Status::unauthenticated("invalid authorization metadata"))?;
        let token = header
            .strip_prefix("Bearer ")
            .filter(|token| !token.is_empty())
            .ok_or_else(|| Status::unauthenticated("expected bearer token"))?;
        let principal = self
            .principals
            .get(token)
            .cloned()
            .ok_or_else(|| Status::unauthenticated("invalid bearer token"))?;
        Ok(AuthContext {
            principal,
            node_id: None,
            auth_kind: AuthKind::Client,
        })
    }
}

/// Interceptor that establishes the only trusted caller identity.
#[derive(Clone)]
pub struct AuthInterceptor {
    authenticator: Arc<dyn Authenticator>,
}

impl AuthInterceptor {
    pub fn new(authenticator: Arc<dyn Authenticator>) -> Self {
        Self { authenticator }
    }
}

impl Interceptor for AuthInterceptor {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        let auth = self.authenticator.authenticate(request.metadata())?;
        if let Some(node_id) = auth.node_id {
            request.extensions_mut().insert(node_id);
        }
        request.extensions_mut().insert(auth);
        Ok(request)
    }
}

/// The complete synchronous backend required by the public service.
pub trait PublicApiBackend: RawApi + TxnApi + AdminApi + Send + Sync + 'static {}

impl<T> PublicApiBackend for T where T: RawApi + TxnApi + AdminApi + Send + Sync + 'static {}

#[derive(Clone)]
struct BlockingBackend {
    inner: Arc<dyn PublicApiBackend>,
}

impl BlockingBackend {
    async fn call<T, F>(&self, operation: F) -> Result<T, Status>
    where
        T: Send + 'static,
        F: FnOnce(&dyn PublicApiBackend) -> kv9_common::Result<T> + Send + 'static,
    {
        let inner = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || operation(inner.as_ref()))
            .await
            .map_err(|error| Status::internal(format!("blocking API worker failed: {error}")))?
            .map_err(error_status)
    }
}

/// Public tonic service implementation.
#[derive(Clone)]
pub struct Kv9Grpc {
    backend: BlockingBackend,
}

pub type AuthenticatedKv9Service = tonic::service::interceptor::InterceptedService<
    proto::kv9_server::Kv9Server<Kv9Grpc>,
    AuthInterceptor,
>;

impl Kv9Grpc {
    pub fn new(backend: Arc<dyn PublicApiBackend>) -> Self {
        Self {
            backend: BlockingBackend { inner: backend },
        }
    }

    /// Builds the authenticated service registered on the server-owned listener.
    pub fn authenticated_service(
        self,
        authenticator: Arc<dyn Authenticator>,
    ) -> AuthenticatedKv9Service {
        let service = proto::kv9_server::Kv9Server::new(self)
            .max_decoding_message_size(MAX_MESSAGE_BYTES)
            .max_encoding_message_size(MAX_MESSAGE_BYTES);
        tonic::service::interceptor::InterceptedService::new(
            service,
            AuthInterceptor::new(authenticator),
        )
    }
}

/// Small blocking client used by the single `kv9` binary's administrative CLI
/// and the external-process acceptance gate.
pub fn create_keyspace_blocking(
    address: &str,
    token: &str,
    name: String,
    tenant_id: u64,
    api_type: ApiType,
) -> Result<proto::CreateKeyspaceResponse, Error> {
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|error| Error::Config(format!("create client runtime: {error}")))?;
    runtime.block_on(async move {
        let mut client = proto::kv9_client::Kv9Client::connect(format!("http://{address}"))
            .await
            .map_err(|error| Error::Raft(format!("connect public gRPC {address}: {error}")))?;
        let mut request = Request::new(proto::CreateKeyspaceRequest {
            name,
            tenant_id,
            api_type: match api_type {
                ApiType::Txn => proto::ApiType::Txn as i32,
                ApiType::Raw => proto::ApiType::Raw as i32,
            },
        });
        let authorization = format!("Bearer {token}")
            .parse()
            .map_err(|_| Error::Config("client token is not valid metadata".into()))?;
        request
            .metadata_mut()
            .insert("authorization", authorization);
        client
            .create_keyspace(request)
            .await
            .map(Response::into_inner)
            .map_err(|status| Error::Raft(format!("CreateKeyspace RPC: {status}")))
    })
}

pub fn admit_node_blocking(
    address: &str,
    token: &str,
    node: NodeId,
    node_addr: String,
    ttl_seconds: u64,
) -> Result<proto::MembershipChangeResponse, Error> {
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|error| Error::Config(format!("create client runtime: {error}")))?;
    runtime.block_on(async move {
        let mut client = proto::kv9_client::Kv9Client::connect(format!("http://{address}"))
            .await
            .map_err(|error| Error::Raft(format!("connect public gRPC {address}: {error}")))?;
        let mut request = Request::new(proto::AdmitNodeRequest {
            node_id: node.0,
            addr: node_addr,
            ttl_seconds,
        });
        request.metadata_mut().insert(
            "authorization",
            format!("Bearer {token}")
                .parse()
                .map_err(|_| Error::Config("client token is not valid metadata".into()))?,
        );
        client
            .admit_node(request)
            .await
            .map(Response::into_inner)
            .map_err(|status| Error::Raft(format!("AdmitNode RPC: {status}")))
    })
}

pub fn promote_node_blocking(
    address: &str,
    token: &str,
    node: NodeId,
) -> Result<proto::MembershipChangeResponse, Error> {
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|error| Error::Config(format!("create client runtime: {error}")))?;
    runtime.block_on(async move {
        let mut client = proto::kv9_client::Kv9Client::connect(format!("http://{address}"))
            .await
            .map_err(|error| Error::Raft(format!("connect public gRPC {address}: {error}")))?;
        let mut request = Request::new(proto::PromoteNodeRequest { node_id: node.0 });
        request.metadata_mut().insert(
            "authorization",
            format!("Bearer {token}")
                .parse()
                .map_err(|_| Error::Config("client token is not valid metadata".into()))?,
        );
        client
            .promote_node(request)
            .await
            .map(Response::into_inner)
            .map_err(|status| Error::Raft(format!("PromoteNode RPC: {status}")))
    })
}

/// Outcome of a raw client call, with not-leader kept as a *structured* case.
///
/// The CLI must be able to tell "this key does not exist" from "you asked the wrong
/// node" — collapsing both into a generic error would make the acceptance script unable
/// to distinguish a real miss from a misdirected request.
#[derive(Debug)]
pub enum RawClientOutcome<T> {
    Ok(T),
    /// The node refused because it does not lead. `leader` is `None` mid-election.
    NotLeader {
        leader: Option<u64>,
    },
}

/// One scanned row as the client surfaces it: `(key, value)`, both raw bytes.
pub type RawRow = (Vec<u8>, Vec<u8>);

/// Blocking raw-KV client used by the CLI and the external acceptance gate.
///
/// Every call goes through the public gRPC surface — the acceptance script must exercise
/// the same path a real client would, not an in-process shortcut.
pub struct RawClient {
    runtime: tokio::runtime::Runtime,
    address: String,
    token: String,
    keyspace: u32,
}

impl RawClient {
    pub fn connect(address: &str, token: &str, keyspace: u32) -> Result<Self, Error> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|error| Error::Config(format!("create client runtime: {error}")))?;
        Ok(Self {
            runtime,
            address: address.to_owned(),
            token: token.to_owned(),
            keyspace,
        })
    }

    fn context(&self) -> Option<proto::RequestContext> {
        Some(proto::RequestContext {
            keyspace_id: self.keyspace,
            region_epoch: Some(proto::RegionEpoch {
                conf_ver: 1,
                version: 1,
            }),
        })
    }

    fn call<T, F, Fut>(&self, label: &str, build: F) -> Result<RawClientOutcome<T>, Error>
    where
        F: FnOnce(proto::kv9_client::Kv9Client<tonic::transport::Channel>, MetadataMap) -> Fut,
        Fut: std::future::Future<Output = Result<T, Status>>,
    {
        let url = format!("http://{}", self.address);
        let authorization = format!("Bearer {}", self.token);
        let label = label.to_owned();
        self.runtime.block_on(async move {
            let client = proto::kv9_client::Kv9Client::connect(url.clone())
                .await
                .map_err(|error| Error::Raft(format!("connect public gRPC {url}: {error}")))?;
            let mut metadata = MetadataMap::new();
            metadata.insert(
                "authorization",
                authorization
                    .parse()
                    .map_err(|_| Error::Config("invalid client token".into()))?,
            );
            match build(client, metadata).await {
                Ok(value) => Ok(RawClientOutcome::Ok(value)),
                Err(status) if status.code() == tonic::Code::FailedPrecondition => {
                    // Read the hint from metadata, never by parsing the message: the
                    // prose is for humans and may be reworded.
                    let leader = status
                        .metadata()
                        .get(LEADER_HINT_KEY)
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.parse::<u64>().ok());
                    Ok(RawClientOutcome::NotLeader { leader })
                }
                Err(status) => Err(Error::Raft(format!("{label} RPC: {status}"))),
            }
        })
    }

    pub fn put(&self, key: Vec<u8>, value: Vec<u8>) -> Result<RawClientOutcome<()>, Error> {
        let context = self.context();
        self.call("RawPut", move |mut client, metadata| async move {
            let mut request = Request::from_parts(
                metadata,
                Default::default(),
                proto::RawPutRequest {
                    context,
                    key,
                    value,
                },
            );
            *request.extensions_mut() = Default::default();
            client.raw_put(request).await.map(|_| ())
        })
    }

    pub fn get(&self, key: Vec<u8>) -> Result<RawClientOutcome<Option<Vec<u8>>>, Error> {
        let context = self.context();
        self.call("RawGet", move |mut client, metadata| async move {
            let request = Request::from_parts(
                metadata,
                Default::default(),
                proto::RawGetRequest { context, key },
            );
            client.raw_get(request).await.map(|response| {
                let value = response.into_inner().value;
                value.and_then(|v| if v.found { Some(v.value) } else { None })
            })
        })
    }

    pub fn delete(&self, key: Vec<u8>) -> Result<RawClientOutcome<()>, Error> {
        let context = self.context();
        self.call("RawDelete", move |mut client, metadata| async move {
            let request = Request::from_parts(
                metadata,
                Default::default(),
                proto::RawDeleteRequest { context, key },
            );
            client.raw_delete(request).await.map(|_| ())
        })
    }

    pub fn scan(
        &self,
        start: Vec<u8>,
        end: Vec<u8>,
        limit: u32,
    ) -> Result<RawClientOutcome<Vec<RawRow>>, Error> {
        let context = self.context();
        self.call("RawScan", move |mut client, metadata| async move {
            let request = Request::from_parts(
                metadata,
                Default::default(),
                proto::RawScanRequest {
                    context,
                    start,
                    end,
                    limit,
                },
            );
            client.raw_scan(request).await.map(|response| {
                response
                    .into_inner()
                    .pairs
                    .into_iter()
                    .map(|pair| (pair.key, pair.value))
                    .collect()
            })
        })
    }

    pub fn delete_range(
        &self,
        start: Vec<u8>,
        end: Vec<u8>,
    ) -> Result<RawClientOutcome<()>, Error> {
        let context = self.context();
        self.call("RawDeleteRange", move |mut client, metadata| async move {
            let request = Request::from_parts(
                metadata,
                Default::default(),
                proto::RawDeleteRangeRequest {
                    context,
                    start,
                    end,
                },
            );
            client.raw_delete_range(request).await.map(|_| ())
        })
    }
}

fn auth_context<T>(request: &Request<T>) -> Result<AuthContext, Status> {
    let auth = request
        .extensions()
        .get::<AuthContext>()
        .cloned()
        .ok_or_else(|| Status::unauthenticated("authenticated identity missing"))?;
    if auth.auth_kind != AuthKind::Client || auth.node_id.is_some() {
        return Err(Status::permission_denied("client identity required"));
    }
    Ok(auth)
}

fn request_context(
    context: Option<proto::RequestContext>,
    auth: &AuthContext,
) -> Result<RequestContext, Status> {
    let context = context.ok_or_else(|| Status::invalid_argument("request context is required"))?;
    if context.keyspace_id > KeyspaceId::MAX {
        return Err(Status::invalid_argument("keyspace id exceeds 3-byte width"));
    }
    let epoch = context
        .region_epoch
        .ok_or_else(|| Status::invalid_argument("region epoch is required"))?;
    Ok(RequestContext {
        keyspace: KeyspaceId(context.keyspace_id),
        region_epoch: RegionEpoch {
            conf_ver: epoch.conf_ver,
            version: epoch.version,
        },
        caller: Some(auth.principal.to_string()),
    })
}

fn nonzero_limit(limit: u32) -> Result<usize, Status> {
    if limit == 0 {
        Err(Status::invalid_argument(
            "scan limit must be greater than zero",
        ))
    } else {
        Ok(limit as usize)
    }
}

fn optional_value(value: Option<Vec<u8>>) -> proto::OptionalValue {
    match value {
        Some(value) => proto::OptionalValue { found: true, value },
        None => proto::OptionalValue {
            found: false,
            value: Vec::new(),
        },
    }
}

fn scan_response(pairs: Vec<(Vec<u8>, Vec<u8>)>) -> proto::ScanResponse {
    proto::ScanResponse {
        pairs: pairs
            .into_iter()
            .map(|(key, value)| proto::KeyValue { key, value })
            .collect(),
    }
}

fn error_status(error: Error) -> Status {
    let message = error.to_string();
    match error {
        Error::KeyspaceIdOutOfRange(_)
        | Error::MalformedKey(_)
        | Error::InvalidKeyMode(_)
        | Error::Config(_) => Status::invalid_argument(message),
        Error::KeyspaceNotFound(_) | Error::RegionNotFound => Status::not_found(message),
        Error::ApiTypeMismatch { .. }
        | Error::StaleEpoch { .. }
        | Error::SplitCrossesKeyspace
        | Error::CrossTxnGroup { .. } => Status::failed_precondition(message),
        Error::WriteConflict(_) | Error::KeyIsLocked => Status::aborted(message),
        Error::TsoUnavailable(_) | Error::MetaNotReady(_) | Error::Raft(_) => {
            Status::unavailable(message)
        }
        // Deliberately *not* `unavailable`: this follower is perfectly healthy, the
        // request simply arrived at the wrong node. `unavailable` invites a client to
        // transparently retry the same address, and reports a working node as broken.
        Error::NotLeader { leader } => {
            let mut status = Status::failed_precondition(message);
            // Machine-readable redirect. The human message is prose and may be reworded;
            // a client that parsed it would break silently when someone edits the text.
            if let Some(node_id) = leader {
                if let Ok(value) = node_id.to_string().parse() {
                    status.metadata_mut().insert(LEADER_HINT_KEY, value);
                }
            }
            status
        }
        Error::Engine(_) => Status::internal(message),
        Error::NotImplemented(_) => Status::unimplemented(message),
    }
}

/// Response metadata carrying the node a client should retry against, when known.
///
/// Absent when this node does not know who leads (e.g. mid-election): the client should
/// re-run discovery rather than hot-loop against a node that just refused it.
pub const LEADER_HINT_KEY: &str = "kv9-leader-node-id";

fn api_type(value: i32) -> Result<ApiType, Status> {
    match proto::ApiType::try_from(value) {
        Ok(proto::ApiType::Txn) => Ok(ApiType::Txn),
        Ok(proto::ApiType::Raw) => Ok(ApiType::Raw),
        Ok(proto::ApiType::Unspecified) | Err(_) => {
            Err(Status::invalid_argument("api_type must be TXN or RAW"))
        }
    }
}

#[tonic::async_trait]
impl proto::kv9_server::Kv9 for Kv9Grpc {
    async fn raw_get(
        &self,
        request: Request<proto::RawGetRequest>,
    ) -> Result<Response<proto::RawGetResponse>, Status> {
        let auth = auth_context(&request)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        let value = self
            .backend
            .call(move |backend| backend.raw_get(&context, &request.key))
            .await?;
        Ok(Response::new(proto::RawGetResponse {
            value: Some(optional_value(value)),
        }))
    }

    async fn raw_batch_get(
        &self,
        request: Request<proto::RawBatchGetRequest>,
    ) -> Result<Response<proto::RawBatchGetResponse>, Status> {
        let auth = auth_context(&request)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        let values = self
            .backend
            .call(move |backend| backend.raw_batch_get(&context, &request.keys))
            .await?;
        Ok(Response::new(proto::RawBatchGetResponse {
            values: values.into_iter().map(optional_value).collect(),
        }))
    }

    async fn raw_put(
        &self,
        request: Request<proto::RawPutRequest>,
    ) -> Result<Response<proto::Empty>, Status> {
        let auth = auth_context(&request)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        self.backend
            .call(move |backend| backend.raw_put(&context, request.key, request.value))
            .await?;
        Ok(Response::new(proto::Empty {}))
    }

    async fn raw_batch_put(
        &self,
        request: Request<proto::RawBatchPutRequest>,
    ) -> Result<Response<proto::Empty>, Status> {
        let auth = auth_context(&request)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        let pairs: Vec<_> = request
            .pairs
            .into_iter()
            .map(|pair| (pair.key, pair.value))
            .collect();
        self.backend
            .call(move |backend| backend.raw_batch_put(&context, &pairs))
            .await?;
        Ok(Response::new(proto::Empty {}))
    }

    async fn raw_delete(
        &self,
        request: Request<proto::RawDeleteRequest>,
    ) -> Result<Response<proto::Empty>, Status> {
        let auth = auth_context(&request)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        self.backend
            .call(move |backend| backend.raw_delete(&context, &request.key))
            .await?;
        Ok(Response::new(proto::Empty {}))
    }

    async fn raw_scan(
        &self,
        request: Request<proto::RawScanRequest>,
    ) -> Result<Response<proto::ScanResponse>, Status> {
        let auth = auth_context(&request)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        let limit = nonzero_limit(request.limit)?;
        let pairs = self
            .backend
            .call(move |backend| backend.raw_scan(&context, &request.start, &request.end, limit))
            .await?;
        Ok(Response::new(scan_response(pairs)))
    }

    async fn raw_delete_range(
        &self,
        request: Request<proto::RawDeleteRangeRequest>,
    ) -> Result<Response<proto::Empty>, Status> {
        let auth = auth_context(&request)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        self.backend
            .call(move |backend| backend.raw_delete_range(&context, &request.start, &request.end))
            .await?;
        Ok(Response::new(proto::Empty {}))
    }

    async fn kv_get(
        &self,
        request: Request<proto::KvGetRequest>,
    ) -> Result<Response<proto::KvGetResponse>, Status> {
        let auth = auth_context(&request)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        let value = self
            .backend
            .call(move |backend| {
                backend.kv_get(&context, &request.key, TimeStamp(request.start_ts))
            })
            .await?;
        Ok(Response::new(proto::KvGetResponse {
            value: Some(optional_value(value)),
        }))
    }

    async fn kv_batch_get(
        &self,
        request: Request<proto::KvBatchGetRequest>,
    ) -> Result<Response<proto::KvBatchGetResponse>, Status> {
        let auth = auth_context(&request)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        let values = self
            .backend
            .call(move |backend| {
                backend.kv_batch_get(&context, &request.keys, TimeStamp(request.start_ts))
            })
            .await?;
        Ok(Response::new(proto::KvBatchGetResponse {
            values: values.into_iter().map(optional_value).collect(),
        }))
    }

    async fn kv_scan(
        &self,
        request: Request<proto::KvScanRequest>,
    ) -> Result<Response<proto::ScanResponse>, Status> {
        let auth = auth_context(&request)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        let limit = nonzero_limit(request.limit)?;
        let pairs = self
            .backend
            .call(move |backend| {
                backend.kv_scan(
                    &context,
                    &request.start,
                    &request.end,
                    limit,
                    TimeStamp(request.start_ts),
                )
            })
            .await?;
        Ok(Response::new(scan_response(pairs)))
    }

    async fn kv_prewrite(
        &self,
        request: Request<proto::KvPrewriteRequest>,
    ) -> Result<Response<proto::Empty>, Status> {
        let auth = auth_context(&request)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        let mutations = request
            .mutations
            .into_iter()
            .map(|mutation| {
                let value = match mutation.operation {
                    Some(proto::mutation::Operation::PutValue(value)) => Some(value),
                    Some(proto::mutation::Operation::Delete(true)) => None,
                    Some(proto::mutation::Operation::Delete(false)) | None => {
                        return Err(Status::invalid_argument(
                            "mutation operation must be put or delete",
                        ));
                    }
                };
                Ok((mutation.key, value))
            })
            .collect::<Result<Vec<_>, Status>>()?;
        self.backend
            .call(move |backend| {
                backend.kv_prewrite(
                    &context,
                    &mutations,
                    &request.primary,
                    TimeStamp(request.start_ts),
                )
            })
            .await?;
        Ok(Response::new(proto::Empty {}))
    }

    async fn kv_commit(
        &self,
        request: Request<proto::KvCommitRequest>,
    ) -> Result<Response<proto::Empty>, Status> {
        let auth = auth_context(&request)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        self.backend
            .call(move |backend| {
                backend.kv_commit(
                    &context,
                    &request.keys,
                    TimeStamp(request.start_ts),
                    TimeStamp(request.commit_ts),
                )
            })
            .await?;
        Ok(Response::new(proto::Empty {}))
    }

    async fn kv_pessimistic_lock(
        &self,
        request: Request<proto::KvPessimisticLockRequest>,
    ) -> Result<Response<proto::Empty>, Status> {
        let auth = auth_context(&request)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        self.backend
            .call(move |backend| {
                backend.kv_pessimistic_lock(&context, &request.keys, TimeStamp(request.start_ts))
            })
            .await?;
        Ok(Response::new(proto::Empty {}))
    }

    async fn kv_pessimistic_rollback(
        &self,
        request: Request<proto::KvPessimisticRollbackRequest>,
    ) -> Result<Response<proto::Empty>, Status> {
        let auth = auth_context(&request)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        self.backend
            .call(move |backend| {
                backend.kv_pessimistic_rollback(
                    &context,
                    &request.keys,
                    TimeStamp(request.start_ts),
                )
            })
            .await?;
        Ok(Response::new(proto::Empty {}))
    }

    async fn kv_resolve_lock(
        &self,
        request: Request<proto::KvResolveLockRequest>,
    ) -> Result<Response<proto::Empty>, Status> {
        let auth = auth_context(&request)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        self.backend
            .call(move |backend| {
                backend.kv_resolve_lock(
                    &context,
                    TimeStamp(request.start_ts),
                    request.commit_ts.map(TimeStamp),
                )
            })
            .await?;
        Ok(Response::new(proto::Empty {}))
    }

    async fn kv_cleanup(
        &self,
        request: Request<proto::KvCleanupRequest>,
    ) -> Result<Response<proto::Empty>, Status> {
        let auth = auth_context(&request)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        self.backend
            .call(move |backend| {
                backend.kv_cleanup(&context, &request.key, TimeStamp(request.start_ts))
            })
            .await?;
        Ok(Response::new(proto::Empty {}))
    }

    async fn kv_check_txn_status(
        &self,
        request: Request<proto::KvCheckTxnStatusRequest>,
    ) -> Result<Response<proto::Empty>, Status> {
        let auth = auth_context(&request)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        self.backend
            .call(move |backend| {
                backend.kv_check_txn_status(&context, &request.primary, TimeStamp(request.lock_ts))
            })
            .await?;
        Ok(Response::new(proto::Empty {}))
    }

    async fn create_keyspace(
        &self,
        request: Request<proto::CreateKeyspaceRequest>,
    ) -> Result<Response<proto::CreateKeyspaceResponse>, Status> {
        let auth = auth_context(&request)?;
        let request = request.into_inner();
        let api_type = api_type(request.api_type)?;
        let caller = auth.principal.to_string();
        let id = self
            .backend
            .call(move |backend| {
                backend.create_keyspace(
                    &caller,
                    &request.name,
                    TenantId(request.tenant_id),
                    api_type,
                    TxnGroupId::DEFAULT,
                )
            })
            .await?;
        Ok(Response::new(proto::CreateKeyspaceResponse {
            keyspace_id: id.keyspace.0,
            proposed_term: id.proposed.map(|position| position.term),
            proposed_index: id.proposed.map(|position| position.index),
        }))
    }

    async fn list_keyspaces(
        &self,
        request: Request<proto::ListKeyspacesRequest>,
    ) -> Result<Response<proto::ListKeyspacesResponse>, Status> {
        let auth = auth_context(&request)?;
        let caller = auth.principal.to_string();
        let keyspaces = self
            .backend
            .call(move |backend| backend.list_keyspaces(&caller))
            .await?;
        Ok(Response::new(proto::ListKeyspacesResponse {
            keyspaces: keyspaces
                .into_iter()
                .map(|keyspace| proto::Keyspace {
                    id: keyspace.id.0,
                    name: keyspace.name,
                    tenant_id: keyspace.tenant.0,
                    api_type: match keyspace.api_type {
                        ApiType::Txn => proto::ApiType::Txn as i32,
                        ApiType::Raw => proto::ApiType::Raw as i32,
                    },
                    txn_group_id: (keyspace.api_type == ApiType::Txn)
                        .then_some(keyspace.txn_group.0),
                })
                .collect(),
        }))
    }

    async fn get_region(
        &self,
        request: Request<proto::GetRegionRequest>,
    ) -> Result<Response<proto::GetRegionResponse>, Status> {
        let auth = auth_context(&request)?;
        let request = request.into_inner();
        if request.keyspace_id > KeyspaceId::MAX {
            return Err(Status::invalid_argument("keyspace id exceeds 3-byte width"));
        }
        let caller = auth.principal.to_string();
        let region = self
            .backend
            .call(move |backend| {
                backend.get_region(&caller, KeyspaceId(request.keyspace_id), &request.key)
            })
            .await?;
        Ok(Response::new(proto::GetRegionResponse {
            region: Some(proto::RegionLocation {
                region_id: region.region.0,
                epoch: Some(proto::RegionEpoch {
                    conf_ver: region.epoch.conf_ver,
                    version: region.epoch.version,
                }),
                leader_node_id: region.leader.map(|node| node.0),
            }),
        }))
    }

    async fn split_region(
        &self,
        request: Request<proto::SplitRegionRequest>,
    ) -> Result<Response<proto::Empty>, Status> {
        let auth = auth_context(&request)?;
        let request = request.into_inner();
        let caller = auth.principal.to_string();
        self.backend
            .call(move |backend| {
                backend.split_region(&caller, RegionId(request.region_id), request.split_key)
            })
            .await?;
        Ok(Response::new(proto::Empty {}))
    }

    async fn cluster_info(
        &self,
        request: Request<proto::ClusterInfoRequest>,
    ) -> Result<Response<proto::ClusterInfoResponse>, Status> {
        let auth = auth_context(&request)?;
        let caller = auth.principal.to_string();
        let info = self
            .backend
            .call(move |backend| backend.cluster_info(&caller))
            .await?;
        Ok(Response::new(proto::ClusterInfoResponse {
            node_count: info.node_count as u64,
            keyspace_count: info.keyspace_count as u64,
            region_count: info.region_count as u64,
        }))
    }

    async fn admit_node(
        &self,
        request: Request<proto::AdmitNodeRequest>,
    ) -> Result<Response<proto::MembershipChangeResponse>, Status> {
        let auth = auth_context(&request)?;
        let request = request.into_inner();
        let caller = auth.principal.to_string();
        let result = self
            .backend
            .call(move |backend| {
                backend.admit_node(
                    &caller,
                    NodeId(request.node_id),
                    &request.addr,
                    request.ttl_seconds,
                )
            })
            .await?;
        Ok(Response::new(proto::MembershipChangeResponse {
            applied_term: result.applied.term,
            applied_index: result.applied.index,
            voters: result.voters,
            learners: result.learners,
        }))
    }

    async fn promote_node(
        &self,
        request: Request<proto::PromoteNodeRequest>,
    ) -> Result<Response<proto::MembershipChangeResponse>, Status> {
        let auth = auth_context(&request)?;
        let request = request.into_inner();
        let caller = auth.principal.to_string();
        let result = self
            .backend
            .call(move |backend| backend.promote_node(&caller, NodeId(request.node_id)))
            .await?;
        Ok(Response::new(proto::MembershipChangeResponse {
            applied_term: result.applied.term,
            applied_index: result.applied.index,
            voters: result.voters,
            learners: result.learners,
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use kv9_common::{Keyspace, Result, UserKey, Value};
    use tonic::Code;

    use super::*;
    use crate::api::{ClusterInfo, RegionLocation};

    #[derive(Default)]
    struct FakeBackend {
        callers: Mutex<Vec<String>>,
    }

    impl RawApi for FakeBackend {
        fn raw_get(&self, ctx: &RequestContext, _key: &[u8]) -> Result<Option<Value>> {
            self.callers
                .lock()
                .unwrap()
                .push(ctx.caller.clone().unwrap());
            Ok(None)
        }
        fn raw_batch_get(&self, _: &RequestContext, _: &[UserKey]) -> Result<Vec<Option<Value>>> {
            Err(Error::NotImplemented("raw_batch_get"))
        }
        fn raw_put(&self, _: &RequestContext, _: UserKey, _: Value) -> Result<()> {
            Err(Error::NotImplemented("raw_put"))
        }
        fn raw_batch_put(&self, _: &RequestContext, _: &[(UserKey, Value)]) -> Result<()> {
            Err(Error::NotImplemented("raw_batch_put"))
        }
        fn raw_delete(&self, _: &RequestContext, _: &[u8]) -> Result<()> {
            Err(Error::NotImplemented("raw_delete"))
        }
        fn raw_scan(
            &self,
            _: &RequestContext,
            _: &[u8],
            _: &[u8],
            _: usize,
        ) -> Result<Vec<(UserKey, Value)>> {
            Err(Error::NotImplemented("raw_scan"))
        }
        fn raw_delete_range(&self, _: &RequestContext, _: &[u8], _: &[u8]) -> Result<()> {
            Err(Error::NotImplemented("raw_delete_range"))
        }
    }

    impl TxnApi for FakeBackend {
        fn kv_get(&self, _: &RequestContext, _: &[u8], _: TimeStamp) -> Result<Option<Value>> {
            Err(Error::NotImplemented("kv_get"))
        }
        fn kv_batch_get(
            &self,
            _: &RequestContext,
            _: &[UserKey],
            _: TimeStamp,
        ) -> Result<Vec<Option<Value>>> {
            Err(Error::NotImplemented("kv_batch_get"))
        }
        fn kv_scan(
            &self,
            _: &RequestContext,
            _: &[u8],
            _: &[u8],
            _: usize,
            _: TimeStamp,
        ) -> Result<Vec<(UserKey, Value)>> {
            Err(Error::NotImplemented("kv_scan"))
        }
        fn kv_prewrite(
            &self,
            _: &RequestContext,
            _: &[(UserKey, Option<Value>)],
            _: &[u8],
            _: TimeStamp,
        ) -> Result<()> {
            Err(Error::NotImplemented("kv_prewrite"))
        }
        fn kv_commit(
            &self,
            _: &RequestContext,
            _: &[UserKey],
            _: TimeStamp,
            _: TimeStamp,
        ) -> Result<()> {
            Err(Error::NotImplemented("kv_commit"))
        }
        fn kv_pessimistic_lock(
            &self,
            _: &RequestContext,
            _: &[UserKey],
            _: TimeStamp,
        ) -> Result<()> {
            Err(Error::NotImplemented("kv_pessimistic_lock"))
        }
        fn kv_pessimistic_rollback(
            &self,
            _: &RequestContext,
            _: &[UserKey],
            _: TimeStamp,
        ) -> Result<()> {
            Err(Error::NotImplemented("kv_pessimistic_rollback"))
        }
        fn kv_resolve_lock(
            &self,
            _: &RequestContext,
            _: TimeStamp,
            _: Option<TimeStamp>,
        ) -> Result<()> {
            Err(Error::NotImplemented("kv_resolve_lock"))
        }
        fn kv_cleanup(&self, _: &RequestContext, _: &[u8], _: TimeStamp) -> Result<()> {
            Err(Error::NotImplemented("kv_cleanup"))
        }
        fn kv_check_txn_status(&self, _: &RequestContext, _: &[u8], _: TimeStamp) -> Result<()> {
            Err(Error::NotImplemented("kv_check_txn_status"))
        }
    }

    impl AdminApi for FakeBackend {
        fn create_keyspace(
            &self,
            _: &str,
            _: &str,
            _: TenantId,
            _: ApiType,
            _: TxnGroupId,
        ) -> Result<crate::api::CreateKeyspaceResult> {
            Err(Error::NotImplemented("create_keyspace"))
        }
        fn list_keyspaces(&self, _: &str) -> Result<Vec<Keyspace>> {
            Err(Error::NotImplemented("list_keyspaces"))
        }
        fn get_region(&self, _: &str, _: KeyspaceId, _: &[u8]) -> Result<RegionLocation> {
            Err(Error::NotImplemented("get_region"))
        }
        fn split_region(&self, _: &str, _: RegionId, _: UserKey) -> Result<()> {
            Err(Error::NotImplemented("split_region"))
        }
        fn cluster_info(&self, caller: &str) -> Result<ClusterInfo> {
            self.callers.lock().unwrap().push(caller.to_owned());
            Ok(ClusterInfo {
                node_count: 3,
                keyspace_count: 2,
                region_count: 1,
            })
        }
    }

    fn request_context_message() -> proto::RequestContext {
        proto::RequestContext {
            keyspace_id: 1,
            region_epoch: Some(proto::RegionEpoch {
                conf_ver: 1,
                version: 1,
            }),
        }
    }

    fn authenticated<T>(message: T) -> Request<T> {
        let mut request = Request::new(message);
        request.extensions_mut().insert(AuthContext {
            principal: Arc::from("alice"),
            node_id: None,
            auth_kind: AuthKind::Client,
        });
        request
    }

    #[tokio::test]
    async fn trusted_principal_reaches_backend_and_missing_identity_is_rejected() {
        use proto::kv9_server::Kv9;

        let backend = Arc::new(FakeBackend::default());
        let service = Kv9Grpc::new(backend.clone());
        let response = service
            .cluster_info(authenticated(proto::ClusterInfoRequest {}))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(response.node_count, 3);
        assert_eq!(backend.callers.lock().unwrap().as_slice(), ["alice"]);

        let status = service
            .cluster_info(Request::new(proto::ClusterInfoRequest {}))
            .await
            .unwrap_err();
        assert_eq!(status.code(), Code::Unauthenticated);
    }

    #[tokio::test]
    async fn real_grpc_wire_enforces_token_and_propagates_principal() {
        let backend = Arc::new(FakeBackend::default());
        let authenticator =
            Arc::new(TokenAuthenticator::new([("wire-secret", "wire-client")]).unwrap());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let server = tonic::transport::Server::builder()
            .add_service(Kv9Grpc::new(backend.clone()).authenticated_service(authenticator))
            .serve_with_incoming_shutdown(incoming, async {
                let _ = shutdown_rx.await;
            });
        let server_task = tokio::spawn(server);

        let endpoint = format!("http://{address}");
        let mut client = proto::kv9_client::Kv9Client::connect(endpoint)
            .await
            .unwrap();
        let status = client
            .cluster_info(proto::ClusterInfoRequest {})
            .await
            .unwrap_err();
        assert_eq!(status.code(), Code::Unauthenticated);

        let mut request = Request::new(proto::ClusterInfoRequest {});
        request
            .metadata_mut()
            .insert("authorization", "Bearer wire-secret".parse().unwrap());
        let response = client.cluster_info(request).await.unwrap().into_inner();
        assert_eq!(response.node_count, 3);
        assert_eq!(backend.callers.lock().unwrap().as_slice(), ["wire-client"]);

        shutdown_tx.send(()).unwrap();
        server_task.await.unwrap().unwrap();
    }

    #[test]
    fn token_authenticator_rejects_missing_and_bad_tokens() {
        let authenticator = TokenAuthenticator::new([("secret", "alice")]).unwrap();
        let metadata = MetadataMap::new();
        assert_eq!(
            authenticator.authenticate(&metadata).unwrap_err().code(),
            Code::Unauthenticated
        );

        let mut metadata = MetadataMap::new();
        metadata.insert("authorization", "Bearer wrong".parse().unwrap());
        assert_eq!(
            authenticator.authenticate(&metadata).unwrap_err().code(),
            Code::Unauthenticated
        );

        metadata.insert("authorization", "Bearer secret".parse().unwrap());
        assert_eq!(
            authenticator
                .authenticate(&metadata)
                .unwrap()
                .principal
                .as_ref(),
            "alice"
        );
    }

    #[tokio::test]
    async fn malformed_context_and_not_implemented_are_typed_statuses() {
        use proto::kv9_server::Kv9;

        let service = Kv9Grpc::new(Arc::new(FakeBackend::default()));
        let status = service
            .raw_get(authenticated(proto::RawGetRequest {
                context: None,
                key: b"key".to_vec(),
            }))
            .await
            .unwrap_err();
        assert_eq!(status.code(), Code::InvalidArgument);

        let status = service
            .raw_put(authenticated(proto::RawPutRequest {
                context: Some(request_context_message()),
                key: b"key".to_vec(),
                value: b"value".to_vec(),
            }))
            .await
            .unwrap_err();
        assert_eq!(status.code(), Code::Unimplemented);
    }

    /// A client's redirect decision must key on the status code and a stable metadata
    /// key, never on the human-readable message — prose gets reworded, and a client that
    /// parsed it would start silently failing to redirect.
    #[test]
    fn not_leader_maps_to_failed_precondition_and_carries_the_hint_only_when_known() {
        // With a known leader: redirectable.
        let status = error_status(Error::NotLeader { leader: Some(7) });
        assert_eq!(status.code(), Code::FailedPrecondition);
        assert_eq!(
            status
                .metadata()
                .get(LEADER_HINT_KEY)
                .map(|v| v.to_str().unwrap().to_owned()),
            Some("7".to_owned())
        );

        // Mid-election there is no leader to name: the key must be absent, not empty or
        // "0", so a client can tell "retry node 7" from "go re-discover".
        let unknown = error_status(Error::NotLeader { leader: None });
        assert_eq!(unknown.code(), Code::FailedPrecondition);
        assert!(
            unknown.metadata().get(LEADER_HINT_KEY).is_none(),
            "an unknown leader must omit the hint entirely"
        );

        // Control: not-leader is distinguishable from the unavailable family, so a client
        // does not transparently retry the same healthy follower.
        assert_ne!(
            error_status(Error::NotLeader { leader: Some(7) }).code(),
            error_status(Error::Raft("stepped down".into())).code()
        );
    }

    #[test]
    fn errors_map_to_stable_grpc_codes() {
        assert_eq!(error_status(Error::RegionNotFound).code(), Code::NotFound);
        assert_eq!(
            error_status(Error::StaleEpoch {
                region: RegionId(1)
            })
            .code(),
            Code::FailedPrecondition
        );
        assert_eq!(error_status(Error::KeyIsLocked).code(), Code::Aborted);
        assert_eq!(
            error_status(Error::MetaNotReady("bootstrapping".into())).code(),
            Code::Unavailable
        );
        assert_eq!(
            error_status(Error::NotImplemented("later")).code(),
            Code::Unimplemented
        );
    }
}
