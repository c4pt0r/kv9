//! The v0 API surface as Rust traits (DESIGN §11).
//!
//! Transport is gRPC; these traits are the synchronous core contract behind tonic's
//! blocking boundary, with optional asynchronous RawKV read preparation. Every data request
//! carries `(keyspace_id, region_epoch)` so the router can resolve keyspace→region,
//! epoch-check, and validate the API type against the keyspace declaration.

use std::sync::Arc;

use kv9_common::{KeyspaceId, Result, UserKey, Value};
use kv9_raw::RawMutation;
use kv9_region::RegionEpoch;

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

/// The raw API for `raw` keyspaces (DESIGN §11 Raw surface).
pub trait RawApi: Send + Sync + 'static {
    /// Distinct TTL contract: implementations must never lower this to legacy
    /// Raw operations. One deadline includes preparation, apply and read fences.
    fn prepare_raw_ttl(
        self: Arc<Self>,
        _ctx: RequestContext,
        _operation: crate::raw_ttl::Operation,
        _deadline: std::time::Instant,
    ) -> crate::raw_ttl::Preparation {
        Box::new(|| Err(kv9_common::Error::NotImplemented("Raw TTL protocol")))
    }

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
                RawWrite::Batch(mutations) => self.raw_batch(&ctx, &mutations),
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
    /// Atomic range batch (DESIGN §9.1): mixed puts/deletes, ONE Raft entry, every key
    /// inside one range or the whole batch is refused with `RangeCrossesRegion`.
    fn raw_batch(&self, ctx: &RequestContext, mutations: &[RawMutation])
        -> Result<AppliedPosition>;
    fn raw_delete(&self, ctx: &RequestContext, key: &[u8]) -> Result<AppliedPosition>;
    fn raw_scan(
        &self,
        ctx: &RequestContext,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<(UserKey, Value)>>;
    /// Cross-range pagination: `Some(cursor)` means the walk paused at a
    /// foreign-leader chunk and the caller continues from exactly there,
    /// even over an empty page. Single-range backends never pause.
    fn raw_scan_paged(
        &self,
        ctx: &RequestContext,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<ScanPage> {
        self.raw_scan(ctx, start, end, limit)
            .map(|pairs| (pairs, None))
    }

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
    Put {
        key: UserKey,
        value: Value,
    },
    BatchPut(Vec<(UserKey, Value)>),
    /// Atomic range batch of mixed puts/deletes (DESIGN §9.1).
    Batch(Vec<RawMutation>),
    Delete {
        key: UserKey,
    },
}

impl RawWrite {
    /// The key that routes the write: the first key of a batch, the key of a point write.
    pub fn anchor(&self) -> &[u8] {
        match self {
            RawWrite::Put { key, .. } | RawWrite::Delete { key } => key,
            RawWrite::BatchPut(pairs) => pairs.first().map(|(k, _)| k.as_slice()).unwrap_or(&[]),
            RawWrite::Batch(mutations) => mutations.first().map(RawMutation::key).unwrap_or(&[]),
        }
    }
}

pub type OperationCompletion<T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<T>> + Send>>;
pub type OperationPreparation<T> = Box<dyn FnOnce() -> Result<OperationCompletion<T>> + Send>;
pub type RawWriteCompletion = OperationCompletion<AppliedPosition>;
pub type RawWritePreparation = OperationPreparation<AppliedPosition>;

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
/// One scan page and its optional cross-range resume cursor.
pub type ScanPage = (Vec<(UserKey, Value)>, Option<Vec<u8>>);

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeleteRangeReceipt {
    pub committed_chunks: u64,
    /// Position of the last chunk that applied; `None` when no chunk was needed.
    pub last_applied: Option<AppliedPosition>,
    /// `Some` when a cross-range walk paused at a foreign-leader chunk: the
    /// caller continues from exactly here. Committed chunks stay committed.
    pub resume_from: Option<Vec<u8>>,
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
    /// Canonical manifest of the operation's fixed, catalog-registered capture.
    /// Planning itself performs no object I/O; earlier retries may have uploaded.
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
pub struct RecordMigrationAbortResult {
    pub abort: kv9_meta::data_groups::abort::MigrationAbort,
    /// False confirms the identical committed row; the abort is a NEW commit.
    pub changed: bool,
    pub applied: AppliedPosition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetachAbortedLearnerResult {
    pub detached: kv9_common::NodeId,
    /// False confirms an already-absent learner.
    pub changed: bool,
    /// The sorted voter set after the applied configuration change.
    pub voters: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordGroupCompactionResult {
    pub decision: kv9_meta::data_groups::compaction::GroupCompaction,
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
pub struct PromoteMigrationVoterResult {
    pub destination: kv9_common::NodeId,
    /// False confirms an existing voter; the receipt stays a NEW commit.
    pub changed: bool,
    /// The sorted voter set after the applied configuration change.
    pub voters: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordSourceRemovalResult {
    pub decision: kv9_meta::data_groups::removal::RemovalDecision,
    pub changed: bool,
    pub applied: AppliedPosition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveSourceReplicaResult {
    pub removed: kv9_common::NodeId,
    /// False confirms an already-removed replica; the receipt stays a NEW commit.
    pub changed: bool,
    /// The sorted voter set after the applied configuration change.
    pub voters: Vec<u64>,
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
    /// Enable the TTL format on an exact range through the all-replica gate.
    /// Every success is this invocation's new applied confirmation receipt.
    fn prepare_raw_ttl_activation(
        self: Arc<Self>,
        _caller: String,
        _binding: kv9_common::data_range::DataRange,
        _deadline: std::time::Instant,
    ) -> OperationPreparation<AppliedPosition> {
        Box::new(|| Err(kv9_common::Error::NotImplemented("Raw TTL activation")))
    }
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
    fn record_migration_abort(
        &self,
        _caller: &str,
        _root: kv9_common::RootDigest,
        _operation: [u8; 16],
    ) -> Result<RecordMigrationAbortResult> {
        Err(kv9_common::Error::NotImplemented(
            "AdminApi::record_migration_abort",
        ))
    }
    fn detach_aborted_learner(
        &self,
        _caller: &str,
        _root: kv9_common::RootDigest,
        _operation: [u8; 16],
    ) -> Result<DetachAbortedLearnerResult> {
        Err(kv9_common::Error::NotImplemented(
            "AdminApi::detach_aborted_learner",
        ))
    }
    fn record_group_compaction(
        &self,
        _caller: &str,
        _root: kv9_common::RootDigest,
        _region: kv9_common::RegionId,
        _floor: AppliedPosition,
    ) -> Result<RecordGroupCompactionResult> {
        Err(kv9_common::Error::NotImplemented(
            "AdminApi::record_group_compaction",
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
    fn promote_migration_voter(
        &self,
        _caller: &str,
        _root: kv9_common::RootDigest,
        _operation: [u8; 16],
    ) -> Result<PromoteMigrationVoterResult> {
        Err(kv9_common::Error::NotImplemented(
            "AdminApi::promote_migration_voter",
        ))
    }
    fn record_source_removal(
        &self,
        _caller: &str,
        _root: kv9_common::RootDigest,
        _operation: [u8; 16],
        _source: kv9_meta::data_groups::InitialReplica,
    ) -> Result<RecordSourceRemovalResult> {
        Err(kv9_common::Error::NotImplemented(
            "AdminApi::record_source_removal",
        ))
    }
    fn remove_source_replica(
        &self,
        _caller: &str,
        _root: kv9_common::RootDigest,
        _operation: [u8; 16],
    ) -> Result<RemoveSourceReplicaResult> {
        Err(kv9_common::Error::NotImplemented(
            "AdminApi::remove_source_replica",
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
    ) -> Result<CreateKeyspaceResult>;
    fn list_keyspaces(&self, caller: &str) -> Result<Vec<kv9_common::Keyspace>>;
    fn get_region(&self, caller: &str, keyspace: KeyspaceId, key: &[u8]) -> Result<RegionLocation>;
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
    fn revoke_admission(
        &self,
        _caller: &str,
        _node: kv9_common::NodeId,
    ) -> Result<MembershipChangeResult> {
        Err(kv9_common::Error::NotImplemented(
            "AdminApi::revoke_admission",
        ))
    }
    fn promote_node(
        &self,
        _caller: &str,
        _node: kv9_common::NodeId,
    ) -> Result<MembershipChangeResult> {
        Err(kv9_common::Error::NotImplemented("AdminApi::promote_node"))
    }
    /// Remove a voter from the metadata group through its own log. Leader-only; at
    /// least three voters must remain and a leader never removes itself
    /// (docs/VOTER-REPLACEMENT.md): promote the replacement first.
    fn remove_node(
        &self,
        _caller: &str,
        _node: kv9_common::NodeId,
    ) -> Result<MembershipChangeResult> {
        Err(kv9_common::Error::NotImplemented("AdminApi::remove_node"))
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
    /// This node's status document, as `(name, value)` lines (docs/KV9-ADMIN.md §4).
    /// Served by every node about itself; never a leader-only read.
    fn node_status(&self, _caller: &str) -> Result<NodeStatusReport> {
        Err(kv9_common::Error::NotImplemented("AdminApi::node_status"))
    }
    /// The catalog's node directory joined with admissions and the leader's Raft
    /// view of each peer. Linearizable, leader-only.
    fn list_nodes(&self, _caller: &str) -> Result<NodeDirectory> {
        Err(kv9_common::Error::NotImplemented("AdminApi::list_nodes"))
    }
    /// Every committed data group with its activation, range bindings, migrations
    /// and newest compaction floor. Linearizable, leader-only.
    fn list_data_groups(&self, _caller: &str) -> Result<Vec<DataGroupSummary>> {
        Err(kv9_common::Error::NotImplemented(
            "AdminApi::list_data_groups",
        ))
    }
    /// One page of the TASKS history after `after` (exclusive), optionally one kind.
    fn list_tasks(&self, _caller: &str, _after: u64, _limit: usize, _kind: u64) -> Result<TaskPage> {
        Err(kv9_common::Error::NotImplemented("AdminApi::list_tasks"))
    }
    /// Hand leadership of `region` (the metadata group or a data group) to a
    /// caught-up voter (docs/LEADER-TRANSFER.md). Leader-only; nothing is committed.
    fn transfer_leader(
        &self,
        _caller: &str,
        _region: kv9_common::RegionId,
        _transferee: kv9_common::NodeId,
    ) -> Result<LeaderTransferResult> {
        Err(kv9_common::Error::NotImplemented("AdminApi::transfer_leader"))
    }
    /// Commit a migration intent and the replica plan that drives it, in one
    /// catalog transaction (docs/REPLICA-ADDITION.md §1).
    #[allow(clippy::too_many_arguments)]
    fn add_replica(
        &self,
        _caller: &str,
        _root: kv9_common::RootDigest,
        _operation: [u8; 16],
        _creation_task: u64,
        _destination: kv9_common::NodeId,
        _mode: kv9_meta::data_groups::addition::PlanMode,
        _source: Option<kv9_common::NodeId>,
    ) -> Result<AddReplicaResult> {
        Err(kv9_common::Error::NotImplemented("AdminApi::add_replica"))
    }
    /// Commit the intent that `node`'s current store leaves the cluster
    /// (docs/DRAIN-NODE.md); the drained node hands away its leaderships and the
    /// metadata leader moves its replicas and membership from there.
    fn drain_node(&self, _caller: &str, _node: kv9_common::NodeId) -> Result<DrainResult> {
        Err(kv9_common::Error::NotImplemented("AdminApi::drain_node"))
    }
    /// Commit a split intent for the bound range of `parent_region` at `split_key`,
    /// creating and activating both children on the parent's stores in the same
    /// transaction (docs/RANGE-SPLIT.md §2). Nothing seals, copies or reroutes.
    fn split_range(
        &self,
        _caller: &str,
        _root: kv9_common::RootDigest,
        _operation: [u8; 16],
        _parent_region: kv9_common::RegionId,
        _split_key: &[u8],
    ) -> Result<SplitRangeResult> {
        Err(kv9_common::Error::NotImplemented("AdminApi::split_range"))
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataGroupReplica {
    pub node_id: u64,
    pub store_incarnation: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataGroupKeyspace {
    pub keyspace_id: u32,
    pub tenant_id: u64,
    pub bind_task: u64,
    pub conf_ver: u64,
    pub version: u64,
    pub start: Vec<u8>,
    pub end: Vec<u8>,
    pub sealed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataGroupMigration {
    pub task: u64,
    pub operation: [u8; 16],
    pub destination: DataGroupReplica,
    pub evidence_task: u64,
    pub abort_task: u64,
    pub removal_task: u64,
    pub truncation_task: u64,
}

/// What this node observed after asking raft to transfer leadership.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LeaderTransferResult {
    pub previous_leader: u64,
    /// The leader observed after the transfer; `None` only when the wait expired.
    pub leader: Option<u64>,
    pub term: u64,
    /// `false`: raft abandoned the transfer and the previous leader leads again.
    pub completed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddReplicaResult {
    pub plan: kv9_meta::data_groups::addition::ReplicaPlan,
    pub migration: kv9_meta::data_groups::migration::MigrationIntent,
    /// False confirms an identical committed plan; the receipt is a NEW commit.
    pub changed: bool,
    pub applied: AppliedPosition,
}

/// One committed replica plan of a group, with its catalog-only stage.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataGroupPlan {
    pub task: u64,
    pub operation: [u8; 16],
    pub migration_task: u64,
    pub mode: &'static str,
    pub destination: DataGroupReplica,
    pub source: Option<DataGroupReplica>,
    pub stage: &'static str,
}

/// One row of [`AdminApi::list_data_groups`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataGroupSummary {
    pub creation_task: u64,
    pub region_id: u64,
    pub operation: [u8; 16],
    pub replicas: Vec<DataGroupReplica>,
    pub activated: bool,
    pub keyspaces: Vec<DataGroupKeyspace>,
    pub migrations: Vec<DataGroupMigration>,
    pub compaction_floor: Option<AppliedPosition>,
    pub plans: Vec<DataGroupPlan>,
    /// Committed membership now (`committed_members`): the nodes a describe
    /// should ask for the group's live view.
    pub members: Vec<DataGroupReplica>,
    /// Committed split intents whose parent is this group (docs/RANGE-SPLIT.md §2).
    pub splits: Vec<DataGroupSplit>,
}

/// One committed split intent of a group, with its catalog-only stage.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataGroupSplit {
    pub task: u64,
    pub operation: [u8; 16],
    pub split_key: Vec<u8>,
    pub child_low_task: u64,
    pub child_high_task: u64,
    pub child_low_region: u64,
    pub child_high_region: u64,
    pub stage: &'static str,
    /// `(evidence task, child task, digest, rows)` per evidenced child.
    pub evidence: Vec<(u64, u64, [u8; 32], u64)>,
}

/// The receipt of [`AdminApi::split_range`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitRangeResult {
    pub intent: kv9_meta::data_groups::split::SplitIntent,
    pub child_low: kv9_meta::data_groups::CreationIntent,
    pub child_high: kv9_meta::data_groups::CreationIntent,
    /// False confirms an identical committed intent.
    pub changed: bool,
    pub applied: AppliedPosition,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskRow {
    pub id: u64,
    pub kind: u64,
    pub kind_name: &'static str,
    pub state: u64,
    pub created_unix: u64,
    pub payload_bytes: u64,
    pub operation: Vec<u8>,
    pub reference_task: u64,
}

#[derive(Debug, Clone, Default)]
pub struct TaskPage {
    pub tasks: Vec<TaskRow>,
    /// Cursor for the next page; 0 when exhausted.
    pub next_after: u64,
}

/// [`AdminApi::node_status`]: the status document and the local directory view.
#[derive(Debug, Clone, Default)]
pub struct NodeStatusReport {
    pub fields: Vec<(String, String)>,
    /// `(node_id, address)` from this node's applied catalog, without a barrier.
    pub peers: Vec<(u64, String)>,
}

/// One row of [`AdminApi::list_nodes`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeDirectoryRow {
    pub node_id: u64,
    pub address: String,
    pub store_incarnation: Vec<u8>,
    pub endpoint_generation: u64,
    pub active: bool,
    pub last_heartbeat_unix: u64,
    /// `voter`, `learner` or `none`, from the answering leader's Raft configuration.
    pub raft_role: &'static str,
    pub matched_index: u64,
    pub recent_active: bool,
    /// `pending`, `consumed`, `revoked`, `superseded`, or empty when no admission row exists.
    pub admission_state: &'static str,
    pub admission_expires_unix: u64,
    /// A committed drain names this node's current store (docs/DRAIN-NODE.md).
    pub draining: bool,
    /// Draining, and already in no data group and out of the metadata group.
    pub drained: bool,
}

/// The receipt of [`AdminApi::drain_node`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrainResult {
    pub drain: kv9_meta::data_groups::drain::DrainIntent,
    /// False confirms an identical committed drain.
    pub changed: bool,
    pub applied: AppliedPosition,
}

#[derive(Debug, Clone, Default)]
pub struct NodeDirectory {
    pub leader_node_id: u64,
    pub raft_committed: u64,
    pub nodes: Vec<NodeDirectoryRow>,
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
