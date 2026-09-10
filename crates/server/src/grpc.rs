//! Public gRPC transport for the synchronous kv9 API surface.
//!
//! The transport deliberately owns only a `BlockingBackend` (private by design). Every call into the
//! potentially blocking engine is made through [`tokio::task::spawn_blocking`].
//! Point reads may finish on a memory-only view after asynchronous quorum
//! preparation. Unfinished jobs cross the blocking boundary with their reservation.

use std::{collections::HashMap, sync::Arc};

use kv9_common::{
    ApiType, AppliedPosition, Error, KeyspaceId, NodeId, RegionId, TenantId, TimeStamp, TimelineId,
    TxnGroupId,
};
use kv9_region::RegionEpoch;
use kv9_txn::{QualifiedKey, TimelineGeneration, TxnDescriptor, TxnId, TxnStatus};
use tonic::{metadata::MetadataMap, service::Interceptor, Request, Response, Status};

use crate::admission::{PublicAdmission, PublicApiLimits, Refusal, Reservation, WorkClass};
use crate::api::{AdminApi, RawApi, RequestContext, RequestOrigin, TxnApi};

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
    admission: Arc<PublicAdmission>,
}

impl BlockingBackend {
    async fn prepared_write(
        &self,
        mut reservation: Reservation,
        preparation: crate::api::RawWritePreparation,
    ) -> Result<AppliedPosition, Status> {
        // Dropping the RPC's JoinHandle detaches this task. It owns the SAME
        // reservation until preparation and the internal logical wait finish.
        // The blocking closure owns it while queued/running, so cancelling the
        // outer RPC cannot release capacity around a potentially live proposal.
        tokio::spawn(async move {
            let (prepared, reservation) = tokio::task::spawn_blocking(move || {
                reservation.start();
                (preparation(), reservation)
            })
            .await
            .map_err(|error| {
                Status::internal(format!("blocking write preparation failed: {error}"))
            })?;
            let result = match prepared {
                Ok(completion) => completion.await,
                Err(error) => Err(error),
            };
            reservation.finish(result.is_err());
            result.map_err(error_status)
        })
        .await
        .map_err(|error| Status::internal(format!("write completion task failed: {error}")))?
    }

    async fn prepared_read<T: Send + 'static>(
        &self,
        mut reservation: Reservation,
        preparation: crate::api::RawReadPreparation<T>,
    ) -> Result<T, Status> {
        // Preparation is an async, cancellable read wait. Once the engine job
        // exists, move the SAME reservation into it so RPC cancellation cannot
        // release capacity while synchronous work is still executing.
        reservation.start();
        let job = match preparation.await {
            Ok(job) => job,
            Err(error) => {
                reservation.finish(true);
                return Err(error_status(error));
            }
        };
        let job = match job {
            crate::api::RawReadJob::Completed(value) => {
                self.admission.record_prepared_read(true);
                reservation.finish(false);
                return Ok(value);
            }
            crate::api::RawReadJob::Blocking(job) => {
                self.admission.record_prepared_read(false);
                job
            }
        };
        tokio::task::spawn_blocking(move || {
            let result = job();
            reservation.finish(result.is_err());
            result
        })
        .await
        .map_err(|error| Status::internal(format!("blocking read worker failed: {error}")))?
        .map_err(error_status)
    }

    async fn call<T, F>(&self, mut reservation: Reservation, operation: F) -> Result<T, Status>
    where
        T: Send + 'static,
        F: FnOnce(&dyn PublicApiBackend) -> kv9_common::Result<T> + Send + 'static,
    {
        let inner = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            reservation.start();
            let result = operation(inner.as_ref());
            reservation.finish(result.is_err());
            result
        })
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
        Self::with_limits(backend, PublicApiLimits::default()).expect("valid default limits")
    }

    pub fn with_limits(
        backend: Arc<dyn PublicApiBackend>,
        limits: PublicApiLimits,
    ) -> Result<Self, Error> {
        Ok(Self {
            backend: BlockingBackend {
                inner: backend,
                admission: PublicAdmission::new(limits)?,
            },
        })
    }

    pub fn admission(&self) -> Arc<PublicAdmission> {
        self.backend.admission.clone()
    }

    fn reserve<T: prost::Message>(
        &self,
        request: &Request<T>,
        class: WorkClass,
    ) -> Result<Reservation, Status> {
        self.backend
            .admission
            .reserve(class, request.get_ref().encoded_len())
            .map_err(admission_status)
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
            .map_err(|status| membership_rpc_error("AdmitNode", status))
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
            .map_err(|status| membership_rpc_error("PromoteNode", status))
    })
}

/// Only the exclusive wire refusal permits another membership attempt. In
/// particular, prose, transport failures and mixed/duplicate control metadata
/// cannot prove that a ticket or configuration change was never committed.
fn membership_rpc_error(rpc: &str, status: Status) -> Error {
    match crate::client::classify_status(&status, false) {
        crate::client::Reason::NotLeader { leader } => Error::NotLeader {
            leader: leader.map(NodeId),
        },
        _ => Error::Raft(format!("{rpc} RPC: {status}")),
    }
}

/// Outcome of a raw client call, with not-leader kept as a *structured* case.
///
/// The CLI must be able to tell "this key does not exist" from "you asked the wrong
/// node" — collapsing both into a generic error would make the acceptance script unable
/// to distinguish a real miss from a misdirected request.
#[derive(Debug)]
pub enum RawClientOutcome<T> {
    Ok(T),
    /// An exclusive, typed refusal before backend submission.
    AdmissionRefused {
        reason: &'static str,
    },
    /// The node refused because it does not lead. `leader` is `None` mid-election.
    NotLeader {
        leader: Option<kv9_common::NodeId>,
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
                Err(status) if status.metadata().contains_key(ADMISSION_REFUSED_KEY) => {
                    admission_refusal(&status)
                        .map(|reason| RawClientOutcome::AdmissionRefused { reason })
                        .ok_or_else(|| {
                            Error::Raft(format!("{label} RPC: invalid admission refusal: {status}"))
                        })
                }
                // The marker, not the code: stale epoch and API-type mismatch also map to
                // FAILED_PRECONDITION, so treating the code as "not leader" would silently
                // convert a real rejection into a pointless redirect.
                Err(status) if not_leader_marked(&status) => {
                    // Read the hint from metadata, never by parsing the message: the
                    // prose is for humans and may be reworded.
                    let leader = status
                        .metadata()
                        .get(LEADER_HINT_KEY)
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.parse::<u64>().ok())
                        .map(kv9_common::NodeId);
                    Ok(RawClientOutcome::NotLeader { leader })
                }
                // A partial write is a *typed* outcome, not prose. Emitting the metadata
                // without parsing it back would leave the contract half-implemented: the
                // server publishes stable fields and the client still shows a sentence.
                Err(status) if partial_write_marked(&status) => {
                    Err(partial_delete_range_from_status(&status))
                }
                // The typed unconfirmed family, decoded from the marker the
                // server sent — exact values only, fail-closed: an unknown
                // value is a protocol error, never a guessed phase.
                Err(status) if status.metadata().get(READ_UNCONFIRMED_KEY).is_some() => {
                    Err(read_unconfirmed_from_status(&status))
                }
                Err(status) => Err(Error::Raft(format!("{label} RPC: {status}"))),
            }
        })
    }

    pub fn put(
        &self,
        key: Vec<u8>,
        value: Vec<u8>,
    ) -> Result<RawClientOutcome<proto::RawWriteResponse>, Error> {
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
            client.raw_put(request).await.map(Response::into_inner)
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

    pub fn delete(&self, key: Vec<u8>) -> Result<RawClientOutcome<proto::RawWriteResponse>, Error> {
        let context = self.context();
        self.call("RawDelete", move |mut client, metadata| async move {
            let request = Request::from_parts(
                metadata,
                Default::default(),
                proto::RawDeleteRequest { context, key },
            );
            client.raw_delete(request).await.map(Response::into_inner)
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
    ) -> Result<RawClientOutcome<proto::RawDeleteRangeResponse>, Error> {
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
            client
                .raw_delete_range(request)
                .await
                .map(Response::into_inner)
        })
    }
}

/// Is this status a not-leader redirect?
///
/// Code **and** marker value. The code alone is ambiguous — stale epoch and API-type
/// mismatch are also `FAILED_PRECONDITION` — and mere presence is not enough either: a
/// header reading `false` asserts the opposite of what we would conclude from it.
///
/// This mirrors [`partial_write_marked`] deliberately. The value-not-presence bug was
/// found in that one and fixed only there; the identical check here survived, because a
/// fix applied to the instance you were shown is not a fix applied to the class.
fn not_leader_marked(status: &Status) -> bool {
    status.code() == tonic::Code::FailedPrecondition
        && status
            .metadata()
            .get(NOT_LEADER_KEY)
            .and_then(|value| value.to_str().ok())
            == Some("true")
}

/// Is this status a partial-write report?
///
/// Both the code *and* the marker: plain `ABORTED` is used for write conflicts, so the
/// code alone would misread an ordinary abort as a half-finished range delete.
fn partial_write_marked(status: &Status) -> bool {
    // The marker's *value* must be `true`, not merely present: a header that says
    // `false` asserts the opposite, and treating it as partial would invent a receipt
    // for a call that reported none.
    status.code() == tonic::Code::Aborted
        && status
            .metadata()
            .get(PARTIAL_WRITE_KEY)
            .and_then(|value| value.to_str().ok())
            == Some("true")
}

/// Decode a partial-write report into the typed error.
///
/// A missing or malformed field is a *protocol* error, never a defaulted zero: inventing
/// "0 chunks committed" for a call that may have deleted a great deal is the most
/// dangerous possible guess.
fn partial_delete_range_from_status(status: &Status) -> Error {
    let field = |key: &str| -> std::result::Result<u64, Error> {
        status
            .metadata()
            .get(key)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| {
                Error::Raft(format!(
                    "partial-write response missing or malformed metadata: {key}"
                ))
            })
    };
    match (
        field(COMMITTED_CHUNKS_KEY),
        field(LAST_APPLIED_TERM_KEY),
        field(LAST_APPLIED_INDEX_KEY),
    ) {
        (Ok(committed_chunks), Ok(last_applied_term), Ok(last_applied_index)) => {
            Error::PartialDeleteRange {
                committed_chunks,
                last_applied_term,
                last_applied_index,
                cause: status.message().to_owned(),
            }
        }
        (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => e,
    }
}

/// Decode the server-sent `kv9-read-unconfirmed` marker back into the SAME
/// typed error the server rendered from — exact values only. `quorum` and
/// `apply` are the whole protocol; anything else means the two ends disagree
/// about the contract, and guessing a default phase would hand the caller a
/// confidently wrong reaction (the two phases demand different ones). The
/// caller only enters here when the marker is PRESENT; absence is not this
/// family (a transport failure carries no server metadata at all).
fn read_unconfirmed_from_status(status: &Status) -> Error {
    // Code + marker TOGETHER are the protocol (the same exclusivity rule as
    // NotLeader and PartialWrite): the server only ever renders this family
    // as UNAVAILABLE, so a marker riding any other code is a contract
    // violation between the two ends — fail closed, never a typed outcome.
    if status.code() != tonic::Code::Unavailable {
        return Error::Raft(format!(
            "protocol error: kv9-read-unconfirmed marker on non-UNAVAILABLE \
             status ({:?}); refusing to treat it as an established outcome",
            status.code()
        ));
    }
    match status
        .metadata()
        .get(READ_UNCONFIRMED_KEY)
        .and_then(|value| value.to_str().ok())
    {
        Some("quorum") => Error::ReadUnconfirmed {
            phase: kv9_common::ReadBarrierPhase::QuorumConfirmation,
        },
        Some("apply") => Error::ReadUnconfirmed {
            phase: kv9_common::ReadBarrierPhase::ApplyCatchUp,
        },
        other => Error::Raft(format!(
            "protocol error: kv9-read-unconfirmed carries unrecognized value {other:?} \
             (expected exactly 'quorum' or 'apply'); refusing to guess a phase"
        )),
    }
}

fn applied_response(applied: crate::api::AppliedPosition) -> proto::RawWriteResponse {
    proto::RawWriteResponse {
        applied_term: applied.term,
        applied_index: applied.index,
    }
}

fn receipt_response(receipt: crate::api::DeleteRangeReceipt) -> proto::RawDeleteRangeResponse {
    let last = receipt
        .last_applied
        .unwrap_or(crate::api::AppliedPosition { term: 0, index: 0 });
    proto::RawDeleteRangeResponse {
        committed_chunks: receipt.committed_chunks,
        last_applied_term: last.term,
        last_applied_index: last.index,
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
        origin: RequestOrigin::from_transport(auth.principal.clone()),
    })
}

fn qualified_key(key: Option<proto::QualifiedKey>) -> Result<QualifiedKey, Status> {
    let key = key.ok_or_else(|| Status::invalid_argument("qualified key is required"))?;
    if key.keyspace_id > KeyspaceId::MAX {
        return Err(Status::invalid_argument("keyspace id exceeds 3-byte width"));
    }
    Ok(QualifiedKey {
        keyspace: KeyspaceId(key.keyspace_id),
        user_key: key.user_key,
    })
}

fn transaction_descriptor(
    transaction: Option<proto::TransactionDescriptor>,
) -> Result<TxnDescriptor, Status> {
    let transaction = transaction
        .ok_or_else(|| Status::invalid_argument("transaction descriptor is required"))?;
    if transaction.keyspace_id > KeyspaceId::MAX {
        return Err(Status::invalid_argument("keyspace id exceeds 3-byte width"));
    }
    let id = transaction
        .id
        .ok_or_else(|| Status::invalid_argument("transaction id is required"))?;
    let primary = qualified_key(transaction.primary)?;
    let keyspace = KeyspaceId(transaction.keyspace_id);
    if primary.keyspace != keyspace {
        return Err(Status::invalid_argument(
            "transaction and primary keyspace ids must match",
        ));
    }
    Ok(TxnDescriptor {
        keyspace,
        id: TxnId {
            txn_group: TxnGroupId(id.txn_group_id),
            timeline: TimelineId(id.timeline_id),
            timeline_generation: TimelineGeneration(id.timeline_generation),
            start_ts: TimeStamp(id.start_ts),
        },
        primary,
    })
}

fn transaction_message(transaction: TxnDescriptor) -> proto::TransactionDescriptor {
    proto::TransactionDescriptor {
        keyspace_id: transaction.keyspace.0,
        id: Some(proto::TransactionId {
            txn_group_id: transaction.id.txn_group.0,
            timeline_id: transaction.id.timeline.0,
            timeline_generation: transaction.id.timeline_generation.0,
            start_ts: transaction.id.start_ts.0,
        }),
        primary: Some(proto::QualifiedKey {
            keyspace_id: transaction.primary.keyspace.0,
            user_key: transaction.primary.user_key,
        }),
    }
}

fn txn_status_response(status: TxnStatus) -> proto::KvCheckTxnStatusResponse {
    let decision = match status {
        TxnStatus::Locked => {
            proto::kv_check_txn_status_response::Decision::Locked(proto::TxnLocked {})
        }
        TxnStatus::Committed { commit_ts } => {
            proto::kv_check_txn_status_response::Decision::CommittedTs(commit_ts.0)
        }
        TxnStatus::RolledBack => {
            proto::kv_check_txn_status_response::Decision::RolledBack(proto::TxnRolledBack {})
        }
    };
    proto::KvCheckTxnStatusResponse {
        decision: Some(decision),
    }
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
    // This match has no `_ =>` arm, and the omission is load-bearing: it is what forces every
    // new Error variant to be given a deliberate protocol mapping instead of drifting into
    // whatever generic code happened to be nearby. The compiler refuses the change until
    // someone decides.
    //
    // Its strength and its fragility are the same fact. Adding a catch-all -- say
    // `_ => Status::internal(message)` -- compiles instantly, leaves every test green, and
    // silently deletes the guarantee: from then on new errors reach clients under a code
    // nobody chose. It would look like tidying up. It is not.
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
        // `internal`, not `invalid_argument`. The basis, stated so it can be falsified: this
        // error must only ever be raised by an internal object-store/drain path, never by
        // anything a request parameter can steer. Under that condition it means one file-id
        // was assigned to two distinct objects -- our invariant, not the caller's mistake --
        // and blaming the caller would send someone hunting through their own request for a
        // fault that is ours.
        //
        // Note what is and is not established today. It is currently unreachable because
        // *nothing* constructs it outside the engine: the drain worker does not exist yet.
        // "Only the drain worker will raise it" is a well-founded expectation about the wiring
        // to come, not a verified property of the code as it stands. RE-CHECK THIS WHEN THE
        // DRAIN IS WIRED: if any request path can then trigger an object write, `internal`
        // starts misattributing our fault to the caller and this arm must be revisited.
        //
        // A fixed string, deliberately NOT `message`. The Display form names the object key,
        // which is an internal physical identifier; the client can do nothing with it and it
        // describes our storage layout. The rich form stays on the Error for logs and
        // diagnostics -- the redaction is at the wire boundary, not at the source, so we do
        // not lose the detail where it is actually useful.
        Error::ObjectContentMismatch { .. } => Status::internal("object store invariant violation"),
        Error::TsoUnavailable(_) | Error::MetaNotReady(_) | Error::Raft(_) => {
            Status::unavailable(message)
        }
        // UNAVAILABLE + a server-sent marker naming which barrier half never
        // arrived. The marker (not the code) is the protocol: transport
        // failures carry no server metadata, so nothing can impersonate the
        // typed unconfirmed family (see READ_UNCONFIRMED_KEY). The match is
        // exhaustive over the phase enum — a new phase forces a wire word.
        Error::ReadUnconfirmed { phase } => {
            let mut status = Status::unavailable(message);
            status.metadata_mut().insert(
                READ_UNCONFIRMED_KEY,
                match phase {
                    kv9_common::ReadBarrierPhase::QuorumConfirmation => "quorum",
                    kv9_common::ReadBarrierPhase::ApplyCatchUp => "apply",
                }
                .parse()
                .expect("static ascii"),
            );
            status
        }
        // Deliberately *not* `unavailable`: this follower is perfectly healthy, the
        // request simply arrived at the wrong node. `unavailable` invites a client to
        // transparently retry the same address, and reports a working node as broken.
        Error::NotLeader { leader } => {
            let mut status = Status::failed_precondition(message);
            // A marker that is ALWAYS present, because the status code alone cannot carry
            // this: stale epoch and API-type mismatch are failed-precondition too, and the
            // leader hint is absent mid-election — so "failed_precondition without a hint"
            // is ambiguous between "wrong node" and "wrong request". A client keying on
            // the code alone would misroute every one of them.
            status
                .metadata_mut()
                .insert(NOT_LEADER_KEY, "true".parse().expect("static ascii"));
            // Machine-readable redirect. The human message is prose and may be reworded;
            // a client that parsed it would break silently when someone edits the text.
            if let Some(node_id) = leader {
                if let Ok(value) = node_id.0.to_string().parse() {
                    status.metadata_mut().insert(LEADER_HINT_KEY, value);
                }
            }
            status
        }
        Error::RangeCrossesRegion => Status::failed_precondition(message),
        // ABORTED, not a generic failure: some chunks committed. The stable metadata is
        // the protocol; the message text is prose and must not be parsed.
        Error::PartialDeleteRange {
            committed_chunks,
            last_applied_term,
            last_applied_index,
            ..
        } => {
            let mut status = Status::aborted(message);
            let meta = status.metadata_mut();
            meta.insert(PARTIAL_WRITE_KEY, "true".parse().expect("static ascii"));
            for (key, value) in [
                (COMMITTED_CHUNKS_KEY, committed_chunks),
                (LAST_APPLIED_TERM_KEY, last_applied_term),
                (LAST_APPLIED_INDEX_KEY, last_applied_index),
            ] {
                // `expect`, not `if let Ok`: a decimal u64 is always valid ASCII, so this
                // cannot fail — and if it somehow did, silently omitting a field the
                // client is contractually required to read would turn a stable protocol
                // into a guess. Fail loud rather than ship a half-populated contract.
                meta.insert(
                    key,
                    value
                        .to_string()
                        .parse()
                        .expect("a decimal u64 is valid ASCII metadata"),
                );
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

/// Marks a refusal as "you reached a node that does not lead", independent of the hint.
///
/// Present on every `NotLeader` status, including when no leader is known. Without it a
/// client cannot separate a misdirected request from a genuinely failed precondition,
/// since both share `FAILED_PRECONDITION`.
pub const NOT_LEADER_KEY: &str = "kv9-not-leader";

/// Exclusive refusal before public backend execution. The code alone is not evidence
/// that a mutation was refused; clients must validate this marker and its value.
pub const ADMISSION_REFUSED_KEY: &str = "kv9-admission-refused";

fn admission_status(reason: Refusal) -> Status {
    let mut status = Status::resource_exhausted("public backend admission limit reached");
    status.metadata_mut().insert(
        ADMISSION_REFUSED_KEY,
        reason.label().parse().expect("static ASCII"),
    );
    status
}

/// Validate the complete public pre-execution refusal contract, failing closed.
pub fn admission_refusal(status: &Status) -> Option<&'static str> {
    if status.code() != tonic::Code::ResourceExhausted
        || [
            NOT_LEADER_KEY,
            LEADER_HINT_KEY,
            READ_UNCONFIRMED_KEY,
            PARTIAL_WRITE_KEY,
            COMMITTED_CHUNKS_KEY,
            LAST_APPLIED_TERM_KEY,
            LAST_APPLIED_INDEX_KEY,
        ]
        .iter()
        .any(|key| status.metadata().contains_key(*key))
        || status
            .metadata()
            .get_all(ADMISSION_REFUSED_KEY)
            .iter()
            .count()
            != 1
    {
        return None;
    }
    match status
        .metadata()
        .get(ADMISSION_REFUSED_KEY)?
        .to_str()
        .ok()?
    {
        "request_count" => Some("request_count"),
        "encoded_bytes" => Some("encoded_bytes"),
        "request_too_large" => Some("request_too_large"),
        _ => None,
    }
}

/// Marks an establishing read that could not confirm its quorum barrier — value is
/// ASCII `quorum` (no live quorum acknowledged the read; the isolated-leader shape)
/// or `apply` (quorum confirmed, local apply lagged; bounded same-node retry can
/// succeed). Presence of this SERVER-SENT marker is what makes the unconfirmed
/// family machine-distinguishable from a transport failure: a severed connection
/// yields a client-side error that carries no server metadata at all, so it can
/// never impersonate this. Shares `UNAVAILABLE` with other conditions — the code
/// alone is deliberately not the protocol (same rule as `NOT_LEADER_KEY`).
pub const READ_UNCONFIRMED_KEY: &str = "kv9-read-unconfirmed";

/// Marks a failure that nonetheless committed part of its work.
pub const PARTIAL_WRITE_KEY: &str = "kv9-partial-write";
/// How many range-delete chunks committed before the failure.
pub const COMMITTED_CHUNKS_KEY: &str = "kv9-committed-chunks";
/// The term of the last chunk that applied.
pub const LAST_APPLIED_TERM_KEY: &str = "kv9-last-applied-term";
/// The index of the last chunk that applied.
pub const LAST_APPLIED_INDEX_KEY: &str = "kv9-last-applied-index";

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
        let reservation = self.reserve(&request, WorkClass::RawRead)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        let value = self
            .backend
            .prepared_read(
                reservation,
                self.backend
                    .inner
                    .clone()
                    .prepare_raw_get(context, request.key),
            )
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
        let reservation = self.reserve(&request, WorkClass::RawRead)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        let values = self
            .backend
            .call(reservation, move |backend| {
                backend.raw_batch_get(&context, &request.keys)
            })
            .await?;
        Ok(Response::new(proto::RawBatchGetResponse {
            values: values.into_iter().map(optional_value).collect(),
        }))
    }

    async fn raw_put(
        &self,
        request: Request<proto::RawPutRequest>,
    ) -> Result<Response<proto::RawWriteResponse>, Status> {
        let auth = auth_context(&request)?;
        let reservation = self.reserve(&request, WorkClass::RawWrite)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        let applied = self
            .backend
            .prepared_write(
                reservation,
                self.backend.inner.clone().prepare_raw_write(
                    context,
                    crate::api::RawWrite::Put {
                        key: request.key,
                        value: request.value,
                    },
                ),
            )
            .await?;
        Ok(Response::new(applied_response(applied)))
    }

    async fn raw_batch_put(
        &self,
        request: Request<proto::RawBatchPutRequest>,
    ) -> Result<Response<proto::RawWriteResponse>, Status> {
        let auth = auth_context(&request)?;
        let reservation = self.reserve(&request, WorkClass::RawWrite)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        let pairs: Vec<_> = request
            .pairs
            .into_iter()
            .map(|pair| (pair.key, pair.value))
            .collect();
        let applied = self
            .backend
            .prepared_write(
                reservation,
                self.backend
                    .inner
                    .clone()
                    .prepare_raw_write(context, crate::api::RawWrite::BatchPut(pairs)),
            )
            .await?;
        Ok(Response::new(applied_response(applied)))
    }

    async fn raw_delete(
        &self,
        request: Request<proto::RawDeleteRequest>,
    ) -> Result<Response<proto::RawWriteResponse>, Status> {
        let auth = auth_context(&request)?;
        let reservation = self.reserve(&request, WorkClass::RawWrite)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        self.backend
            .prepared_write(
                reservation,
                self.backend
                    .inner
                    .clone()
                    .prepare_raw_write(context, crate::api::RawWrite::Delete { key: request.key }),
            )
            .await
            .map(applied_response)
            .map(Response::new)
    }

    async fn raw_scan(
        &self,
        request: Request<proto::RawScanRequest>,
    ) -> Result<Response<proto::ScanResponse>, Status> {
        let auth = auth_context(&request)?;
        let reservation = self.reserve(&request, WorkClass::RawRead)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        let limit = nonzero_limit(request.limit)?;
        let pairs = self
            .backend
            .call(reservation, move |backend| {
                backend.raw_scan(&context, &request.start, &request.end, limit)
            })
            .await?;
        Ok(Response::new(scan_response(pairs)))
    }

    async fn raw_delete_range(
        &self,
        request: Request<proto::RawDeleteRangeRequest>,
    ) -> Result<Response<proto::RawDeleteRangeResponse>, Status> {
        let auth = auth_context(&request)?;
        let reservation = self.reserve(&request, WorkClass::RawWrite)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        self.backend
            .call(reservation, move |backend| {
                backend.raw_delete_range(&context, &request.start, &request.end)
            })
            .await
            .map(receipt_response)
            .map(Response::new)
    }

    async fn kv_begin(
        &self,
        request: Request<proto::KvBeginRequest>,
    ) -> Result<Response<proto::KvBeginResponse>, Status> {
        let auth = auth_context(&request)?;
        let reservation = self.reserve(&request, WorkClass::Transaction)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        let primary = qualified_key(request.primary)?;
        let transaction = self
            .backend
            .call(reservation, move |backend| {
                backend.kv_begin(&context, primary)
            })
            .await?;
        Ok(Response::new(proto::KvBeginResponse {
            transaction: Some(transaction_message(transaction)),
        }))
    }

    async fn kv_get(
        &self,
        request: Request<proto::KvGetRequest>,
    ) -> Result<Response<proto::KvGetResponse>, Status> {
        let auth = auth_context(&request)?;
        let reservation = self.reserve(&request, WorkClass::Transaction)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        let transaction = transaction_descriptor(request.transaction)?;
        let value = self
            .backend
            .call(reservation, move |backend| {
                backend.kv_get(&context, &request.key, &transaction)
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
        let reservation = self.reserve(&request, WorkClass::Transaction)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        let transaction = transaction_descriptor(request.transaction)?;
        let values = self
            .backend
            .call(reservation, move |backend| {
                backend.kv_batch_get(&context, &request.keys, &transaction)
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
        let reservation = self.reserve(&request, WorkClass::Transaction)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        let limit = nonzero_limit(request.limit)?;
        let transaction = transaction_descriptor(request.transaction)?;
        let pairs = self
            .backend
            .call(reservation, move |backend| {
                backend.kv_scan(&context, &request.start, &request.end, limit, &transaction)
            })
            .await?;
        Ok(Response::new(scan_response(pairs)))
    }

    async fn kv_prewrite(
        &self,
        request: Request<proto::KvPrewriteRequest>,
    ) -> Result<Response<proto::Empty>, Status> {
        let auth = auth_context(&request)?;
        let reservation = self.reserve(&request, WorkClass::Transaction)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        let transaction = transaction_descriptor(request.transaction)?;
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
            .call(reservation, move |backend| {
                backend.kv_prewrite(&context, &mutations, &transaction)
            })
            .await?;
        Ok(Response::new(proto::Empty {}))
    }

    async fn kv_commit(
        &self,
        request: Request<proto::KvCommitRequest>,
    ) -> Result<Response<proto::Empty>, Status> {
        let auth = auth_context(&request)?;
        let reservation = self.reserve(&request, WorkClass::Transaction)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        let transaction = transaction_descriptor(request.transaction)?;
        self.backend
            .call(reservation, move |backend| {
                backend.kv_commit(&context, &request.keys, &transaction)
            })
            .await?;
        Ok(Response::new(proto::Empty {}))
    }

    async fn kv_pessimistic_lock(
        &self,
        request: Request<proto::KvPessimisticLockRequest>,
    ) -> Result<Response<proto::Empty>, Status> {
        let auth = auth_context(&request)?;
        let reservation = self.reserve(&request, WorkClass::Transaction)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        let transaction = transaction_descriptor(request.transaction)?;
        self.backend
            .call(reservation, move |backend| {
                backend.kv_pessimistic_lock(&context, &request.keys, &transaction)
            })
            .await?;
        Ok(Response::new(proto::Empty {}))
    }

    async fn kv_pessimistic_rollback(
        &self,
        request: Request<proto::KvPessimisticRollbackRequest>,
    ) -> Result<Response<proto::Empty>, Status> {
        let auth = auth_context(&request)?;
        let reservation = self.reserve(&request, WorkClass::Transaction)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        let transaction = transaction_descriptor(request.transaction)?;
        self.backend
            .call(reservation, move |backend| {
                backend.kv_pessimistic_rollback(&context, &request.keys, &transaction)
            })
            .await?;
        Ok(Response::new(proto::Empty {}))
    }

    async fn kv_resolve_lock(
        &self,
        request: Request<proto::KvResolveLockRequest>,
    ) -> Result<Response<proto::Empty>, Status> {
        let auth = auth_context(&request)?;
        let reservation = self.reserve(&request, WorkClass::Transaction)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        let transaction = transaction_descriptor(request.transaction)?;
        self.backend
            .call(reservation, move |backend| {
                backend.kv_resolve_lock(&context, &transaction)
            })
            .await?;
        Ok(Response::new(proto::Empty {}))
    }

    async fn kv_cleanup(
        &self,
        request: Request<proto::KvCleanupRequest>,
    ) -> Result<Response<proto::Empty>, Status> {
        let auth = auth_context(&request)?;
        let reservation = self.reserve(&request, WorkClass::Transaction)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        let transaction = transaction_descriptor(request.transaction)?;
        self.backend
            .call(reservation, move |backend| {
                backend.kv_cleanup(&context, &request.key, &transaction)
            })
            .await?;
        Ok(Response::new(proto::Empty {}))
    }

    async fn kv_check_txn_status(
        &self,
        request: Request<proto::KvCheckTxnStatusRequest>,
    ) -> Result<Response<proto::KvCheckTxnStatusResponse>, Status> {
        let auth = auth_context(&request)?;
        let reservation = self.reserve(&request, WorkClass::Transaction)?;
        let request = request.into_inner();
        let context = request_context(request.context, &auth)?;
        let transaction = transaction_descriptor(request.transaction)?;
        let status = self
            .backend
            .call(reservation, move |backend| {
                backend.kv_check_txn_status(&context, &transaction)
            })
            .await?;
        Ok(Response::new(txn_status_response(status)))
    }

    async fn create_keyspace(
        &self,
        request: Request<proto::CreateKeyspaceRequest>,
    ) -> Result<Response<proto::CreateKeyspaceResponse>, Status> {
        let auth = auth_context(&request)?;
        let reservation = self.reserve(&request, WorkClass::MetadataWrite)?;
        let request = request.into_inner();
        let api_type = api_type(request.api_type)?;
        let caller = auth.principal.to_string();
        let id = self
            .backend
            .call(reservation, move |backend| {
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
        let reservation = self.reserve(&request, WorkClass::MetadataRead)?;
        let caller = auth.principal.to_string();
        let keyspaces = self
            .backend
            .call(reservation, move |backend| backend.list_keyspaces(&caller))
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
        let reservation = self.reserve(&request, WorkClass::MetadataRead)?;
        let request = request.into_inner();
        if request.keyspace_id > KeyspaceId::MAX {
            return Err(Status::invalid_argument("keyspace id exceeds 3-byte width"));
        }
        let caller = auth.principal.to_string();
        let region = self
            .backend
            .call(reservation, move |backend| {
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
        let reservation = self.reserve(&request, WorkClass::MetadataWrite)?;
        let request = request.into_inner();
        let caller = auth.principal.to_string();
        self.backend
            .call(reservation, move |backend| {
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
        let reservation = self.reserve(&request, WorkClass::MetadataRead)?;
        let caller = auth.principal.to_string();
        let info = self
            .backend
            .call(reservation, move |backend| backend.cluster_info(&caller))
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
        let reservation = self.reserve(&request, WorkClass::MetadataWrite)?;
        let request = request.into_inner();
        let caller = auth.principal.to_string();
        let result = self
            .backend
            .call(reservation, move |backend| {
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
            join_ticket: result.join_ticket.unwrap_or_default(),
        }))
    }

    async fn promote_node(
        &self,
        request: Request<proto::PromoteNodeRequest>,
    ) -> Result<Response<proto::MembershipChangeResponse>, Status> {
        let auth = auth_context(&request)?;
        let reservation = self.reserve(&request, WorkClass::MetadataWrite)?;
        let request = request.into_inner();
        let caller = auth.principal.to_string();
        let result = self
            .backend
            .call(reservation, move |backend| {
                backend.promote_node(&caller, NodeId(request.node_id))
            })
            .await?;
        Ok(Response::new(proto::MembershipChangeResponse {
            applied_term: result.applied.term,
            applied_index: result.applied.index,
            voters: result.voters,
            learners: result.learners,
            join_ticket: String::new(),
        }))
    }

    async fn get_node_endpoint(
        &self,
        request: Request<proto::GetNodeEndpointRequest>,
    ) -> Result<Response<proto::GetNodeEndpointResponse>, Status> {
        let auth = auth_context(&request)?;
        let reservation = self.reserve(&request, WorkClass::MetadataRead)?;
        let caller = auth.principal.to_string();
        let node = NodeId(request.into_inner().node_id);
        let result = self
            .backend
            .call(reservation, move |backend| {
                backend.get_node_endpoint(&caller, node)
            })
            .await?;
        Ok(Response::new(proto::GetNodeEndpointResponse {
            cluster_id: result.cluster.as_bytes().to_vec(),
            endpoint: result.endpoint.map(crate::endpoints::encode_endpoint),
        }))
    }

    async fn change_node_endpoint(
        &self,
        request: Request<proto::ChangeNodeEndpointRequest>,
    ) -> Result<Response<proto::ChangeNodeEndpointResponse>, Status> {
        let auth = auth_context(&request)?;
        let reservation = self.reserve(&request, WorkClass::MetadataWrite)?;
        let caller = auth.principal.to_string();
        let change = crate::endpoints::decode_change(request.into_inner()).map_err(error_status)?;
        let result = self
            .backend
            .call(reservation, move |backend| {
                backend.change_node_endpoint(&caller, change)
            })
            .await?;
        Ok(Response::new(crate::endpoints::encode_update(result)))
    }
}

#[cfg(test)]
mod tests {
    mod admission;

    use std::sync::Mutex;

    use kv9_common::{Keyspace, Result, UserKey, Value};
    use tonic::Code;

    use super::*;
    use crate::api::{ClusterInfo, RegionLocation};

    struct RawPreparationGate {
        entered: tokio::sync::mpsc::UnboundedSender<()>,
        release: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
        dropped: tokio::sync::mpsc::UnboundedSender<()>,
    }

    #[derive(Default)]
    struct FakeBackend {
        raw_preparation_gate: Option<RawPreparationGate>,
        raw_completed: bool,
        membership_hint: Option<NodeId>,
        raw_gate: Option<(
            tokio::sync::mpsc::UnboundedSender<()>,
            Mutex<std::sync::mpsc::Receiver<()>>,
        )>,
        callers: Mutex<Vec<String>>,
    }

    impl RawApi for FakeBackend {
        fn prepare_raw_get(
            self: Arc<Self>,
            ctx: RequestContext,
            key: UserKey,
        ) -> crate::api::RawReadPreparation<Option<Value>> {
            Box::pin(async move {
                if let Some(gate) = &self.raw_preparation_gate {
                    struct Dropped(tokio::sync::mpsc::UnboundedSender<()>);
                    impl Drop for Dropped {
                        fn drop(&mut self) {
                            let _ = self.0.send(());
                        }
                    }
                    let _dropped = Dropped(gate.dropped.clone());
                    let release = gate
                        .release
                        .lock()
                        .unwrap()
                        .take()
                        .expect("one controlled preparation per backend");
                    gate.entered.send(()).unwrap();
                    release.await.expect("test must release held preparation");
                }
                if self.raw_completed {
                    return self
                        .raw_get(&ctx, &key)
                        .map(crate::api::RawReadJob::Completed);
                }
                Ok(crate::api::RawReadJob::Blocking(Box::new(move || {
                    self.raw_get(&ctx, &key)
                })))
            })
        }

        fn raw_get(&self, ctx: &RequestContext, _key: &[u8]) -> Result<Option<Value>> {
            self.callers
                .lock()
                .unwrap()
                .push(ctx.origin.label().to_owned());
            if let Some((entered, release)) = &self.raw_gate {
                entered.send(()).unwrap();
                release
                    .lock()
                    .unwrap()
                    .recv_timeout(std::time::Duration::from_secs(10))
                    .expect("test must release held backend");
            }
            Ok(None)
        }
        fn raw_batch_get(&self, _: &RequestContext, _: &[UserKey]) -> Result<Vec<Option<Value>>> {
            Err(Error::NotImplemented("raw_batch_get"))
        }
        fn raw_put(
            &self,
            _: &RequestContext,
            _: UserKey,
            _: Value,
        ) -> Result<crate::api::AppliedPosition> {
            Err(Error::NotImplemented("raw_put"))
        }
        fn raw_batch_put(
            &self,
            _: &RequestContext,
            _: &[(UserKey, Value)],
        ) -> Result<crate::api::AppliedPosition> {
            Err(Error::NotImplemented("raw_batch_put"))
        }
        fn raw_delete(&self, _: &RequestContext, _: &[u8]) -> Result<crate::api::AppliedPosition> {
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
        fn raw_delete_range(
            &self,
            _: &RequestContext,
            _: &[u8],
            _: &[u8],
        ) -> Result<crate::api::DeleteRangeReceipt> {
            Err(Error::NotImplemented("raw_delete_range"))
        }
    }

    impl TxnApi for FakeBackend {
        fn kv_begin(&self, _: &RequestContext, _: QualifiedKey) -> Result<TxnDescriptor> {
            Err(Error::NotImplemented("kv_begin"))
        }
        fn kv_get(&self, _: &RequestContext, _: &[u8], _: &TxnDescriptor) -> Result<Option<Value>> {
            Err(Error::NotImplemented("kv_get"))
        }
        fn kv_batch_get(
            &self,
            _: &RequestContext,
            _: &[UserKey],
            _: &TxnDescriptor,
        ) -> Result<Vec<Option<Value>>> {
            Err(Error::NotImplemented("kv_batch_get"))
        }
        fn kv_scan(
            &self,
            _: &RequestContext,
            _: &[u8],
            _: &[u8],
            _: usize,
            _: &TxnDescriptor,
        ) -> Result<Vec<(UserKey, Value)>> {
            Err(Error::NotImplemented("kv_scan"))
        }
        fn kv_prewrite(
            &self,
            _: &RequestContext,
            _: &[(UserKey, Option<Value>)],
            _: &TxnDescriptor,
        ) -> Result<()> {
            Err(Error::NotImplemented("kv_prewrite"))
        }
        fn kv_commit(&self, _: &RequestContext, _: &[UserKey], _: &TxnDescriptor) -> Result<()> {
            Err(Error::NotImplemented("kv_commit"))
        }
        fn kv_pessimistic_lock(
            &self,
            _: &RequestContext,
            _: &[UserKey],
            _: &TxnDescriptor,
        ) -> Result<()> {
            Err(Error::NotImplemented("kv_pessimistic_lock"))
        }
        fn kv_pessimistic_rollback(
            &self,
            _: &RequestContext,
            _: &[UserKey],
            _: &TxnDescriptor,
        ) -> Result<()> {
            Err(Error::NotImplemented("kv_pessimistic_rollback"))
        }
        fn kv_resolve_lock(&self, _: &RequestContext, _: &TxnDescriptor) -> Result<()> {
            Err(Error::NotImplemented("kv_resolve_lock"))
        }
        fn kv_cleanup(&self, _: &RequestContext, _: &[u8], _: &TxnDescriptor) -> Result<()> {
            Err(Error::NotImplemented("kv_cleanup"))
        }
        fn kv_check_txn_status(&self, _: &RequestContext, _: &TxnDescriptor) -> Result<TxnStatus> {
            Err(Error::NotImplemented("kv_check_txn_status"))
        }
    }

    impl AdminApi for FakeBackend {
        fn admit_node(
            &self,
            _: &str,
            _: NodeId,
            _: &str,
            _: u64,
        ) -> Result<crate::api::MembershipChangeResult> {
            Err(Error::NotLeader {
                leader: self.membership_hint,
            })
        }

        fn promote_node(&self, _: &str, _: NodeId) -> Result<crate::api::MembershipChangeResult> {
            Err(Error::NotLeader {
                leader: self.membership_hint,
            })
        }

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

    fn transaction_descriptor_message() -> proto::TransactionDescriptor {
        proto::TransactionDescriptor {
            keyspace_id: 7,
            id: Some(proto::TransactionId {
                txn_group_id: 11,
                timeline_id: 13,
                timeline_generation: 17,
                start_ts: 19,
            }),
            primary: Some(proto::QualifiedKey {
                keyspace_id: 7,
                user_key: b"primary".to_vec(),
            }),
        }
    }

    #[test]
    fn transaction_descriptor_conversion_preserves_every_identity_coordinate() {
        let decoded = transaction_descriptor(Some(transaction_descriptor_message())).unwrap();
        assert_eq!(decoded.keyspace, KeyspaceId(7));
        assert_eq!(decoded.id.txn_group, TxnGroupId(11));
        assert_eq!(decoded.id.timeline, TimelineId(13));
        assert_eq!(decoded.id.timeline_generation, TimelineGeneration(17));
        assert_eq!(decoded.id.start_ts, TimeStamp(19));
        assert_eq!(decoded.primary.keyspace, KeyspaceId(7));
        assert_eq!(decoded.primary.user_key, b"primary");

        assert_eq!(
            transaction_message(decoded),
            transaction_descriptor_message()
        );
    }

    #[test]
    fn transaction_descriptor_rejects_missing_or_mismatched_qualified_identity() {
        assert_eq!(
            transaction_descriptor(None).unwrap_err().code(),
            Code::InvalidArgument
        );

        let mut missing_id = transaction_descriptor_message();
        missing_id.id = None;
        assert_eq!(
            transaction_descriptor(Some(missing_id))
                .unwrap_err()
                .message(),
            "transaction id is required"
        );

        let mut mismatched = transaction_descriptor_message();
        mismatched.primary.as_mut().unwrap().keyspace_id = 8;
        assert_eq!(
            transaction_descriptor(Some(mismatched))
                .unwrap_err()
                .message(),
            "transaction and primary keyspace ids must match"
        );

        let mut oversized = transaction_descriptor_message();
        oversized.primary.as_mut().unwrap().keyspace_id = KeyspaceId::MAX + 1;
        assert_eq!(
            transaction_descriptor(Some(oversized))
                .unwrap_err()
                .message(),
            "keyspace id exceeds 3-byte width"
        );
    }

    #[test]
    fn transaction_status_has_three_explicit_wire_decisions() {
        use proto::kv_check_txn_status_response::Decision;

        assert_eq!(
            txn_status_response(TxnStatus::Locked).decision,
            Some(Decision::Locked(proto::TxnLocked {}))
        );
        assert_eq!(
            txn_status_response(TxnStatus::Committed {
                commit_ts: TimeStamp(23),
            })
            .decision,
            Some(Decision::CommittedTs(23))
        );
        assert_eq!(
            txn_status_response(TxnStatus::RolledBack).decision,
            Some(Decision::RolledBack(proto::TxnRolledBack {}))
        );
    }

    #[test]
    fn membership_refusal_rejects_ambiguous_wire_metadata() {
        for rpc in ["AdmitNode", "PromoteNode"] {
            let refused = || {
                error_status(Error::NotLeader {
                    leader: Some(NodeId(7)),
                })
            };
            let mut duplicate = refused();
            duplicate
                .metadata_mut()
                .append(NOT_LEADER_KEY, "true".parse().unwrap());
            let mut mixed = refused();
            mixed
                .metadata_mut()
                .insert(PARTIAL_WRITE_KEY, "true".parse().unwrap());
            let mut malformed = refused();
            malformed
                .metadata_mut()
                .insert(LEADER_HINT_KEY, "07".parse().unwrap());
            let mut future = refused();
            future
                .metadata_mut()
                .insert("kv9-future-result", "unknown".parse().unwrap());
            for status in [
                duplicate,
                mixed,
                malformed,
                future,
                Status::failed_precondition("not leader; try node 7"),
                Status::unavailable("response was lost after commitment"),
            ] {
                assert!(
                    matches!(membership_rpc_error(rpc, status), Error::Raft(_)),
                    "ambiguous membership outcome was converted to a retryable refusal"
                );
            }
        }
    }

    #[tokio::test]
    async fn blocking_membership_clients_preserve_real_wire_refusals() {
        for hint in [None, Some(NodeId(7))] {
            let backend = Arc::new(FakeBackend {
                membership_hint: hint,
                ..Default::default()
            });
            let authenticator =
                Arc::new(TokenAuthenticator::new([("wire-secret", "wire-client")]).unwrap());
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap().to_string();
            let (stop, stopped) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(
                tonic::transport::Server::builder()
                    .add_service(Kv9Grpc::new(backend).authenticated_service(authenticator))
                    .serve_with_incoming_shutdown(
                        tokio_stream::wrappers::TcpListenerStream::new(listener),
                        async {
                            let _ = stopped.await;
                        },
                    ),
            );
            tokio::task::spawn_blocking(move || {
                for result in [
                    admit_node_blocking(
                        &address,
                        "wire-secret",
                        NodeId(4),
                        "127.0.0.1:12345".into(),
                        120,
                    ),
                    promote_node_blocking(&address, "wire-secret", NodeId(4)),
                ] {
                    assert!(
                        matches!(result, Err(Error::NotLeader { leader }) if leader == hint),
                        "membership wire refusal lost its typed leader hint: {result:?}"
                    );
                }
                assert!(
                    matches!(
                        admit_node_blocking(
                            &address,
                            "bad-token",
                            NodeId(4),
                            "127.0.0.1:12345".into(),
                            120
                        ),
                        Err(Error::Raft(_))
                    ),
                    "authentication failure cannot become a retryable refusal"
                );
            })
            .await
            .unwrap();
            stop.send(()).unwrap();
            server.await.unwrap().unwrap();
        }
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

    /// The four fields are the contract; the prose is not. Pins both the code and every
    /// metadata key a client is required to read.
    #[test]
    fn partial_delete_range_maps_to_aborted_with_the_full_receipt() {
        let status = error_status(Error::PartialDeleteRange {
            committed_chunks: 3,
            last_applied_term: 4,
            last_applied_index: 91,
            cause: "engine error: disk full".into(),
        });
        assert_eq!(status.code(), Code::Aborted);
        let get = |key: &str| {
            status
                .metadata()
                .get(key)
                .map(|v| v.to_str().unwrap().to_owned())
        };
        assert_eq!(get(PARTIAL_WRITE_KEY), Some("true".to_owned()));
        assert_eq!(get(COMMITTED_CHUNKS_KEY), Some("3".to_owned()));
        assert_eq!(get(LAST_APPLIED_TERM_KEY), Some("4".to_owned()));
        assert_eq!(get(LAST_APPLIED_INDEX_KEY), Some("91".to_owned()));
    }

    /// A plain abort (write conflict) must not be mistaken for a half-finished range
    /// delete: the code alone is ambiguous, which is why the marker exists.
    #[test]
    fn an_ordinary_abort_is_not_read_as_a_partial_write() {
        let conflict = error_status(Error::WriteConflict("busy".into()));
        assert_eq!(conflict.code(), Code::Aborted, "control: same code");
        assert!(
            !partial_write_marked(&conflict),
            "an ordinary abort must not be decoded as a partial write"
        );

        let partial = error_status(Error::PartialDeleteRange {
            committed_chunks: 1,
            last_applied_term: 1,
            last_applied_index: 2,
            cause: String::new(),
        });
        assert!(
            partial_write_marked(&partial),
            "control: this one is partial"
        );
    }

    /// Round-trips the receipt, and refuses to invent numbers when a field is missing --
    /// defaulting to zero would report "nothing committed" for a call that deleted plenty.
    #[test]
    fn a_partial_receipt_round_trips_and_a_missing_field_is_a_protocol_error() {
        let status = error_status(Error::PartialDeleteRange {
            committed_chunks: 5,
            last_applied_term: 2,
            last_applied_index: 77,
            cause: "boom".into(),
        });
        match partial_delete_range_from_status(&status) {
            Error::PartialDeleteRange {
                committed_chunks,
                last_applied_term,
                last_applied_index,
                ..
            } => {
                assert_eq!(
                    (committed_chunks, last_applied_term, last_applied_index),
                    (5, 2, 77)
                );
            }
            other => panic!("expected a partial receipt, got {other}"),
        }

        let mut truncated = Status::aborted("partial");
        truncated
            .metadata_mut()
            .insert(PARTIAL_WRITE_KEY, "true".parse().unwrap());
        assert!(
            matches!(partial_delete_range_from_status(&truncated), Error::Raft(_)),
            "a missing count must be a protocol error, never a defaulted zero"
        );
    }

    /// Same rule as the partial marker, and pinned separately because the two checks live
    /// in different code paths — fixing one taught nothing to the other.
    #[test]
    fn the_not_leader_marker_must_say_true_not_merely_exist() {
        for (value, expected) in [
            ("true", true),
            ("false", false),
            ("yes", false),
            ("", false),
        ] {
            let mut status = Status::failed_precondition("x");
            if !value.is_empty() {
                status
                    .metadata_mut()
                    .insert(NOT_LEADER_KEY, value.parse().unwrap());
            }
            assert_eq!(not_leader_marked(&status), expected, "marker {value:?}");
        }
        // Control: a real NotLeader still decodes, and an unrelated failed-precondition
        // (stale epoch) does not become a redirect.
        assert!(not_leader_marked(&error_status(Error::NotLeader {
            leader: Some(NodeId(7))
        })));
        assert!(!not_leader_marked(&error_status(Error::StaleEpoch {
            region: RegionId(1)
        })));
    }

    /// The marker's value carries meaning: `false` asserts the opposite of `true`, so
    /// accepting mere presence would invent a receipt for a call that reported none.
    #[test]
    fn the_partial_marker_must_say_true_not_merely_exist() {
        for (value, expected) in [
            ("true", true),
            ("false", false),
            ("yes", false),
            ("", false),
        ] {
            let mut status = Status::aborted("x");
            if !value.is_empty() {
                status
                    .metadata_mut()
                    .insert(PARTIAL_WRITE_KEY, value.parse().unwrap());
            }
            assert_eq!(
                partial_write_marked(&status),
                expected,
                "marker value {value:?} should decide partial={expected}"
            );
        }
    }

    /// A client's redirect decision must key on the status code and a stable metadata
    /// key, never on the human-readable message — prose gets reworded, and a client that
    /// parsed it would start silently failing to redirect.
    #[test]
    fn not_leader_maps_to_failed_precondition_and_carries_the_hint_only_when_known() {
        // With a known leader: redirectable.
        let status = error_status(Error::NotLeader {
            leader: Some(NodeId(7)),
        });
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
            error_status(Error::NotLeader {
                leader: Some(NodeId(7))
            })
            .code(),
            error_status(Error::Raft("stepped down".into())).code()
        );
    }

    /// The unconfirmed family survives the wire IN BOTH DIRECTIONS: server
    /// renders phase → marker word, client decodes marker word → the SAME
    /// typed error. Both phases pinned individually (a single-defect mutant
    /// that renders the apply branch as "quorum" reds on the ApplyCatchUp
    /// row). Fail-closed: an unrecognized marker value is a protocol error,
    /// never a guessed phase; a plain UNAVAILABLE without the marker never
    /// decodes into this family (transport failures carry no server
    /// metadata, so nothing can impersonate it).
    #[test]
    fn read_unconfirmed_roundtrips_typed_in_both_phases() {
        use kv9_common::ReadBarrierPhase;
        for phase in [
            ReadBarrierPhase::QuorumConfirmation,
            ReadBarrierPhase::ApplyCatchUp,
        ] {
            let status = error_status(Error::ReadUnconfirmed { phase });
            assert_eq!(status.code(), Code::Unavailable);
            let decoded = read_unconfirmed_from_status(&status);
            assert!(
                matches!(decoded, Error::ReadUnconfirmed { phase: p } if p == phase),
                "phase {phase:?} must roundtrip to the SAME typed error \
                 (an inverted or collapsed mapping reds here): {decoded:?}"
            );
        }

        // Fail-closed: a marker value outside the two-word protocol is a
        // protocol error — the client must refuse to guess a phase.
        let mut garbled = Status::unavailable("read barrier unconfirmed");
        garbled
            .metadata_mut()
            .insert(READ_UNCONFIRMED_KEY, "later".parse().unwrap());
        let decoded = read_unconfirmed_from_status(&garbled);
        assert!(
            matches!(&decoded, Error::Raft(m) if m.contains("unrecognized")),
            "an unknown marker value must decode as a protocol error, never \
             a guessed phase: {decoded:?}"
        );

        // Code + marker exclusivity (Tess's blocker on fead4ed): a VALID
        // marker riding the WRONG code must not enter the typed family —
        // the server never renders this family under any code but
        // UNAVAILABLE, so the combination means the ends disagree.
        let mut wrong_code = Status::failed_precondition("not this family");
        wrong_code
            .metadata_mut()
            .insert(READ_UNCONFIRMED_KEY, "quorum".parse().unwrap());
        let decoded = read_unconfirmed_from_status(&wrong_code);
        assert!(
            matches!(&decoded, Error::Raft(m) if m.contains("non-UNAVAILABLE")),
            "wrong code + valid marker must fail closed as a protocol error, \
             never decode as ReadUnconfirmed: {decoded:?}"
        );

        // A plain UNAVAILABLE carries no marker: it is NOT this family. The
        // decode arm is only entered on marker presence — pinned here so the
        // predicate cannot silently widen to code-only matching.
        let plain = Status::unavailable("some other unavailability");
        assert!(
            plain.metadata().get(READ_UNCONFIRMED_KEY).is_none(),
            "control: the plain status must not carry the marker"
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

        // Internal, and the wire text must not carry the object key: it is a physical
        // identifier describing our storage layout, useless to a client. The redaction lives
        // at this boundary, so a later switch back to the generic `message` -- which does
        // name the key -- has to fail here.
        let mismatch = error_status(Error::ObjectContentMismatch {
            key: "sst/secret-layout-000001".into(),
        });
        assert_eq!(mismatch.code(), Code::Internal);
        assert!(
            !mismatch.message().contains("sst/secret-layout-000001"),
            "object key leaked to the wire: {}",
            mismatch.message()
        );
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
