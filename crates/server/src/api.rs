//! The v0 API surface as Rust traits (DESIGN §11).
//!
//! Transport is gRPC; these traits are the synchronous core contract behind tonic's
//! blocking boundary, with an optional asynchronous point-read preparation. Every data request
//! carries `(keyspace_id, region_epoch)` so the router can resolve keyspace→region,
//! epoch-check, and validate the API type against the keyspace declaration.

use std::sync::Arc;

use kv9_common::{KeyspaceId, Result, UserKey, Value};
use kv9_region::RegionEpoch;
use kv9_txn::{QualifiedKey, TxnDescriptor, TxnStatus};

/// Transport-established origin label for a request.
///
/// This is deliberately not named `Principal`: preserving the current interceptor-derived
/// label does not add TLS, tenant ACLs, or a tenant-isolation security claim.  Its private
/// representation prevents request-body decoding from constructing the label directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestOrigin(Arc<str>);

impl RequestOrigin {
    pub(crate) fn from_transport(label: impl Into<Arc<str>>) -> Self {
        Self(label.into())
    }

    pub fn label(&self) -> &str {
        &self.0
    }
}

/// Context threaded on every data request (DESIGN §11).
#[derive(Debug, Clone)]
pub struct RequestContext {
    pub keyspace: KeyspaceId,
    pub region_epoch: RegionEpoch,
    /// Transport-derived label. It is not authentication or authorization.
    pub origin: RequestOrigin,
}

/// The transactional API for `txn` keyspaces (DESIGN §11 Txn surface).
pub trait TxnApi {
    fn kv_begin(&self, ctx: &RequestContext, primary: QualifiedKey) -> Result<TxnDescriptor>;
    fn kv_get(
        &self,
        ctx: &RequestContext,
        key: &[u8],
        transaction: &TxnDescriptor,
    ) -> Result<Option<Value>>;
    fn kv_batch_get(
        &self,
        ctx: &RequestContext,
        keys: &[UserKey],
        transaction: &TxnDescriptor,
    ) -> Result<Vec<Option<Value>>>;
    fn kv_scan(
        &self,
        ctx: &RequestContext,
        start: &[u8],
        end: &[u8],
        limit: usize,
        transaction: &TxnDescriptor,
    ) -> Result<Vec<(UserKey, Value)>>;
    fn kv_prewrite(
        &self,
        ctx: &RequestContext,
        mutations: &[(UserKey, Option<Value>)],
        transaction: &TxnDescriptor,
    ) -> Result<()>;
    fn kv_commit(
        &self,
        ctx: &RequestContext,
        keys: &[UserKey],
        transaction: &TxnDescriptor,
    ) -> Result<()>;
    fn kv_pessimistic_lock(
        &self,
        ctx: &RequestContext,
        keys: &[UserKey],
        transaction: &TxnDescriptor,
    ) -> Result<()>;
    fn kv_pessimistic_rollback(
        &self,
        ctx: &RequestContext,
        keys: &[UserKey],
        transaction: &TxnDescriptor,
    ) -> Result<()>;
    fn kv_resolve_lock(&self, ctx: &RequestContext, transaction: &TxnDescriptor) -> Result<()>;
    fn kv_cleanup(
        &self,
        ctx: &RequestContext,
        key: &[u8],
        transaction: &TxnDescriptor,
    ) -> Result<()>;
    fn kv_check_txn_status(
        &self,
        ctx: &RequestContext,
        transaction: &TxnDescriptor,
    ) -> Result<TxnStatus>;
}

/// The raw API for `raw` keyspaces (DESIGN §11 Raw surface).
pub trait RawApi: Send + Sync + 'static {
    /// Prepare a point read without blocking an async worker. The default
    /// defers all work to the blocking boundary. Implementations may complete
    /// a memory-only read after its quorum credential, using only try-locks;
    /// contended or potentially blocking work must return a blocking job.
    fn prepare_raw_get(
        self: Arc<Self>,
        ctx: RequestContext,
        key: UserKey,
    ) -> RawReadPreparation<Option<Value>> {
        Box::pin(async move {
            Ok(RawReadJob::Blocking(Box::new(move || {
                self.raw_get(&ctx, &key)
            })))
        })
    }

    fn raw_get(&self, ctx: &RequestContext, key: &[u8]) -> Result<Option<Value>>;
    fn raw_batch_get(&self, ctx: &RequestContext, keys: &[UserKey]) -> Result<Vec<Option<Value>>>;
    fn raw_put(&self, ctx: &RequestContext, key: UserKey, value: Value) -> Result<AppliedPosition>;
    fn raw_batch_put(
        &self,
        ctx: &RequestContext,
        kvs: &[(UserKey, Value)],
    ) -> Result<AppliedPosition>;
    fn raw_delete(&self, ctx: &RequestContext, key: &[u8]) -> Result<AppliedPosition>;
    fn raw_scan(
        &self,
        ctx: &RequestContext,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<(UserKey, Value)>>;
    fn raw_delete_range(
        &self,
        ctx: &RequestContext,
        start: &[u8],
        end: &[u8],
    ) -> Result<DeleteRangeReceipt>;
}

/// Preparation either finished the read or transfers unexecuted blocking work.
/// A completed read has already checked its context and consumed its view;
/// it is not a cached credential that may authorize a later engine access.
pub enum RawReadJob<T> {
    Completed(T),
    Blocking(Box<dyn FnOnce() -> Result<T> + Send>),
}

impl<T> RawReadJob<T> {
    /// Execute from a synchronous caller. Async callers must dispatch the
    /// `Blocking` variant to a blocking worker, as the public handler does.
    pub fn run(self) -> Result<T> {
        match self {
            Self::Completed(value) => Ok(value),
            Self::Blocking(job) => job(),
        }
    }
}
pub type RawReadPreparation<T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<RawReadJob<T>>> + Send>>;

/// How far a chunked range delete got.
///
/// Returned on success; on partial failure the same numbers travel in
/// [`Error::PartialDeleteRange`](kv9_common::Error::PartialDeleteRange). Either way the
/// caller can tell "nothing happened" from "some of it happened", which a bare error or a
/// bare `()` cannot express.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeleteRangeReceipt {
    pub committed_chunks: u64,
    /// Position of the last chunk that applied; `None` when no chunk was needed.
    pub last_applied: Option<AppliedPosition>,
}

/// A resolved region location handed back by routing (DESIGN §11 `GetRegion`).
#[derive(Debug, Clone)]
pub struct RegionLocation {
    pub region: kv9_common::RegionId,
    pub epoch: RegionEpoch,
    pub leader: Option<kv9_common::NodeId>,
}

/// Result of creating a keyspace. Production returns the exact Raft proposal
/// identity so acceptance and clients can correlate the write across failover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreateKeyspaceResult {
    pub keyspace: KeyspaceId,
    pub proposed: Option<AppliedPosition>,
}

// Defined in `kv9-common`, re-exported here so the existing public path keeps working.
// It moved because it is a cross-layer apply receipt -- the drain worker needs it to decide
// WAL truncation eligibility -- not a server data-transfer type. Fields and semantics are
// unchanged by the move. See the type's own docs for why it must not be conflated with
// `kv9_raft::ProposedAt`, which has the same shape and a different meaning.
pub use kv9_common::AppliedPosition;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MembershipChangeResult {
    pub applied: AppliedPosition,
    pub voters: Vec<u64>,
    pub learners: Vec<u64>,
    /// One-time credential returned only by admission creation. Promotion and
    /// every later query return `None`; the committed catalog stores only its hash.
    pub join_ticket: Option<String>,
}

pub use kv9_meta::endpoint::{EndpointChange, EndpointRefusal, NodeEndpoint};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointReadResult {
    pub cluster: kv9_common::ClusterId,
    pub endpoint: Option<NodeEndpoint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointUpdateResult {
    Changed {
        endpoint: NodeEndpoint,
        applied: AppliedPosition,
    },
    /// The receipt confirms the current last transition. It does not recover
    /// an earlier invocation's position or identify which caller won that CAS.
    Confirmed {
        endpoint: NodeEndpoint,
        confirmation: AppliedPosition,
    },
    Refused(EndpointRefusal),
}

/// The admin / meta API (DESIGN §11 Admin surface). Authenticated from day one.
pub trait AdminApi {
    fn create_keyspace(
        &self,
        caller: &str,
        name: &str,
        tenant: kv9_common::TenantId,
        api_type: kv9_common::ApiType,
        txn_group: kv9_common::TxnGroupId,
    ) -> Result<CreateKeyspaceResult>;
    fn list_keyspaces(&self, caller: &str) -> Result<Vec<kv9_common::Keyspace>>;
    fn get_region(&self, caller: &str, keyspace: KeyspaceId, key: &[u8]) -> Result<RegionLocation>;
    fn split_region(
        &self,
        caller: &str,
        region: kv9_common::RegionId,
        split_key: UserKey,
    ) -> Result<()>;
    fn cluster_info(&self, caller: &str) -> Result<ClusterInfo>;
    fn admit_node(
        &self,
        _caller: &str,
        _node: kv9_common::NodeId,
        _addr: &str,
        _ttl_seconds: u64,
    ) -> Result<MembershipChangeResult> {
        Err(kv9_common::Error::NotImplemented("AdminApi::admit_node"))
    }
    fn promote_node(
        &self,
        _caller: &str,
        _node: kv9_common::NodeId,
    ) -> Result<MembershipChangeResult> {
        Err(kv9_common::Error::NotImplemented("AdminApi::promote_node"))
    }
    fn get_node_endpoint(
        &self,
        _caller: &str,
        _node: kv9_common::NodeId,
    ) -> Result<EndpointReadResult> {
        Err(kv9_common::Error::NotImplemented(
            "AdminApi::get_node_endpoint",
        ))
    }
    fn change_node_endpoint(
        &self,
        _caller: &str,
        _request: EndpointChange,
    ) -> Result<EndpointUpdateResult> {
        Err(kv9_common::Error::NotImplemented(
            "AdminApi::change_node_endpoint",
        ))
    }
}

/// A snapshot of cluster state (DESIGN §11 `ClusterInfo`).
#[derive(Debug, Clone, Default)]
pub struct ClusterInfo {
    pub node_count: usize,
    pub keyspace_count: usize,
    pub region_count: usize,
}

/// The router API: locate a region for a key (DESIGN §11 Router surface).
pub trait RouterApi {
    fn locate(&self, keyspace: KeyspaceId, key: &[u8]) -> Result<RegionLocation>;
}
