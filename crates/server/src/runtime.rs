//! Real Phase-1 metadata-node runtime.
//!
//! This is the process boundary missing from the earlier deterministic harness:
//! fixed seed identities, real TCP discovery/Raft traffic, durable Raft state,
//! durable catalog apply, election-first bootstrap, and a machine-readable status
//! file for external acceptance. The status file is evidence; log timing is not.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kv9_common::{
    persist_root_bundle, ApiType, ClusterId, Config, Error, KeyspaceId, NodeId, RegionId, Result,
    RootDescriptor, RootDigest, SeedPeer, StoreIdentity, StoreIncarnation, TenantId, TimeStamp,
    TxnGroupId, UserKey, Value, META_REGION_0,
};
use kv9_engine::{Engine, ReadView, WalEngine};
use kv9_meta::admission::INVALID_JOIN_TICKET_MESSAGE;
use kv9_meta::bootstrap::{init_marker_exists, write_init_marker};
use kv9_meta::codec::memcmp_uint;
use kv9_meta::schema::{ColumnId, NODES_DESC, SCHEMA_VERSION_DESC};
use kv9_meta::tables::Tables;
use kv9_meta::{Bootstrap, BootstrapEvent, BootstrapState};
use kv9_meta::{ColumnValue, RowValue};
use kv9_raft::driver::{DriverAppliedPosition, NodeDriver};
use kv9_raft::grpc::{
    grpc_discover, grpc_register, pb::kv9_raft_server::Kv9RaftServer, DiscoveryError,
    GrpcDiscoveryState, GrpcTransport, JoinIdentity, RaftGrpcService, RegisterError,
    RegisterOutcome, RegistrationBackend, RegistrationError, RegistrationReceipt, RootWireIdentity,
    CLUSTER_TOKEN_KEY, NODE_ID_KEY,
};
use kv9_raft::storage::DiskRaftStorage;
use kv9_raft::transport::voter_set_fingerprint;
use kv9_raft::{Command, MemStateMachine, ProposedAt, RaftGroup, RaftPeer, Role};
use tonic::metadata::MetadataMap;
use tonic::Status;

use crate::api::{
    AdminApi, AppliedPosition, ClusterInfo, CreateKeyspaceResult, DeleteRangeReceipt,
    MembershipChangeResult, RawApi, RegionLocation, RequestContext, TxnApi,
};
use crate::grpc::{
    AuthContext, AuthInterceptor, AuthKind, Authenticator, Kv9Grpc, TokenAuthenticator,
};
use kv9_txn::{LeaderRead, RawExecutor, RawWriteOptions};

use crate::Node;

const TICK: Duration = Duration::from_millis(20);
const DISCOVERY_INTERVAL: Duration = Duration::from_millis(200);
const DISCOVERY_TIMEOUT: Duration = Duration::from_millis(50);
const DISCOVERY_LAST_OUTCOME_MAX_CHARS: usize = 160;
const DISCOVERY_ERROR_PREFIX: &str = "error:";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiscoveryRejection {
    NodeId,
    VoterFingerprint,
}

impl DiscoveryRejection {
    fn label(self) -> &'static str {
        match self {
            Self::NodeId => "rejected_node_id",
            Self::VoterFingerprint => "rejected_voter_fingerprint",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DiscoveryLastOutcome {
    Local,
    NotAttempted,
    AcceptedInitialized,
    AcceptedUninitialized,
    Rejected(DiscoveryRejection),
    RejectedRootIdentity,
    ConnectFailed,
    Timeout,
    Error(String),
}

impl DiscoveryLastOutcome {
    fn label(&self) -> String {
        match self {
            Self::Local => "local".into(),
            Self::NotAttempted => "not_attempted".into(),
            Self::AcceptedInitialized => "accepted_initialized".into(),
            Self::AcceptedUninitialized => "accepted_uninitialized".into(),
            Self::Rejected(reason) => reason.label().into(),
            Self::RejectedRootIdentity => "rejected_root_identity".into(),
            Self::ConnectFailed => "connect_failed".into(),
            Self::Timeout => "timeout".into(),
            Self::Error(detail) => format!("{DISCOVERY_ERROR_PREFIX}{detail}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiscoveryObservation {
    seed: SeedPeer,
    attempts: u64,
    accepted: u64,
    errors: u64,
    rejected_root_identity: u64,
    rejected_node_id: u64,
    rejected_voter_fingerprint: u64,
    last: DiscoveryLastOutcome,
}

impl DiscoveryObservation {
    fn new(seed: SeedPeer, local: bool) -> Self {
        Self {
            seed,
            attempts: 0,
            accepted: 0,
            errors: 0,
            rejected_root_identity: 0,
            rejected_node_id: 0,
            rejected_voter_fingerprint: 0,
            last: if local {
                DiscoveryLastOutcome::Local
            } else {
                DiscoveryLastOutcome::NotAttempted
            },
        }
    }

    fn record_attempt(&mut self) {
        self.attempts = self.attempts.saturating_add(1);
    }

    fn record_accepted(&mut self, initialized: bool) {
        self.accepted = self.accepted.saturating_add(1);
        self.last = if initialized {
            DiscoveryLastOutcome::AcceptedInitialized
        } else {
            DiscoveryLastOutcome::AcceptedUninitialized
        };
    }

    fn record_error(&mut self, error: &DiscoveryError) {
        match error {
            DiscoveryError::RootIdentityMismatch => {
                self.rejected_root_identity = self.rejected_root_identity.saturating_add(1);
                self.last = DiscoveryLastOutcome::RejectedRootIdentity;
            }
            DiscoveryError::Connect(_) => {
                self.errors = self.errors.saturating_add(1);
                self.last = DiscoveryLastOutcome::ConnectFailed;
            }
            DiscoveryError::Timeout => {
                self.errors = self.errors.saturating_add(1);
                self.last = DiscoveryLastOutcome::Timeout;
            }
            DiscoveryError::Failed(detail) => {
                self.errors = self.errors.saturating_add(1);
                self.last = DiscoveryLastOutcome::Error(bounded_discovery_detail(detail));
            }
        }
    }

    fn record_rejected(&mut self, reason: DiscoveryRejection) {
        match reason {
            DiscoveryRejection::NodeId => {
                self.rejected_node_id = self.rejected_node_id.saturating_add(1);
            }
            DiscoveryRejection::VoterFingerprint => {
                self.rejected_voter_fingerprint = self.rejected_voter_fingerprint.saturating_add(1);
            }
        }
        self.last = DiscoveryLastOutcome::Rejected(reason);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegistrationLastOutcome {
    NotAttempted,
    Registered,
    NotLeader,
    RejectedInvalidTicket,
    ConnectFailed,
    Timeout,
    Failed,
}

impl RegistrationLastOutcome {
    fn label(self) -> &'static str {
        match self {
            Self::NotAttempted => "not_attempted",
            Self::Registered => "registered",
            Self::NotLeader => "not_leader",
            Self::RejectedInvalidTicket => "rejected_invalid_ticket",
            Self::ConnectFailed => "connect_failed",
            Self::Timeout => "timeout",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RegistrationObservation {
    attempts: u64,
    errors: u64,
    last: RegistrationLastOutcome,
}

impl RegistrationObservation {
    fn new() -> Self {
        Self {
            attempts: 0,
            errors: 0,
            last: RegistrationLastOutcome::NotAttempted,
        }
    }

    fn record_attempt(&mut self) {
        self.attempts = self.attempts.saturating_add(1);
    }

    fn record_registered(&mut self) {
        self.last = RegistrationLastOutcome::Registered;
    }

    fn record_not_leader(&mut self) {
        self.errors = self.errors.saturating_add(1);
        self.last = RegistrationLastOutcome::NotLeader;
    }

    fn record_error(&mut self, error: &RegisterError) {
        self.errors = self.errors.saturating_add(1);
        self.last = match error {
            RegisterError::InvalidTicket => RegistrationLastOutcome::RejectedInvalidTicket,
            RegisterError::Connect(_) => RegistrationLastOutcome::ConnectFailed,
            RegisterError::Timeout => RegistrationLastOutcome::Timeout,
            RegisterError::Failed(_) => RegistrationLastOutcome::Failed,
        };
    }
}

fn bounded_discovery_detail(detail: &str) -> String {
    detail
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .take(DISCOVERY_LAST_OUTCOME_MAX_CHARS - DISCOVERY_ERROR_PREFIX.len())
        .collect()
}

#[derive(Debug)]
struct RuntimeDiscovery {
    node: NodeId,
    root: RootWireIdentity,
    initialized: AtomicBool,
    /// The bootstrap fingerprint — present ONLY until initialization:
    /// `set_cluster_id` takes it, so the post-init zero in answers comes
    /// from the value being GONE, not from a condition someone can delete
    /// or invert (structural, per the Cindy/Tess retirement criterion; the
    /// old `if initialized` guard is deliberately absent, not stacked).
    voter_fp: Mutex<Option<u64>>,
    /// The cluster identity, set exactly once at/after initialization; the
    /// discovery contract couples it to `initialized` (an initialized answer
    /// MUST name its cluster — the service refuses otherwise).
    cluster_id: Mutex<Option<kv9_common::ClusterId>>,
}

impl RuntimeDiscovery {
    fn new(node: NodeId, initialized: bool, voter_fp: u64, root: RootWireIdentity) -> Self {
        Self {
            node,
            root,
            initialized: AtomicBool::new(initialized),
            voter_fp: Mutex::new(if initialized { None } else { Some(voter_fp) }),
            cluster_id: Mutex::new(None),
        }
    }

    fn set_cluster_id(&self, id: kv9_common::ClusterId) {
        *self.cluster_id.lock().expect("cluster id poisoned") = Some(id);
        // Retirement moment: the fingerprint ceases to exist here.
        self.voter_fp.lock().expect("fp poisoned").take();
        self.initialized.store(true, Ordering::Release);
    }
}

impl GrpcDiscoveryState for RuntimeDiscovery {
    fn answer(&self) -> (NodeId, bool, u64) {
        (
            self.node,
            self.initialized.load(Ordering::Acquire),
            // 0 after initialization because the value is GONE (taken at
            // `set_cluster_id`), not because a branch remembered to zero it.
            self.voter_fp.lock().expect("fp poisoned").unwrap_or(0),
        )
    }

    fn root_identity(&self) -> RootWireIdentity {
        self.root
    }

    fn cluster_id(&self) -> Option<kv9_common::ClusterId> {
        *self.cluster_id.lock().expect("cluster id poisoned")
    }
}

/// Authentication material supplied at process startup. Values are deliberately
/// kept out of `Config` and status/debug surfaces so credentials are not serialized
/// or printed accidentally.
pub struct RuntimeAuth {
    pub cluster_token: String,
    pub client_tokens: Vec<(String, String)>,
}

impl RuntimeAuth {
    pub fn validate(&self) -> Result<()> {
        if self.cluster_token.is_empty() {
            return Err(Error::Config("cluster token must be non-empty".into()));
        }
        TokenAuthenticator::new(self.client_tokens.clone()).map(|_| ())
    }
}

#[derive(Clone)]
struct ClusterAuthenticator {
    expected_token: Arc<str>,
    voters: Arc<HashSet<NodeId>>,
    node: Arc<Node<WalEngine>>,
}

impl Authenticator for ClusterAuthenticator {
    fn authenticate(&self, metadata: &MetadataMap) -> std::result::Result<AuthContext, Status> {
        let token = metadata
            .get(CLUSTER_TOKEN_KEY)
            .ok_or_else(|| Status::unauthenticated("cluster token required"))?
            .to_str()
            .map_err(|_| Status::unauthenticated("invalid cluster token metadata"))?;
        if token != self.expected_token.as_ref() {
            return Err(Status::unauthenticated("cluster token mismatch"));
        }
        let node_id = metadata
            .get(NODE_ID_KEY)
            .ok_or_else(|| Status::unauthenticated("declared node identity required"))?
            .to_str()
            .map_err(|_| Status::unauthenticated("invalid node identity metadata"))?
            .parse::<u64>()
            .map(NodeId)
            .map_err(|_| Status::unauthenticated("invalid node identity"))?;
        let catalog_allows = if self.voters.contains(&node_id) {
            true
        } else {
            let txn = self
                .node
                .meta_raft
                .store
                .begin()
                .map_err(|_| Status::unavailable("membership catalog unavailable"))?;
            let admitted = kv9_meta::admission::admission(&txn, node_id)
                .map_err(|_| Status::unavailable("membership catalog unavailable"))?
                .is_some_and(|admission| {
                    admission.state != kv9_meta::admission::AdmissionState::Revoked
                });
            let registered = txn
                .get(&NODES_DESC, &[memcmp_uint(node_id.0)])
                .map_err(|_| Status::unavailable("membership catalog unavailable"))?
                .is_some();
            admitted || registered
        };
        if !catalog_allows {
            return Err(Status::permission_denied(
                "declared node is neither a voter nor admitted/registered",
            ));
        }
        Ok(AuthContext {
            principal: Arc::from(format!("node:{}", node_id.0)),
            node_id: Some(node_id),
            auth_kind: AuthKind::Node,
        })
    }
}

/// Runtime-specific API backend. It delegates reads to the assembled node, but
/// proposals go through `NodeDriver` so the response can return and verify the
/// exact `(term,index)` that the production apply loop committed.
struct RuntimeBackend {
    node: Arc<Node<WalEngine>>,
    driver: Arc<NodeDriver<DiskRaftStorage, WalEngine>>,
    transport: Arc<GrpcTransport>,
}

impl RuntimeBackend {
    /// Public requests may arrive as soon as the listener binds, before election-first
    /// bootstrap has applied the default tenant/catalog rows. Expose that lifecycle state
    /// directly instead of letting a planner misreport missing bootstrap data as a caller
    /// integrity error. Internal discovery, registration, and seed apply do not use this
    /// API backend gate and therefore remain able to advance the node to `Serving`.
    fn ensure_serving(&self) -> Result<()> {
        let state = self
            .node
            .meta
            .lock()
            .expect("meta poisoned")
            .bootstrap
            .state();
        if matches!(state, BootstrapState::Serving { .. }) {
            Ok(())
        } else {
            Err(Error::MetaNotReady(format!(
                "metadata node is not serving (bootstrap_state={state:?})"
            )))
        }
    }

    fn commit_catalog(&self, command: &kv9_raft::Command) -> Result<AppliedPosition> {
        let proposed = self.driver.propose(command)?;
        match self
            .driver
            .wait_applied(proposed, Duration::from_secs(10))?
        {
            true => Ok(AppliedPosition {
                term: proposed.term,
                index: proposed.index.0,
            }),
            false => Err(Error::Raft(format!(
                "catalog proposal at term {} index {} was overwritten",
                proposed.term, proposed.index.0
            ))),
        }
    }
}

impl AdminApi for RuntimeBackend {
    fn create_keyspace(
        &self,
        _caller: &str,
        name: &str,
        tenant: TenantId,
        api_type: ApiType,
        _txn_group: TxnGroupId,
    ) -> Result<CreateKeyspaceResult> {
        self.ensure_serving()?;
        let _guard = self.node.meta_raft.lock_catalog_txn();
        let (keyspace, command) = self
            .node
            .build_create_keyspace_command(name, tenant, api_type)?;
        let proposed = self.driver.propose(&command)?;
        match self
            .driver
            .wait_applied(proposed, Duration::from_secs(10))?
        {
            true => Ok(CreateKeyspaceResult {
                keyspace,
                proposed: Some(AppliedPosition {
                    term: proposed.term,
                    index: proposed.index.0,
                }),
            }),
            false => Err(Error::Raft(format!(
                "keyspace proposal at term {} index {} was overwritten",
                proposed.term, proposed.index.0
            ))),
        }
    }

    fn list_keyspaces(&self, caller: &str) -> Result<Vec<kv9_common::Keyspace>> {
        self.ensure_serving()?;
        self.node.list_keyspaces(caller)
    }

    fn get_region(&self, caller: &str, keyspace: KeyspaceId, key: &[u8]) -> Result<RegionLocation> {
        self.ensure_serving()?;
        self.node.get_region(caller, keyspace, key)
    }

    fn split_region(&self, caller: &str, region: RegionId, split_key: UserKey) -> Result<()> {
        self.ensure_serving()?;
        self.node.split_region(caller, region, split_key)
    }

    fn cluster_info(&self, caller: &str) -> Result<ClusterInfo> {
        self.ensure_serving()?;
        self.node.cluster_info(caller)
    }

    fn admit_node(
        &self,
        _caller: &str,
        node: NodeId,
        addr: &str,
        ttl_seconds: u64,
    ) -> Result<MembershipChangeResult> {
        self.ensure_serving()?;
        if self.driver.status().role != Role::Leader {
            return Err(Error::Raft("admit-node must be sent to the leader".into()));
        }
        if ttl_seconds == 0 {
            return Err(Error::Config(
                "admission ttl must be greater than zero".into(),
            ));
        }
        let expires = unix_now()
            .checked_add(ttl_seconds)
            .ok_or_else(|| Error::Config("admission expiry overflows u64".into()))?;
        let ticket = format!("{}{}", StoreIncarnation::mint()?, StoreIncarnation::mint()?);
        let ticket_sha256 = kv9_common::RootDigest::sha256(ticket.as_bytes());
        let _guard = self.node.meta_raft.lock_catalog_txn();
        let mut txn = self.node.meta_raft.store.begin()?;
        kv9_meta::admission::admit_node_with_ticket_hash(
            &mut txn,
            node,
            addr,
            kv9_meta::admission::AdmittedRole::Learner,
            ticket_sha256.as_bytes(),
            expires,
        )?;
        let applied = self.commit_catalog(&kv9_raft::Command::from_batch(&txn.into_batch()))?;
        let status = self.driver.status();
        Ok(MembershipChangeResult {
            applied,
            voters: status.voters,
            learners: status.learners,
            join_ticket: Some(ticket),
        })
    }

    fn promote_node(&self, _caller: &str, node: NodeId) -> Result<MembershipChangeResult> {
        self.ensure_serving()?;
        if self.driver.status().role != Role::Leader {
            return Err(Error::Raft(
                "promote-node must be sent to the leader".into(),
            ));
        }
        let proposed = self.driver.promote_voter(node)?;
        let receipt = self
            .driver
            .wait_conf_applied(proposed, Duration::from_secs(10))?;
        Ok(MembershipChangeResult {
            applied: AppliedPosition {
                term: receipt.applied.term,
                index: receipt.applied.index.0,
            },
            voters: receipt.voters,
            learners: receipt.learners,
            join_ticket: None,
        })
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn membership_node_row(
    node: NodeId,
    addr: &str,
    state: u64,
    heartbeat: u64,
    store_incarnation: StoreIncarnation,
) -> RowValue {
    let mut row = RowValue::new();
    row.set(ColumnId(1), ColumnValue::Uint(node.0));
    row.set(ColumnId(2), ColumnValue::Text(addr.to_string()));
    row.set(ColumnId(3), ColumnValue::Uint(state));
    row.set(ColumnId(4), ColumnValue::Uint(heartbeat));
    row.set(
        ColumnId(5),
        ColumnValue::Bytes(store_incarnation.as_bytes().to_vec()),
    );
    row
}

/// The current-term barrier gate for bootstrap takeover (task #40): may a
/// WaitForBootstrap node that finds itself raft leader promote to
/// Initializing? Ordering is load-bearing and lives ONLY here:
///
///   1. sample (role, term) — must be Leader;
///   2. the unified driver watermark must have reached THIS term
///      (contiguity ⇒ every committed entry at or below it — including any
///      earlier init — is applied locally; the election no-op guarantees the
///      term is reachable with no application proposal);
///   3. re-sample — still the same-term leader (a demotion between 1 and 2
///      must not ride the old sample);
///   4. only NOW read the catalog: empty PROVES no init has committed
///      anywhere — read it before the barrier and a committed-but-unapplied
///      init is invisible, the takeover re-proposes, the duplicate commits,
///      and initialize_cluster poisons the cluster at apply.
///
/// Deleting step 2 turns the deterministic frozen-apply regression red; the
/// step-3 re-confirm guards a between-samples demotion race that a
/// single-node test cannot construct — its coverage is the Chaos E2E layer.
fn bootstrap_takeover_proven<S, E>(
    driver: &NodeDriver<S, E>,
    node: &Node<WalEngine>,
) -> Result<bool>
where
    S: kv9_raft::rawnode::PersistentRaftStorage,
    E: kv9_engine::Engine + 'static,
{
    let before = driver.status();
    if before.role != Role::Leader {
        return Ok(false);
    }
    let Some(wm) = driver.driver_applied() else {
        return Ok(false);
    };
    if wm.term != before.term {
        return Ok(false);
    }
    let confirm = driver.status();
    if confirm.role != Role::Leader || confirm.term != before.term {
        return Ok(false);
    }
    let txn = node.meta_raft.store.begin()?;
    Ok(kv9_meta::admission::cluster_id(&txn)?.is_none())
}

fn registration_error(error: Error) -> RegistrationError {
    match &error {
        Error::Config(message) if message == INVALID_JOIN_TICKET_MESSAGE => {
            RegistrationError::InvalidTicket
        }
        _ => RegistrationError::Failed(error),
    }
}

impl RegistrationBackend for RuntimeBackend {
    fn register(
        &self,
        node: NodeId,
        addr: &str,
        cluster_id: ClusterId,
        join_ticket_sha256: &[u8],
        store_incarnation: StoreIncarnation,
    ) -> std::result::Result<RegistrationReceipt, RegistrationError> {
        let leader = self.driver.status();
        if leader.role != Role::Leader {
            return Err(RegistrationError::NotLeader {
                leader: leader.leader_id,
            });
        }
        let now = unix_now();
        let canonical_addr: std::net::SocketAddr = addr.parse().map_err(|_| {
            RegistrationError::Failed(Error::Config(
                "registration address must be a canonical socket address".into(),
            ))
        })?;
        let canonical = canonical_addr.to_string();

        // The leader must know the new endpoint before proposing AddLearner;
        // otherwise raft-rs emits catch-up traffic to an unknown peer and the
        // registration receipt can never become locally observable there.
        self.transport.register_peer(node, canonical_addr);

        // Serialize the catalog half across retries/revocation. Consuming an
        // admission and inserting a Joining node are one command; if the
        // later ConfChange loses leadership, a retry recognizes this durable
        // intermediate state and completes instead of wedging on "consumed".
        let _catalog_guard = self.node.meta_raft.lock_catalog_txn();
        let existing = {
            let txn = self
                .node
                .meta_raft
                .store
                .begin()
                .map_err(RegistrationError::Failed)?;
            kv9_meta::admission::admission(&txn, node).map_err(RegistrationError::Failed)?
        };
        match existing {
            Some(admission) if admission.state == kv9_meta::admission::AdmissionState::Consumed => {
                if admission.cluster_id != cluster_id || admission.addr != canonical {
                    return Err(RegistrationError::Failed(Error::Config(
                        "consumed admission does not match this registration".into(),
                    )));
                }
                let txn = self
                    .node
                    .meta_raft
                    .store
                    .begin()
                    .map_err(RegistrationError::Failed)?;
                let bound = txn
                    .get(&NODES_DESC, &[memcmp_uint(node.0)])
                    .map_err(RegistrationError::Failed)?
                    .and_then(|row| match row.value.get(ColumnId(5)) {
                        Some(ColumnValue::Bytes(bytes)) => Some(bytes.clone()),
                        _ => None,
                    });
                if bound.as_deref() != Some(store_incarnation.as_bytes()) {
                    return Err(RegistrationError::Failed(Error::Config(
                        "registered node store incarnation does not match".into(),
                    )));
                }
            }
            Some(admission) if admission.state == kv9_meta::admission::AdmissionState::Pending => {
                let mut txn = self
                    .node
                    .meta_raft
                    .store
                    .begin()
                    .map_err(RegistrationError::Failed)?;
                kv9_meta::admission::consume_admission_with_ticket(
                    &mut txn,
                    node,
                    cluster_id,
                    &canonical,
                    join_ticket_sha256,
                    now,
                )
                .map_err(registration_error)?;
                if txn
                    .get(&NODES_DESC, &[memcmp_uint(node.0)])
                    .map_err(RegistrationError::Failed)?
                    .is_some()
                {
                    txn.update(
                        &NODES_DESC,
                        &[memcmp_uint(node.0)],
                        vec![
                            (ColumnId(2), ColumnValue::Text(canonical.clone())),
                            (ColumnId(3), ColumnValue::Uint(1)),
                            (ColumnId(4), ColumnValue::Uint(now)),
                            (
                                ColumnId(5),
                                ColumnValue::Bytes(store_incarnation.as_bytes().to_vec()),
                            ),
                        ],
                    )
                    .map_err(RegistrationError::Failed)?;
                } else {
                    txn.insert(
                        &NODES_DESC,
                        &[memcmp_uint(node.0)],
                        membership_node_row(node, &canonical, 1, now, store_incarnation),
                    )
                    .map_err(RegistrationError::Failed)?;
                }
                self.commit_catalog(&kv9_raft::Command::from_batch(&txn.into_batch()))
                    .map_err(RegistrationError::Failed)?;
            }
            Some(_) => {
                return Err(RegistrationError::Failed(Error::Config(
                    "admission is revoked".into(),
                )))
            }
            None => {
                return Err(RegistrationError::Failed(Error::Config(format!(
                    "no admission record for node {}",
                    node.0
                ))))
            }
        }

        let status = self.driver.status();
        if !status.voters.contains(&node.0) && !status.learners.contains(&node.0) {
            let proposed = self
                .driver
                .add_learner(node)
                .map_err(RegistrationError::Failed)?;
            self.driver
                .wait_conf_applied(proposed, Duration::from_secs(10))
                .map_err(RegistrationError::Failed)?;
        }

        // Mark the catalog row active only after the live ConfState contains
        // the node. This final command is the receipt: observing it on the
        // joiner proves every preceding admission and membership step.
        let mut txn = self
            .node
            .meta_raft
            .store
            .begin()
            .map_err(RegistrationError::Failed)?;
        txn.update(
            &NODES_DESC,
            &[memcmp_uint(node.0)],
            vec![
                (ColumnId(3), ColumnValue::Uint(2)),
                (ColumnId(4), ColumnValue::Uint(now)),
            ],
        )
        .map_err(RegistrationError::Failed)?;
        let applied = self
            .commit_catalog(&kv9_raft::Command::from_batch(&txn.into_batch()))
            .map_err(RegistrationError::Failed)?;
        let status = self.driver.status();
        Ok(RegistrationReceipt {
            applied_term: applied.term,
            applied_index: applied.index,
            voters: status.voters,
            learners: status.learners,
        })
    }
}

/// How many keys one `delete_range` chunk may carry.
///
/// A range delete expands to explicit per-key deletes, so an unbounded range would build
/// an unbounded raft entry (DESIGN §13 principle 13 — no unquota'd in-memory path). The
/// cost is that a large range is several entries: each chunk applies atomically, the range
/// as a whole does not.
const RAW_DELETE_RANGE_CHUNK: usize = 1024;

/// The context gate: keyspace, region and epoch, all decided from **one** `MetaTxn`.
///
/// The context arrives from the wire already deserialized and otherwise unexamined. Without
/// this, a client could name a keyspace that was never created, or write raw bytes into a
/// `txn` keyspace where Percolator expects its own lock/write structure, or act on a region
/// whose epoch has since moved — none of which would error.
///
/// **Every lookup shares one transaction.** Reading the keyspace from one snapshot and the
/// region from another lets a split commit in between, and the verdict then describes a
/// state that never existed at any instant. That is why `ReadView` exists in
/// `crates/engine`, and it binds harder here because the conclusion is an authorisation.
///
/// One context authorises exactly **one region**: a range or batch spanning regions is the
/// client's to split, because a single epoch cannot speak for two regions.
///
/// A free function so the production endpoints and the tests call the *same* code — a gate
/// verified through a parallel re-implementation is not verified.
fn check_context<E: kv9_engine::Engine>(
    store: &kv9_meta::store::MetaStore<E>,
    keyspace_id: KeyspaceId,
    epoch: &kv9_region::RegionEpoch,
    span: KeySpan<'_>,
) -> Result<()> {
    let txn = store.begin()?;

    let keyspace =
        Tables::<E>::keyspace_in(&txn, keyspace_id)?.ok_or(Error::KeyspaceNotFound(keyspace_id))?;
    if keyspace.api_type != ApiType::Raw {
        return Err(Error::ApiTypeMismatch {
            keyspace: keyspace_id,
        });
    }

    let tables = Tables::new(store);
    let region = tables
        .region_for_key_in(&txn, keyspace_id, span.anchor())?
        .ok_or(Error::RegionNotFound)?;

    // Epoch before span: a stale epoch and a cross-region request are different failures
    // and the client reacts differently (refresh routing vs. split the request).
    if region.epoch_conf != epoch.conf_ver || region.epoch_ver != epoch.version {
        return Err(Error::StaleEpoch { region: region.id });
    }

    span.assert_within(&region, &txn, &tables, keyspace_id)
}

/// Does a half-open range ending at `end` stay inside a region ending at `region_end`?
///
/// Both "empty" values mean "to the end of the enclosing space", but of *different* spaces:
/// an empty `region_end` is the last region of the keyspace, while an empty `end` asks for
/// the whole keyspace. So an empty `end` is only satisfiable by a region that itself runs
/// to the end — otherwise the request reaches past this region and the client must split
/// it. Kept as a pure function because that asymmetry is the whole rule and is easy to get
/// backwards.
fn range_end_within_region(end: &[u8], region_end: &[u8]) -> bool {
    if region_end.is_empty() {
        // Trailing region: nothing can be beyond it, including an unbounded end.
        return true;
    }
    !end.is_empty() && end <= region_end
}

/// Which keys a request touches, so the region gate can check the right thing.
///
/// Modelled as a type rather than a set of flags because the three cases genuinely differ:
/// a point authorises one key, a batch must prove *every* key lands in one region, and a
/// range must prove its whole half-open span does. Collapsing them would mean checking the
/// first key and hoping.
enum KeySpan<'a> {
    Point(&'a [u8]),
    Batch(Vec<&'a [u8]>),
    /// Half-open `[start, end)`; an empty `end` means "to the end of the keyspace".
    Range {
        start: &'a [u8],
        end: &'a [u8],
    },
}

impl<'a> KeySpan<'a> {
    /// The key used to resolve the region. An empty range start means the keyspace's
    /// first key, which `region_for_key` already treats as the leading region.
    fn anchor(&self) -> &[u8] {
        match self {
            KeySpan::Point(key) => key,
            KeySpan::Batch(keys) => keys.first().copied().unwrap_or(&[]),
            KeySpan::Range { start, .. } => start,
        }
    }

    /// Prove the whole span lies inside `region`.
    fn assert_within<E: kv9_engine::Engine>(
        &self,
        region: &kv9_meta::tables::Region,
        txn: &kv9_meta::store::MetaTxn<'_, E>,
        tables: &Tables<'_, E>,
        keyspace: KeyspaceId,
    ) -> Result<()> {
        match self {
            // Already resolved by this key.
            KeySpan::Point(_) => Ok(()),
            KeySpan::Batch(keys) => {
                for key in keys {
                    let owner = tables
                        .region_for_key_in(txn, keyspace, key)?
                        .ok_or(Error::RegionNotFound)?;
                    if owner.id != region.id {
                        return Err(Error::RangeCrossesRegion);
                    }
                }
                Ok(())
            }
            KeySpan::Range { end, .. } => {
                if range_end_within_region(end, &region.end_key) {
                    Ok(())
                } else {
                    Err(Error::RangeCrossesRegion)
                }
            }
        }
    }
}

/// The chunk loop of a range delete, separated from the machinery that plans and commits.
///
/// Kept standalone so a failure can be injected at chunk N in a test. The interesting
/// behaviour here is not the deleting — it is what is reported when the loop stops early,
/// and that is exactly the part a live-cluster test cannot easily force.
fn run_delete_range<V, P, C>(
    start: &[u8],
    end: &[u8],
    mut revalidate: V,
    mut plan_next: P,
    mut commit: C,
) -> Result<DeleteRangeReceipt>
where
    V: FnMut(&[u8]) -> Result<()>,
    P: FnMut(Option<&[u8]>) -> Result<Option<(kv9_engine::WriteBatch, UserKey)>>,
    C: FnMut(kv9_engine::WriteBatch) -> Result<AppliedPosition>,
{
    let mut cursor: Option<UserKey> = None;
    let mut committed_chunks = 0u64;
    let mut last_applied: Option<AppliedPosition> = None;
    // Both sides can fail after work has landed, and both must preserve the receipt.
    // Planning the *next* chunk re-acquires the leader read, so a leadership change right
    // after chunk 1 commits surfaces here as a NotLeader from `plan_next` — the most
    // realistic partial window there is, and the one that would otherwise discard the
    // receipt and tell the caller nothing happened.
    macro_rules! preserving_receipt {
        ($result:expr) => {
            match $result {
                Ok(value) => value,
                Err(error) if committed_chunks > 0 => {
                    let last =
                        last_applied.expect("committed_chunks > 0 implies a recorded position");
                    return Err(Error::PartialDeleteRange {
                        committed_chunks,
                        last_applied_term: last.term,
                        last_applied_index: last.index,
                        // Diagnosis only; deliberately not part of the protocol.
                        cause: error.to_string(),
                    });
                }
                Err(error) => return Err(error),
            }
        };
    }

    loop {
        // Exhaustion is decided here, BEFORE the validator (@Tess).
        //
        // The cursor can land exactly on `end`: deleting `[a, a\0)` covers `a` and advances
        // to `a\0`, which *is* `end`. The remaining range is empty and the delete is
        // complete. Asking the validator about `[end, end)` invites it to resolve a region
        // for `end` itself, and when `end` sits on a region boundary that is the NEXT
        // region — whose epoch the caller never claimed. A finished delete would then report
        // `StaleEpoch`: a false failure on a request that fully succeeded. For a receipt
        // that is the worse direction, because the caller acts on it and redoes work that is
        // already done. Only for a bounded `end`; an empty `end` means "to the end of the
        // keyspace" and no cursor can reach it.
        // The loop owns `start` as well as `end` so the FIRST round is decided by the same
        // expression as every later one. Deriving the remaining start inside the caller's
        // closure left round 1 with nothing to compare: the cursor is `None`, the closure
        // resolved it back to `start`, and an already-empty bounded range (`start >= end`)
        // reached the validator before anyone asked whether there was work to do. A
        // zero-work request could then be refused as stale.
        let remaining_start = cursor.as_deref().unwrap_or(start);
        if !end.is_empty() && remaining_start >= end {
            return Ok(DeleteRangeReceipt {
                committed_chunks,
                last_applied,
            });
        }
        // Revalidate the authorisation for the REMAINING range, every round including the
        // first, and structurally before planning.
        //
        // A range delete longer than one chunk becomes N independent raft entries over an
        // unbounded wall-clock window. The leader gate was already re-taken per chunk; the
        // context gate — keyspace, api_type, region, epoch, span-within-region — was checked
        // once before the loop, so chunks 2..N wrote under an authorisation validated
        // against a state that may no longer hold. `check_context` exists precisely to
        // refuse acting on a region whose epoch has moved, and it was being consulted only
        // about the first chunk.
        //
        // It is a required closure rather than a call the endpoint remembers to make: with
        // one seam and no second entry-point check, an endpoint that omits the validator
        // does not compile. Failure preserves the receipt, so a caller learns a stale
        // authorisation stopped it mid-range rather than being told nothing happened.
        //
        // This NARROWS the window; it does not close it. A split already committed to the
        // log but not yet applied locally still reads as the old epoch here, and
        // `Command::Write` carries no expected epoch, so apply cannot refuse it — task #48
        // layer 2, the fenced envelope. Do not read this as the fix.
        preserving_receipt!(revalidate(remaining_start));
        let Some((batch, last_key)) = preserving_receipt!(plan_next(cursor.as_deref())) else {
            return Ok(DeleteRangeReceipt {
                committed_chunks,
                last_applied,
            });
        };
        // Only "partial" once something has actually committed. Failing before the first
        // chunk really is "nothing happened", and dressing that up as partial would be
        // its own lie — in the opposite direction.
        let position = preserving_receipt!(commit(batch));
        committed_chunks += 1;
        last_applied = Some(position);
        cursor = Some(last_key);
    }
}

/// The raw data plane. This is the layer that holds **both** the driver and the store, so
/// it is the only place a raw write can legitimately become a committed raft entry.
///
/// Every write here follows the same shape as `create_keyspace`: build the command,
/// propose it, and wait for *that exact* `(term, index)` to apply. Writing the local
/// engine directly would be faster and would silently fork the cluster.
impl RuntimeBackend {
    /// Validate the request context before any key is encoded.
    ///
    /// Thin wiring; the decision lives in [`check_context`] so tests exercise the same
    /// function production does rather than a re-implementation of it.
    fn validated_context(&self, ctx: &RequestContext, span: KeySpan<'_>) -> Result<()> {
        check_context(
            &self.node.meta_raft.store,
            ctx.keyspace,
            &ctx.region_epoch,
            span,
        )
    }

    /// A read view over applied state, refused unless this node currently leads.
    ///
    /// Not linearizable: `check_quorum` bounds how long a deposed leader keeps believing
    /// it leads, but within that window this returns stale data. See `LeaderRead`.
    fn leader_read(&self) -> Result<(Box<dyn ReadView + '_>, Option<NodeId>, bool)> {
        let status = self.driver.status();
        let is_leader = status.role == Role::Leader;
        let hint = status.leader_id;
        let view = self.node.meta_raft.store.engine().snapshot()?;
        Ok((view, hint, is_leader))
    }

    /// Replicate one planned batch and wait for its exact position to apply.
    ///
    /// Returns that position so the caller can hand it back to the client. Deriving it
    /// afterwards from the status file cannot prove identity: a concurrent command moves
    /// the same number, so the client would be shown someone else's write.
    fn commit_batch(&self, batch: kv9_engine::WriteBatch) -> Result<AppliedPosition> {
        if batch.mutations().is_empty() {
            return Ok(AppliedPosition { term: 0, index: 0 });
        }
        // `write_from_batch`, not `from_batch`: the latter yields a `CatalogTxn`, and
        // sharing the catalog's wire tag would replay user data through the catalog path
        // and inherit its serializing lock.
        let command = Command::write_from_batch(&batch);
        let proposed = self.driver.propose(&command)?;
        match self.driver.wait_applied(proposed, RAW_APPLY_DEADLINE)? {
            true => Ok(AppliedPosition {
                term: proposed.term,
                index: proposed.index.0,
            }),
            // The slot was taken by a different entry: a new leader overwrote this
            // position. Success is judged on (term, index), never on elapsed time.
            false => Err(Error::Raft(format!(
                "raw write at term {} index {} was overwritten before it applied",
                proposed.term, proposed.index.0
            ))),
        }
    }
}

/// How long to wait for a raw write's own `(term, index)` to reach the state machine.
const RAW_APPLY_DEADLINE: Duration = Duration::from_secs(10);

impl RawApi for RuntimeBackend {
    fn raw_get(&self, ctx: &RequestContext, key: &[u8]) -> Result<Option<Value>> {
        self.ensure_serving()?;
        self.validated_context(ctx, KeySpan::Point(key))?;
        let (view, hint, is_leader) = self.leader_read()?;
        let read = LeaderRead::new(view.as_ref(), is_leader, hint)?;
        RawExecutor.get(&read, ctx.keyspace, key)
    }

    fn raw_batch_get(&self, ctx: &RequestContext, keys: &[UserKey]) -> Result<Vec<Option<Value>>> {
        self.ensure_serving()?;
        self.validated_context(
            ctx,
            KeySpan::Batch(keys.iter().map(|k| k.as_slice()).collect()),
        )?;
        let (view, hint, is_leader) = self.leader_read()?;
        let read = LeaderRead::new(view.as_ref(), is_leader, hint)?;
        RawExecutor.batch_get(&read, ctx.keyspace, keys)
    }

    fn raw_put(&self, ctx: &RequestContext, key: UserKey, value: Value) -> Result<AppliedPosition> {
        self.ensure_serving()?;
        self.validated_context(ctx, KeySpan::Point(&key))?;
        let plan = RawExecutor.plan_put(ctx.keyspace, &key, value, RawWriteOptions::default())?;
        self.commit_batch(plan)
    }

    fn raw_batch_put(
        &self,
        ctx: &RequestContext,
        pairs: &[(UserKey, Value)],
    ) -> Result<AppliedPosition> {
        self.ensure_serving()?;
        self.validated_context(
            ctx,
            KeySpan::Batch(pairs.iter().map(|(k, _)| k.as_slice()).collect()),
        )?;
        // One batch ⇒ one entry ⇒ all of these land together or none do.
        let plan = RawExecutor.plan_batch_put(ctx.keyspace, pairs, RawWriteOptions::default())?;
        self.commit_batch(plan)
    }

    fn raw_delete(&self, ctx: &RequestContext, key: &[u8]) -> Result<AppliedPosition> {
        self.ensure_serving()?;
        self.validated_context(ctx, KeySpan::Point(key))?;
        let plan = RawExecutor.plan_delete(ctx.keyspace, key)?;
        self.commit_batch(plan)
    }

    fn raw_scan(
        &self,
        ctx: &RequestContext,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<(UserKey, Value)>> {
        self.ensure_serving()?;
        self.validated_context(ctx, KeySpan::Range { start, end })?;
        let (view, hint, is_leader) = self.leader_read()?;
        let read = LeaderRead::new(view.as_ref(), is_leader, hint)?;
        RawExecutor.scan(&read, ctx.keyspace, start, end, limit)
    }

    fn raw_delete_range(
        &self,
        ctx: &RequestContext,
        start: &[u8],
        end: &[u8],
    ) -> Result<DeleteRangeReceipt> {
        self.ensure_serving()?;
        // One chunk in memory at a time: read a bounded chunk, commit it, resume strictly
        // after the last key it covered. Planning the whole range up front bounded the
        // raft *entry* while leaving the planner unbounded.
        // There is deliberately NO context check before this call. The loop revalidates on
        // every round including the first, so a second entry-point check would be a parallel
        // path that could drift from the one the loop applies — and the endpoint would then
        // be authorised by a rule the loop does not use.
        run_delete_range(
            start,
            end,
            |remaining_start| {
                self.validated_context(
                    ctx,
                    KeySpan::Range {
                        start: remaining_start,
                        end,
                    },
                )
            },
            |cursor| {
                // Planning reads the range, so it needs the same leader gate as a scan.
                let (view, hint, is_leader) = self.leader_read()?;
                let read = LeaderRead::new(view.as_ref(), is_leader, hint)?;
                RawExecutor.plan_delete_range_chunk(
                    &read,
                    ctx.keyspace,
                    cursor,
                    start,
                    end,
                    RAW_DELETE_RANGE_CHUNK,
                )
            },
            |batch| self.commit_batch(batch),
        )
    }
}

impl TxnApi for RuntimeBackend {
    fn kv_get(&self, ctx: &RequestContext, key: &[u8], ts: TimeStamp) -> Result<Option<Value>> {
        self.ensure_serving()?;
        self.node.kv_get(ctx, key, ts)
    }
    fn kv_batch_get(
        &self,
        ctx: &RequestContext,
        keys: &[UserKey],
        ts: TimeStamp,
    ) -> Result<Vec<Option<Value>>> {
        self.ensure_serving()?;
        self.node.kv_batch_get(ctx, keys, ts)
    }
    fn kv_scan(
        &self,
        ctx: &RequestContext,
        start: &[u8],
        end: &[u8],
        limit: usize,
        ts: TimeStamp,
    ) -> Result<Vec<(UserKey, Value)>> {
        self.ensure_serving()?;
        self.node.kv_scan(ctx, start, end, limit, ts)
    }
    fn kv_prewrite(
        &self,
        ctx: &RequestContext,
        mutations: &[(UserKey, Option<Value>)],
        primary: &[u8],
        ts: TimeStamp,
    ) -> Result<()> {
        self.ensure_serving()?;
        self.node.kv_prewrite(ctx, mutations, primary, ts)
    }
    fn kv_commit(
        &self,
        ctx: &RequestContext,
        keys: &[UserKey],
        start_ts: TimeStamp,
        commit_ts: TimeStamp,
    ) -> Result<()> {
        self.ensure_serving()?;
        self.node.kv_commit(ctx, keys, start_ts, commit_ts)
    }
    fn kv_pessimistic_lock(
        &self,
        ctx: &RequestContext,
        keys: &[UserKey],
        ts: TimeStamp,
    ) -> Result<()> {
        self.ensure_serving()?;
        self.node.kv_pessimistic_lock(ctx, keys, ts)
    }
    fn kv_pessimistic_rollback(
        &self,
        ctx: &RequestContext,
        keys: &[UserKey],
        ts: TimeStamp,
    ) -> Result<()> {
        self.ensure_serving()?;
        self.node.kv_pessimistic_rollback(ctx, keys, ts)
    }
    fn kv_resolve_lock(
        &self,
        ctx: &RequestContext,
        start_ts: TimeStamp,
        commit_ts: Option<TimeStamp>,
    ) -> Result<()> {
        self.ensure_serving()?;
        self.node.kv_resolve_lock(ctx, start_ts, commit_ts)
    }
    fn kv_cleanup(&self, ctx: &RequestContext, key: &[u8], ts: TimeStamp) -> Result<()> {
        self.ensure_serving()?;
        self.node.kv_cleanup(ctx, key, ts)
    }
    fn kv_check_txn_status(
        &self,
        ctx: &RequestContext,
        primary: &[u8],
        lock_ts: TimeStamp,
    ) -> Result<()> {
        self.ensure_serving()?;
        self.node.kv_check_txn_status(ctx, primary, lock_ts)
    }
}

/// A running real-process metadata member.
pub struct NodeRuntime {
    node: Arc<Node<WalEngine>>,
    driver: Arc<NodeDriver<DiskRaftStorage, WalEngine>>,
    transport: Arc<GrpcTransport>,
    discovery: Arc<RuntimeDiscovery>,
    driver_thread: Option<std::thread::JoinHandle<()>>,
    grpc_runtime: tokio::runtime::Runtime,
    grpc_shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    grpc_server: Option<tokio::task::JoinHandle<std::result::Result<(), tonic::transport::Error>>>,
    cluster_token: String,
    voters: Vec<NodeId>,
    seeds: Vec<SeedPeer>,
    discovery_observations: BTreeMap<u64, DiscoveryObservation>,
    advertised_endpoint_observation: Option<DiscoveryObservation>,
    registration_observation: RegistrationObservation,
    data_dir: PathBuf,
    status_path: PathBuf,
    addr: std::net::SocketAddr,
    root: RootDescriptor,
    store_identity: StoreIdentity,
    join_ticket_sha256: Option<RootDigest>,
    // NOTE: no voter_fp field. The fingerprint lives ONLY in the FSM's
    // pre-initialization states (full-path structural retirement, task #24):
    // after initialization there is no runtime field left to misread as
    // identity. The discovery ANSWER side keeps its copy solely to serve
    // pre-init peers, and zeroes it once initialized.
    campaign_started: bool,
    initial_proposal: Option<(ProposedAt, kv9_common::ClusterId)>,
    registration_receipt: Option<RegistrationReceipt>,
    next_discovery: Instant,
    next_advertised_endpoint_probe: Instant,
}

impl NodeRuntime {
    /// Start a node whose creation authority and store identity were explicitly
    /// provisioned. This function persists/verifies that bundle before opening
    /// Raft or the catalog: ordinary process startup can never infer permission
    /// to create a cluster from an empty directory or a reachable quorum.
    pub fn start_with_root(
        id: NodeId,
        config: Config,
        auth: RuntimeAuth,
        root: RootDescriptor,
        store_identity: StoreIdentity,
    ) -> Result<Self> {
        Self::start_with_root_and_ticket(id, config, auth, root, store_identity, None)
    }

    /// Start with a one-time join credential. Initial voters never need one;
    /// a new store must present it to the leader before its admission can be
    /// consumed and AddLearner proposed.
    pub fn start_with_root_and_ticket(
        id: NodeId,
        config: Config,
        auth: RuntimeAuth,
        root: RootDescriptor,
        store_identity: StoreIdentity,
        join_ticket: Option<&str>,
    ) -> Result<Self> {
        root.validate()?;
        store_identity.verify(&root, id)?;
        config.validate()?;
        auth.validate()?;
        let addr = config.addr.parse().map_err(|_| {
            Error::Config(format!(
                "addr must be a numeric socket address: {}",
                config.addr
            ))
        })?;
        let seeds: Vec<SeedPeer> = root
            .voters
            .iter()
            .map(|voter| SeedPeer {
                node_id: voter.node_id,
                addr: voter.addr,
            })
            .collect();
        if !config.join.is_empty() && config.join != seeds {
            return Err(Error::Config(
                "runtime seed set does not match the canonical root descriptor".into(),
            ));
        }
        // The root address is the canonical advertised endpoint; `addr` is
        // only the local listener bind. They are intentionally allowed to
        // differ (for example a stable Kubernetes Service ClusterIP advertising
        // a Pod that binds 0.0.0.0). Peer identity never comes from the bind
        // address.
        let joining = seeds.iter().all(|seed| seed.node_id != id);
        let join_ticket_sha256 = join_ticket.map(|ticket| RootDigest::sha256(ticket.as_bytes()));

        let data_dir = PathBuf::from(&config.data_dir);
        persist_root_bundle(&data_dir, &root, &store_identity)?;
        let voters: Vec<NodeId> = seeds.iter().map(|seed| seed.node_id).collect();
        let voter_fp = voter_set_fingerprint(
            &seeds
                .iter()
                .map(|seed| (seed.node_id.0, seed.addr))
                .collect::<Vec<_>>(),
        );
        let voter_ids: Vec<u64> = voters.iter().map(|node| node.0).collect();
        let (storage, was_pristine) = DiskRaftStorage::open(&data_dir.join("raft"), &voter_ids)?;
        let peer = Arc::new(RaftPeer::with_storage(id, META_REGION_0, storage)?);

        let (engine, replay) = WalEngine::open(data_dir.join("catalog.wal"))?;
        if replay.discarded_tail_bytes > 0 {
            eprintln!(
                "node {} recovered catalog WAL after discarding {} torn tail bytes",
                id.0, replay.discarded_tail_bytes
            );
        }
        let engine = Arc::new(engine);
        let raft: Arc<dyn RaftGroup> = peer.clone();
        let node = Arc::new(Node::with_raft_and_engine(
            id,
            config,
            raft,
            engine.clone(),
        )?);

        // Initialized-authority is the CLUSTER IDENTITY, not the schema row
        // (task #24 gate 2; Tess's finding on the old preflight): a catalog
        // that has schema but cannot name its cluster is corrupt or from a
        // pre-identity build — fail closed rather than publish initialized.
        let local_identity = node.local_cluster_identity()?;
        if local_identity.is_some_and(|cluster_id| cluster_id != root.cluster_id) {
            return Err(Error::MetaNotReady(
                "catalog cluster identity does not match the durable root descriptor".into(),
            ));
        }
        if local_identity.is_some() {
            let txn = node.meta_raft.store.begin()?;
            match kv9_meta::root::certified_root(&txn)? {
                Some(certified) if certified == root => {}
                Some(_) => {
                    return Err(Error::MetaNotReady(
                        "catalog root certificate does not match the durable root descriptor"
                            .into(),
                    ))
                }
                None => {
                    return Err(Error::MetaNotReady(
                        "catalog identity exists without a committed root certificate".into(),
                    ))
                }
            }
        }
        if local_identity.is_none() && catalog_initialized(&node)? {
            return Err(Error::MetaNotReady(
                "catalog has schema but no cluster identity; refusing to treat \
                 this data-dir as initialized (corrupt or pre-identity catalog)"
                    .into(),
            ));
        }
        let marker_initialized = init_marker_exists(&data_dir);
        let mut bootstrap = if joining {
            Bootstrap::join_existing_at(id, voters.clone(), root.cluster_id, voter_fp, &data_dir)?
        } else {
            Bootstrap::with_seeds_fp(id, voters.clone(), voter_fp)
        };
        if init_marker_exists(&data_dir) {
            bootstrap.mark_data_dir_initialized();
        }
        // A non-pristine Raft member must never form a second cluster, even if
        // it crashed before the marker rename. It rejoins and waits for catalog.
        // This fence prevents an initial voter with durable Raft history from
        // ever re-entering creation. A joiner has no creation authority in
        // the first place; retaining a failed pre-registration Raft open must
        // not suppress its next discovery attempt with a corrected ticket.
        if !was_pristine && !joining {
            bootstrap.mark_data_dir_initialized();
        }
        if local_identity.is_some() && !marker_initialized {
            write_init_marker(&data_dir)?;
            bootstrap.mark_data_dir_initialized();
        }
        node.meta.lock().expect("meta poisoned").bootstrap = bootstrap;

        let discovery = Arc::new(RuntimeDiscovery::new(
            id,
            marker_initialized || local_identity.is_some(),
            voter_fp,
            RootWireIdentity {
                bootstrap_generation: root.bootstrap_generation,
                root_digest: root.digest(),
            },
        ));
        if let Some(idty) = local_identity {
            discovery.set_cluster_id(idty);
        }
        let grpc_runtime = tokio::runtime::Runtime::new()
            .map_err(|error| Error::Config(format!("create gRPC runtime: {error}")))?;
        let transport = GrpcTransport::new(
            id,
            Some(auth.cluster_token.clone()),
            grpc_runtime.handle().clone(),
            root.digest(),
        );
        for seed in &seeds {
            if seed.node_id != id {
                transport.register_peer(seed.node_id, seed.addr);
            }
        }
        let driver = NodeDriver::new(
            peer,
            transport.clone(),
            MemStateMachine::with_engine(engine)?,
        );
        let driver_thread = Some(driver.spawn(TICK));
        let status_path = data_dir.join("status");

        let backend = Arc::new(RuntimeBackend {
            node: node.clone(),
            driver: driver.clone(),
            transport: transport.clone(),
        });
        let client_authenticator = Arc::new(TokenAuthenticator::new(auth.client_tokens)?);
        let public_service =
            Kv9Grpc::new(backend.clone()).authenticated_service(client_authenticator);
        let cluster_authenticator = Arc::new(ClusterAuthenticator {
            expected_token: Arc::from(auth.cluster_token.clone()),
            voters: Arc::new(voters.iter().copied().collect()),
            node: node.clone(),
        });
        let raft_service = RaftGrpcService::new(id, transport.inbox_sender(), discovery.clone())
            .with_registration(backend as Arc<dyn RegistrationBackend>);
        let raft_service = tonic::service::interceptor::InterceptedService::new(
            Kv9RaftServer::new(raft_service),
            AuthInterceptor::new(cluster_authenticator),
        );
        let listener = grpc_runtime
            .block_on(tokio::net::TcpListener::bind(addr))
            .map_err(|error| Error::Config(format!("bind gRPC listener {addr}: {error}")))?;
        let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
        let (grpc_shutdown_tx, grpc_shutdown_rx) = tokio::sync::oneshot::channel();
        let grpc_server = grpc_runtime.spawn(
            tonic::transport::Server::builder()
                .add_service(public_service)
                .add_service(raft_service)
                .serve_with_incoming_shutdown(incoming, async {
                    let _ = grpc_shutdown_rx.await;
                }),
        );
        let discovery_observations = seeds
            .iter()
            .copied()
            .map(|seed| {
                (
                    seed.node_id.0,
                    DiscoveryObservation::new(seed, seed.node_id == id),
                )
            })
            .collect();
        let advertised_endpoint_observation = seeds
            .iter()
            .copied()
            .find(|seed| seed.node_id == id)
            .map(|seed| DiscoveryObservation::new(seed, false));

        Ok(Self {
            node,
            driver,
            transport,
            discovery,
            driver_thread,
            grpc_runtime,
            grpc_shutdown: Some(grpc_shutdown_tx),
            grpc_server: Some(grpc_server),
            cluster_token: auth.cluster_token,
            voters,
            seeds,
            discovery_observations,
            advertised_endpoint_observation,
            registration_observation: RegistrationObservation::new(),
            data_dir,
            status_path,
            addr,
            root,
            store_identity,
            join_ticket_sha256,
            campaign_started: false,
            initial_proposal: None,
            registration_receipt: None,
            next_discovery: Instant::now(),
            next_advertised_endpoint_probe: Instant::now(),
        })
    }

    pub fn status_path(&self) -> &Path {
        &self.status_path
    }

    /// Stay resident and advance bootstrap. Normal OS termination signals use
    /// the platform default action; no shutdown hook is required for safety
    /// because both durable logs fsync before visibility/messages.
    pub fn run(mut self) -> Result<()> {
        loop {
            self.check_grpc_server()?;
            if let Some(fatal) = self.driver.status().fatal {
                self.write_status()?;
                return Err(Error::Raft(fatal));
            }
            self.observe_advertised_endpoint();
            self.sync_registered_peers()?;
            self.advance_bootstrap()?;
            self.write_status()?;
            std::thread::sleep(TICK);
        }
    }

    /// Probe the canonical advertised address independently of the listener
    /// bind. They may legitimately differ (Service/Pod, NAT, wildcard bind),
    /// so equality is not an identity rule; an authenticated discovery round
    /// to the advertised endpoint is the actual configuration diagnostic.
    fn observe_advertised_endpoint(&mut self) {
        if Instant::now() < self.next_advertised_endpoint_probe {
            return;
        }
        self.next_advertised_endpoint_probe = Instant::now() + DISCOVERY_INTERVAL;
        let Some(observation) = self.advertised_endpoint_observation.as_mut() else {
            return;
        };
        observation.record_attempt();
        match grpc_discover(
            self.grpc_runtime.handle(),
            self.node.id,
            observation.seed.addr,
            DISCOVERY_TIMEOUT,
            Some(self.cluster_token.clone()),
            self.discovery.root_identity(),
        ) {
            Ok(answer) if answer.node == self.node.id => {
                observation.record_accepted(answer.initialized)
            }
            Ok(_) => observation.record_rejected(DiscoveryRejection::NodeId),
            Err(error) => observation.record_error(&error),
        }
    }

    /// Every replica learns dynamic transport endpoints from the replicated
    /// nodes catalog. The registration leader installs the address eagerly;
    /// followers converge here before/after applying the ordered ConfChange,
    /// and raft heartbeat retry completes learner catch-up without a second
    /// out-of-band address authority.
    fn sync_registered_peers(&self) -> Result<()> {
        const MAX_PHASE1_NODES: usize = 1024;
        let txn = self.node.meta_raft.store.begin()?;
        let rows = txn.scan(&NODES_DESC, MAX_PHASE1_NODES + 1)?;
        if rows.len() > MAX_PHASE1_NODES {
            return Err(Error::Config(format!(
                "nodes catalog reached Phase-1 limit {MAX_PHASE1_NODES}"
            )));
        }
        for row in rows {
            let node = match row.value.get(ColumnId(1)) {
                Some(ColumnValue::Uint(id)) if *id != 0 => NodeId(*id),
                _ => continue,
            };
            if node == self.node.id {
                continue;
            }
            let addr = match row.value.get(ColumnId(2)) {
                Some(ColumnValue::Text(addr)) if !addr.is_empty() => addr,
                _ => continue,
            };
            let addr = addr.parse().map_err(|_| {
                Error::Config(format!(
                    "nodes catalog contains non-canonical address for node {}",
                    node.0
                ))
            })?;
            self.transport.register_peer(node, addr);
        }
        Ok(())
    }

    fn check_grpc_server(&mut self) -> Result<()> {
        let finished = self
            .grpc_server
            .as_ref()
            .is_some_and(tokio::task::JoinHandle::is_finished);
        if !finished {
            return Ok(());
        }
        let server = self.grpc_server.take().expect("checked above");
        match self.grpc_runtime.block_on(server) {
            Ok(Ok(())) => Err(Error::Raft("gRPC server stopped unexpectedly".into())),
            Ok(Err(error)) => Err(Error::Raft(format!("gRPC server failed: {error}"))),
            Err(error) => Err(Error::Raft(format!("gRPC server task failed: {error}"))),
        }
    }

    fn advance_bootstrap(&mut self) -> Result<()> {
        let state = self
            .node
            .meta
            .lock()
            .expect("meta poisoned")
            .bootstrap
            .state();
        match state {
            BootstrapState::Discovering { .. } => self.advance_discovery(),
            BootstrapState::BootstrapElection { .. } => self.advance_election(),
            BootstrapState::Initializing { .. } => self.advance_initialization(),
            BootstrapState::WaitForBootstrap { .. } | BootstrapState::Joining { .. } => {
                self.advance_joining()
            }
            BootstrapState::Serving { .. } => Ok(()),
        }
    }

    fn advance_discovery(&mut self) -> Result<()> {
        let locally_fenced = self
            .node
            .meta
            .lock()
            .expect("meta poisoned")
            .bootstrap
            .data_dir_initialized();
        // LOCAL CATALOG FIRST: if this node's own durable catalog names a
        // cluster, the cluster exists — a lost marker or silent peers must
        // never lead to an "uninitialized quorum" here. Scope (corrected
        // after Tess's re-review): the constructor preflight above (the
        // catalog_initialized check at start) already repairs the marker for
        // the crash-window restart; this branch is the identity-carrying
        // second line for the tick itself, and the hook where join-existing
        // verifies the expected id. The seam commit switches the preflight's
        // authority from the schema row to the ClusterId as well.
        {
            let local = self.node.local_cluster_identity()?;
            let mut meta = self.node.meta.lock().expect("meta poisoned");
            if meta.bootstrap.observe_local_identity(local)? {
                return Ok(());
            }
        }
        if locally_fenced {
            // Marker present but the catalog cannot name the cluster yet
            // (e.g. an old empty-format marker with a not-yet-replayed
            // catalog): rule (c) already blocks re-init; just wait.
            return Ok(());
        }
        if Instant::now() < self.next_discovery {
            return Ok(());
        }
        self.next_discovery = Instant::now() + DISCOVERY_INTERVAL;

        // The fingerprint lives in the FSM's pre-initialization states and
        // NOWHERE else in this struct — this loop only runs in Discovering,
        // so it is always present here; after initialization there is no
        // field left to misread (full-path retirement, Tess's item 3).
        let bootstrap_fp = {
            let meta = self.node.meta.lock().expect("meta poisoned");
            match meta.bootstrap.bootstrap_fingerprint() {
                Some(fp) => fp,
                None => return Ok(()), // no longer Discovering: nothing to do
            }
        };
        let mut uninitialized = vec![self.node.id];
        let mut found_initialized: Option<ClusterId> = None;
        for seed in &self.seeds {
            if seed.node_id == self.node.id {
                continue;
            }
            self.discovery_observations
                .get_mut(&seed.node_id.0)
                .expect("every declared seed has a bounded observation slot")
                .record_attempt();
            let answer = match grpc_discover(
                self.grpc_runtime.handle(),
                self.node.id,
                seed.addr,
                DISCOVERY_TIMEOUT,
                Some(self.cluster_token.clone()),
                self.discovery.root_identity(),
            ) {
                Ok(answer) => answer,
                Err(error) => {
                    self.discovery_observations
                        .get_mut(&seed.node_id.0)
                        .expect("every declared seed has a bounded observation slot")
                        .record_error(&error);
                    continue;
                }
            };
            // Both the address→identity mapping and (pre-init) the complete
            // declared voter set must match. A rejected answer is recorded,
            // but it still cannot become bootstrap evidence.
            if let Err(reason) = validate_discovery_answer(*seed, bootstrap_fp, &answer) {
                self.discovery_observations
                    .get_mut(&seed.node_id.0)
                    .expect("every declared seed has a bounded observation slot")
                    .record_rejected(reason);
                continue;
            }
            self.discovery_observations
                .get_mut(&seed.node_id.0)
                .expect("every declared seed has a bounded observation slot")
                .record_accepted(answer.initialized);
            if answer.initialized {
                let id = answer
                    .cluster_id
                    .expect("grpc_discover enforces initialized iff cluster_id");
                if let Some(seen) = found_initialized {
                    if seen != id {
                        return Err(Error::MetaNotReady(
                            "declared seeds report different cluster identities".into(),
                        ));
                    }
                }
                found_initialized = Some(id);
            } else {
                uninitialized.push(answer.node);
            }
        }
        if let Some(cluster_id) = found_initialized {
            // Carry the answer's identity into the FSM. Initial voters still
            // wait for their local replicated catalog before Serving; a
            // non-member has no local catalog yet, so discarding this value
            // would leave join-existing permanently stuck in Discovering.
            self.node
                .meta
                .lock()
                .expect("meta poisoned")
                .bootstrap
                .on_event(BootstrapEvent::FoundInitialized { cluster_id })?;
            return Ok(());
        }
        let mut meta = self.node.meta.lock().expect("meta poisoned");
        // Insufficient evidence is expected while peers start; silence never
        // changes the voter denominator and never becomes an answer.
        let _ = meta.bootstrap.discovered_uninitialized(&uninitialized);
        Ok(())
    }

    fn advance_election(&mut self) -> Result<()> {
        if !self.campaign_started {
            self.driver.peer().campaign()?;
            self.campaign_started = true;
        }
        let status = self.driver.status();
        let Some(leader) = status.leader_id else {
            return Ok(());
        };
        let event = if leader == self.node.id && status.role == Role::Leader {
            BootstrapEvent::WonElection
        } else {
            BootstrapEvent::LostElection
        };
        self.node
            .meta
            .lock()
            .expect("meta poisoned")
            .bootstrap
            .on_event(event)?;
        Ok(())
    }

    fn advance_initialization(&mut self) -> Result<()> {
        if self.driver.status().role != Role::Leader {
            // Lost leadership before the init committed. NOT fatal (task
            // #40): erroring here kills the runtime and strands the new
            // leader in WaitForBootstrap forever. Demote so the bootstrap
            // role tracks leadership, and drop the stale-term proposal — it
            // can never commit, and a later re-promotion must propose fresh
            // (behind the takeover barrier) rather than wait on a dead
            // position.
            self.initial_proposal = None;
            self.node
                .meta
                .lock()
                .expect("meta poisoned")
                .bootstrap
                .on_event(BootstrapEvent::LostElection)?;
            return Ok(());
        }
        if self.initial_proposal.is_none() {
            // Creation authority existed before Raft opened. Election chooses
            // which provisioned voter may submit the root; it never mints a
            // new identity and therefore cannot fork creation after a retry.
            let cluster_id = self.root.cluster_id;
            let cmd = self
                .node
                .build_initial_metadata_command_for_root(&self.voters, &self.root)?;
            self.initial_proposal = Some((self.driver.propose(&cmd)?, cluster_id));
            return Ok(());
        }
        let (proposal, cluster_id) = self.initial_proposal.expect("set above");
        match self.driver.wait_applied(proposal, Duration::from_millis(1)) {
            Ok(true) => {
                self.verify_certified_root()?;
                write_init_marker(&self.data_dir)?;
                self.discovery.set_cluster_id(cluster_id);
                self.node
                    .meta
                    .lock()
                    .expect("meta poisoned")
                    .bootstrap
                    .on_event(BootstrapEvent::MetadataInitialized { cluster_id })?;
                Ok(())
            }
            Ok(false) => Err(Error::Raft(format!(
                "bootstrap proposal at term {} index {} was overwritten",
                proposal.term, proposal.index.0
            ))),
            // A one-millisecond condition poll timing out means "pending".
            Err(_) => Ok(()),
        }
    }

    fn advance_joining(&mut self) -> Result<()> {
        let joining_mode = self
            .node
            .meta
            .lock()
            .expect("meta poisoned")
            .bootstrap
            .is_joining_mode();
        if joining_mode {
            return self.advance_registration();
        }
        // The catalog names the cluster once the winner's init applied here;
        // until then there is nothing to join yet.
        let cluster_id = {
            let txn = self.node.meta_raft.store.begin()?;
            match kv9_meta::admission::cluster_id(&txn)? {
                Some(id) => id,
                None => {
                    // Catalog still empty. If THIS node is now the raft
                    // leader, take over initialization — but only through
                    // the current-term barrier gate (see
                    // bootstrap_takeover_proven for why the order matters).
                    if bootstrap_takeover_proven(&self.driver, &self.node)? {
                        let mut meta = self.node.meta.lock().expect("meta poisoned");
                        if matches!(
                            meta.bootstrap.state(),
                            BootstrapState::WaitForBootstrap { .. }
                        ) {
                            meta.bootstrap.on_event(BootstrapEvent::WonElection)?;
                        }
                    }
                    return Ok(());
                }
            }
        };
        self.verify_certified_root()?;
        write_init_marker(&self.data_dir)?;
        self.discovery.set_cluster_id(cluster_id);
        let mut meta = self.node.meta.lock().expect("meta poisoned");
        match meta.bootstrap.state() {
            BootstrapState::WaitForBootstrap { .. } => {
                // Catalog exists locally: fingerprint retires, register.
                meta.bootstrap
                    .on_event(BootstrapEvent::MetadataInitialized { cluster_id })?;
                meta.bootstrap.on_event(BootstrapEvent::Registered)?;
            }
            BootstrapState::Joining { .. } => {
                meta.bootstrap.on_event(BootstrapEvent::Registered)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Join-existing client state: obtain the leader's exact registration
    /// receipt, then wait until that same entry and ClusterId are applied on
    /// this replica before exposing Serving.
    fn advance_registration(&mut self) -> Result<()> {
        let cluster_id = self
            .node
            .meta
            .lock()
            .expect("meta poisoned")
            .bootstrap
            .cluster_id()
            .ok_or_else(|| Error::MetaNotReady("joining without a cluster identity".into()))?;

        // A node that previously completed registration has both the init
        // marker (written only after the exact receipt was observed) and the
        // committed Active row + durable ConfState. On restart, those three
        // durable facts are the receipt; do not require a live registration
        // leader merely to re-enter Serving.
        if init_marker_exists(&self.data_dir)
            && self.node.local_cluster_identity()? == Some(cluster_id)
            && self.local_membership_is_active()?
        {
            self.discovery.set_cluster_id(cluster_id);
            self.node
                .meta
                .lock()
                .expect("meta poisoned")
                .bootstrap
                .on_event(BootstrapEvent::Registered)?;
            return Ok(());
        }

        if self.registration_receipt.is_none() {
            let ticket = self.join_ticket_sha256.ok_or_else(|| {
                Error::Config(
                    "joining an existing cluster requires KV9_JOIN_TICKET until registration completes"
                        .into(),
                )
            })?;
            for seed in &self.seeds {
                self.registration_observation.record_attempt();
                match grpc_register(
                    self.grpc_runtime.handle(),
                    self.node.id,
                    seed.addr,
                    &self.addr.to_string(),
                    JoinIdentity {
                        cluster_id,
                        ticket_sha256: ticket,
                        store_incarnation: self.store_identity.store_incarnation,
                    },
                    DISCOVERY_TIMEOUT,
                    Some(self.cluster_token.clone()),
                ) {
                    Ok(RegisterOutcome::Registered(receipt)) => {
                        self.registration_observation.record_registered();
                        self.registration_receipt = Some(receipt);
                        break;
                    }
                    Ok(RegisterOutcome::NotLeader { .. }) => {
                        self.registration_observation.record_not_leader();
                    }
                    Err(error) => {
                        self.registration_observation.record_error(&error);
                        if error == RegisterError::InvalidTicket {
                            break;
                        }
                    }
                }
            }
            return Ok(());
        }

        let receipt = self.registration_receipt.as_ref().expect("checked above");
        let exact = ProposedAt {
            term: receipt.applied_term,
            index: kv9_raft::LogIndex(receipt.applied_index),
        };
        match self.driver.wait_applied(exact, Duration::from_millis(1)) {
            Ok(true) => {}
            Ok(false) => {
                return Err(Error::Raft(format!(
                    "registration receipt at term {} index {} was overwritten",
                    exact.term, exact.index.0
                )))
            }
            Err(_) => return Ok(()),
        }
        let local = self.node.local_cluster_identity()?;
        if local != Some(cluster_id) {
            return Ok(());
        }
        write_init_marker(&self.data_dir)?;
        self.discovery.set_cluster_id(cluster_id);
        self.node
            .meta
            .lock()
            .expect("meta poisoned")
            .bootstrap
            .on_event(BootstrapEvent::Registered)?;
        Ok(())
    }

    fn local_membership_is_active(&self) -> Result<bool> {
        let status = self.driver.status();
        if !status.voters.contains(&self.node.id.0) && !status.learners.contains(&self.node.id.0) {
            return Ok(false);
        }
        let txn = self.node.meta_raft.store.begin()?;
        let Some(row) = txn.get(&NODES_DESC, &[memcmp_uint(self.node.id.0)])? else {
            return Ok(false);
        };
        Ok(matches!(
            row.value.get(ColumnId(3)),
            Some(ColumnValue::Uint(2))
        ))
    }

    fn verify_certified_root(&self) -> Result<()> {
        let txn = self.node.meta_raft.store.begin()?;
        match kv9_meta::root::certified_root(&txn)? {
            Some(root) if root == self.root => Ok(()),
            Some(_) => Err(Error::MetaNotReady(
                "committed root certificate does not match the durable root descriptor".into(),
            )),
            None => Err(Error::MetaNotReady(
                "cluster identity exists without a committed root certificate".into(),
            )),
        }
    }

    fn write_status(&self) -> Result<()> {
        let raft = self.driver.status();
        let bootstrap = self
            .node
            .meta
            .lock()
            .expect("meta poisoned")
            .bootstrap
            .state();
        let role = match raft.role {
            Role::Leader => "leader",
            Role::Follower => "follower",
            Role::Candidate => "candidate",
            Role::Learner => "learner",
            // In neither voter nor learner set: a config-identity fault that
            // must never render as a healthy follower (task #24).
            Role::Unconfigured => "unconfigured",
        };
        // Once raft is initialized, its committed ConfState is the membership
        // authority. `self.voters` is only the boot-time seed declaration and
        // becomes stale after the first learner/promotion change.
        let meta_voters = format_u64_ids(&raft.voters);
        let meta_learners = format_u64_ids(&raft.learners);
        let cluster_id = match bootstrap {
            BootstrapState::Joining { cluster_id } | BootstrapState::Serving { cluster_id } => {
                Some(cluster_id)
            }
            _ => self.node.local_cluster_identity()?,
        };
        let pending_admissions = {
            let txn = self.node.meta_raft.store.begin()?;
            let nodes = kv9_meta::admission::pending_admissions(&txn)?
                .into_iter()
                .map(|admission| admission.node_id.0)
                .collect::<Vec<_>>();
            format_u64_ids(&nodes)
        };
        let discovery_status = format_discovery_observations(&self.discovery_observations);
        let advertised_endpoint_status =
            format_advertised_endpoint(self.advertised_endpoint_observation.as_ref());
        // Rendered as complete labeled lines by the single tested helper —
        // see render_driver_applied for why no tuple crosses this boundary.
        let driver_applied_lines = render_driver_applied(raft.driver_applied);
        let body = format!(
            "pid={}\nnode_id={}\ncluster_id={}\nbootstrap_generation={}\nroot_digest={}\nstore_incarnation={}\nleader_id={}\nrole={}\nmeta_voters={}\nmeta_learners={}\npending_admissions={}\nconf_index={}\nterm={}\nraft_committed={}\napplied_index={}\napplied_term={}\n{}bootstrap_state={:?}\nadvertised_endpoint={}\nregistration_attempts={}\nregistration_errors={}\nregistration_last={}\n{}fatal={}\n",
            std::process::id(),
            raft.node_id.0,
            cluster_id.map_or_else(String::new, |id| id.to_string()),
            self.root.bootstrap_generation,
            self.root.digest(),
            self.store_identity.store_incarnation,
            raft.leader_id.map_or(0, |id| id.0),
            role,
            meta_voters,
            meta_learners,
            pending_admissions,
            raft.conf_index,
            raft.term,
            raft.raft_committed,
            raft.applied_index,
            raft.applied_term,
            driver_applied_lines,
            bootstrap,
            advertised_endpoint_status,
            self.registration_observation.attempts,
            self.registration_observation.errors,
            self.registration_observation.last.label(),
            discovery_status,
            raft.fatal.as_deref().unwrap_or(""),
        );
        let tmp = self.data_dir.join("status.tmp");
        fs::write(&tmp, body)
            .and_then(|_| fs::rename(&tmp, &self.status_path))
            .map_err(|e| Error::Config(format!("write {}: {e}", self.status_path.display())))
    }
}

/// Render the unified driver watermark as its two COMPLETE status lines. The
/// key<->value binding lives only here (review finding: a (String, String)
/// tuple passed the pairing through two more positional, same-type,
/// swap-compilable points — and a swapped pair would make the barrier's
/// `term == leader_term` comparison pass COINCIDENTALLY in small clusters, a
/// randomly-arriving false green on the minting gate, the e2ecc5a family).
/// Both lines come from one snapshot: both `none` (nothing proven this run;
/// NEVER rendered as 0) or both decimal; mixed or label-swapped output is
/// unrepresentable outside this function, and the unit test asserts the full
/// labeled text, so an internal swap reds it.
fn render_driver_applied(pos: Option<DriverAppliedPosition>) -> String {
    match pos {
        Some(pos) => format!(
            "driver_applied_index={}\ndriver_applied_term={}\n",
            pos.index, pos.term
        ),
        None => "driver_applied_index=none\ndriver_applied_term=none\n".to_string(),
    }
}

fn format_u64_ids(nodes: &[u64]) -> String {
    let mut ids = nodes.to_vec();
    ids.sort_unstable();
    ids.into_iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

fn format_advertised_endpoint(observation: Option<&DiscoveryObservation>) -> String {
    match observation {
        Some(observation) => format!(
            "addr={},attempts={},reachable={},errors={},rejected_root_identity={},rejected_node_id={},last={}",
            observation.seed.addr,
            observation.attempts,
            observation.accepted,
            observation.errors,
            observation.rejected_root_identity,
            observation.rejected_node_id,
            observation.last.label(),
        ),
        None => "not_declared".into(),
    }
}

fn format_discovery_observations(observations: &BTreeMap<u64, DiscoveryObservation>) -> String {
    observations
        .iter()
        .map(|(node, observation)| {
            format!(
                "discovery_seed_{node}=addr={},attempts={},accepted={},errors={},rejected_root_identity={},rejected_node_id={},rejected_voter_fingerprint={},last={}\n",
                observation.seed.addr,
                observation.attempts,
                observation.accepted,
                observation.errors,
                observation.rejected_root_identity,
                observation.rejected_node_id,
                observation.rejected_voter_fingerprint,
                observation.last.label(),
            )
        })
        .collect::<Vec<_>>()
        .join("")
}

fn validate_discovery_answer(
    declared: SeedPeer,
    expected_voter_fp: u64,
    answer: &kv9_raft::grpc::DiscoverAnswer,
) -> std::result::Result<(), DiscoveryRejection> {
    if answer.node != declared.node_id {
        return Err(DiscoveryRejection::NodeId);
    }
    if answer.initialized {
        // Post-init authority is the ClusterId (decode guarantees it is
        // present on an initialized answer); the fingerprint has retired and
        // responders publish 0 — comparing it here would re-animate it.
        // Wrong-cluster protection: initial-bootstrap voters only adopt an
        // identity from their OWN catalog; join-existing verifies the id
        // against its expectation inside the FSM.
        Ok(())
    } else if answer.voter_fingerprint == expected_voter_fp {
        Ok(())
    } else {
        Err(DiscoveryRejection::VoterFingerprint)
    }
}

impl Drop for NodeRuntime {
    fn drop(&mut self) {
        self.driver.stop();
        if let Some(handle) = self.driver_thread.take() {
            let _ = handle.join();
        }
        if let Some(shutdown) = self.grpc_shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(server) = self.grpc_server.take() {
            let _ = self.grpc_runtime.block_on(server);
        }
    }
}

fn catalog_initialized(node: &Node<WalEngine>) -> Result<bool> {
    Ok(node
        .meta_raft
        .store
        .begin()?
        .get(&SCHEMA_VERSION_DESC, &[memcmp_uint(0)])?
        .is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kv9_common::RegionId;
    use kv9_engine::testing::FaultyEngine;
    use kv9_engine::{ColumnFamily, Engine, MemEngine};
    use kv9_raft::transport::{InProcHub, RaftTransport};
    use kv9_raft::Command;
    use tonic::Code;

    /// task #40, the OTHER refusal arm (review round: the frozen-window test
    /// lands in the None arm, so the term COMPARISON — the line the frozen
    /// contract's "index alone is not enough" sentence exists for — was never
    /// evaluated on a refusal path; deleting it alone stayed green). The
    /// dangerous state is a watermark that EXISTS but carries an old term:
    /// entries applied at term N, then a re-election with apply frozen, so
    /// the new term-N+1 leader still holds Some(term N). The gate must
    /// refuse on the term comparison; removing that comparison (keeping the
    /// Some binding) turns exactly this test red.
    #[test]
    fn takeover_gate_refuses_a_stale_term_watermark() {
        use kv9_raft::transport::InProcHub;
        // Two drivers over an in-process hub: real elections, real terms.
        let hub = InProcHub::new();
        let ids = [NodeId(1), NodeId(2)];
        let mk = |id: NodeId| {
            let peer = Arc::new(RaftPeer::new(id, META_REGION_0, &ids).unwrap());
            NodeDriver::new(
                peer,
                Arc::new(hub.endpoint(id)) as Arc<dyn kv9_raft::transport::RaftTransport>,
                MemStateMachine::new(),
            )
        };
        let d1 = mk(NodeId(1));
        let d2 = mk(NodeId(2));
        let pump = |n: usize| {
            for _ in 0..n {
                d1.tick_and_step().unwrap();
                d2.tick_and_step().unwrap();
            }
        };

        // Term 1: n1 leads; a committed command applies on BOTH nodes, so
        // n2's watermark becomes Some(term 1, ..).
        d1.peer().campaign().unwrap();
        for _ in 0..200 {
            pump(1);
            if d1.status().role == Role::Leader {
                break;
            }
        }
        assert_eq!(d1.status().role, Role::Leader);
        let at = d1
            .propose(&Command::Put {
                cf: 0,
                key: b"seed".to_vec(),
                value: b"x".to_vec(),
            })
            .unwrap();
        for _ in 0..200 {
            pump(1);
            if d2.driver_applied().is_some_and(|wm| wm.index >= at.index.0) {
                break;
            }
        }
        let wm1 = d2.driver_applied().expect("n2 applied at term 1");
        assert_eq!(
            wm1.term, at.term,
            "window precondition: watermark at term 1"
        );

        // Freeze n2's apply, then move leadership: check_quorum makes the
        // live leader sticky (it refuses votes while its lease holds), so
        // first tick n1 ALONE past the election timeout with no quorum
        // contact — check_quorum's own discipline steps it down — and only
        // then let n2 campaign. n2's watermark stays Some(term 1) while its
        // leader term moves on.
        d2.pause_apply(true);
        for _ in 0..40 {
            d1.tick_and_step().unwrap(); // no n2 ticks: no heartbeat ACKs
        }
        assert_ne!(
            d1.status().role,
            Role::Leader,
            "check_quorum must depose a leader with no quorum contact \
             (test precondition)"
        );
        d2.peer().campaign().unwrap();
        for _ in 0..300 {
            // n1 answers messages but its election timer stays still (step,
            // not tick): a deposed n1 whose timer also fired would become a
            // competing pre-candidate and 2-node vote-splits livelock.
            d1.step().unwrap();
            d2.tick_and_step().unwrap();
            let s = d2.status();
            if s.role == Role::Leader && s.term > at.term {
                break;
            }
        }
        let s2 = d2.status();
        assert_eq!(s2.role, Role::Leader, "n2 must win the term-2 election");
        assert!(s2.term > at.term);
        let wm = d2.driver_applied().expect("watermark still present");
        assert!(
            wm.term < s2.term,
            "window precondition: watermark EXISTS but from an old term \
             (Some({}, ..) vs leader term {})",
            wm.term,
            s2.term
        );

        // The gate must refuse on the TERM comparison — a stale-term
        // watermark means committed-but-unapplied entries (this term's no-op
        // at minimum) may hide an init.
        let (backend, _rt, dir) = pre_serving_runtime_backend();
        assert!(
            !bootstrap_takeover_proven(&d2, &backend.node).unwrap(),
            "gate must refuse a watermark from an older term: entries \
             committed in the current term are provably not yet applied"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// task #40: the takeover gate consumes the current-term barrier — with
    /// the deterministic committed-but-unapplied window, not a race. The
    /// apply freeze holds the driver watermark back while raft elects and
    /// commits; the gate must refuse until the watermark reaches the leader
    /// term, then prove the catalog empty, then (control) refuse once a
    /// cluster identity exists. Removing the watermark conjunct from
    /// bootstrap_takeover_proven turns the frozen-window assertion red.
    #[test]
    fn takeover_gate_waits_for_the_current_term_barrier() {
        let (backend, _rt, dir) = pre_serving_runtime_backend();
        let driver = &backend.driver;

        // Not leader yet: gate refuses on role.
        assert!(
            !bootstrap_takeover_proven(driver, &backend.node).unwrap(),
            "gate must refuse before leadership"
        );

        // Freeze apply BEFORE campaigning: leadership and commits proceed,
        // the watermark does not — the committed-but-unapplied window, held
        // open deterministically.
        driver.pause_apply(true);
        driver.peer().campaign().unwrap();
        for _ in 0..100 {
            driver.tick_and_step().unwrap();
            if driver.status().role == Role::Leader {
                break;
            }
        }
        assert_eq!(driver.status().role, Role::Leader);
        // Pump more ticks: the election no-op COMMITS but must not apply.
        for _ in 0..20 {
            driver.tick_and_step().unwrap();
        }
        let status = driver.status();
        assert!(
            status.raft_committed > 0,
            "the election no-op must have committed (window precondition)"
        );
        assert!(
            driver
                .driver_applied()
                .is_none_or(|wm| wm.term < status.term),
            "apply freeze must hold the watermark below the leader term \
             (window precondition)"
        );
        assert!(
            !bootstrap_takeover_proven(driver, &backend.node).unwrap(),
            "gate must refuse while the current-term barrier is unmet: a \
             committed-but-unapplied init would be invisible and the \
             takeover would double-propose into an apply-time poison"
        );

        // Unfreeze: the watermark catches up to the leader term and the gate
        // opens on the provably-empty catalog.
        driver.pause_apply(false);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            driver.tick_and_step().unwrap();
            let s = driver.status();
            if driver.driver_applied().is_some_and(|wm| wm.term == s.term) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "watermark never reached the leader term after unpause"
            );
        }
        assert!(
            bootstrap_takeover_proven(driver, &backend.node).unwrap(),
            "gate must open once the barrier is met and the catalog is empty"
        );

        // Control: a present cluster identity closes the gate — takeover
        // must never re-initialize an initialized cluster.
        {
            let mut txn = backend.node.meta_raft.store.begin().unwrap();
            kv9_meta::admission::initialize_cluster(
                &mut txn,
                kv9_common::ClusterId::from_bytes([9; 16]),
                1,
            )
            .unwrap();
            txn.commit().unwrap();
        }
        assert!(
            !bootstrap_takeover_proven(driver, &backend.node).unwrap(),
            "gate must refuse once a cluster identity exists"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn pre_serving_runtime_backend() -> (RuntimeBackend, tokio::runtime::Runtime, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "kv9-pre-serving-gate-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let (storage, _) = DiskRaftStorage::open(&dir.join("raft"), &[1]).unwrap();
        let peer = Arc::new(RaftPeer::with_storage(NodeId(1), META_REGION_0, storage).unwrap());
        let (engine, _) = WalEngine::open(dir.join("catalog.wal")).unwrap();
        let engine = Arc::new(engine);
        let node = Arc::new(
            Node::with_raft_and_engine(NodeId(1), Config::default(), peer.clone(), engine.clone())
                .unwrap(),
        );
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let transport = GrpcTransport::new(
            NodeId(1),
            None,
            runtime.handle().clone(),
            kv9_common::RootDigest::from_bytes([0; 32]),
        );
        let driver = NodeDriver::new(
            peer,
            transport.clone(),
            MemStateMachine::with_engine(engine).unwrap(),
        );
        (
            RuntimeBackend {
                node,
                driver,
                transport,
            },
            runtime,
            dir,
        )
    }

    fn assert_meta_not_ready<T: std::fmt::Debug>(result: Result<T>) {
        match result {
            Err(Error::MetaNotReady(message)) => {
                assert!(
                    message.contains("Discovering"),
                    "wrong readiness detail: {message}"
                );
            }
            other => panic!("pre-Serving public request escaped the readiness gate: {other:?}"),
        }
    }

    /// Frozen status contract: the two driver_applied_* lines come from one
    /// snapshot — both `none` or both decimal, and the key<->value binding is
    /// asserted on the FULL labeled text (an index/term swap anywhere inside
    /// the helper reds here; outside the helper no tuple exists to swap).
    /// `none` is fail-closed ("nothing proven this run"), never rendered as 0.
    #[test]
    fn driver_applied_renders_as_labeled_pair_never_mixed() {
        assert_eq!(
            render_driver_applied(None),
            "driver_applied_index=none\ndriver_applied_term=none\n"
        );
        assert_eq!(
            render_driver_applied(Some(DriverAppliedPosition { term: 3, index: 17 })),
            "driver_applied_index=17\ndriver_applied_term=3\n"
        );
    }

    #[test]
    fn every_public_request_is_meta_not_ready_before_planning() {
        let (backend, runtime, dir) = pre_serving_runtime_backend();
        let ctx = RequestContext {
            keyspace: KeyspaceId(100),
            region_epoch: epoch(1, 1),
            caller: Some("test-client".into()),
        };

        // Admin: CreateKeyspace used to reach catalog planning here and leak an FK
        // violation for the bootstrap-owned default tenant. The other calls are listed
        // explicitly so a newly added/rewired public method cannot quietly skip the gate.
        assert_meta_not_ready(backend.create_keyspace(
            "test-client",
            "too-early",
            TenantId::DEFAULT,
            ApiType::Raw,
            TxnGroupId(0),
        ));
        assert_meta_not_ready(backend.list_keyspaces("test-client"));
        assert_meta_not_ready(backend.get_region("test-client", KeyspaceId(100), b"k"));
        assert_meta_not_ready(backend.split_region("test-client", RegionId(1), b"m".to_vec()));
        assert_meta_not_ready(backend.cluster_info("test-client"));
        assert_meta_not_ready(backend.admit_node("test-client", NodeId(2), "127.0.0.1:20161", 60));
        assert_meta_not_ready(backend.promote_node("test-client", NodeId(2)));

        assert_meta_not_ready(backend.raw_get(&ctx, b"k"));
        assert_meta_not_ready(backend.raw_batch_get(&ctx, &[b"k".to_vec()]));
        assert_meta_not_ready(backend.raw_put(&ctx, b"k".to_vec(), b"v".to_vec()));
        assert_meta_not_ready(backend.raw_batch_put(&ctx, &[(b"k".to_vec(), b"v".to_vec())]));
        assert_meta_not_ready(backend.raw_delete(&ctx, b"k"));
        assert_meta_not_ready(backend.raw_scan(&ctx, b"", b"", 10));
        assert_meta_not_ready(backend.raw_delete_range(&ctx, b"", b""));

        assert_meta_not_ready(backend.kv_get(&ctx, b"k", TimeStamp(1)));
        assert_meta_not_ready(backend.kv_batch_get(&ctx, &[b"k".to_vec()], TimeStamp(1)));
        assert_meta_not_ready(backend.kv_scan(&ctx, b"", b"", 10, TimeStamp(1)));
        assert_meta_not_ready(backend.kv_prewrite(
            &ctx,
            &[(b"k".to_vec(), Some(b"v".to_vec()))],
            b"k",
            TimeStamp(1),
        ));
        assert_meta_not_ready(backend.kv_commit(
            &ctx,
            &[b"k".to_vec()],
            TimeStamp(1),
            TimeStamp(2),
        ));
        assert_meta_not_ready(backend.kv_pessimistic_lock(&ctx, &[b"k".to_vec()], TimeStamp(1)));
        assert_meta_not_ready(backend.kv_pessimistic_rollback(
            &ctx,
            &[b"k".to_vec()],
            TimeStamp(1),
        ));
        assert_meta_not_ready(backend.kv_resolve_lock(&ctx, TimeStamp(1), Some(TimeStamp(2))));
        assert_meta_not_ready(backend.kv_cleanup(&ctx, b"k", TimeStamp(1)));
        assert_meta_not_ready(backend.kv_check_txn_status(&ctx, b"k", TimeStamp(1)));

        drop(backend);
        drop(runtime);
        fs::remove_dir_all(dir).unwrap();
    }

    /// Builds a node with a raw keyspace whose single region has been replaced by two
    /// adjacent ones, so cross-region cases are constructible. Regions go in through the
    /// same encoded apply path production uses.
    fn two_region_keyspace() -> (crate::Node<kv9_engine::MemEngine>, KeyspaceId) {
        use kv9_common::{RegionId, TenantId};
        use kv9_meta::codec::{memcmp_uint, ColumnValue, RowValue};
        use kv9_meta::schema::ColumnId;
        use kv9_meta::schema::REGIONS_DESC;

        let node = crate::Node::new(NodeId(1), kv9_common::Config::default()).unwrap();
        node.bootstrap().unwrap();
        let keyspace = node
            .create_keyspace("gated", TenantId::DEFAULT, ApiType::Raw)
            .unwrap();

        let initial = Tables::new(&node.meta_raft.store)
            .region_for_key(keyspace, b"")
            .unwrap()
            .expect("CreateKeyspace creates the initial region")
            .id;
        let row = |id: u64, start: &[u8], end: &[u8]| {
            let mut r = RowValue::new();
            r.set(ColumnId(1), ColumnValue::Uint(id));
            r.set(ColumnId(2), ColumnValue::Uint(keyspace.0 as u64));
            r.set(ColumnId(3), ColumnValue::Bytes(start.to_vec()));
            r.set(ColumnId(4), ColumnValue::Bytes(end.to_vec()));
            r.set(ColumnId(5), ColumnValue::Uint(1));
            r.set(ColumnId(6), ColumnValue::Uint(1));
            r.set(ColumnId(7), ColumnValue::Uint(0));
            r
        };
        let mut seed = node.meta_raft.store.begin().unwrap();
        seed.delete(&REGIONS_DESC, &[memcmp_uint(initial.0)])
            .unwrap();
        // [a, m) and [m, ) -- the second is the keyspace's trailing region.
        seed.insert(&REGIONS_DESC, &[memcmp_uint(300)], row(300, b"a", b"m"))
            .unwrap();
        seed.insert(&REGIONS_DESC, &[memcmp_uint(301)], row(301, b"m", b""))
            .unwrap();
        node.meta_raft
            .propose_apply(Command::from_batch(&seed.into_batch()))
            .unwrap();
        let _ = RegionId(0);
        (node, keyspace)
    }

    fn epoch(conf: u64, ver: u64) -> kv9_region::RegionEpoch {
        kv9_region::RegionEpoch {
            conf_ver: conf,
            version: ver,
        }
    }

    /// A point at the right epoch passes; the same point at a stale epoch is refused, and
    /// the error names the region so a client knows what to refresh.
    #[test]
    fn a_stale_epoch_is_refused_and_names_the_region() {
        let (node, keyspace) = two_region_keyspace();
        let store = &node.meta_raft.store;

        check_context(store, keyspace, &epoch(1, 1), KeySpan::Point(b"b"))
            .expect("control: the current epoch is accepted");

        for (conf, ver) in [(2, 1), (1, 2)] {
            match check_context(store, keyspace, &epoch(conf, ver), KeySpan::Point(b"b")) {
                Err(Error::StaleEpoch { region }) => assert_eq!(
                    region.0, 300,
                    "the error must name the region whose epoch moved"
                ),
                other => panic!(
                    "epoch ({conf},{ver}) should be stale, got ok={}",
                    other.is_ok()
                ),
            }
        }
    }

    /// A batch must prove *every* key lands in one region. Checking the first and hoping
    /// would let the second key be written under an epoch that never authorised it.
    #[test]
    fn a_batch_spanning_two_regions_is_refused() {
        let (node, keyspace) = two_region_keyspace();
        let store = &node.meta_raft.store;

        check_context(
            store,
            keyspace,
            &epoch(1, 1),
            KeySpan::Batch(vec![b"b", b"c"]),
        )
        .expect("control: keys in one region are accepted");

        // b is in [a,m); z is in [m,) -- same epoch, different regions.
        assert!(
            matches!(
                check_context(
                    store,
                    keyspace,
                    &epoch(1, 1),
                    KeySpan::Batch(vec![b"b", b"z"])
                ),
                Err(Error::RangeCrossesRegion)
            ),
            "a batch crossing regions must be refused, not silently split"
        );
    }

    /// Range boundaries through the real gate, including the half-open edge and the
    /// asymmetry of the two "unbounded" meanings.
    #[test]
    fn range_boundaries_are_enforced_by_the_gate() {
        let (node, keyspace) = two_region_keyspace();
        let store = &node.meta_raft.store;
        let range = |start: &'static [u8], end: &'static [u8]| {
            check_context(store, keyspace, &epoch(1, 1), KeySpan::Range { start, end })
        };

        range(b"a", b"c").expect("inside [a,m)");
        range(b"a", b"m").expect("end == region end is inside: the range stops short of m");
        assert!(
            matches!(range(b"a", b"z"), Err(Error::RangeCrossesRegion)),
            "an end past the region boundary crosses into the next region"
        );
        assert!(
            matches!(range(b"a", b""), Err(Error::RangeCrossesRegion)),
            "an unbounded end asks for the whole keyspace, which [a,m) cannot satisfy"
        );
        range(b"m", b"").expect("unbounded end IS satisfiable by the trailing region");
    }

    /// The asymmetry between the two "empty means to the end" values is the whole rule,
    /// and it is easy to get backwards -- an empty `end` asks for the whole *keyspace*,
    /// while an empty `region_end` only says this region is the keyspace's last.
    #[test]
    fn an_unbounded_range_end_is_only_legal_in_the_trailing_region() {
        // Trailing region (empty end_key): everything is inside it, including unbounded.
        assert!(
            range_end_within_region(b"", b""),
            "unbounded in trailing region"
        );
        assert!(
            range_end_within_region(b"m", b""),
            "bounded in trailing region"
        );

        // A bounded region cannot satisfy "to the end of the keyspace".
        assert!(
            !range_end_within_region(b"", b"m"),
            "an unbounded end reaches past a region that is not the last"
        );

        // Ordinary containment, and the half-open boundary itself.
        assert!(range_end_within_region(b"a", b"m"), "end before region end");
        assert!(
            range_end_within_region(b"m", b"m"),
            "end == region end is inside: the range is half-open, so it stops short of m"
        );
        assert!(
            !range_end_within_region(b"z", b"m"),
            "end past region end crosses into the next region"
        );
    }

    /// Plans `total` one-key chunks, then stops. Each chunk deletes a distinct key so
    /// the *effect* of each chunk is separately observable.
    fn planner(
        total: usize,
    ) -> impl FnMut(Option<&[u8]>) -> Result<Option<(kv9_engine::WriteBatch, UserKey)>> {
        let mut issued = 0usize;
        move |_cursor| {
            if issued >= total {
                return Ok(None);
            }
            issued += 1;
            let key = vec![b'k', issued as u8];
            let mut batch = kv9_engine::WriteBatch::new();
            batch.delete(ColumnFamily::Default, key.clone());
            Ok(Some((batch, key)))
        }
    }

    /// An engine pre-loaded with the keys the planner will delete, so a test can ask
    /// which chunks actually took effect rather than trusting a counter.
    fn seeded_engine(keys: usize) -> MemEngine {
        let engine = MemEngine::new();
        let mut batch = kv9_engine::WriteBatch::new();
        for i in 1..=keys {
            batch.put(ColumnFamily::Default, vec![b'k', i as u8], b"v".to_vec());
        }
        engine.write(batch).unwrap();
        engine
    }

    fn present(engine: &MemEngine, i: u8) -> bool {
        engine
            .get(ColumnFamily::Default, &[b'k', i])
            .unwrap()
            .is_some()
    }

    /// The authorisation is re-checked on **every** chunk, and a verdict that turns stale
    /// mid-range stops the delete with a receipt rather than finishing under it.
    ///
    /// This is the assertion the layer-1 change actually owes. The planner/cursor tests in
    /// `kv9-txn` cover the enabling change; none of them touch this loop, and deleting the
    /// `revalidate` call left the entire workspace green — the mutation below is what makes
    /// "re-checked per chunk" a claim with evidence behind it rather than a comment.
    ///
    /// Mutation: remove `preserving_receipt!(revalidate(..))` from the loop. Chunk 2 then
    /// commits, `present(2)` becomes false, and this test reds on "exactly one chunk". The
    /// stale verdict is delivered on the SECOND call, after chunk 1 has really committed, so
    /// the partial-receipt path is exercised too, not just the refusal.
    #[test]
    fn a_stale_authorisation_stops_the_range_and_reports_a_partial_receipt() {
        let engine = seeded_engine(4);
        let mut checks = 0u64;
        let result = run_delete_range(
            b"",
            b"",
            |_remaining_start| {
                checks += 1;
                // Passes for chunk 1, stale from chunk 2 on — the region moved under us.
                if checks >= 2 {
                    return Err(Error::StaleEpoch {
                        region: kv9_common::RegionId(300),
                    });
                }
                Ok(())
            },
            planner(4),
            |batch| {
                engine.write(batch).unwrap();
                Ok(AppliedPosition { term: 7, index: 11 })
            },
        );

        match result {
            Err(Error::PartialDeleteRange {
                committed_chunks,
                cause,
                ..
            }) => {
                assert_eq!(committed_chunks, 1, "exactly one chunk may have committed");
                assert!(
                    cause.contains("epoch"),
                    "the receipt must name the stale authorisation as the cause, got: {cause}"
                );
            }
            other => panic!("expected a partial receipt naming the stale epoch, got {other:?}"),
        }
        assert!(
            !present(&engine, 1),
            "chunk 1 committed before the verdict turned"
        );
        assert!(
            present(&engine, 2),
            "chunk 2 must NOT have been planned or committed once the authorisation went stale"
        );
        assert_eq!(
            checks, 2,
            "the validator is consulted once per round, not once per call"
        );
    }

    /// Exhaustion is decided before the validator is consulted, so a delete that finishes
    /// exactly on `end` cannot be reported as a failure.
    ///
    /// A semantically real half-open range: `[a, a\0)` contains exactly `a`. The planner
    /// deletes `a` and returns the next cursor `a\0`, which *is* `end`. Asking the validator
    /// about the empty `[a\0, a\0)` would resolve a region for `end` itself — the next
    /// region when `end` is a boundary — and a completed delete would come back as
    /// `StaleEpoch`. The validator here fails if it is consulted at that point.
    #[test]
    fn a_cursor_landing_exactly_on_end_completes_instead_of_revalidating() {
        let engine = MemEngine::new();
        let mut seed = kv9_engine::WriteBatch::new();
        seed.put(ColumnFamily::Default, b"a".to_vec(), b"v".to_vec());
        engine.write(seed).unwrap();

        let mut issued = false;
        let mut seen = Vec::new();
        let receipt = run_delete_range(
            b"a",
            b"a\0",
            |remaining_start| {
                seen.push(remaining_start.to_vec());
                Ok(())
            },
            |_cursor| {
                if issued {
                    return Ok(None);
                }
                issued = true;
                let mut batch = kv9_engine::WriteBatch::new();
                batch.delete(ColumnFamily::Default, b"a".to_vec());
                // The planner advances past the key it covered: `a` -> `a\0`.
                Ok(Some((batch, b"a\0".to_vec())))
            },
            |batch| {
                engine.write(batch).unwrap();
                Ok(AppliedPosition { term: 7, index: 11 })
            },
        )
        .expect("a delete that ends exactly on `end` is complete, not stale");

        assert_eq!(receipt.committed_chunks, 1);
        assert_eq!(
            seen,
            vec![b"a".to_vec()],
            "validated once for the real remainder `[a, a\\0)`; the empty remainder that \
             follows must never reach the validator"
        );
        assert!(
            engine.get(ColumnFamily::Default, b"a").unwrap().is_none(),
            "the one key in range must actually be gone"
        );
    }

    /// A bounded range that is *already* empty must complete without consulting the
    /// validator at all — round one has no cursor, and deriving the remaining start outside
    /// the loop left that round with nothing to compare against.
    ///
    /// `start == end` is zero work. If `end` sits on a region boundary the validator would
    /// resolve the NEXT region, whose epoch the caller never claimed, and a request that
    /// asked for nothing would be refused as stale. False failure on a receipt is the
    /// harmful direction: the caller retries work that never existed.
    #[test]
    fn an_initially_empty_bounded_range_completes_without_consulting_the_validator() {
        let receipt = run_delete_range(
            b"m",
            b"m",
            |remaining_start| {
                panic!("the validator must not be asked about a range that is already empty, got {remaining_start:?}")
            },
            |_cursor| panic!("nothing may be planned for an empty range"),
            |_batch| panic!("nothing may be committed for an empty range"),
        )
        .expect("an already-empty bounded range is complete, not stale");

        assert_eq!(receipt.committed_chunks, 0);
        assert_eq!(receipt.last_applied, None);
    }

    /// Failing at chunk 2 must leave chunk 1's deletion *in the engine* and chunks 2+
    /// untouched — the receipt has to describe reality, not just count calls.
    #[test]
    fn a_failure_at_the_second_chunk_commits_exactly_the_first_chunk() {
        let engine = seeded_engine(4);
        let mut attempts = 0u64;
        let result = run_delete_range(
            b"",
            b"",
            |_| Ok(()),
            planner(4),
            |batch| {
                attempts += 1;
                if attempts == 2 {
                    return Err(Error::Engine("injected disk failure".into()));
                }
                engine.write(batch)?;
                Ok(AppliedPosition {
                    term: 7,
                    index: 100 + attempts,
                })
            },
        );

        match result {
            Err(Error::PartialDeleteRange {
                committed_chunks,
                last_applied_term,
                last_applied_index,
                cause,
            }) => {
                assert_eq!(committed_chunks, 1);
                assert_eq!(last_applied_term, 7);
                assert_eq!(
                    last_applied_index, 101,
                    "the position of the chunk that DID commit"
                );
                assert!(cause.contains("injected"));
            }
            other => panic!("expected a partial receipt, got ok={}", other.is_ok()),
        }
        assert!(!present(&engine, 1), "chunk 1's delete must have landed");
        assert!(present(&engine, 2), "chunk 2 failed, its key must remain");
        assert!(present(&engine, 3), "later chunks must not have run");
    }

    /// The most realistic partial window: chunk 1 commits, then leadership moves, so
    /// *planning* the next chunk fails. The receipt must survive that too — it is not a
    /// commit-side-only concern.
    #[test]
    fn a_failure_while_planning_the_next_chunk_still_reports_what_committed() {
        let engine = seeded_engine(4);
        let mut planned = 0usize;
        let mut committed = 0u64;
        let result = run_delete_range(
            b"",
            b"",
            |_| Ok(()),
            |_cursor| {
                planned += 1;
                if planned == 2 {
                    return Err(Error::NotLeader {
                        leader: Some(NodeId(3)),
                    });
                }
                let key = vec![b'k', planned as u8];
                let mut batch = kv9_engine::WriteBatch::new();
                batch.delete(ColumnFamily::Default, key.clone());
                Ok(Some((batch, key)))
            },
            |batch| {
                committed += 1;
                engine.write(batch)?;
                Ok(AppliedPosition {
                    term: 9,
                    index: 200 + committed,
                })
            },
        );

        match result {
            Err(Error::PartialDeleteRange {
                committed_chunks,
                last_applied_term,
                last_applied_index,
                ..
            }) => {
                assert_eq!(committed_chunks, 1);
                assert_eq!((last_applied_term, last_applied_index), (9, 201));
            }
            other => panic!(
                "a plan-side failure must preserve the receipt, ok={}",
                other.is_ok()
            ),
        }
        assert!(!present(&engine, 1), "the committed chunk really applied");
        assert!(present(&engine, 2), "nothing after it did");
    }

    /// Failing before anything commits is not partial, and must leave state untouched --
    /// claiming otherwise tells a caller data is gone when none is.
    #[test]
    fn a_failure_on_the_very_first_chunk_is_not_partial_and_changes_nothing() {
        let engine = seeded_engine(4);
        let result = run_delete_range(
            b"",
            b"",
            |_| Ok(()),
            planner(4),
            |_batch| Err(Error::Engine("injected disk failure".into())),
        );
        assert!(matches!(result, Err(Error::Engine(_))));
        for i in 1..=4u8 {
            assert!(present(&engine, i), "no key may have been deleted");
        }
    }

    #[test]
    fn an_empty_range_succeeds_with_a_zero_receipt() {
        let receipt = run_delete_range(
            b"",
            b"",
            |_| Ok(()),
            planner(0),
            |_batch| panic!("nothing should be committed"),
        )
        .unwrap();
        assert_eq!(receipt.committed_chunks, 0);
        assert_eq!(receipt.last_applied, None);
    }

    #[test]
    fn every_chunk_committing_reports_the_last_position_and_empties_the_range() {
        let engine = seeded_engine(3);
        let mut attempts = 0u64;
        let receipt = run_delete_range(
            b"",
            b"",
            |_| Ok(()),
            planner(3),
            |batch| {
                attempts += 1;
                engine.write(batch)?;
                Ok(AppliedPosition {
                    term: 2,
                    index: 50 + attempts,
                })
            },
        )
        .unwrap();
        assert_eq!(receipt.committed_chunks, 3);
        assert_eq!(
            receipt.last_applied,
            Some(AppliedPosition { term: 2, index: 53 })
        );
        for i in 1..=3u8 {
            assert!(!present(&engine, i), "every chunk applied");
        }
    }

    #[test]
    fn cluster_auth_requires_token_and_declared_voter_identity() {
        let dir = std::env::temp_dir().join(format!(
            "kv9-cluster-auth-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let (wal, _) = WalEngine::open(dir.join("catalog.wal")).unwrap();
        let peer = Arc::new(RaftPeer::new(NodeId(1), META_REGION_0, &[NodeId(1)]).unwrap());
        let node = Arc::new(
            Node::with_raft_and_engine(NodeId(1), Config::default(), peer, Arc::new(wal)).unwrap(),
        );
        let authenticator = ClusterAuthenticator {
            expected_token: Arc::from("secret"),
            voters: Arc::new([NodeId(1), NodeId(2)].into_iter().collect()),
            node,
        };
        let mut metadata = MetadataMap::new();
        assert_eq!(
            authenticator.authenticate(&metadata).unwrap_err().code(),
            Code::Unauthenticated
        );

        metadata.insert(CLUSTER_TOKEN_KEY, "secret".parse().unwrap());
        metadata.insert(NODE_ID_KEY, "9".parse().unwrap());
        assert_eq!(
            authenticator.authenticate(&metadata).unwrap_err().code(),
            Code::PermissionDenied
        );

        metadata.insert(NODE_ID_KEY, "2".parse().unwrap());
        let auth = authenticator.authenticate(&metadata).unwrap();
        assert_eq!(auth.node_id, Some(NodeId(2)));
        assert_eq!(auth.auth_kind, AuthKind::Node);
        assert_eq!(auth.principal.as_ref(), "node:2");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn discovery_vote_requires_both_declared_identity_and_voter_set() {
        let declared = SeedPeer {
            node_id: NodeId(3),
            addr: "127.0.0.1:20163".parse().unwrap(),
        };
        let ours = 0x1111;
        let ans = |node: u64, initialized: bool, fp: u64| kv9_raft::grpc::DiscoverAnswer {
            node: NodeId(node),
            initialized,
            voter_fingerprint: fp,
            cluster_id: if initialized {
                Some(kv9_common::ClusterId::from_bytes([9; 16]))
            } else {
                None
            },
            bootstrap_generation: kv9_common::BootstrapGeneration::from_bytes([0; 16]),
            root_digest: kv9_common::RootDigest::from_bytes([0; 32]),
        };
        assert_eq!(
            validate_discovery_answer(declared, ours, &ans(3, false, ours)),
            Ok(())
        );
        assert_eq!(
            validate_discovery_answer(declared, ours, &ans(9, false, ours)),
            Err(DiscoveryRejection::NodeId)
        );
        assert_eq!(
            validate_discovery_answer(declared, ours, &ans(3, false, 0x9999)),
            Err(DiscoveryRejection::VoterFingerprint)
        );
        // Post-init: identity travels as the ClusterId; the retired
        // fingerprint (responders publish 0) must NOT gate the answer…
        assert_eq!(
            validate_discovery_answer(declared, ours, &ans(3, true, 0)),
            Ok(())
        );
        // …but the declared node identity still must match.
        assert_eq!(
            validate_discovery_answer(declared, ours, &ans(9, true, 0)),
            Err(DiscoveryRejection::NodeId)
        );
    }

    #[test]
    fn discovery_observations_are_bounded_single_line_and_saturating() {
        let seed = SeedPeer {
            node_id: NodeId(2),
            addr: "127.0.0.1:20162".parse().unwrap(),
        };
        let mut observation = DiscoveryObservation::new(seed, false);
        observation.attempts = u64::MAX;
        observation.errors = u64::MAX;
        observation.record_attempt();
        observation.record_error(&DiscoveryError::Failed(format!(
            "discovery rpc {}: first line\n{}",
            seed.addr,
            "x".repeat(DISCOVERY_LAST_OUTCOME_MAX_CHARS * 2)
        )));
        assert_eq!(observation.attempts, u64::MAX);
        assert_eq!(observation.errors, u64::MAX);
        let DiscoveryLastOutcome::Error(detail) = &observation.last else {
            panic!("the most recent error must stay queryable");
        };
        assert!(observation.last.label().chars().count() <= DISCOVERY_LAST_OUTCOME_MAX_CHARS);
        assert!(!detail.chars().any(char::is_control));

        let status = format_discovery_observations(&BTreeMap::from([(2, observation.clone())]));
        assert_eq!(status.lines().count(), 1);
        assert!(status.contains("attempts=18446744073709551615"));
        assert!(status.contains("errors=18446744073709551615"));
        assert!(status.contains("last=error:discovery rpc"));

        observation.record_error(&DiscoveryError::RootIdentityMismatch);
        let status = format_discovery_observations(&BTreeMap::from([(2, observation)]));
        assert!(status.contains("rejected_root_identity=1"));
        assert!(status.ends_with("last=rejected_root_identity\n"));
    }

    #[test]
    fn discovery_observations_distinguish_acceptance_and_both_rejections() {
        let seed = SeedPeer {
            node_id: NodeId(3),
            addr: "127.0.0.1:20163".parse().unwrap(),
        };
        let mut observation = DiscoveryObservation::new(seed, false);
        observation.record_attempt();
        observation.record_rejected(DiscoveryRejection::NodeId);
        observation.record_attempt();
        observation.record_rejected(DiscoveryRejection::VoterFingerprint);
        observation.record_attempt();
        observation.record_accepted(false);

        assert_eq!(observation.attempts, 3);
        assert_eq!(observation.accepted, 1);
        assert_eq!(observation.errors, 0);
        assert_eq!(observation.rejected_node_id, 1);
        assert_eq!(observation.rejected_voter_fingerprint, 1);
        assert_eq!(
            observation.last,
            DiscoveryLastOutcome::AcceptedUninitialized
        );
    }

    #[test]
    fn local_seed_is_explicitly_not_a_network_attempt() {
        let seed = SeedPeer {
            node_id: NodeId(1),
            addr: "127.0.0.1:20161".parse().unwrap(),
        };
        let observation = DiscoveryObservation::new(seed, true);
        assert_eq!(observation.attempts, 0);
        assert_eq!(observation.last, DiscoveryLastOutcome::Local);
    }

    #[test]
    fn registration_observation_preserves_typed_reasons_without_diagnostic_text() {
        let mut observation = RegistrationObservation::new();
        observation.record_attempt();
        observation.record_error(&RegisterError::InvalidTicket);
        assert_eq!(observation.attempts, 1);
        assert_eq!(observation.errors, 1);
        assert_eq!(observation.last.label(), "rejected_invalid_ticket");

        observation.record_attempt();
        observation.record_error(&RegisterError::Failed("x".repeat(4096)));
        assert_eq!(observation.last.label(), "failed");
        assert!(!observation.last.label().contains('x'));

        assert!(matches!(
            registration_error(Error::Config(INVALID_JOIN_TICKET_MESSAGE.into())),
            RegistrationError::InvalidTicket
        ));
        assert!(matches!(
            registration_error(Error::Config("admission expired".into())),
            RegistrationError::Failed(Error::Config(message)) if message == "admission expired"
        ));
    }

    #[test]
    fn advertised_endpoint_status_proves_a_network_attempt_or_declares_absence() {
        assert_eq!(format_advertised_endpoint(None), "not_declared");
        let seed = SeedPeer {
            node_id: NodeId(1),
            addr: "127.0.0.1:20161".parse().unwrap(),
        };
        let mut observation = DiscoveryObservation::new(seed, false);
        observation.record_attempt();
        observation.record_accepted(true);
        let rendered = format_advertised_endpoint(Some(&observation));
        assert!(rendered.contains("addr=127.0.0.1:20161"));
        assert!(rendered.contains("attempts=1"));
        assert!(rendered.contains("reachable=1"));
        assert!(rendered.ends_with("last=accepted_initialized"));
    }

    #[test]
    fn status_membership_is_canonical_regardless_of_source_order() {
        assert_eq!(format_u64_ids(&[5, 1, 3]), "1,3,5");
        assert_eq!(format_u64_ids(&[9, 4, 7]), "4,7,9");
        assert_eq!(format_u64_ids(&[]), "");
    }

    #[test]
    fn wal_apply_failure_poisons_the_driver_without_false_success() {
        let dir = std::env::temp_dir().join(format!(
            "kv9-server-faulty-wal-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let (wal, _) = WalEngine::open(dir.join("catalog.wal")).unwrap();
        let engine = Arc::new(FaultyEngine::new(wal));

        let hub = InProcHub::new();
        let peer = Arc::new(RaftPeer::new(NodeId(1), RegionId(1), &[NodeId(1)]).unwrap());
        let endpoint = hub.endpoint(NodeId(1));
        let driver = NodeDriver::new(
            peer,
            Arc::new(endpoint) as Arc<dyn RaftTransport>,
            MemStateMachine::with_engine(engine.clone()).unwrap(),
        );
        driver.peer().campaign().unwrap();
        for _ in 0..50 {
            driver.tick_and_step().unwrap();
            if driver.status().role == Role::Leader {
                break;
            }
        }
        assert_eq!(driver.status().role, Role::Leader);

        let healthy = driver
            .propose(&Command::Put {
                cf: 0,
                key: b"healthy".to_vec(),
                value: b"landed".to_vec(),
            })
            .unwrap();
        for _ in 0..50 {
            driver.tick_and_step().unwrap();
            if driver
                .wait_applied(healthy, Duration::from_millis(1))
                .unwrap_or(false)
            {
                break;
            }
        }
        assert!(driver
            .wait_applied(healthy, Duration::from_millis(5))
            .unwrap());
        let healthy_watermark = driver.status().applied_index;
        let attempts_before = engine.write_attempts();

        engine.start_failing_writes();
        let failed = driver
            .propose(&Command::Put {
                cf: 0,
                key: b"must-not-land".to_vec(),
                value: b"lie".to_vec(),
            })
            .unwrap();
        let mut saw_fatal = false;
        for _ in 0..50 {
            if driver.tick_and_step().is_err() {
                saw_fatal = true;
                break;
            }
        }
        assert!(saw_fatal, "the real-WAL apply failure must poison the pump");
        assert_eq!(
            engine.write_attempts(),
            attempts_before + 1,
            "the failure path must have actually attempted the durable write"
        );
        assert_eq!(driver.status().applied_index, healthy_watermark);
        assert!(driver
            .wait_applied(failed, Duration::from_millis(1))
            .is_err());
        assert_eq!(
            engine.get(ColumnFamily::Default, b"must-not-land").unwrap(),
            None
        );

        // Clearing the simulated disk fault must not silently unpoison a replica
        // that has already skipped a committed entry; recovery requires restart.
        engine.stop_failing_writes();
        assert!(driver.tick_and_step().is_err());
        assert!(driver.status().fatal.is_some());
        drop(driver);
        drop(engine);
        let _ = fs::remove_dir_all(&dir);
    }
}
