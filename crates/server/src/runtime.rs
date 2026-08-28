//! Real Phase-1 metadata-node runtime.
//!
//! This is the process boundary missing from the earlier deterministic harness:
//! fixed seed identities, real TCP discovery/Raft traffic, durable Raft state,
//! durable catalog apply, election-first bootstrap, and a machine-readable status
//! file for external acceptance. The status file is evidence; log timing is not.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kv9_common::{
    ApiType, ClusterId, Config, Error, KeyspaceId, NodeId, RegionId, Result, SeedPeer, TenantId,
    TimeStamp, TxnGroupId, UserKey, Value, META_REGION_0,
};
use kv9_engine::{Engine, ReadView, WalEngine};
use kv9_meta::bootstrap::{init_marker_exists, write_init_marker};
use kv9_meta::codec::memcmp_uint;
use kv9_meta::schema::{ColumnId, NODES_DESC, SCHEMA_VERSION_DESC};
use kv9_meta::{Bootstrap, BootstrapEvent, BootstrapState};
use kv9_meta::{ColumnValue, RowValue};
use kv9_raft::driver::NodeDriver;
use kv9_raft::grpc::{
    grpc_discover, grpc_register, pb::kv9_raft_server::Kv9RaftServer, GrpcDiscoveryState,
    GrpcTransport, RaftGrpcService, RegisterOutcome, RegistrationBackend, RegistrationError,
    RegistrationReceipt, CLUSTER_TOKEN_KEY, NODE_ID_KEY,
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

#[derive(Debug)]
struct RuntimeDiscovery {
    node: NodeId,
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
    fn new(node: NodeId, initialized: bool, voter_fp: u64) -> Self {
        Self {
            node,
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
        self.node.list_keyspaces(caller)
    }

    fn get_region(&self, caller: &str, keyspace: KeyspaceId, key: &[u8]) -> Result<RegionLocation> {
        self.node.get_region(caller, keyspace, key)
    }

    fn split_region(&self, caller: &str, region: RegionId, split_key: UserKey) -> Result<()> {
        self.node.split_region(caller, region, split_key)
    }

    fn cluster_info(&self, caller: &str) -> Result<ClusterInfo> {
        self.node.cluster_info(caller)
    }

    fn admit_node(
        &self,
        _caller: &str,
        node: NodeId,
        addr: &str,
        ttl_seconds: u64,
    ) -> Result<MembershipChangeResult> {
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
        let _guard = self.node.meta_raft.lock_catalog_txn();
        let mut txn = self.node.meta_raft.store.begin()?;
        kv9_meta::admission::admit_node(
            &mut txn,
            node,
            addr,
            kv9_meta::admission::AdmittedRole::Learner,
            expires,
        )?;
        let applied = self.commit_catalog(&kv9_raft::Command::from_batch(&txn.into_batch()))?;
        let status = self.driver.status();
        Ok(MembershipChangeResult {
            applied,
            voters: status.voters,
            learners: status.learners,
        })
    }

    fn promote_node(&self, _caller: &str, node: NodeId) -> Result<MembershipChangeResult> {
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
        })
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn membership_node_row(node: NodeId, addr: &str, state: u64, heartbeat: u64) -> RowValue {
    let mut row = RowValue::new();
    row.set(ColumnId(1), ColumnValue::Uint(node.0));
    row.set(ColumnId(2), ColumnValue::Text(addr.to_string()));
    row.set(ColumnId(3), ColumnValue::Uint(state));
    row.set(ColumnId(4), ColumnValue::Uint(heartbeat));
    row.set(ColumnId(5), ColumnValue::Bytes(Vec::new()));
    row
}

impl RegistrationBackend for RuntimeBackend {
    fn register(
        &self,
        node: NodeId,
        addr: &str,
        cluster_id: ClusterId,
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
            }
            Some(admission) if admission.state == kv9_meta::admission::AdmissionState::Pending => {
                let mut txn = self
                    .node
                    .meta_raft
                    .store
                    .begin()
                    .map_err(RegistrationError::Failed)?;
                kv9_meta::admission::consume_admission(&mut txn, node, cluster_id, &canonical, now)
                    .map_err(RegistrationError::Failed)?;
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
                        ],
                    )
                    .map_err(RegistrationError::Failed)?;
                } else {
                    txn.insert(
                        &NODES_DESC,
                        &[memcmp_uint(node.0)],
                        membership_node_row(node, &canonical, 1, now),
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

/// The chunk loop of a range delete, separated from the machinery that plans and commits.
///
/// Kept standalone so a failure can be injected at chunk N in a test. The interesting
/// behaviour here is not the deleting — it is what is reported when the loop stops early,
/// and that is exactly the part a live-cluster test cannot easily force.
fn run_delete_range<P, C>(mut plan_next: P, mut commit: C) -> Result<DeleteRangeReceipt>
where
    P: FnMut(Option<&[u8]>) -> Result<Option<(kv9_engine::WriteBatch, UserKey)>>,
    C: FnMut(kv9_engine::WriteBatch) -> Result<AppliedPosition>,
{
    let mut cursor: Option<UserKey> = None;
    let mut committed_chunks = 0u64;
    let mut last_applied: Option<AppliedPosition> = None;
    loop {
        let Some((batch, last_key)) = plan_next(cursor.as_deref())? else {
            return Ok(DeleteRangeReceipt {
                committed_chunks,
                last_applied,
            });
        };
        let position = match commit(batch) {
            Ok(position) => position,
            // Only "partial" once something has actually committed. Failing before the
            // first chunk really is "nothing happened", and dressing that up as partial
            // would be its own lie — in the opposite direction.
            Err(error) if committed_chunks > 0 => {
                let last = last_applied.unwrap_or(AppliedPosition { term: 0, index: 0 });
                return Err(Error::PartialDeleteRange {
                    committed_chunks,
                    last_applied_term: last.term,
                    last_applied_index: last.index,
                    // Diagnosis only; deliberately not part of the protocol.
                    cause: error.to_string(),
                });
            }
            Err(error) => return Err(error),
        };
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
    /// The context arrived from the wire and was only *deserialized*; nothing had checked
    /// that the keyspace exists or that it is a raw keyspace. Encoding
    /// `ctx.keyspace` straight into a physical key means a client can name a keyspace that
    /// was never created, or write raw bytes into a `txn` keyspace where Percolator
    /// expects its own lock/write structure — neither of which would error anywhere.
    ///
    /// Every raw entry point goes through here: point, batch and range alike. A gate that
    /// only some callers use is not a gate.
    fn validated_context(&self, ctx: &RequestContext) -> Result<()> {
        let keyspaces = self.node.list_keyspaces("raw")?;
        let keyspace = keyspaces
            .iter()
            .find(|candidate| candidate.id == ctx.keyspace)
            .ok_or(Error::KeyspaceNotFound(ctx.keyspace))?;
        if keyspace.api_type != ApiType::Raw {
            return Err(Error::ApiTypeMismatch {
                keyspace: ctx.keyspace,
            });
        }
        Ok(())
    }

    /// A read view over applied state, refused unless this node currently leads.
    ///
    /// Not linearizable: `check_quorum` bounds how long a deposed leader keeps believing
    /// it leads, but within that window this returns stale data. See `LeaderRead`.
    fn leader_read(&self) -> Result<(Box<dyn ReadView + '_>, Option<u64>, bool)> {
        let status = self.driver.status();
        let is_leader = status.role == Role::Leader;
        let hint = status.leader_id.map(|id| id.0);
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
        self.validated_context(ctx)?;
        let (view, hint, is_leader) = self.leader_read()?;
        let read = LeaderRead::new(view.as_ref(), is_leader, hint)?;
        RawExecutor.get(&read, ctx.keyspace, key)
    }

    fn raw_batch_get(&self, ctx: &RequestContext, keys: &[UserKey]) -> Result<Vec<Option<Value>>> {
        self.validated_context(ctx)?;
        let (view, hint, is_leader) = self.leader_read()?;
        let read = LeaderRead::new(view.as_ref(), is_leader, hint)?;
        RawExecutor.batch_get(&read, ctx.keyspace, keys)
    }

    fn raw_put(
        &self,
        ctx: &RequestContext,
        key: UserKey,
        value: Value,
    ) -> Result<AppliedPosition> {
        self.validated_context(ctx)?;
        let plan = RawExecutor.plan_put(ctx.keyspace, &key, value, RawWriteOptions::default())?;
        self.commit_batch(plan)
    }

    fn raw_batch_put(
        &self,
        ctx: &RequestContext,
        pairs: &[(UserKey, Value)],
    ) -> Result<AppliedPosition> {
        self.validated_context(ctx)?;
        // One batch ⇒ one entry ⇒ all of these land together or none do.
        let plan = RawExecutor.plan_batch_put(ctx.keyspace, pairs, RawWriteOptions::default())?;
        self.commit_batch(plan)
    }

    fn raw_delete(&self, ctx: &RequestContext, key: &[u8]) -> Result<AppliedPosition> {
        self.validated_context(ctx)?;
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
        self.validated_context(ctx)?;
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
        self.validated_context(ctx)?;
        // One chunk in memory at a time: read a bounded chunk, commit it, resume strictly
        // after the last key it covered. Planning the whole range up front bounded the
        // raft *entry* while leaving the planner unbounded.
        run_delete_range(
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
        self.node.kv_get(ctx, key, ts)
    }
    fn kv_batch_get(
        &self,
        ctx: &RequestContext,
        keys: &[UserKey],
        ts: TimeStamp,
    ) -> Result<Vec<Option<Value>>> {
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
        self.node.kv_scan(ctx, start, end, limit, ts)
    }
    fn kv_prewrite(
        &self,
        ctx: &RequestContext,
        mutations: &[(UserKey, Option<Value>)],
        primary: &[u8],
        ts: TimeStamp,
    ) -> Result<()> {
        self.node.kv_prewrite(ctx, mutations, primary, ts)
    }
    fn kv_commit(
        &self,
        ctx: &RequestContext,
        keys: &[UserKey],
        start_ts: TimeStamp,
        commit_ts: TimeStamp,
    ) -> Result<()> {
        self.node.kv_commit(ctx, keys, start_ts, commit_ts)
    }
    fn kv_pessimistic_lock(
        &self,
        ctx: &RequestContext,
        keys: &[UserKey],
        ts: TimeStamp,
    ) -> Result<()> {
        self.node.kv_pessimistic_lock(ctx, keys, ts)
    }
    fn kv_pessimistic_rollback(
        &self,
        ctx: &RequestContext,
        keys: &[UserKey],
        ts: TimeStamp,
    ) -> Result<()> {
        self.node.kv_pessimistic_rollback(ctx, keys, ts)
    }
    fn kv_resolve_lock(
        &self,
        ctx: &RequestContext,
        start_ts: TimeStamp,
        commit_ts: Option<TimeStamp>,
    ) -> Result<()> {
        self.node.kv_resolve_lock(ctx, start_ts, commit_ts)
    }
    fn kv_cleanup(&self, ctx: &RequestContext, key: &[u8], ts: TimeStamp) -> Result<()> {
        self.node.kv_cleanup(ctx, key, ts)
    }
    fn kv_check_txn_status(
        &self,
        ctx: &RequestContext,
        primary: &[u8],
        lock_ts: TimeStamp,
    ) -> Result<()> {
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
    data_dir: PathBuf,
    status_path: PathBuf,
    addr: std::net::SocketAddr,
    // NOTE: no voter_fp field. The fingerprint lives ONLY in the FSM's
    // pre-initialization states (full-path structural retirement, task #24):
    // after initialization there is no runtime field left to misread as
    // identity. The discovery ANSWER side keeps its copy solely to serve
    // pre-init peers, and zeroes it once initialized.
    campaign_started: bool,
    initial_proposal: Option<(ProposedAt, kv9_common::ClusterId)>,
    registration_receipt: Option<RegistrationReceipt>,
    next_discovery: Instant,
}

impl NodeRuntime {
    /// Assemble and start the shared gRPC listener + Raft pump. Bootstrap advances in
    /// [`Self::run`], after every process is already able to answer discovery.
    pub fn start(id: NodeId, config: Config, auth: RuntimeAuth) -> Result<Self> {
        Self::start_with_cluster(id, config, auth, None)
    }

    /// Start either an initial declared voter (`expected_cluster_id=None`) or a
    /// join-existing node (`Some(id)`, with self absent from `config.join`).
    pub fn start_with_cluster(
        id: NodeId,
        config: Config,
        auth: RuntimeAuth,
        expected_cluster_id: Option<ClusterId>,
    ) -> Result<Self> {
        config.validate()?;
        auth.validate()?;
        let addr = config.addr.parse().map_err(|_| {
            Error::Config(format!(
                "addr must be a numeric socket address: {}",
                config.addr
            ))
        })?;
        let seeds = if config.join.is_empty() {
            vec![SeedPeer { node_id: id, addr }]
        } else {
            config.join.clone()
        };
        let own = seeds.iter().find(|seed| seed.node_id == id);
        let joining = match (own, expected_cluster_id) {
            (Some(seed), None) => {
                if seed.addr != addr {
                    return Err(Error::Config(format!(
                        "seed voter set declares node {} at {}, but addr is {}",
                        id.0, seed.addr, addr
                    )));
                }
                false
            }
            (None, Some(_)) => true,
            (None, None) => {
                return Err(Error::Config(format!(
                "node {} is absent from the seed voter set; join-existing mode requires cluster id",
                id.0
            )))
            }
            (Some(_), Some(_)) => {
                return Err(Error::Config(
                    "cluster id is only valid when this node is absent from the seed voter set"
                        .into(),
                ))
            }
        };

        let data_dir = PathBuf::from(&config.data_dir);
        fs::create_dir_all(&data_dir)
            .map_err(|e| Error::Config(format!("create {}: {e}", data_dir.display())))?;
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
        if local_identity.is_none() && catalog_initialized(&node)? {
            return Err(Error::MetaNotReady(
                "catalog has schema but no cluster identity; refusing to treat \
                 this data-dir as initialized (corrupt or pre-identity catalog)"
                    .into(),
            ));
        }
        let marker_initialized = init_marker_exists(&data_dir);
        let mut bootstrap = if joining {
            Bootstrap::join_existing_at(
                id,
                voters.clone(),
                expected_cluster_id.expect("joining checked above"),
                voter_fp,
                &data_dir,
            )?
        } else {
            Bootstrap::with_seeds_fp(id, voters.clone(), voter_fp)
        };
        if init_marker_exists(&data_dir) {
            bootstrap.mark_data_dir_initialized();
        }
        // A non-pristine Raft member must never form a second cluster, even if
        // it crashed before the marker rename. It rejoins and waits for catalog.
        if !was_pristine {
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
            data_dir,
            status_path,
            addr,
            campaign_started: false,
            initial_proposal: None,
            registration_receipt: None,
            next_discovery: Instant::now(),
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
            self.sync_registered_peers()?;
            self.advance_bootstrap()?;
            self.write_status()?;
            std::thread::sleep(TICK);
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
            if let Ok(answer) = grpc_discover(
                self.grpc_runtime.handle(),
                self.node.id,
                seed.addr,
                DISCOVERY_TIMEOUT,
                Some(self.cluster_token.clone()),
            ) {
                // Both the address→identity mapping and (pre-init) the
                // complete declared voter set must match. A valid answer
                // about another cluster is still not a vote in this cluster.
                if !discovery_answer_matches(*seed, bootstrap_fp, &answer) {
                    continue;
                }
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
            return Err(Error::MetaNotReady(
                "bootstrap initializer lost leadership before catalog commit".into(),
            ));
        }
        if self.initial_proposal.is_none() {
            // Mint the cluster identity ONCE, right before proposing: it rides
            // in the same committed transaction as the seed rows, so identity
            // is exactly as crash-safe as the rest of the bootstrap (a crashed
            // initializer re-elects and re-mints — the old proposal either
            // committed, in which case initialize_cluster refuses the second
            // mint, or it never did and the new id is THE id).
            let cluster_id = kv9_common::ClusterId::mint()?;
            let cmd = self
                .node
                .build_initial_metadata_command_for(&self.voters, cluster_id)?;
            self.initial_proposal = Some((self.driver.propose(&cmd)?, cluster_id));
            return Ok(());
        }
        let (proposal, cluster_id) = self.initial_proposal.expect("set above");
        match self.driver.wait_applied(proposal, Duration::from_millis(1)) {
            Ok(true) => {
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
                None => return Ok(()),
            }
        };
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
            for seed in &self.seeds {
                match grpc_register(
                    self.grpc_runtime.handle(),
                    self.node.id,
                    seed.addr,
                    &self.addr.to_string(),
                    cluster_id,
                    DISCOVERY_TIMEOUT,
                    Some(self.cluster_token.clone()),
                ) {
                    Ok(RegisterOutcome::Registered(receipt)) => {
                        self.registration_receipt = Some(receipt);
                        break;
                    }
                    Ok(RegisterOutcome::NotLeader { .. }) | Err(_) => continue,
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
        let body = format!(
            "pid={}\nnode_id={}\ncluster_id={}\nleader_id={}\nrole={}\nmeta_voters={}\nmeta_learners={}\npending_admissions={}\nconf_index={}\nterm={}\nraft_committed={}\napplied_index={}\napplied_term={}\nbootstrap_state={:?}\nfatal={}\n",
            std::process::id(),
            raft.node_id.0,
            cluster_id.map_or_else(String::new, |id| id.to_string()),
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
            bootstrap,
            raft.fatal.as_deref().unwrap_or(""),
        );
        let tmp = self.data_dir.join("status.tmp");
        fs::write(&tmp, body)
            .and_then(|_| fs::rename(&tmp, &self.status_path))
            .map_err(|e| Error::Config(format!("write {}: {e}", self.status_path.display())))
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

fn discovery_answer_matches(
    declared: SeedPeer,
    expected_voter_fp: u64,
    answer: &kv9_raft::grpc::DiscoverAnswer,
) -> bool {
    if answer.node != declared.node_id {
        return false;
    }
    if answer.initialized {
        // Post-init authority is the ClusterId (decode guarantees it is
        // present on an initialized answer); the fingerprint has retired and
        // responders publish 0 — comparing it here would re-animate it.
        // Wrong-cluster protection: initial-bootstrap voters only adopt an
        // identity from their OWN catalog; join-existing verifies the id
        // against its expectation inside the FSM.
        true
    } else {
        answer.voter_fingerprint == expected_voter_fp
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
    use kv9_engine::{ColumnFamily, Engine};
    use kv9_raft::transport::{InProcHub, RaftTransport};
    use kv9_raft::Command;
    use tonic::Code;

    /// Plans `total` one-key chunks, then stops.
    fn planner(total: usize) -> impl FnMut(Option<&[u8]>) -> Result<Option<(kv9_engine::WriteBatch, UserKey)>>
    {
        let mut issued = 0usize;
        move |_cursor| {
            if issued >= total {
                return Ok(None);
            }
            issued += 1;
            let mut batch = kv9_engine::WriteBatch::new();
            batch.delete(ColumnFamily::Default, vec![b'k', issued as u8]);
            Ok(Some((batch, vec![b'k', issued as u8])))
        }
    }

    /// Failing at chunk N must report exactly what committed — the position of chunk N-1,
    /// not zeros and not the position it was attempting.
    #[test]
    fn a_failure_at_the_second_chunk_reports_the_first_chunk_as_committed() {
        let mut attempts = 0u64;
        let result = run_delete_range(planner(4), |_batch| {
            attempts += 1;
            if attempts == 2 {
                return Err(Error::Engine("injected disk failure".into()));
            }
            Ok(AppliedPosition {
                term: 7,
                index: 100 + attempts,
            })
        });

        match result {
            Err(Error::PartialDeleteRange {
                committed_chunks,
                last_applied_term,
                last_applied_index,
                cause,
            }) => {
                assert_eq!(committed_chunks, 1, "exactly one chunk committed");
                assert_eq!(last_applied_term, 7);
                assert_eq!(
                    last_applied_index, 101,
                    "the position of the chunk that DID commit, not the one that failed"
                );
                assert!(cause.contains("injected"), "the cause survives for humans");
            }
            other => panic!("expected a partial receipt, got {:?}", other.is_ok()),
        }
    }

    /// Failing before anything commits is not partial — claiming otherwise would be a lie
    /// in the opposite direction, telling a caller data is gone when none is.
    #[test]
    fn a_failure_on_the_very_first_chunk_is_not_reported_as_partial() {
        let result = run_delete_range(planner(4), |_batch| {
            Err(Error::Engine("injected disk failure".into()))
        });
        assert!(
            matches!(result, Err(Error::Engine(_))),
            "the original error must survive untouched"
        );
    }

    /// A range needing no chunks is a successful no-op, not an error.
    #[test]
    fn an_empty_range_succeeds_with_a_zero_receipt() {
        let receipt = run_delete_range(planner(0), |_batch| {
            panic!("nothing should be committed")
        })
        .unwrap();
        assert_eq!(receipt.committed_chunks, 0);
        assert_eq!(receipt.last_applied, None);
    }

    #[test]
    fn every_chunk_committing_reports_the_last_position() {
        let mut attempts = 0u64;
        let receipt = run_delete_range(planner(3), |_batch| {
            attempts += 1;
            Ok(AppliedPosition {
                term: 2,
                index: 50 + attempts,
            })
        })
        .unwrap();
        assert_eq!(receipt.committed_chunks, 3);
        assert_eq!(
            receipt.last_applied,
            Some(AppliedPosition { term: 2, index: 53 })
        );
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
        };
        assert!(discovery_answer_matches(
            declared,
            ours,
            &ans(3, false, ours)
        ));
        assert!(!discovery_answer_matches(
            declared,
            ours,
            &ans(9, false, ours)
        ));
        assert!(!discovery_answer_matches(
            declared,
            ours,
            &ans(3, false, 0x9999)
        ));
        // Post-init: identity travels as the ClusterId; the retired
        // fingerprint (responders publish 0) must NOT gate the answer…
        assert!(discovery_answer_matches(declared, ours, &ans(3, true, 0)));
        // …but the declared node identity still must match.
        assert!(!discovery_answer_matches(declared, ours, &ans(9, true, 0)));
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
