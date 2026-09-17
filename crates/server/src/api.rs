//! The v0 API surface as Rust traits (DESIGN §11).
//!
//! Transport is gRPC; these traits are the synchronous core contract behind tonic's
//! blocking boundary, with optional asynchronous RawKV read preparation. Every data request
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
    /// Resolve an exact scoped data group. Never delegates to a legacy backend.
    fn routed_target(&self, _scope: &kv9_common::data_range::DataRange) -> Result<Arc<dyn RawApi>> {
        Err(kv9_common::Error::NotImplemented("RawApi::routed_target"))
    }

    /// Prepare a write on the blocking boundary, then await its completion
    /// without retaining a blocking worker. The public boundary owns the
    /// admission reservation across both phases, including RPC cancellation.
    fn prepare_raw_write(
        self: Arc<Self>,
        ctx: RequestContext,
        operation: RawWrite,
    ) -> RawWritePreparation {
        Box::new(move || {
            let at = match operation {
                RawWrite::Put { key, value } => self.raw_put(&ctx, key, value),
                RawWrite::BatchPut(pairs) => self.raw_batch_put(&ctx, &pairs),
                RawWrite::Delete { key } => self.raw_delete(&ctx, &key),
            }?;
            Ok(Box::pin(async move { Ok(at) }))
        })
    }
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

    /// Prepare one atomic batch read. Implementations must preserve one
    /// quorum-established view for the complete ordered result, including
    /// duplicates. The default keeps the synchronous backend contract.
    fn prepare_raw_batch_get(
        self: Arc<Self>,
        ctx: RequestContext,
        keys: Vec<UserKey>,
    ) -> RawReadPreparation<Vec<Option<Value>>> {
        Box::pin(async move {
            Ok(RawReadJob::Blocking(Box::new(move || {
                self.raw_batch_get(&ctx, &keys)
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

/// Owned operations that preserve the existing point/batch RawKV wire APIs.
pub enum RawWrite {
    Put { key: UserKey, value: Value },
    BatchPut(Vec<(UserKey, Value)>),
    Delete { key: UserKey },
}

pub type RawWriteCompletion =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<AppliedPosition>> + Send>>;
pub type RawWritePreparation = Box<dyn FnOnce() -> Result<RawWriteCompletion> + Send>;

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

/// An exact receipt for durable creation/activation desire, not group readiness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateDataGroupResult {
    pub intent: kv9_meta::data_groups::CreationIntent,
    /// False denotes a NEW confirmation, not the original mutation receipt.
    pub changed: bool,
    pub applied: AppliedPosition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrateDataGroupResult {
    pub intent: kv9_meta::data_groups::migration::MigrationIntent,
    /// False denotes a NEW confirmation, not the original mutation receipt.
    pub changed: bool,
    pub applied: AppliedPosition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachMigrationLearnerResult {
    pub destination: kv9_common::NodeId,
    /// False confirms an existing learner; the receipt stays a NEW commit.
    pub changed: bool,
    /// The advanced engine cut whose capture names the learner.
    pub cut: AppliedPosition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanMigrationImageResult {
    /// Canonical checkpoint manifest for the group's current durable cut.
    /// A description for owner binding; no object exists for it yet.
    pub manifest: Vec<u8>,
    pub cut: AppliedPosition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureMigrationImageResult {
    /// Canonical bounded snapshot record (KV9RSN01) the installer consumes.
    pub record: Vec<u8>,
    pub image_digest: kv9_common::RootDigest,
    pub cut: AppliedPosition,
    /// Commit position of the attached configuration; None for the initial one.
    pub configuration_applied_at: Option<AppliedPosition>,
    pub objects: u64,
    pub object_bytes: u64,
    pub source_owner: kv9_common::retention::OwnerId,
    pub destination_owner: kv9_common::retention::OwnerId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmitInstallEvidenceResult {
    /// Canonical KV9EVD01 receipt replaying durable adoption facts.
    pub receipt: Vec<u8>,
    pub image_digest: kv9_common::RootDigest,
    pub cut: AppliedPosition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordInstallEvidenceResult {
    pub evidence: kv9_meta::data_groups::evidence::InstallEvidence,
    /// False confirms the identical committed row; the receipt is a NEW commit.
    pub changed: bool,
    pub applied: AppliedPosition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordSourceTruncationResult {
    pub decision: kv9_meta::data_groups::truncation::TruncationDecision,
    pub changed: bool,
    pub applied: AppliedPosition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TruncateSourceLogResult {
    pub floor: AppliedPosition,
    /// The retained log's first index after compaction.
    pub first_index: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindMigrationImageResult {
    /// Tracking-only published ledger owners; no transfer or release follows.
    pub source_owner: kv9_common::retention::OwnerId,
    pub destination_owner: kv9_common::retention::OwnerId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateDataKeyspaceResult {
    pub range: kv9_common::data_range::DataRange,
    pub changed: bool,
    pub applied: AppliedPosition,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionUpdateResult {
    pub revision: u64,
    /// False denotes a fresh confirmation of existing state, never recovery
    /// of a previous invocation's mutation receipt.
    pub changed: bool,
    pub applied: AppliedPosition,
}

/// The admin / meta API (DESIGN §11 Admin surface). Authenticated from day one.
pub trait AdminApi {
    fn lookup_raw_route(
        &self,
        _root: kv9_common::RootDigest,
        _tenant: kv9_common::TenantId,
        _keyspace: KeyspaceId,
        _key: &[u8],
    ) -> Result<RawRouteLookup> {
        Err(kv9_common::Error::NotImplemented(
            "AdminApi::lookup_raw_route",
        ))
    }

    fn create_data_keyspace(
        &self,
        _caller: &str,
        _root: kv9_common::RootDigest,
        _creation_task: u64,
        _name: &str,
        _tenant: kv9_common::TenantId,
    ) -> Result<CreateDataKeyspaceResult> {
        Err(kv9_common::Error::NotImplemented(
            "AdminApi::create_data_keyspace",
        ))
    }
    fn create_data_group(
        &self,
        _caller: &str,
        _root: kv9_common::RootDigest,
        _operation: [u8; 16],
        _voters: &[kv9_common::NodeId],
    ) -> Result<CreateDataGroupResult> {
        Err(kv9_common::Error::NotImplemented(
            "AdminApi::create_data_group",
        ))
    }
    fn migrate_data_group(
        &self,
        _caller: &str,
        _root: kv9_common::RootDigest,
        _operation: [u8; 16],
        _creation_task: u64,
        _destination: kv9_common::NodeId,
    ) -> Result<MigrateDataGroupResult> {
        Err(kv9_common::Error::NotImplemented(
            "AdminApi::migrate_data_group",
        ))
    }
    fn attach_migration_learner(
        &self,
        _caller: &str,
        _root: kv9_common::RootDigest,
        _operation: [u8; 16],
    ) -> Result<AttachMigrationLearnerResult> {
        Err(kv9_common::Error::NotImplemented(
            "AdminApi::attach_migration_learner",
        ))
    }
    fn plan_migration_image(
        &self,
        _caller: &str,
        _root: kv9_common::RootDigest,
        _operation: [u8; 16],
    ) -> Result<PlanMigrationImageResult> {
        Err(kv9_common::Error::NotImplemented(
            "AdminApi::plan_migration_image",
        ))
    }
    fn capture_migration_image(
        &self,
        _caller: &str,
        _root: kv9_common::RootDigest,
        _operation: [u8; 16],
    ) -> Result<CaptureMigrationImageResult> {
        Err(kv9_common::Error::NotImplemented(
            "AdminApi::capture_migration_image",
        ))
    }
    fn bind_migration_image(
        &self,
        _caller: &str,
        _root: kv9_common::RootDigest,
        _operation: [u8; 16],
        _manifest: &[u8],
    ) -> Result<BindMigrationImageResult> {
        Err(kv9_common::Error::NotImplemented(
            "AdminApi::bind_migration_image",
        ))
    }
    fn emit_install_evidence(
        &self,
        _caller: &str,
        _root: kv9_common::RootDigest,
        _region: kv9_common::RegionId,
    ) -> Result<EmitInstallEvidenceResult> {
        Err(kv9_common::Error::NotImplemented(
            "AdminApi::emit_install_evidence",
        ))
    }
    fn record_install_evidence(
        &self,
        _caller: &str,
        _root: kv9_common::RootDigest,
        _receipt: &[u8],
    ) -> Result<RecordInstallEvidenceResult> {
        Err(kv9_common::Error::NotImplemented(
            "AdminApi::record_install_evidence",
        ))
    }
    fn record_source_truncation(
        &self,
        _caller: &str,
        _root: kv9_common::RootDigest,
        _operation: [u8; 16],
        _floor: AppliedPosition,
    ) -> Result<RecordSourceTruncationResult> {
        Err(kv9_common::Error::NotImplemented(
            "AdminApi::record_source_truncation",
        ))
    }
    fn truncate_source_log(
        &self,
        _caller: &str,
        _root: kv9_common::RootDigest,
        _operation: [u8; 16],
    ) -> Result<TruncateSourceLogResult> {
        Err(kv9_common::Error::NotImplemented(
            "AdminApi::truncate_source_log",
        ))
    }
    fn apply_retention(&self, _caller: &str, _request: Vec<u8>) -> Result<RetentionUpdateResult> {
        Err(kv9_common::Error::NotImplemented(
            "AdminApi::apply_retention",
        ))
    }
    fn get_retention_owner(
        &self,
        _caller: &str,
        _root: kv9_common::RootDigest,
        _owner: kv9_common::retention::OwnerId,
    ) -> Result<Option<Vec<u8>>> {
        Err(kv9_common::Error::NotImplemented(
            "AdminApi::get_retention_owner",
        ))
    }
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

/// Directory entries are routing hints. A range is returned only after a
/// metadata quorum barrier; data requests must still present its exact scope.
#[derive(Debug, Clone)]
pub struct RawRouteLookup {
    pub root: kv9_common::RootDigest,
    pub metadata_peers: Vec<kv9_meta::endpoint::NodeEndpoint>,
    pub metadata_leader: Option<kv9_common::NodeId>,
    pub range: Option<kv9_common::data_range::DataRange>,
    pub replicas: Vec<kv9_meta::endpoint::NodeEndpoint>,
    pub data_leader: Option<kv9_common::NodeId>,
}
