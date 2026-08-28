//! Real Phase-1 metadata-node runtime.
//!
//! This is the process boundary missing from the earlier deterministic harness:
//! fixed seed identities, real TCP discovery/Raft traffic, durable Raft state,
//! durable catalog apply, election-first bootstrap, and a machine-readable status
//! file for external acceptance. The status file is evidence; log timing is not.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use kv9_common::{
    ApiType, Config, Error, KeyspaceId, NodeId, RegionId, Result, SeedPeer, TenantId, TimeStamp,
    TxnGroupId, UserKey, Value, META_REGION_0,
};
use kv9_engine::WalEngine;
use kv9_meta::bootstrap::{init_marker_exists, write_init_marker};
use kv9_meta::codec::memcmp_uint;
use kv9_meta::schema::SCHEMA_VERSION_DESC;
use kv9_meta::{Bootstrap, BootstrapEvent, BootstrapState};
use kv9_raft::driver::NodeDriver;
use kv9_raft::grpc::{
    grpc_discover, pb::kv9_raft_server::Kv9RaftServer, GrpcDiscoveryState, GrpcTransport,
    RaftGrpcService, CLUSTER_TOKEN_KEY, NODE_ID_KEY,
};
use kv9_raft::storage::DiskRaftStorage;
use kv9_raft::transport::voter_set_fingerprint;
use kv9_raft::{MemStateMachine, ProposedAt, RaftGroup, RaftPeer, Role};
use tonic::metadata::MetadataMap;
use tonic::Status;

use crate::api::{
    AdminApi, AppliedPosition, ClusterInfo, CreateKeyspaceResult, RawApi, RegionLocation,
    RequestContext, TxnApi,
};
use crate::grpc::{
    AuthContext, AuthInterceptor, AuthKind, Authenticator, Kv9Grpc, TokenAuthenticator,
};
use crate::Node;

const TICK: Duration = Duration::from_millis(20);
const DISCOVERY_INTERVAL: Duration = Duration::from_millis(200);
const DISCOVERY_TIMEOUT: Duration = Duration::from_millis(50);

#[derive(Debug)]
struct RuntimeDiscovery {
    node: NodeId,
    initialized: AtomicBool,
    voter_fp: u64,
}

impl RuntimeDiscovery {
    fn new(node: NodeId, initialized: bool, voter_fp: u64) -> Self {
        Self {
            node,
            initialized: AtomicBool::new(initialized),
            voter_fp,
        }
    }

    fn set_initialized(&self) {
        self.initialized.store(true, Ordering::Release);
    }
}

impl GrpcDiscoveryState for RuntimeDiscovery {
    fn answer(&self) -> (NodeId, bool, u64) {
        (
            self.node,
            self.initialized.load(Ordering::Acquire),
            self.voter_fp,
        )
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
        if !self.voters.contains(&node_id) {
            return Err(Status::permission_denied(
                "declared node is not in the fixed voter set",
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
}

impl RawApi for RuntimeBackend {
    fn raw_get(&self, ctx: &RequestContext, key: &[u8]) -> Result<Option<Value>> {
        self.node.raw_get(ctx, key)
    }
    fn raw_batch_get(&self, ctx: &RequestContext, keys: &[UserKey]) -> Result<Vec<Option<Value>>> {
        self.node.raw_batch_get(ctx, keys)
    }
    fn raw_put(&self, ctx: &RequestContext, key: UserKey, value: Value) -> Result<()> {
        self.node.raw_put(ctx, key, value)
    }
    fn raw_batch_put(&self, ctx: &RequestContext, pairs: &[(UserKey, Value)]) -> Result<()> {
        self.node.raw_batch_put(ctx, pairs)
    }
    fn raw_delete(&self, ctx: &RequestContext, key: &[u8]) -> Result<()> {
        self.node.raw_delete(ctx, key)
    }
    fn raw_scan(
        &self,
        ctx: &RequestContext,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<(UserKey, Value)>> {
        self.node.raw_scan(ctx, start, end, limit)
    }
    fn raw_delete_range(&self, ctx: &RequestContext, start: &[u8], end: &[u8]) -> Result<()> {
        self.node.raw_delete_range(ctx, start, end)
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
    voter_fp: u64,
    campaign_started: bool,
    initial_proposal: Option<(ProposedAt, kv9_common::ClusterId)>,
    next_discovery: Instant,
}

impl NodeRuntime {
    /// Assemble and start the shared gRPC listener + Raft pump. Bootstrap advances in
    /// [`Self::run`], after every process is already able to answer discovery.
    pub fn start(id: NodeId, config: Config, auth: RuntimeAuth) -> Result<Self> {
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
        let own = seeds
            .iter()
            .find(|seed| seed.node_id == id)
            .ok_or_else(|| {
                Error::Config(format!(
                    "fixed seed voter set does not include node {}",
                    id.0
                ))
            })?;
        if own.addr != addr {
            return Err(Error::Config(format!(
                "seed voter set declares node {} at {}, but addr is {}",
                id.0, own.addr, addr
            )));
        }

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

        let catalog_initialized = catalog_initialized(&node)?;
        let marker_initialized = init_marker_exists(&data_dir);
        let mut bootstrap = Bootstrap::with_seeds_at(id, voters.clone(), &data_dir);
        // A non-pristine Raft member must never form a second cluster, even if
        // it crashed before the marker rename. It rejoins and waits for catalog.
        if !was_pristine {
            bootstrap.mark_data_dir_initialized();
        }
        if catalog_initialized && !marker_initialized {
            write_init_marker(&data_dir)?;
            bootstrap.mark_data_dir_initialized();
        }
        node.meta.lock().expect("meta poisoned").bootstrap = bootstrap;

        let discovery = Arc::new(RuntimeDiscovery::new(
            id,
            marker_initialized || catalog_initialized,
            voter_fp,
        ));
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
        });
        let client_authenticator = Arc::new(TokenAuthenticator::new(auth.client_tokens)?);
        let public_service = Kv9Grpc::new(backend).authenticated_service(client_authenticator);
        let cluster_authenticator = Arc::new(ClusterAuthenticator {
            expected_token: Arc::from(auth.cluster_token.clone()),
            voters: Arc::new(voters.iter().copied().collect()),
        });
        let raft_service = RaftGrpcService::new(id, transport.inbox_sender(), discovery.clone());
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
            voter_fp,
            campaign_started: false,
            initial_proposal: None,
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
            self.advance_bootstrap()?;
            self.write_status()?;
            std::thread::sleep(TICK);
        }
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

        let mut uninitialized = vec![self.node.id];
        let mut found_initialized = false;
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
                // Both the address→identity mapping and the complete declared
                // voter set must match. A valid answer about another cluster is
                // still not a vote in this cluster.
                if !discovery_answer_matches(*seed, self.voter_fp, answer) {
                    continue;
                }
                let (answer_id, initialized, _) = answer;
                if initialized {
                    found_initialized = true;
                } else {
                    uninitialized.push(answer_id);
                }
            }
        }
        if found_initialized {
            // A peer says the cluster exists. The identity still comes from
            // the local catalog once raft replay delivers it (this runtime
            // path only runs for declared voters, which replicate without
            // admission; a NON-voter joiner learns the id from the discovery
            // answer itself — that extension lands with the registration
            // seam, msg-flagged for @Tess).
            return self.try_found_initialized();
        }
        let mut meta = self.node.meta.lock().expect("meta poisoned");
        // Insufficient evidence is expected while peers start; silence never
        // changes the voter denominator and never becomes an answer.
        let _ = meta.bootstrap.discovered_uninitialized(&uninitialized);
        Ok(())
    }

    /// Fire the join transition once the LOCAL catalog can name the cluster —
    /// "initialized somewhere" without an identity is not enough to leave
    /// Discovering (declared voters replicate the init entries without
    /// admission; the non-voter path learns the id from the discovery answer
    /// itself when the registration seam lands).
    fn try_found_initialized(&mut self) -> Result<()> {
        let local = self.node.local_cluster_identity()?;
        let mut meta = self.node.meta.lock().expect("meta poisoned");
        meta.bootstrap.observe_local_identity(local)?;
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
                self.discovery.set_initialized();
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
        self.discovery.set_initialized();
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
        let body = format!(
            "pid={}\nnode_id={}\nleader_id={}\nrole={}\nmeta_voters={}\nmeta_learners={}\nconf_index={}\nterm={}\nraft_committed={}\napplied_index={}\napplied_term={}\nbootstrap_state={:?}\nfatal={}\n",
            std::process::id(),
            raft.node_id.0,
            raft.leader_id.map_or(0, |id| id.0),
            role,
            meta_voters,
            meta_learners,
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
    answer: (NodeId, bool, u64),
) -> bool {
    answer.0 == declared.node_id && answer.2 == expected_voter_fp
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

    #[test]
    fn cluster_auth_requires_token_and_declared_voter_identity() {
        let authenticator = ClusterAuthenticator {
            expected_token: Arc::from("secret"),
            voters: Arc::new([NodeId(1), NodeId(2)].into_iter().collect()),
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
    }

    #[test]
    fn discovery_vote_requires_both_declared_identity_and_voter_set() {
        let declared = SeedPeer {
            node_id: NodeId(3),
            addr: "127.0.0.1:20163".parse().unwrap(),
        };
        let ours = 0x1111;
        assert!(discovery_answer_matches(
            declared,
            ours,
            (NodeId(3), false, ours)
        ));
        assert!(!discovery_answer_matches(
            declared,
            ours,
            (NodeId(9), false, ours)
        ));
        assert!(!discovery_answer_matches(
            declared,
            ours,
            (NodeId(3), false, 0x9999)
        ));
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
