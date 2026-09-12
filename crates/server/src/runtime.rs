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
    RootDescriptor, RootDigest, SeedPeer, StoreIdentity, StoreIncarnation, TenantId, TxnGroupId,
    UserKey, Value, META_REGION_0,
};
use kv9_engine::{Engine, ReadView, ReplicatedEngine, WalEngine};
use kv9_meta::admission::INVALID_JOIN_TICKET_MESSAGE;
use kv9_meta::bootstrap::{init_marker_exists, write_init_marker};
use kv9_meta::codec::memcmp_uint;
use kv9_meta::schema::{ColumnId, NODES_DESC, SCHEMA_VERSION_DESC};
use kv9_meta::tables::Tables;
use kv9_meta::{Bootstrap, BootstrapEvent, BootstrapState};
use kv9_meta::{ColumnValue, RowValue};
use kv9_raft::driver::{
    ApplyWaitError, ApplyWaitOutcome, DriverAppliedPosition, NodeDriver, ReadBarrier,
};
use kv9_raft::grpc::{
    grpc_discover, grpc_register, pb::kv9_raft_server::Kv9RaftServer, DiscoveryError,
    GrpcDiscoveryState, GrpcTransport, JoinIdentity, LeaderHint, RaftGrpcService, RegisterError,
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

use crate::fence::CatalogFenceAdjudicator;
use crate::Node;

mod endpoint_recovery;
use endpoint_recovery::EndpointRecovery;

const TICK: Duration = Duration::from_millis(20);
const DISCOVERY_INTERVAL: Duration = Duration::from_millis(200);
const DISCOVERY_TIMEOUT: Duration = Duration::from_millis(50);
// Registration can require several durable consensus steps. Keep its one
// absolute pass budget separate from the short discovery health probe.
const REGISTRATION_PASS_TIMEOUT: Duration = Duration::from_secs(5);
const DISCOVERY_LAST_OUTCOME_MAX_CHARS: usize = 160;
const DISCOVERY_ERROR_PREFIX: &str = "error:";

fn grpc_incoming(listener: tokio::net::TcpListener) -> tonic::transport::server::TcpIncoming {
    // Tonic ignores Server::tcp_nodelay for caller-supplied incoming streams.
    // Configure accepted sockets while retaining this already-owned listener.
    // This covers both public replies and node-internal Raft/discovery traffic.
    tonic::transport::server::TcpIncoming::from(listener).with_nodelay(Some(true))
}

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
    RejectedInvalidIncarnation,
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
            Self::RejectedInvalidIncarnation => "rejected_invalid_incarnation",
            Self::ConnectFailed => "connect_failed",
            Self::Timeout => "timeout",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RegistrationObservation {
    attempts: u64,
    errors: u64,
    last: RegistrationLastOutcome,
    /// The last NotLeader hint observed: leader id plus either the canonical
    /// endpoint it carried or the reason none was usable. Kept so a wedged
    /// registration scene shows WHERE the client was pointed, not just a
    /// count of not_leader answers (the 972-loop lesson: the scene had no
    /// record of what the hints said). Diagnostic text ONLY — machine state
    /// lives in `last_walk`, never in this string.
    last_hint: Option<String>,
    /// The last walk pass's typed terminal verdict; the status line renders
    /// this enum directly (no string parsing anywhere downstream).
    last_walk: Option<WalkTerminal>,
}

impl RegistrationObservation {
    fn new() -> Self {
        Self {
            attempts: 0,
            errors: 0,
            last: RegistrationLastOutcome::NotAttempted,
            last_hint: None,
            last_walk: None,
        }
    }

    fn record_attempt(&mut self) {
        self.attempts = self.attempts.saturating_add(1);
    }

    fn record_registered(&mut self) {
        self.last = RegistrationLastOutcome::Registered;
    }

    fn record_not_leader(&mut self, hint: &str) {
        self.errors = self.errors.saturating_add(1);
        self.last = RegistrationLastOutcome::NotLeader;
        self.last_hint = Some(hint.to_string());
    }

    fn record_error(&mut self, error: &RegisterError) {
        self.errors = self.errors.saturating_add(1);
        self.last = match error {
            RegisterError::InvalidTicket => RegistrationLastOutcome::RejectedInvalidTicket,
            RegisterError::InvalidIncarnation => {
                RegistrationLastOutcome::RejectedInvalidIncarnation
            }
            RegisterError::Connect(_) => RegistrationLastOutcome::ConnectFailed,
            RegisterError::Timeout => RegistrationLastOutcome::Timeout,
            RegisterError::Failed(_) => RegistrationLastOutcome::Failed,
        };
    }
}

/// The typed terminal verdict of ONE registration walk pass — every way a
/// pass can end, mutually exclusive. When more than one stop-cause was
/// observed in the same pass, the final verdict is chosen by a fixed
/// precedence (most-informative first, pinned by
/// `walk_terminal_precedence_is_fixed`):
///
///   `DeadlineExhausted` — decided the moment the budget hits zero, before
///       any further dial; nothing else can be concluded about candidates
///       that were never tried.
///   `HopCapExhausted` — live hints kept arriving but the hop budget was
///       consumed: the walk was TRUNCATED, which subsumes any cycle or
///       unresolved hint also seen.
///   `HintCycle` — the hint frontier closed on already-visited `(id, addr)`
///       pairs: the directory content was exhausted.
///   `UnresolvedHint` — a redirect was answered but carried no followable
///       endpoint (id-only, no id, or garbled — all fail-closed).
///   `ConnectFailed` / `Timeout` / `Failed` — no redirect was ever answered;
///       the verdict is the LAST dial's error class.
///   `NoCandidates` — the pass had an empty candidate set (config validation
///       upstream makes this unreachable in production).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WalkTerminal {
    Registered,
    RejectedInvalidTicket,
    RejectedInvalidIncarnation,
    DeadlineExhausted,
    HopCapExhausted,
    HintCycle,
    UnresolvedHint,
    ConnectFailed,
    Timeout,
    Failed,
    NoCandidates,
}

impl WalkTerminal {
    fn as_status(self) -> &'static str {
        match self {
            Self::Registered => "registered",
            Self::RejectedInvalidTicket => "rejected_invalid_ticket",
            Self::RejectedInvalidIncarnation => "rejected_invalid_incarnation",
            Self::DeadlineExhausted => "deadline_exhausted",
            Self::HopCapExhausted => "hop_cap_exhausted",
            Self::HintCycle => "hint_cycle",
            Self::UnresolvedHint => "unresolved_hint",
            Self::ConnectFailed => "connect_failed",
            Self::Timeout => "timeout",
            Self::Failed => "failed",
            Self::NoCandidates => "no_candidates",
        }
    }
}

/// What a registration walk pass returned to its caller. `InvalidTicket` is
/// separate from the other terminals because it is a PERMANENT credential
/// rejection: retrying the pass cannot succeed, while every `Unconfirmed`
/// reason is legitimately retried on the next pass.
#[derive(Debug)]
enum WalkOutcome {
    Registered {
        receipt: RegistrationReceipt,
        /// The exact candidate the receipt came from — the leader that
        /// committed this registration. Kept because the joiner needs a
        /// transport route to that leader BEFORE its catalog can name one
        /// (the transport half of the fresh-joiner catch-22: raft
        /// responses to an unknown peer are dropped, so catch-up never
        /// starts). Same pre-TLS trusted-network boundary as the receipt
        /// itself; the applied catalog overwrites it on first sync.
        via: (NodeId, std::net::SocketAddr),
    },
    InvalidTicket,
    InvalidIncarnation,
    Unconfirmed(WalkTerminal),
}

#[derive(Default)]
struct RegistrationSeedCursor {
    next: usize,
}

impl RegistrationSeedCursor {
    fn order(
        &mut self,
        seeds: &[(NodeId, std::net::SocketAddr)],
    ) -> Vec<(NodeId, std::net::SocketAddr)> {
        let mut ordered = seeds.to_vec();
        if !ordered.is_empty() {
            let start = self.next % ordered.len();
            self.next = if start + 1 == ordered.len() {
                0
            } else {
                start + 1
            };
            ordered.rotate_left(start);
        }
        ordered
    }
}

/// One registration pass (task: the 972-not_leader master red): a candidate
/// walk that starts at the declared seeds and FOLLOWS NotLeader hints that
/// carry a canonical endpoint, deduped by `(leader id, addr)` with a small
/// hop cap beyond the seeds.
///
/// The cap guards CYCLES and STALE CHAINS alike: a hinted endpoint is a
/// bounded routing candidate, not proof of the current leader — the answering
/// node resolves it from its own applied directory, which can lag (old
/// leader, superseded address). That is why the correct cap is SMALL, not
/// "big enough for any cycle".
///
/// The pass has exactly TWO budgets and neither ever resets within it:
/// `deadline` is the single absolute time window — every dial receives only
/// `deadline - now()` and a zero remainder ends the pass before the next
/// dial, so following hints can never enlarge the wall-clock cost of a pass;
/// the hop cap bounds how many hints are followed within that window. Every
/// way a pass can end is a typed [`WalkTerminal`] recorded in the
/// observation (rendered verbatim in status), never a silent `None`.
fn registration_walk(
    seeds: &[(NodeId, std::net::SocketAddr)],
    obs: &mut RegistrationObservation,
    deadline: std::time::Instant,
    mut now: impl FnMut() -> std::time::Instant,
    mut dial: impl FnMut(
        std::net::SocketAddr,
        Duration,
    ) -> std::result::Result<RegisterOutcome, RegisterError>,
) -> WalkOutcome {
    const HINT_HOP_CAP: usize = 4;
    fn end(obs: &mut RegistrationObservation, terminal: WalkTerminal) -> WalkOutcome {
        obs.last_walk = Some(terminal);
        match terminal {
            WalkTerminal::RejectedInvalidTicket => WalkOutcome::InvalidTicket,
            WalkTerminal::RejectedInvalidIncarnation => WalkOutcome::InvalidIncarnation,
            other => WalkOutcome::Unconfirmed(other),
        }
    }
    let mut queue: Vec<(NodeId, std::net::SocketAddr)> = seeds.to_vec();
    let mut visited: std::collections::HashSet<(NodeId, std::net::SocketAddr)> =
        queue.iter().copied().collect();
    let mut hint_hops = 0usize;
    let mut saw_cap = false;
    let mut saw_cycle = false;
    let mut saw_unresolved = false;
    let mut last_error: Option<WalkTerminal> = None;
    let mut i = 0usize;
    while i < queue.len() {
        // The ONE absolute budget: every dial gets only what is left of the
        // pass's original window; zero remaining means no further dial, no
        // matter what the frontier still holds.
        let remaining = deadline.saturating_duration_since(now());
        if remaining.is_zero() {
            return end(obs, WalkTerminal::DeadlineExhausted);
        }
        let (cand_id, cand_addr) = queue[i];
        i += 1;
        obs.record_attempt();
        match dial(cand_addr, remaining) {
            Ok(RegisterOutcome::Registered(receipt)) => {
                obs.record_registered();
                obs.last_walk = Some(WalkTerminal::Registered);
                return WalkOutcome::Registered {
                    receipt,
                    via: (cand_id, cand_addr),
                };
            }
            Ok(RegisterOutcome::NotLeader { leader }) => {
                let hint = match &leader {
                    Some(h) => match h.addr {
                        Some(addr) => format!("leader={} addr={addr}", h.id.0),
                        None => format!("leader={} addr=unresolved", h.id.0),
                    },
                    None => "leader=unknown".to_string(),
                };
                obs.record_not_leader(&hint);
                match leader {
                    Some(LeaderHint {
                        id,
                        addr: Some(addr),
                    }) => {
                        if !visited.insert((id, addr)) {
                            saw_cycle = true;
                        } else if hint_hops >= HINT_HOP_CAP {
                            saw_cap = true;
                        } else {
                            hint_hops += 1;
                            // Follow the new routing hint before unrelated
                            // seeds can consume the rest of this pass's budget.
                            queue.insert(i, (id, addr));
                        }
                    }
                    // Id-only or no id at all: nothing followable (the wire
                    // boundary already made addr-without-id unrepresentable).
                    _ => saw_unresolved = true,
                }
            }
            Err(error) => {
                obs.record_error(&error);
                match error {
                    RegisterError::InvalidTicket => {
                        return end(obs, WalkTerminal::RejectedInvalidTicket)
                    }
                    RegisterError::InvalidIncarnation => {
                        return end(obs, WalkTerminal::RejectedInvalidIncarnation)
                    }
                    RegisterError::Connect(_) => last_error = Some(WalkTerminal::ConnectFailed),
                    RegisterError::Timeout => last_error = Some(WalkTerminal::Timeout),
                    RegisterError::Failed(_) => last_error = Some(WalkTerminal::Failed),
                }
            }
        }
    }
    // Frontier exhausted: pick the verdict by the documented precedence.
    let terminal = if saw_cap {
        WalkTerminal::HopCapExhausted
    } else if saw_cycle {
        WalkTerminal::HintCycle
    } else if saw_unresolved {
        WalkTerminal::UnresolvedHint
    } else if let Some(error) = last_error {
        error
    } else {
        WalkTerminal::NoCandidates
    };
    end(obs, terminal)
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
    /// Incarnation-scoped, monotonic permission. A new component starts closed.
    raft_receive: AtomicBool,
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
            raft_receive: AtomicBool::new(false),
            voter_fp: Mutex::new(if initialized { None } else { Some(voter_fp) }),
            cluster_id: Mutex::new(None),
        }
    }

    fn authorize_raft(&self) {
        self.raft_receive.store(true, Ordering::Release);
    }

    fn set_cluster_id(&self, id: kv9_common::ClusterId) {
        *self.cluster_id.lock().expect("cluster id poisoned") = Some(id);
        // Retirement moment: the fingerprint ceases to exist here.
        self.voter_fp.lock().expect("fp poisoned").take();
        self.initialized.store(true, Ordering::Release);
    }
}

impl GrpcDiscoveryState for RuntimeDiscovery {
    fn raft_receive_allowed(&self) -> bool {
        self.raft_receive.load(Ordering::Acquire)
    }

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

/// Self-expiring catch-up capability (interface ruling, review thread on the
/// registration fix): after this node's OWN registration returns a typed
/// `Registered(receipt)`, inbound raft traffic from the receipt's
/// `voters ∪ learners` is admitted WHILE this replica's unified
/// driver-applied watermark has not yet reached the receipt's exact applied
/// position. This is the window a fresh joiner needs to receive the very
/// stream that fills its catalog; without it a joiner can never sync from a
/// dynamic-member leader (catch-22: the authenticator wants catalog rows
/// that only arrive over the stream it is rejecting).
///
/// THIS IS NOT A SECURITY MECHANISM. It is a liveness fix that is sound
/// only under the project's explicit pre-TLS trusted-network threat model:
/// the receipt's member list is NOT end-to-end authenticated (plaintext
/// HTTP; the server never proves anything to the joiner), and the cluster
/// token in those same requests is equally readable by any on-path
/// observer — so this window's safety is attributed to the stated network
/// assumption, deliberately NOT to any mechanism here. The list must never
/// be described as certified membership and must never be reused for
/// routing, membership views, persistent identity, or any authorization
/// outside this window. TLS remains the hard gate for any cross-machine
/// deployment; this capability is not an exemption.
///
/// Lifecycle: installed ONLY on the typed `Registered` outcome (never on
/// `InvalidTicket` or any `Unconfirmed` reason); memory-only — a restart
/// that is still behind re-runs idempotent registration and mints a fresh
/// receipt, this value is never persisted into a second authority; judged
/// per request against the LIVE watermark via [`Self::admits`], so the
/// instant the barrier is reached the fallback is dead globally — no
/// tick-delayed cleanup tail.
///
/// The barrier position is the receipt's exact applied `(term, index)` —
/// the final registration `CatalogTxn` COMMAND, already applied on the
/// leader when the receipt was minted. The production reader hands
/// [`Self::admits`] the unified driver watermark (noops + commands + conf
/// changes — the log-position ruler): by contiguity, reaching the receipt
/// index there implies the receipt command's prefix, including every
/// membership row this window was standing in for, is applied locally.
#[derive(Debug, Clone)]
struct CatchupCapability {
    members: HashSet<NodeId>,
    receipt_position: DriverAppliedPosition,
}

impl CatchupCapability {
    fn from_receipt(receipt: &RegistrationReceipt) -> Self {
        Self {
            members: receipt
                .voters
                .iter()
                .chain(receipt.learners.iter())
                .map(|id| NodeId(*id))
                .collect(),
            receipt_position: DriverAppliedPosition {
                term: receipt.applied_term,
                index: receipt.applied_index,
            },
        }
    }

    /// The window decision, in one place: `true` only while the local
    /// watermark has NOT reached the receipt barrier AND the sender is in
    /// the receipt's member set. Once `watermark.index >= receipt.index`
    /// the answer is `false` forever, even with the capability still in
    /// memory — after the barrier only the catalog speaks.
    fn admits(&self, sender: NodeId, watermark: Option<DriverAppliedPosition>) -> bool {
        let barrier_reached = watermark.is_some_and(|wm| wm.index >= self.receipt_position.index);
        !barrier_reached && self.members.contains(&sender)
    }
}

struct ClusterAuthenticator<
    S: kv9_raft::rawnode::PersistentRaftStorage,
    E: kv9_engine::ReplicatedEngine,
> {
    expected_token: Arc<str>,
    voters: Arc<HashSet<NodeId>>,
    node: Arc<Node<E>>,
    /// For the LIVE unified-watermark read in [`CatchupCapability::admits`]
    /// — the barrier is judged on every request, never on cached state.
    driver: Arc<NodeDriver<S, E>>,
    /// Shared slot with the runtime: `advance_registration` installs the
    /// capability here on the typed `Registered` outcome and nowhere else.
    catchup: Arc<std::sync::Mutex<Option<CatchupCapability>>>,
}

impl<S: kv9_raft::rawnode::PersistentRaftStorage, E: kv9_engine::ReplicatedEngine + 'static>
    Authenticator for ClusterAuthenticator<S, E>
{
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
            // Every catalog failure below rejects BEFORE the catch-up
            // window is ever consulted: a failed read is a failed read,
            // never "not caught up yet" (fail-closed; the `?`s precede the
            // fallback structurally).
            let txn = self
                .node
                .meta_raft
                .store
                .begin()
                .map_err(|_| Status::unavailable("membership catalog unavailable"))?;
            let admission = kv9_meta::admission::admission(&txn, node_id)
                .map_err(|_| Status::unavailable("membership catalog unavailable"))?;
            let revoked = admission.as_ref().is_some_and(|admission| {
                admission.state == kv9_meta::admission::AdmissionState::Revoked
            });
            let admitted = admission.as_ref().is_some_and(|admission| {
                matches!(
                    admission.state,
                    kv9_meta::admission::AdmissionState::Pending
                        | kv9_meta::admission::AdmissionState::Consumed
                )
            });
            let registered = txn
                .get(&NODES_DESC, &[memcmp_uint(node_id.0)])
                .map_err(|_| Status::unavailable("membership catalog unavailable"))?
                .is_some();
            if revoked {
                // The explicit applied verdict comes FIRST, before every
                // positive membership arm: revoking the admission of an
                // already-registered node is the decommission shape, and
                // its still-present membership row must not outvote the
                // revocation (with `registered` checked first, this arm is
                // dead code for exactly the nodes it exists for). Also
                // never eligible for the catch-up window — the window
                // exists for ABSENCE, not for overriding decisions this
                // replica has already applied.
                false
            } else if admitted || registered {
                true
            } else if admission.is_some() {
                // A superseded credential without its existing member is
                // not catalog absence and grants no catch-up fallback.
                false
            } else {
                // Absent from the catalog entirely: the receipt-scoped
                // catch-up window (see CatchupCapability) may still admit
                // a member, judged against the LIVE unified watermark.
                self.catchup
                    .lock()
                    .expect("catchup capability slot poisoned")
                    .as_ref()
                    .is_some_and(|cap| cap.admits(node_id, self.driver.driver_applied()))
            }
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
    endpoint_ready: Arc<AtomicBool>,
    /// The DECLARED initial voters with their durable root-descriptor
    /// addresses — one of the two authoritative sources a NotLeader answer
    /// may resolve a leader endpoint from (the other is the local applied
    /// catalog nodes row). Never bind addresses, request origins, or
    /// client-supplied values.
    initial_voters: Vec<(NodeId, std::net::SocketAddr)>,
}

impl RuntimeBackend {
    /// Recovery control calls must remain available while this node's public
    /// data endpoint awaits confirmation. They still require a local committed
    /// catalog; subsequent barriers establish current consensus authority.
    fn ensure_catalog_ready(&self) -> Result<()> {
        if self.node.local_cluster_identity()?.is_none() {
            return Err(Error::MetaNotReady(
                "catalog cluster identity is missing".into(),
            ));
        }
        Ok(())
    }

    /// Resolve a node's canonical registration endpoint from the LOCAL
    /// APPLIED authoritative directory: the catalog nodes row (dynamic
    /// members register their canonical advertised address there), falling
    /// back to the durable root-descriptor voter list for initial voters
    /// (whose catalog rows carry no address). Returns None when neither
    /// source resolves — the NotLeader hint then degrades to id-only,
    /// fail-closed. The result is a BOUNDED ROUTING CANDIDATE, not proof of
    /// the current leader's endpoint: this node's applied view can lag, so
    /// the value may name an old leader or a superseded address; the
    /// registration client's (id, addr) dedup + hop cap absorb exactly that.
    ///
    /// Applied catalog routes converge after an authorized versioned update.
    /// Immutable root addresses are initial routing candidates and may remain
    /// obsolete after migration. A hint grants no endpoint authority; a fresh
    /// confirmation must validate the current directory and exact store/root.
    fn resolve_registration_endpoint(&self, id: NodeId) -> Option<String> {
        let from_catalog = self
            .node
            .meta_raft
            .store
            .begin()
            .ok()
            .and_then(|txn| txn.get(&NODES_DESC, &[memcmp_uint(id.0)]).ok().flatten())
            .and_then(|row| match row.value.get(ColumnId(2)) {
                Some(ColumnValue::Text(addr)) if !addr.is_empty() => Some(addr.clone()),
                _ => None,
            })
            // Canonical socket addresses only; a garbled row degrades the
            // hint to the next source or to id-only, never to a bad value.
            .filter(|addr| addr.parse::<std::net::SocketAddr>().is_ok());
        from_catalog.or_else(|| {
            self.initial_voters
                .iter()
                .find(|(vid, _)| *vid == id)
                .map(|(_, addr)| addr.to_string())
        })
    }

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
        self.check_serving_state(state)
    }

    fn try_ensure_serving(&self) -> Option<Result<()>> {
        let state = match self.node.meta.try_lock() {
            Ok(meta) => meta.bootstrap.state(),
            Err(std::sync::TryLockError::WouldBlock) => return None,
            Err(std::sync::TryLockError::Poisoned(_)) => panic!("meta poisoned"),
        };
        Some(self.check_serving_state(state))
    }

    fn check_serving_state(&self, state: BootstrapState) -> Result<()> {
        if matches!(state, BootstrapState::Serving { .. })
            && self.endpoint_ready.load(Ordering::Acquire)
        {
            Ok(())
        } else {
            Err(Error::MetaNotReady(format!(
                "metadata node is not serving (bootstrap_state={state:?})"
            )))
        }
    }

    /// Called while holding the catalog mutex, BEFORE reading or allocating.
    /// An ordered command barrier drains earlier ambiguous proposals as well as
    /// committed apply lag. ReadIndex alone does not drain an uncommitted suffix.
    fn prepare_catalog(&self) -> Result<u64> {
        Ok(propose_and_wait(
            &self.driver,
            &kv9_raft::Command::Noop,
            Duration::from_secs(10),
        )?
        .term)
    }

    fn commit_catalog(&self, command: &kv9_raft::Command, term: u64) -> Result<AppliedPosition> {
        observe_proposal_wait(
            &self.driver.metrics().logical_proposal_wait,
            || self.driver.propose_in_term(command, term),
            |at, remaining| self.driver.wait_applied(at, remaining),
            Duration::from_secs(10),
        )
    }
}

/// Propose `command` and wait for ITS exact `(term, index)` to apply, re-proposing
/// on a typed `Replaced` within the deadline budget (task #30).
///
/// Re-proposal is safe for exactly the same reason the clients' NotLeader retry is
/// safe: `Replaced` states the command provably NEVER applied (its position was
/// consumed by another leader's entry — in the 2026-08-31 master red, by an
/// election barrier), so retrying cannot double-apply. Every other failure stays
/// loud and unretried: `Unconfirmed` means the outcome is UNKNOWN, and retrying an
/// unknown outcome is how one write becomes two. If leadership moved to another
/// node, the re-propose surfaces the existing typed NotLeader (with hint) and the
/// client's own retry policy takes over.
fn propose_and_wait<S, E>(
    driver: &NodeDriver<S, E>,
    command: &kv9_raft::Command,
    deadline: Duration,
) -> Result<AppliedPosition>
where
    S: kv9_raft::rawnode::PersistentRaftStorage,
    E: kv9_engine::ReplicatedEngine + 'static,
{
    // The control loop is the scriptable core below; this wrapper binds it to
    // the real driver. Command reuse across re-proposals is BY CONSTRUCTION:
    // the propose closure borrows the one `command`, so there is no second
    // command for a retry to accidentally use.
    observe_proposal_wait(
        &driver.metrics().logical_proposal_wait,
        || driver.propose(command),
        |at, remaining| driver.wait_applied(at, remaining),
        deadline,
    )
}

/// Observe one logical call across all replacement retries. Classify before
/// the existing control loop converts typed unknown outcomes into public errors.
/// The original loop owns the only deadline and all business decisions.
fn observe_proposal_wait(
    metric: &kv9_common::metrics::Latency,
    mut propose: impl FnMut() -> Result<ProposedAt>,
    mut wait: impl FnMut(ProposedAt, Duration) -> std::result::Result<ApplyWaitOutcome, ApplyWaitError>,
    deadline: Duration,
) -> Result<AppliedPosition> {
    use kv9_common::metrics::Outcome;
    let outcome = std::cell::Cell::new(Outcome::Error);
    metric.observe(
        || {
            propose_and_wait_loop(
                || {
                    let result = propose();
                    outcome.set(match &result {
                        Err(Error::NotLeader { .. }) => Outcome::Rejected,
                        _ => Outcome::Error,
                    });
                    result
                },
                |at, remaining| {
                    let result = wait(at, remaining);
                    outcome.set(match &result {
                        Ok(ApplyWaitOutcome::Applied(_)) => Outcome::Success,
                        Ok(ApplyWaitOutcome::Replaced) => Outcome::Replaced,
                        Ok(ApplyWaitOutcome::FenceRejected { .. }) => Outcome::Rejected,
                        // A manifest receipt is invalid for this non-manifest path.
                        Ok(ApplyWaitOutcome::Manifest { .. }) => Outcome::Error,
                        Err(ApplyWaitError::Unconfirmed { .. }) => Outcome::Unconfirmed,
                        Err(ApplyWaitError::Failed(_)) => Outcome::Error,
                    });
                    result
                },
                deadline,
            )
        },
        |_| outcome.get(),
    )
}

/// The re-proposal control loop (task #30 scope addition), extracted over its
/// two effects so every contract clause is testable without a cluster:
///
/// - `Replaced` re-proposes: it states the command provably NEVER applied —
///   the same known-not-applied rule that makes the clients' NotLeader retry
///   safe. The budget is the ORIGINAL deadline (each wait gets the remaining
///   slice, never a fresh one).
/// - `Unconfirmed` and `Failed` return immediately, never retried: an unknown
///   outcome retried is how one write becomes two.
/// - A propose error (e.g. typed NotLeader + hint after leadership moved)
///   surfaces verbatim; the client's own retry policy owns that case.
fn propose_and_wait_loop(
    mut propose: impl FnMut() -> Result<ProposedAt>,
    mut wait: impl FnMut(ProposedAt, Duration) -> std::result::Result<ApplyWaitOutcome, ApplyWaitError>,
    deadline: Duration,
) -> Result<AppliedPosition> {
    let start = std::time::Instant::now();
    loop {
        let proposed = propose()?;
        let remaining = deadline.saturating_sub(start.elapsed());
        if let Some(applied) = settle_proposal(
            proposed,
            wait(proposed, remaining),
            start.elapsed() >= deadline,
        )? {
            return Ok(applied);
        }
    }
}

// The synchronous and asynchronous paths share every receipt/retry decision.
fn settle_proposal(
    proposed: ProposedAt,
    outcome: std::result::Result<ApplyWaitOutcome, ApplyWaitError>,
    retry_exhausted: bool,
) -> Result<Option<AppliedPosition>> {
    match outcome {
        Ok(ApplyWaitOutcome::Applied(at)) => Ok(Some(at)),
        // A fence rejection is a VERDICT, not a transient: the expected
        // epoch is authoritatively stale and will not come back, so this
        // maps to the typed StaleEpoch immediately and is NEVER retried
        // here (retrying re-proposes the same stale expectation; the
        // client must re-route/re-validate). Only Replaced re-proposes.
        Ok(ApplyWaitOutcome::FenceRejected { region, .. }) => Err(Error::StaleEpoch { region }),
        // This path proposes catalog/user writes, never manifest changes:
        // a manifest verdict here means receipt correlation broke (typed,
        // not absorbed into success or retry).
        Ok(ApplyWaitOutcome::Manifest { at, .. }) => Err(Error::Raft(format!(
            "non-manifest proposal received a manifest verdict at term {} index {}",
            at.term, at.index
        ))),
        Ok(ApplyWaitOutcome::Replaced) => {
            if retry_exhausted {
                return Err(Error::Raft(format!(
                    "proposal at term {} index {} was replaced and the retry \
                         budget is exhausted",
                    proposed.term, proposed.index.0
                )));
            }
            Ok(None)
        }
        Err(e @ ApplyWaitError::Unconfirmed { .. }) => Err(e.into()),
        Err(ApplyWaitError::Failed(e)) => Err(e),
    }
}

async fn finish_async_proposal<S, E>(
    driver: Arc<NodeDriver<S, E>>,
    command: Arc<Command>,
    pending: (ProposedAt, kv9_raft::driver::AsyncApplyWait),
    deadline: Instant,
) -> (Result<AppliedPosition>, kv9_common::metrics::Outcome)
where
    S: kv9_raft::rawnode::PersistentRaftStorage,
    E: kv9_engine::ReplicatedEngine + 'static,
{
    finish_async_proposal_loop(
        command,
        pending,
        deadline,
        |wait| wait.wait(),
        move |same_command, same_deadline| {
            let proposer = driver.clone();
            async move {
                tokio::task::spawn_blocking(move || {
                    proposer.propose_with_async_wait(&same_command, same_deadline)
                })
                .await
                .map_err(|error| {
                    Error::Raft(format!(
                        "asynchronous write submission worker failed: {error}"
                    ))
                })?
            }
        },
    )
    .await
}

// The production loop owns the original command and deadline. Its effect
// parameters allow exact receipt, retry identity and deadline controls without
// timing-dependent leader elections. Real-driver tests exercise its producer.
async fn finish_async_proposal_loop<W, WaitFuture, ProposeFuture>(
    command: Arc<Command>,
    mut pending: (ProposedAt, W),
    deadline: Instant,
    mut wait: impl FnMut(W) -> WaitFuture,
    mut propose: impl FnMut(Arc<Command>, Instant) -> ProposeFuture,
) -> (Result<AppliedPosition>, kv9_common::metrics::Outcome)
where
    WaitFuture: std::future::Future<Output = std::result::Result<ApplyWaitOutcome, ApplyWaitError>>,
    ProposeFuture: std::future::Future<Output = Result<(ProposedAt, W)>>,
{
    use kv9_common::metrics::Outcome;
    loop {
        let (proposed, waiter) = pending;
        let outcome = wait(waiter).await;
        let category = match &outcome {
            Ok(ApplyWaitOutcome::Applied(_)) => Outcome::Success,
            Ok(ApplyWaitOutcome::Replaced) => Outcome::Replaced,
            Ok(ApplyWaitOutcome::FenceRejected { .. }) => Outcome::Rejected,
            Ok(ApplyWaitOutcome::Manifest { .. }) => Outcome::Error,
            Err(ApplyWaitError::Unconfirmed { .. }) => Outcome::Unconfirmed,
            Err(ApplyWaitError::Failed(_)) => Outcome::Error,
        };
        match settle_proposal(proposed, outcome, Instant::now() >= deadline) {
            Ok(Some(applied)) => return (Ok(applied), category),
            Err(error) => return (Err(error), category),
            Ok(None) => {}
        }
        pending = match propose(command.clone(), deadline).await {
            Ok(next) => next,
            Err(error) => {
                let category = if matches!(error, Error::NotLeader { .. }) {
                    Outcome::Rejected
                } else {
                    Outcome::Error
                };
                return (Err(error), category);
            }
        };
    }
}

impl AdminApi for RuntimeBackend {
    fn get_node_endpoint(
        &self,
        _caller: &str,
        node: NodeId,
    ) -> Result<crate::api::EndpointReadResult> {
        self.ensure_catalog_ready()?;
        let _barrier = self.driver.read_barrier(READ_BARRIER_DEADLINE)?;
        let txn = self.node.meta_raft.store.begin()?;
        Ok(crate::api::EndpointReadResult {
            cluster: kv9_meta::admission::cluster_id(&txn)?
                .ok_or_else(|| Error::MetaNotReady("catalog cluster identity is missing".into()))?,
            endpoint: kv9_meta::endpoint::node_endpoint(&txn, node)?,
        })
    }

    fn change_node_endpoint(
        &self,
        _caller: &str,
        request: crate::api::EndpointChange,
    ) -> Result<crate::api::EndpointUpdateResult> {
        use crate::api::EndpointUpdateResult;
        use kv9_meta::endpoint::EndpointChangeOutcome;
        self.ensure_catalog_ready()?;
        let status = self.driver.status();
        if status.role != Role::Leader {
            return Err(Error::NotLeader {
                leader: status.leader_id,
            });
        }
        let _guard = self.node.meta_raft.lock_catalog_txn();
        let term = self.prepare_catalog()?;
        let mut txn = self.node.meta_raft.store.begin()?;
        let result = match kv9_meta::endpoint::change_endpoint(&mut txn, request)? {
            EndpointChangeOutcome::Changed(endpoint) => {
                let applied =
                    self.commit_catalog(&kv9_raft::Command::from_batch(&txn.into_batch()), term)?;
                EndpointUpdateResult::Changed { endpoint, applied }
            }
            EndpointChangeOutcome::Confirmed(endpoint) => {
                // A current directory record cannot recover the old proposal
                // identity. Confirm its predicate with a NEW exact term-fenced
                // barrier, and label that receipt separately on the wire.
                drop(txn);
                let confirmation = self.commit_catalog(&kv9_raft::Command::Noop, term)?;
                EndpointUpdateResult::Confirmed {
                    endpoint,
                    confirmation,
                }
            }
            EndpointChangeOutcome::Refused(reason) => {
                return Ok(EndpointUpdateResult::Refused(reason));
            }
        };
        // Preserve the local snapshot/install order through the exact receipt.
        // An error or unknown outcome never grants an eager route update.
        let endpoint = match result {
            EndpointUpdateResult::Changed { endpoint, .. }
            | EndpointUpdateResult::Confirmed { endpoint, .. } => endpoint,
            EndpointUpdateResult::Refused(_) => {
                unreachable!("refusals returned before installation")
            }
        };
        self.transport.register_catalog_peer(
            endpoint.node,
            endpoint.address,
            endpoint.generation,
        )?;
        Ok(result)
    }

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
        let planning_term = self.prepare_catalog()?;
        let (keyspace, command) = self
            .node
            .build_create_keyspace_command(name, tenant, api_type)?;
        let applied = self.commit_catalog(&command, planning_term)?;
        Ok(CreateKeyspaceResult {
            keyspace,
            proposed: Some(applied),
        })
    }

    fn list_keyspaces(&self, caller: &str) -> Result<Vec<kv9_common::Keyspace>> {
        self.ensure_serving()?;
        let _barrier = self.driver.read_barrier(READ_BARRIER_DEADLINE)?;
        self.node.list_keyspaces(caller)
    }

    fn get_region(&self, caller: &str, keyspace: KeyspaceId, key: &[u8]) -> Result<RegionLocation> {
        self.ensure_serving()?;
        let _barrier = self.driver.read_barrier(READ_BARRIER_DEADLINE)?;
        self.node.get_region(caller, keyspace, key)
    }

    fn split_region(&self, caller: &str, region: RegionId, split_key: UserKey) -> Result<()> {
        self.ensure_serving()?;
        self.node.split_region(caller, region, split_key)
    }

    fn cluster_info(&self, caller: &str) -> Result<ClusterInfo> {
        self.ensure_serving()?;
        let _barrier = self.driver.read_barrier(READ_BARRIER_DEADLINE)?;
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
        let status = self.driver.status();
        if status.role != Role::Leader {
            return Err(Error::NotLeader {
                leader: status.leader_id,
            });
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
        let planning_term = self.prepare_catalog()?;
        let mut txn = self.node.meta_raft.store.begin()?;
        kv9_meta::admission::admit_node_with_ticket_hash(
            &mut txn,
            node,
            addr,
            kv9_meta::admission::AdmittedRole::Learner,
            ticket_sha256.as_bytes(),
            expires,
        )?;
        let applied = self.commit_catalog(
            &kv9_raft::Command::from_batch(&txn.into_batch()),
            planning_term,
        )?;
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
        let status = self.driver.status();
        if status.role != Role::Leader {
            return Err(Error::NotLeader {
                leader: status.leader_id,
            });
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
    E: kv9_engine::ReplicatedEngine + 'static,
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
    fn confirm_endpoint(
        &self,
        node: NodeId,
        cluster: ClusterId,
        incarnation: StoreIncarnation,
        address: std::net::SocketAddr,
    ) -> std::result::Result<kv9_raft::grpc::EndpointConfirmationReceipt, RegistrationError> {
        self.ensure_catalog_ready()
            .map_err(RegistrationError::Failed)?;
        let status = self.driver.status();
        if status.role != Role::Leader {
            return Err(RegistrationError::NotLeader {
                leader: status.leader_id,
                leader_addr: status
                    .leader_id
                    .and_then(|id| self.resolve_registration_endpoint(id)),
            });
        }
        let _guard = self.node.meta_raft.lock_catalog_txn();
        let term = self.prepare_catalog().map_err(RegistrationError::Failed)?;
        let txn = self
            .node
            .meta_raft
            .store
            .begin()
            .map_err(RegistrationError::Failed)?;
        if kv9_meta::admission::cluster_id(&txn).map_err(RegistrationError::Failed)?
            != Some(cluster)
        {
            return Err(RegistrationError::Failed(Error::Config(
                "endpoint confirmation cluster mismatch".into(),
            )));
        }
        let endpoint = |id| -> std::result::Result<
            kv9_raft::grpc::EndpointRoute,
            RegistrationError,
        > {
            let row = kv9_meta::endpoint::node_endpoint(&txn, id)
                .map_err(RegistrationError::Failed)?
                .filter(|row| row.active)
                .ok_or_else(|| {
                    RegistrationError::Failed(Error::Config("endpoint member is not active".into()))
                })?;
            Ok(kv9_raft::grpc::EndpointRoute {
                node: row.node,
                incarnation: row.incarnation,
                address: row.address,
                generation: row.generation,
            })
        };
        let subject = endpoint(node)?;
        if subject.incarnation != incarnation {
            return Err(RegistrationError::InvalidIncarnation);
        }
        if subject.address != address {
            return Err(RegistrationError::Failed(Error::Config(
                "advertised endpoint has no current catalog authorization".into(),
            )));
        }
        let responder = endpoint(self.node.id)?;
        drop(txn);
        let applied = self
            .commit_catalog(&kv9_raft::Command::Noop, term)
            .map_err(RegistrationError::Failed)?;
        self.transport
            .register_catalog_peer(subject.node, subject.address, subject.generation)
            .map_err(RegistrationError::Failed)?;
        Ok(kv9_raft::grpc::EndpointConfirmationReceipt {
            subject,
            responder,
            applied,
        })
    }

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
            let leader_addr = leader
                .leader_id
                .and_then(|id| self.resolve_registration_endpoint(id));
            return Err(RegistrationError::NotLeader {
                leader: leader.leader_id,
                leader_addr,
            });
        }
        let now = unix_now();
        let canonical_addr: std::net::SocketAddr = addr.parse().map_err(|_| {
            RegistrationError::Failed(Error::Config(
                "registration address must be a canonical socket address".into(),
            ))
        })?;
        let canonical = canonical_addr.to_string();

        // Serialize the catalog half across retries/revocation. Consuming an
        // admission and inserting a Joining node are one command; if the
        // later ConfChange loses leadership, a retry recognizes this durable
        // intermediate state and completes instead of wedging on "consumed".
        let _catalog_guard = self.node.meta_raft.lock_catalog_txn();
        let planning_term = self.prepare_catalog().map_err(RegistrationError::Failed)?;
        let existing = {
            let txn = self
                .node
                .meta_raft
                .store
                .begin()
                .map_err(RegistrationError::Failed)?;
            kv9_meta::admission::admission(&txn, node).map_err(RegistrationError::Failed)?
        };
        // A renewed ticket cannot authorize an empty disk to reuse an existing
        // replica's durable-log identity. This also precedes endpoint changes.
        {
            let txn = self
                .node
                .meta_raft
                .store
                .begin()
                .map_err(RegistrationError::Failed)?;
            if let Some(row) = txn
                .get(&NODES_DESC, &[memcmp_uint(node.0)])
                .map_err(RegistrationError::Failed)?
            {
                if !matches!(row.value.get(ColumnId(5)), Some(ColumnValue::Bytes(bytes)) if bytes.as_slice() == store_incarnation.as_bytes())
                {
                    return Err(RegistrationError::InvalidIncarnation);
                }
            }
        }
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
                    return Err(RegistrationError::InvalidIncarnation);
                }
                let endpoint = kv9_meta::endpoint::node_endpoint(&txn, node)
                    .map_err(RegistrationError::Failed)?
                    .ok_or_else(|| {
                        RegistrationError::Failed(Error::Config(
                            "consumed registration has no endpoint".into(),
                        ))
                    })?;
                if endpoint.address != canonical_addr {
                    return Err(RegistrationError::Failed(Error::Config(
                        "consumed admission names a superseded endpoint".into(),
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
                    kv9_meta::endpoint::refresh_registration_endpoint(
                        &mut txn,
                        node,
                        store_incarnation,
                        canonical_addr,
                    )
                    .map_err(RegistrationError::Failed)?;
                    txn.update(
                        &NODES_DESC,
                        &[memcmp_uint(node.0)],
                        vec![
                            (ColumnId(3), ColumnValue::Uint(1)),
                            (ColumnId(4), ColumnValue::Uint(now)),
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
                self.commit_catalog(
                    &kv9_raft::Command::from_batch(&txn.into_batch()),
                    planning_term,
                )
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

        // Admission and incarnation checks have succeeded, including the
        // durable consume on the new-member path. Routing must be installed
        // before AddLearner, but must never change for a rejected caller.
        let endpoint = kv9_meta::endpoint::node_endpoint(
            &self
                .node
                .meta_raft
                .store
                .begin()
                .map_err(RegistrationError::Failed)?,
            node,
        )
        .map_err(RegistrationError::Failed)?
        .ok_or_else(|| {
            RegistrationError::Failed(Error::Config(
                "registered endpoint disappeared before route installation".into(),
            ))
        })?;
        self.transport
            .register_catalog_peer(node, endpoint.address, endpoint.generation)
            .map_err(RegistrationError::Failed)?;
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
            .commit_catalog(
                &kv9_raft::Command::from_batch(&txn.into_batch()),
                planning_term,
            )
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

/// The context gate and the capability it mints, isolated so that **only** this module can
/// construct one.
///
/// `ValidatedFence`'s fields were module-private before, which in Rust means visible to every
/// endpoint in `runtime.rs` — and the loop tests were indeed building one directly (@Tess).
/// A capability an endpoint can forge is documentation, not enforcement. Here the parent sees
/// exactly two things: the check that returns one, and the by-value conversion that spends it.
mod gate {
    use super::{Error, KeySpan, Result, Tables};
    use kv9_common::{ApiType, KeyspaceId};

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
    pub(super) fn check_context<E: kv9_engine::Engine>(
        store: &kv9_meta::store::MetaStore<E>,
        keyspace_id: KeyspaceId,
        epoch: &kv9_region::RegionEpoch,
        span: KeySpan<'_>,
    ) -> Result<ValidatedFence> {
        let txn = store.begin()?;
        check_context_in(store, &txn, keyspace_id, epoch, span)
    }

    /// The same gate over a CALLER-OWNED transaction — one body, two entry
    /// shapes. The establishing-read seam calls this with a txn built on the
    /// post-barrier view so context and data are judged on ONE state; with
    /// the gate on its own snapshot, a split that applies between the two
    /// snapshots lets an old epoch authorize a read of state that epoch
    /// never covered (reads have no apply step, so the write-side fence
    /// cannot catch it). Read callers drop the minted fence: the mint is
    /// the write-authorisation half and stays in one body deliberately —
    /// a second, fence-less gate body would be a parallel path free to
    /// drift from the one writes are fenced by.
    pub(super) fn check_context_in<E: kv9_engine::Engine>(
        store: &kv9_meta::store::MetaStore<E>,
        txn: &kv9_meta::store::MetaTxn<'_, E>,
        keyspace_id: KeyspaceId,
        epoch: &kv9_region::RegionEpoch,
        span: KeySpan<'_>,
    ) -> Result<ValidatedFence> {
        let keyspace = Tables::<E>::keyspace_in(txn, keyspace_id)?
            .ok_or(Error::KeyspaceNotFound(keyspace_id))?;
        if keyspace.api_type != ApiType::Raw {
            return Err(Error::ApiTypeMismatch {
                keyspace: keyspace_id,
            });
        }

        let tables = Tables::new(store);
        let region = tables
            .region_for_key_in(txn, keyspace_id, span.anchor())?
            .ok_or(Error::RegionNotFound)?;

        // Epoch before span: a stale epoch and a cross-region request are different failures
        // and the client reacts differently (refresh routing vs. split the request).
        if region.epoch_conf != epoch.conf_ver || region.epoch_ver != epoch.version {
            return Err(Error::StaleEpoch { region: region.id });
        }

        span.assert_within(&region, txn, &tables, keyspace_id)?;

        // The values THIS check resolved, minted as the authorisation for the write that follows.
        // Re-deriving the region at propose time would reopen the very window the fence exists
        // to close: between the two lookups a split can commit, and the write would then be
        // fenced against a region that was never authorised — a fence that always agrees with
        // itself and therefore never fires.
        Ok(ValidatedFence {
            region: region.id,
            epoch: kv9_region::RegionEpoch {
                conf_ver: region.epoch_conf,
                version: region.epoch_ver,
            },
        })
    }

    /// Proof that the context gate authorised **one** write against **one** region.
    ///
    /// Minted only by [`check_context`], on the success path, from the region that check
    /// resolved. Consumed by value at [`RuntimeBackend::commit_batch`], the single place a raw
    /// write becomes a raft entry.
    ///
    /// **Deliberately neither `Clone` nor `Copy`, and that is the entire point of the type.**
    /// The earlier shape threaded a bare `RegionId` down to the commit, which is a `Copy`
    /// scalar: a range delete could validate once before its loop and hand the same region to
    /// every chunk, so chunk N was fenced with chunk 1's authorisation — the stale authorisation
    /// the per-chunk revalidation exists to remove.
    ///
    /// **That property has no test and cannot have a useful one**, so it is enforced by the type
    /// and by [`NOT_CLONE_OR_COPY`]. Measured, not assumed, in both directions:
    ///
    /// - Writing the bad merge (mint once before the loop, move it into the commit closure)
    ///   fails with `E0507: cannot move out of 'outer', a captured variable in an FnMut closure —
    ///   move occurs because 'outer' has type 'ValidatedFence', which does not implement the
    ///   'Copy' trait`.
    /// - Adding `#[derive(Clone, Copy)]` — the old `RegionId` shape — makes that same merge
    ///   compile **and** pass every test in the workspace, including the multi-chunk endpoint
    ///   regression. Against an unmoved catalog each chunk's revalidation resolves identical
    ///   values, so reusing chunk 1's authorisation is observationally identical to minting a
    ///   fresh one. No behavioural test can separate them without manufacturing a catalog change
    ///   mid-loop, which needs a background thread and buys a timing-fragile test.
    ///
    /// Construction is confined to this module and to [`check_context`]'s success path, so no
    /// endpoint can mint its own authorisation without passing the gate (@Tess).
    ///
    /// It carries the epoch it resolved rather than the one the client sent. The gate demands
    /// they be equal, so today they always are; taking it from the catalog means a future
    /// relaxation of that equality cannot silently turn the fence into the client's own claim.
    pub(super) struct ValidatedFence {
        region: kv9_common::RegionId,
        epoch: kv9_region::RegionEpoch,
    }

    impl ValidatedFence {
        /// The wire form the state machine adjudicates against.
        ///
        /// Consumes `self`: one authorisation yields one fence, and the caller has nothing left
        /// to hand to a second batch.
        pub(super) fn into_region_fence(self) -> kv9_raft::RegionFence {
            kv9_raft::RegionFence {
                region_id: self.region.0,
                conf_ver: self.epoch.conf_ver,
                version: self.epoch.version,
            }
        }
    }

    /// Reds at compile time if [`ValidatedFence`] ever gains `Clone` or `Copy`.
    ///
    /// The non-`Copy`-ness is the *only* thing standing between us and the stale-authorisation
    /// merge, and a derive is one line for a future contributor to add "for convenience". A
    /// comment saying "do not derive Copy" does not survive that; this does.
    ///
    /// How it works: an inherent associated const takes precedence over a trait's when both
    /// apply. The inherent `impl` is bounded on `Clone`, so it exists only if `ValidatedFence`
    /// is `Clone` — and then the const-evaluated `panic!` is what resolves, failing the build.
    /// If it is not `Clone`, the inherent impl does not apply and the trait's no-op is used.
    /// `Copy` requires `Clone`, so bounding on `Clone` alone catches both.
    ///
    /// Verified in both directions rather than assumed: without the derive this compiles clean;
    /// with `#[derive(Clone)]` it is
    /// `error[E0080]: evaluation panicked: ValidatedFence must be neither Clone nor Copy`.
    /// `dead_code` because nothing reads it — but Rust const-evaluates every named constant
    /// regardless, which is the property this relies on and which was checked here rather than
    /// assumed: with `#[derive(Clone)]` added, this crate fails to build even though the constant
    /// is still unused.
    #[allow(dead_code)]
    const NOT_CLONE_OR_COPY: () = {
        struct Probe<T>(core::marker::PhantomData<T>);
        trait Fallback {
            const CHECK: () = ();
        }
        impl<T> Fallback for Probe<T> {}
        #[allow(dead_code)]
        impl<T: Clone> Probe<T> {
            const CHECK: () = panic!("ValidatedFence must be neither Clone nor Copy");
        }
        <Probe<ValidatedFence>>::CHECK
    };

    impl ValidatedFence {
        /// A capability for the `run_delete_range` loop tests, which are about control flow —
        /// how many chunks run, what a mid-range failure reports — and need *a* capability
        /// without needing a catalog.
        ///
        /// `#[cfg(test)]`, so it does not exist in a production build and no endpoint can reach
        /// it there. The content assertions deliberately do not use it: they drive a real node,
        /// because a fixture-built fence says nothing about what production builds
        /// (docs/TESTING.md rule 17).
        #[cfg(test)]
        pub(super) fn for_loop_tests() -> Self {
            Self {
                region: kv9_common::RegionId(1),
                epoch: kv9_region::RegionEpoch {
                    conf_ver: 1,
                    version: 1,
                },
            }
        }
    }
}

use gate::{check_context, check_context_in, ValidatedFence};

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
    // Borrow request storage instead of allocating a vector of key references.
    BatchKeys(&'a [UserKey]),
    BatchPairs(&'a [(UserKey, Value)]),
    /// Half-open `[start, end)`; an empty `end` means "to the end of the keyspace".
    Range {
        start: &'a [u8],
        end: &'a [u8],
    },
}

enum PreparedReadView {
    Resident(Box<dyn ReadView>),
    Blocking(std::result::Result<ReadBarrier, kv9_raft::driver::ReadIndexError>),
}

impl<'a> KeySpan<'a> {
    /// The key used to resolve the region. An empty range start means the keyspace's
    /// first key, which `region_for_key` already treats as the leading region.
    fn anchor(&self) -> &[u8] {
        match self {
            KeySpan::Point(key) => key,
            KeySpan::BatchKeys(keys) => keys.first().map(Vec::as_slice).unwrap_or(&[]),
            KeySpan::BatchPairs(pairs) => {
                pairs.first().map(|(key, _)| key.as_slice()).unwrap_or(&[])
            }
            KeySpan::Range { start, .. } => start,
        }
    }

    /// Check the rest of a span after `check_context_in` resolved its anchor
    /// to `region` on this exact transaction. The anchor is already authorized;
    /// every later batch position still performs the original owner lookup.
    fn assert_within<E: kv9_engine::Engine>(
        &self,
        region: &kv9_meta::tables::Region,
        txn: &kv9_meta::store::MetaTxn<'_, E>,
        tables: &Tables<'_, E>,
        keyspace: KeyspaceId,
    ) -> Result<()> {
        let check_key = |key: &[u8]| {
            let owner = tables
                .region_for_key_in(txn, keyspace, key)?
                .ok_or(Error::RegionNotFound)?;
            if owner.id != region.id {
                return Err(Error::RangeCrossesRegion);
            }
            Ok(())
        };
        match self {
            // Already resolved by this key.
            KeySpan::Point(_) => Ok(()),
            KeySpan::BatchKeys(keys) => keys.iter().skip(1).try_for_each(|key| check_key(key)),
            KeySpan::BatchPairs(pairs) => {
                pairs.iter().skip(1).try_for_each(|(key, _)| check_key(key))
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
    // `revalidate` MINTS the authorisation and `commit` CONSUMES it, both once per chunk.
    // Threading the capability through the signature rather than trusting the endpoint to
    // pair them is what makes "fence chunk N with chunk 1's authorisation" unwriteable
    // rather than merely discouraged — see [`ValidatedFence`].
    V: FnMut(&[u8]) -> Result<ValidatedFence>,
    P: FnMut(Option<&[u8]>) -> Result<Option<(kv9_engine::WriteBatch, UserKey)>>,
    C: FnMut(ValidatedFence, kv9_engine::WriteBatch) -> Result<AppliedPosition>,
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
        // layer 2, the fenced envelope.
        //
        // Layer 2 has since landed, and THIS chunk's authorisation is what fences it: the
        // capability returned here is consumed by this round's `commit` and cannot outlive
        // it. A split that commits mid-range is refused at apply for the chunks that follow
        // it, not merely narrowed against.
        let fence = preserving_receipt!(revalidate(remaining_start));
        let Some((batch, last_key)) = preserving_receipt!(plan_next(cursor.as_deref())) else {
            return Ok(DeleteRangeReceipt {
                committed_chunks,
                last_applied,
            });
        };
        // Only "partial" once something has actually committed. Failing before the first
        // chunk really is "nothing happened", and dressing that up as partial would be
        // its own lie — in the opposite direction.
        let position = preserving_receipt!(commit(fence, batch));
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
    fn validated_context(&self, ctx: &RequestContext, span: KeySpan<'_>) -> Result<ValidatedFence> {
        check_context(
            &self.node.meta_raft.store,
            ctx.keyspace,
            &ctx.region_epoch,
            span,
        )
    }

    /// The point-read entry: ONE barrier, ONE view, and the context/region
    /// gate judged on that SAME view before it is handed to the executor.
    /// The alternative — gate on its own snapshot — reopens the epoch hole
    /// the barrier closes: a split committed-but-unapplied passes the old
    /// epoch, the barrier applies it, and the read then serves post-split
    /// state under pre-split authorization.
    fn established_read(
        &self,
        ctx: &RequestContext,
        span: KeySpan<'_>,
    ) -> Result<Box<dyn ReadView + '_>> {
        let barrier = self.driver.read_barrier(READ_BARRIER_DEADLINE)?;
        self.read_view_after_barrier(barrier, ctx, span)
    }

    fn read_view_after_barrier(
        &self,
        barrier: ReadBarrier,
        ctx: &RequestContext,
        span: KeySpan<'_>,
    ) -> Result<Box<dyn ReadView + '_>> {
        let view = self.established_view(barrier)?;
        self.check_read_view(view, ctx, span)
    }

    fn check_read_view<'a>(
        &'a self,
        view: Box<dyn ReadView + 'a>,
        ctx: &RequestContext,
        span: KeySpan<'_>,
    ) -> Result<Box<dyn ReadView + 'a>> {
        let store = &self.node.meta_raft.store;
        let txn = store.begin_at(view);
        // Reads drop the minted fence: it is the WRITE-authorisation half of
        // the gate verdict, and reads have no apply step to present it to.
        let _fence = check_context_in(store, &txn, ctx.keyspace, &ctx.region_epoch, span)?;
        // The SAME view continues into the data read — surrendered, not
        // re-taken; a second snapshot here is exactly the defect above.
        Ok(txn.into_view())
    }

    /// Exchange a read barrier for exactly ONE blocking engine snapshot.
    /// `try_established_resident_view` is the second, nonblocking constructor;
    /// it returns the unconsumed credential when no view was captured.
    ///
    /// Inventory invariant (re-runnable; classify every hit — an
    /// unclassifiable one is a signal that must be explained, not absorbed):
    ///   git grep -n -F '.snapshot()' <head> -- crates/server/src
    /// Expected ENGINE classification: PRODUCTION calls exactly 1 (this
    /// function); TEST calls exactly 1 (the deliberately stale bypass control
    /// in the committed-but-unapplied cell). Classify admission-ledger and
    /// latency-histogram snapshots separately: they construct no engine view.
    /// Other hits are doc/comment text. The invariant is the two ENGINE
    /// counts, never the raw method-name total. Additionally inventory the one
    /// production `.try_resident_snapshot()` call in the resident constructor.
    /// `Engine::snapshot()` is a
    /// public API and the type system cannot forbid a future second call
    /// site; what IS mechanically held is (a) `ReadBarrier` is neither
    /// Clone nor Copy (compile-time probe in kv9-raft), so one barrier
    /// yields one view and loop reuse is unrepresentable, and (b) the
    /// committed-but-unapplied behavioral regression below reds if the
    /// production read path takes its snapshot before the barrier or
    /// bypasses it.
    fn established_view(&self, barrier: ReadBarrier) -> Result<Box<dyn ReadView + '_>> {
        // Consumed: the credential cannot be presented twice.
        let _ = barrier;
        self.node.meta_raft.store.engine().snapshot()
    }

    /// A contended try creates no snapshot and returns the unconsumed barrier
    /// for the blocking path. Success consumes it for exactly one owned view.
    fn try_established_resident_view(
        &self,
        barrier: ReadBarrier,
    ) -> std::result::Result<Box<dyn ReadView>, ReadBarrier> {
        match self.node.meta_raft.store.engine().try_resident_snapshot() {
            Some(view) => {
                let _ = barrier;
                Ok(view)
            }
            None => Err(barrier),
        }
    }

    fn prepared_get_from_view(
        &self,
        view: Box<dyn ReadView + '_>,
        ctx: &RequestContext,
        key: &[u8],
    ) -> Result<Option<Value>> {
        let view = self.check_read_view(view, ctx, KeySpan::Point(key))?;
        // The consumed quorum credential established authority. LeaderRead's
        // hint is used only when is_leader is false, so no driver/status locks
        // are needed here to compute an unused hint.
        let read = LeaderRead::new(view.as_ref(), true, None)?;
        RawExecutor.get(&read, ctx.keyspace, key)
    }

    fn prepared_batch_get_from_view(
        &self,
        view: Box<dyn ReadView + '_>,
        ctx: &RequestContext,
        keys: &[UserKey],
    ) -> Result<Vec<Option<Value>>> {
        let view = self.check_read_view(view, ctx, KeySpan::BatchKeys(keys))?;
        let read = LeaderRead::new(view.as_ref(), true, None)?;
        RawExecutor.batch_get(&read, ctx.keyspace, keys)
    }

    fn blocking_prepared_read<T, F>(
        self: Arc<Self>,
        established: std::result::Result<ReadBarrier, kv9_raft::driver::ReadIndexError>,
        read: F,
    ) -> crate::api::RawReadJob<T>
    where
        T: Send + 'static,
        F: for<'a> FnOnce(&'a Self, Box<dyn ReadView + 'a>) -> Result<T> + Send + 'static,
    {
        crate::api::RawReadJob::Blocking(Box::new(move || {
            self.ensure_serving()?;
            let view = self.established_view(established?)?;
            read(&self, view)
        }))
    }

    /// Point and batch reads exchange the same single-use quorum credential
    /// for exactly one view. Contention transfers the unconsumed credential
    /// and the complete read into the blocking job; it never takes a new view
    /// or repeats the barrier before validating the request context.
    fn prepare_resident_read_view(
        &self,
        established: std::result::Result<ReadBarrier, kv9_raft::driver::ReadIndexError>,
    ) -> Result<PreparedReadView> {
        let Some(serving) = self.try_ensure_serving() else {
            return Ok(PreparedReadView::Blocking(established));
        };
        serving?; // Preserve lifecycle-before-barrier-error ordering.
        let barrier = established?;
        Ok(match self.try_established_resident_view(barrier) {
            Ok(view) => PreparedReadView::Resident(view),
            Err(barrier) => PreparedReadView::Blocking(Ok(barrier)),
        })
    }

    fn finish_prepared_read<T, F>(
        self: Arc<Self>,
        established: std::result::Result<ReadBarrier, kv9_raft::driver::ReadIndexError>,
        read: F,
    ) -> Result<crate::api::RawReadJob<T>>
    where
        T: Send + 'static,
        F: for<'a> FnOnce(&'a Self, Box<dyn ReadView + 'a>) -> Result<T> + Send + 'static,
    {
        Ok(match self.prepare_resident_read_view(established)? {
            PreparedReadView::Resident(view) => {
                crate::api::RawReadJob::Completed(read(&self, view)?)
            }
            PreparedReadView::Blocking(established) => {
                self.blocking_prepared_read(established, read)
            }
        })
    }

    fn finish_prepared_get(
        self: Arc<Self>,
        ctx: RequestContext,
        key: UserKey,
        established: std::result::Result<ReadBarrier, kv9_raft::driver::ReadIndexError>,
    ) -> Result<crate::api::RawReadJob<Option<Value>>> {
        self.finish_prepared_read(established, move |backend, view| {
            backend.prepared_get_from_view(view, &ctx, &key)
        })
    }

    fn finish_prepared_batch_get(
        self: Arc<Self>,
        ctx: RequestContext,
        keys: Vec<UserKey>,
        established: std::result::Result<ReadBarrier, kv9_raft::driver::ReadIndexError>,
    ) -> Result<crate::api::RawReadJob<Vec<Option<Value>>>> {
        match self.prepare_resident_read_view(established)? {
            PreparedReadView::Resident(view) => self.finish_resident_batch_get(ctx, keys, view),
            PreparedReadView::Blocking(established) => Ok(self
                .blocking_prepared_read(established, move |backend, view| {
                    backend.prepared_batch_get_from_view(view, &ctx, &keys)
                })),
        }
    }

    fn finish_resident_batch_get(
        self: Arc<Self>,
        ctx: RequestContext,
        keys: Vec<UserKey>,
        view: Box<dyn ReadView>,
    ) -> Result<crate::api::RawReadJob<Vec<Option<Value>>>> {
        let inline = {
            // Authorize a borrowed wrapper; retain ownership of this exact view
            // in case the values cannot be copied within the async CPU budget.
            let authorized =
                self.check_read_view(Box::new(view.as_ref()), &ctx, KeySpan::BatchKeys(&keys))?;
            let read = LeaderRead::new(authorized.as_ref(), true, None)?;
            RawExecutor.try_batch_get_resident(
                &read,
                ctx.keyspace,
                &keys,
                MAX_RESIDENT_BATCH_READ_BYTES,
            )?
        };
        if let Some(values) = inline {
            return Ok(crate::api::RawReadJob::Completed(values));
        }
        Ok(crate::api::RawReadJob::Blocking(Box::new(move || {
            self.ensure_serving()?;
            // No second barrier, snapshot, or current-epoch lookup: both the
            // authorization and the deferred copy refer to the captured view.
            let read = LeaderRead::new(view.as_ref(), true, None)?;
            RawExecutor.batch_get(&read, ctx.keyspace, &keys)
        })))
    }

    /// Replicate one planned batch and wait for its exact position to apply.
    ///
    /// Returns that position so the caller can hand it back to the client. Deriving it
    /// afterwards from the status file cannot prove identity: a concurrent command moves
    /// the same number, so the client would be shown someone else's write.
    /// `fence` is taken **by value** and is not `Clone`: one authorisation, one write. The
    /// caller cannot hold it back and reuse it for a second batch, which is the whole reason
    /// it is a capability rather than a pair of scalars — see [`ValidatedFence`].
    fn commit_batch(
        &self,
        fence: ValidatedFence,
        batch: kv9_engine::WriteBatch,
    ) -> Result<AppliedPosition> {
        if batch.mutations().is_empty() {
            return Ok(AppliedPosition { term: 0, index: 0 });
        }
        // A FENCED write, not a bare one. `fenced_write_from_batch`, not `from_batch`: the
        // latter yields a `CatalogTxn`, and sharing the catalog's wire tag would replay user
        // data through the catalog path and inherit its serializing lock.
        //
        // The fence carries the gate's own verdict into ordered apply, which is the first
        // point at which a split that committed after the gate ran is visible. The
        // adjudicator refuses the write there rather than letting it land on a region that
        // has moved underneath it.
        let command = Command::fenced_write_from_batch(fence.into_region_fence(), &batch);
        // Success is judged on (term, index), never on elapsed time; a typed
        // Replaced re-proposes within the deadline (provably-never-applied is
        // the one safely retryable outcome — see `propose_and_wait`).
        propose_and_wait(&self.driver, &command, RAW_APPLY_DEADLINE)
    }
}

/// How long to wait for a raw write's own `(term, index)` to reach the state machine.
const RAW_APPLY_DEADLINE: Duration = Duration::from_secs(10);

/// How long an establishing read waits for its quorum barrier + local apply
/// catch-up before failing typed (`ReadIndexError::Unconfirmed`). Bounds the
/// worst-case read latency when the quorum is unreachable (an isolated
/// self-believed leader waits this long, then fails — it never serves stale).
const READ_BARRIER_DEADLINE: Duration = Duration::from_secs(2);

/// Bound inline batch lookup work. Larger low-level requests retain their
/// existing synchronous behavior; this is a scheduling bound, not a wire limit.
const MAX_RESIDENT_BATCH_READ_KEYS: usize = 256;
const MAX_RESIDENT_BATCH_READ_BYTES: usize = 1024 * 1024;

impl RawApi for RuntimeBackend {
    fn prepare_raw_write(
        self: Arc<Self>,
        ctx: RequestContext,
        operation: crate::api::RawWrite,
    ) -> crate::api::RawWritePreparation {
        Box::new(move || {
            self.ensure_serving()?;
            let (fence, batch) = match operation {
                crate::api::RawWrite::Put { key, value } => {
                    let fence = self.validated_context(&ctx, KeySpan::Point(&key))?;
                    let batch = RawExecutor.plan_put(
                        ctx.keyspace,
                        &key,
                        value,
                        RawWriteOptions::default(),
                    )?;
                    (fence, batch)
                }
                crate::api::RawWrite::BatchPut(pairs) => {
                    let fence = self.validated_context(&ctx, KeySpan::BatchPairs(&pairs))?;
                    let batch = RawExecutor.plan_batch_put(
                        ctx.keyspace,
                        &pairs,
                        RawWriteOptions::default(),
                    )?;
                    (fence, batch)
                }
                crate::api::RawWrite::Delete { key } => {
                    let fence = self.validated_context(&ctx, KeySpan::Point(&key))?;
                    let batch = RawExecutor.plan_delete(ctx.keyspace, &key)?;
                    (fence, batch)
                }
            };
            if batch.is_empty() {
                return Ok(
                    Box::pin(async { Ok(AppliedPosition { term: 0, index: 0 }) })
                        as crate::api::RawWriteCompletion,
                );
            }
            // Keep one command and one fence across the exact same replacement
            // policy as commit_batch. Submission stays on this blocking worker.
            let command = Arc::new(Command::fenced_write_from_batch(
                fence.into_region_fence(),
                &batch,
            ));
            let started = Instant::now();
            let deadline = started + RAW_APPLY_DEADLINE;
            let first = self.driver.propose_with_async_wait(&command, deadline);
            let first = match first {
                Ok(value) => value,
                Err(error) => {
                    let outcome = if matches!(error, Error::NotLeader { .. }) {
                        kv9_common::metrics::Outcome::Rejected
                    } else {
                        kv9_common::metrics::Outcome::Error
                    };
                    self.driver
                        .metrics()
                        .logical_proposal_wait
                        .record(started.elapsed(), outcome);
                    return Err(error);
                }
            };
            Ok(Box::pin(async move {
                let (result, outcome) =
                    finish_async_proposal(self.driver.clone(), command, first, deadline).await;
                self.driver
                    .metrics()
                    .logical_proposal_wait
                    .record(started.elapsed(), outcome);
                result
            }) as crate::api::RawWriteCompletion)
        })
    }

    fn prepare_raw_get(
        self: Arc<Self>,
        ctx: RequestContext,
        key: UserKey,
    ) -> crate::api::RawReadPreparation<Option<Value>> {
        // The published endpoint predicate is false through bootstrap/recovery.
        // Its cold path retains the original synchronous lifecycle/error order.
        if !self.endpoint_ready.load(Ordering::Acquire) {
            return Box::pin(async move {
                Ok(crate::api::RawReadJob::Blocking(Box::new(move || {
                    self.raw_get(&ctx, &key)
                })))
            });
        }
        Box::pin(async move {
            let established = self.driver.read_barrier_async(READ_BARRIER_DEADLINE).await;
            self.finish_prepared_get(ctx, key, established)
        })
    }

    fn raw_get(&self, ctx: &RequestContext, key: &[u8]) -> Result<Option<Value>> {
        self.ensure_serving()?;
        let view = self.established_read(ctx, KeySpan::Point(key))?;
        let read = LeaderRead::new(view.as_ref(), true, self.driver.status().leader_id)?;
        RawExecutor.get(&read, ctx.keyspace, key)
    }

    fn prepare_raw_batch_get(
        self: Arc<Self>,
        ctx: RequestContext,
        keys: Vec<UserKey>,
    ) -> crate::api::RawReadPreparation<Vec<Option<Value>>> {
        if !self.endpoint_ready.load(Ordering::Acquire)
            || keys.len() > MAX_RESIDENT_BATCH_READ_KEYS
            || keys
                .iter()
                .try_fold(MAX_RESIDENT_BATCH_READ_BYTES, |left, key| {
                    left.checked_sub(key.len())
                })
                .is_none()
        {
            return Box::pin(async move {
                Ok(crate::api::RawReadJob::Blocking(Box::new(move || {
                    self.raw_batch_get(&ctx, &keys)
                })))
            });
        }
        Box::pin(async move {
            let established = self.driver.read_barrier_async(READ_BARRIER_DEADLINE).await;
            self.finish_prepared_batch_get(ctx, keys, established)
        })
    }

    fn raw_batch_get(&self, ctx: &RequestContext, keys: &[UserKey]) -> Result<Vec<Option<Value>>> {
        self.ensure_serving()?;
        let view = self.established_read(ctx, KeySpan::BatchKeys(keys))?;
        let read = LeaderRead::new(view.as_ref(), true, self.driver.status().leader_id)?;
        RawExecutor.batch_get(&read, ctx.keyspace, keys)
    }

    fn raw_put(&self, ctx: &RequestContext, key: UserKey, value: Value) -> Result<AppliedPosition> {
        self.ensure_serving()?;
        let fence = self.validated_context(ctx, KeySpan::Point(&key))?;
        let plan = RawExecutor.plan_put(ctx.keyspace, &key, value, RawWriteOptions::default())?;
        self.commit_batch(fence, plan)
    }

    fn raw_batch_put(
        &self,
        ctx: &RequestContext,
        pairs: &[(UserKey, Value)],
    ) -> Result<AppliedPosition> {
        self.ensure_serving()?;
        let fence = self.validated_context(ctx, KeySpan::BatchPairs(pairs))?;
        // One batch ⇒ one entry ⇒ all of these land together or none do.
        let plan = RawExecutor.plan_batch_put(ctx.keyspace, pairs, RawWriteOptions::default())?;
        self.commit_batch(fence, plan)
    }

    fn raw_delete(&self, ctx: &RequestContext, key: &[u8]) -> Result<AppliedPosition> {
        self.ensure_serving()?;
        let fence = self.validated_context(ctx, KeySpan::Point(key))?;
        let plan = RawExecutor.plan_delete(ctx.keyspace, key)?;
        self.commit_batch(fence, plan)
    }

    fn raw_scan(
        &self,
        ctx: &RequestContext,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<(UserKey, Value)>> {
        self.ensure_serving()?;
        let view = self.established_read(ctx, KeySpan::Range { start, end })?;
        let read = LeaderRead::new(view.as_ref(), true, self.driver.status().leader_id)?;
        RawExecutor.scan(&read, ctx.keyspace, start, end, limit)
    }

    fn raw_delete_range(
        &self,
        ctx: &RequestContext,
        start: &[u8],
        end: &[u8],
    ) -> Result<DeleteRangeReceipt> {
        self.ensure_serving()?;
        // ONE barrier for the whole request, exchanged for ONE stable snapshot that every
        // chunk plans from (@Tess's ruling; neither my "skip the barrier in the planner" nor
        // @Rafa's "re-anchor per chunk on the previous receipt").
        //
        // What this buys is a statable promise: **every key already acknowledged when the
        // request arrived is deleted.** Keys written after the barrier may survive, and the
        // receipt still reports the non-atomic multi-chunk progress.
        //
        // The two rejected shapes both cost that sentence:
        //
        // - Re-establishing per chunk gives each round a *newer* view, so the delete set is a
        //   moving target: a concurrent insert lands inside the set if it falls after the
        //   cursor and escapes if before. Coverage then depends on scheduling and there is no
        //   promise left to state — plus it is one serial quorum round-trip per chunk
        //   (~977 for a million keys at RAW_DELETE_RANGE_CHUNK).
        // - Skipping the barrier entirely does NOT merely risk over-planning. A key committed
        //   before the request but not yet applied locally is absent from an unestablished
        //   snapshot, so it never enters a batch, is never proposed, and the layer-3 fence
        //   never sees it. **The fence adjudicates writes that are made; an omission is
        //   invisible to it.** That was my error, and it is why the entry barrier is required
        //   rather than merely nice.
        //
        // There is deliberately NO context check here. The loop revalidates on every round
        // including the first, so a second entry-point check would be a parallel path that
        // could drift from the one the loop applies — and the endpoint would then be
        // authorised by a rule the loop does not use.
        let barrier = self.driver.read_barrier(READ_BARRIER_DEADLINE)?;
        let hint = self.driver.status().leader_id;
        // The single established-view construction point in this scope. `established_view`
        // consumes the barrier, and `ReadBarrier` is neither Clone nor Copy, so a second view
        // cannot be built here at all — the loop below borrows this one or reads nothing.
        let view = self.established_view(barrier)?;
        let read = LeaderRead::new(view.as_ref(), true, hint)?;
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
                // Borrows the established view; the closure has no way to reach the
                // establishing seam, because the credential that unlocks it was consumed
                // above and cannot be reproduced.
                RawExecutor.plan_delete_range_chunk(
                    &read,
                    ctx.keyspace,
                    cursor,
                    start,
                    end,
                    RAW_DELETE_RANGE_CHUNK,
                )
            },
            // This chunk's own authorisation, minted by the revalidation two closures up and
            // moved in here. There is no way to write the version that reuses chunk 1's.
            |fence, batch| self.commit_batch(fence, batch),
        )
    }
}

impl TxnApi for RuntimeBackend {
    fn kv_begin(
        &self,
        ctx: &RequestContext,
        primary: kv9_txn::QualifiedKey,
    ) -> Result<kv9_txn::TxnDescriptor> {
        self.ensure_serving()?;
        self.node.kv_begin(ctx, primary)
    }

    fn kv_get(
        &self,
        ctx: &RequestContext,
        key: &[u8],
        transaction: &kv9_txn::TxnDescriptor,
    ) -> Result<Option<Value>> {
        self.ensure_serving()?;
        self.node.kv_get(ctx, key, transaction)
    }
    fn kv_batch_get(
        &self,
        ctx: &RequestContext,
        keys: &[UserKey],
        transaction: &kv9_txn::TxnDescriptor,
    ) -> Result<Vec<Option<Value>>> {
        self.ensure_serving()?;
        self.node.kv_batch_get(ctx, keys, transaction)
    }
    fn kv_scan(
        &self,
        ctx: &RequestContext,
        start: &[u8],
        end: &[u8],
        limit: usize,
        transaction: &kv9_txn::TxnDescriptor,
    ) -> Result<Vec<(UserKey, Value)>> {
        self.ensure_serving()?;
        self.node.kv_scan(ctx, start, end, limit, transaction)
    }
    fn kv_prewrite(
        &self,
        ctx: &RequestContext,
        mutations: &[(UserKey, Option<Value>)],
        transaction: &kv9_txn::TxnDescriptor,
    ) -> Result<()> {
        self.ensure_serving()?;
        self.node.kv_prewrite(ctx, mutations, transaction)
    }
    fn kv_commit(
        &self,
        ctx: &RequestContext,
        keys: &[UserKey],
        transaction: &kv9_txn::TxnDescriptor,
    ) -> Result<()> {
        self.ensure_serving()?;
        self.node.kv_commit(ctx, keys, transaction)
    }
    fn kv_pessimistic_lock(
        &self,
        ctx: &RequestContext,
        keys: &[UserKey],
        transaction: &kv9_txn::TxnDescriptor,
    ) -> Result<()> {
        self.ensure_serving()?;
        self.node.kv_pessimistic_lock(ctx, keys, transaction)
    }
    fn kv_pessimistic_rollback(
        &self,
        ctx: &RequestContext,
        keys: &[UserKey],
        transaction: &kv9_txn::TxnDescriptor,
    ) -> Result<()> {
        self.ensure_serving()?;
        self.node.kv_pessimistic_rollback(ctx, keys, transaction)
    }
    fn kv_resolve_lock(
        &self,
        ctx: &RequestContext,
        transaction: &kv9_txn::TxnDescriptor,
    ) -> Result<()> {
        self.ensure_serving()?;
        self.node.kv_resolve_lock(ctx, transaction)
    }
    fn kv_cleanup(
        &self,
        ctx: &RequestContext,
        key: &[u8],
        transaction: &kv9_txn::TxnDescriptor,
    ) -> Result<()> {
        self.ensure_serving()?;
        self.node.kv_cleanup(ctx, key, transaction)
    }
    fn kv_check_txn_status(
        &self,
        ctx: &RequestContext,
        transaction: &kv9_txn::TxnDescriptor,
    ) -> Result<kv9_txn::TxnStatus> {
        self.ensure_serving()?;
        self.node.kv_check_txn_status(ctx, transaction)
    }
}

/// The things a test may substitute at startup — and the reason each one has to be a
/// startup parameter rather than something a test does afterwards.
///
/// `Default` is exactly production, so the public entry points are visibly unaffected: they
/// pass `StartOverrides::default()` and there is no other way in.
#[derive(Default)]
struct StartOverrides {
    /// `None` installs [`CatalogFenceAdjudicator`], which is what every live node runs.
    ///
    /// It must be chosen here because the driver pump starts on the very next lines and
    /// nothing downstream — not even code holding the finished `RuntimeBackend` — can
    /// substitute it afterwards. Swapping an adjudicator on a *running* node is not merely
    /// unsupported but unsound: whether one is installed is node-local configuration, so a
    /// committed fenced entry arriving mid-swap would be judged by different rules on
    /// different replicas. A test that needs to observe what the production write path hands
    /// the fence therefore has to be here, before the pump, or nowhere.
    #[allow(clippy::type_complexity)]
    adjudicator:
        Option<Box<dyn FnOnce(Arc<Node<WalEngine>>) -> Arc<dyn kv9_raft::FenceAdjudicator>>>,

    /// An **already-bound** listener whose ownership moves into the runtime.
    ///
    /// `None` binds `config.addr` as usual. A test passes `Some`, having bound
    /// `127.0.0.1:0` and asked the kernel which port it got — the same idiom as the five
    /// existing sites (`raft/transport.rs`, `raft/grpc.rs` ×3, `server/grpc.rs`).
    ///
    /// The ownership transfer is the point, not the port number. Reading `local_addr()`,
    /// dropping the listener, and letting startup rebind that port reintroduces exactly the
    /// race the kernel allocation removed: between the drop and the rebind any other process
    /// may take it, so the suite fails intermittently under load and passes when run alone —
    /// the worst possible failure signature. Holding the listener the whole way across means
    /// the port is never unbound and never reserved-then-reclaimed.
    listener: Option<std::net::TcpListener>,
    /// Stop at the durable activation boundary before any Raft tick. This is
    /// compiled only into unit tests; default production startup never defers.
    #[cfg(test)]
    defer_owner: bool,
}

/// The committed Active row and durable ConfState must name this exact store.
/// A mismatched local binding is a recovery error, not a fresh admission.
fn local_member_is_active(
    node: &Node<WalEngine>,
    driver: &NodeDriver<DiskRaftStorage, WalEngine>,
    incarnation: StoreIncarnation,
) -> Result<bool> {
    let status = driver.status();
    let txn = node.meta_raft.store.begin()?;
    let Some(row) = txn.get(&NODES_DESC, &[memcmp_uint(node.id.0)])? else {
        return Ok(false);
    };
    if !matches!(row.value.get(ColumnId(5)), Some(ColumnValue::Bytes(bytes)) if bytes.as_slice() == incarnation.as_bytes())
    {
        return Err(Error::Config(
            "local registered store incarnation does not match durable store identity".into(),
        ));
    }
    Ok(
        (status.voters.contains(&node.id.0) || status.learners.contains(&node.id.0))
            && matches!(row.value.get(ColumnId(3)), Some(ColumnValue::Uint(2))),
    )
}

#[cfg(test)]
type RouteSnapshotGate = (std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>);

/// A running real-process metadata member.
pub struct NodeRuntime {
    node: Arc<Node<WalEngine>>,
    public_admission: Arc<crate::admission::PublicAdmission>,
    raft_io_metrics: Arc<kv9_common::metrics::WalIoMetrics>,
    metrics_exporter: crate::observability::MetricsExporter,
    driver: Arc<NodeDriver<DiskRaftStorage, WalEngine>>,
    transport: Arc<GrpcTransport>,
    discovery: Arc<RuntimeDiscovery>,
    driver_thread: Option<std::thread::JoinHandle<()>>,
    remote_storage: Option<crate::remote_storage::RemoteStorage>,
    grpc_runtime: tokio::runtime::Runtime,
    // Own the executor until public tasks and their transport references stop.
    _peer_runtime: tokio::runtime::Runtime,
    #[cfg(feature = "rpc-experiment")]
    experimental_rpc: Option<crate::rpc_experiment::ExperimentalServer>,
    #[cfg(feature = "rpc-experiment")]
    streaming_rpc: Option<crate::point_stream::StreamServer>,
    grpc_shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    point_shutdown: tokio_util::sync::CancellationToken,
    grpc_server: Option<tokio::task::JoinHandle<std::result::Result<(), tonic::transport::Error>>>,
    cluster_token: String,
    voters: Vec<NodeId>,
    seeds: Vec<SeedPeer>,
    discovery_observations: BTreeMap<u64, DiscoveryObservation>,
    advertised_endpoint_observation: Option<DiscoveryObservation>,
    advertised_addr: std::net::SocketAddr,
    endpoint_ready: Arc<AtomicBool>,
    endpoint_recovery: EndpointRecovery,
    registration_observation: RegistrationObservation,
    registration_seed_cursor: RegistrationSeedCursor,
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
    /// Shared with this node's ClusterAuthenticator; written exactly once,
    /// by `advance_registration` on the typed `Registered` outcome.
    catchup_capability: Arc<std::sync::Mutex<Option<CatchupCapability>>>,
    next_discovery: Instant,
    next_advertised_endpoint_probe: Instant,
    #[cfg(test)]
    route_snapshot_gate: std::sync::Mutex<Option<RouteSnapshotGate>>,
    _store_guard: kv9_common::store_lifecycle::StoreGuard,
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
        Self::start_core(
            id,
            config,
            auth,
            root,
            store_identity,
            join_ticket,
            StartOverrides::default(),
        )
    }

    /// The whole of startup, with the two things a test must be able to substitute passed
    /// in rather than hard-wired. See [`StartOverrides`] for why these two and no others.
    ///
    /// Deliberately private, and deliberately not behind a Cargo feature: substitution is
    /// not a product capability. Both public entry points pass `StartOverrides::default()`,
    /// which is exactly production, and nothing outside this module can reach here.
    #[allow(clippy::too_many_arguments)]
    fn start_core(
        id: NodeId,
        config: Config,
        auth: RuntimeAuth,
        root: RootDescriptor,
        store_identity: StoreIdentity,
        join_ticket: Option<&str>,
        overrides: StartOverrides,
    ) -> Result<Self> {
        let public_limits = crate::admission::PublicApiLimits::from_env()?;
        root.validate()?;
        store_identity.verify(&root, id)?;
        config.validate()?;
        let advertised_override = config
            .advertise_addr
            .as_deref()
            .map(crate::endpoints::socket)
            .transpose()?;
        auth.validate()?;
        // The address the config REQUESTS. What we end up listening on is read back off the
        // socket further down and shadows this — see the `local_addr()` call after the bind.
        let requested_addr: std::net::SocketAddr = config.addr.parse().map_err(|_| {
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
        // The explicit advertisement (or root address by default) is canonical; `addr` is
        // only the local listener bind. They are intentionally allowed to
        // differ (for example a stable Kubernetes Service ClusterIP advertising
        // a Pod that binds 0.0.0.0). Peer identity never comes from the bind
        // address.
        let joining = seeds.iter().all(|seed| seed.node_id != id);
        let join_ticket_sha256 = join_ticket.map(|ticket| RootDigest::sha256(ticket.as_bytes()));

        let data_dir = PathBuf::from(&config.data_dir);
        let mut store_guard = kv9_common::store_lifecycle::StoreGuard::lock(&data_dir)?;
        let legacy = store_guard.record().is_none();
        let recover_only = if legacy {
            let (saved_root, saved_identity) = kv9_common::load_root_bundle(&data_dir)?;
            if saved_root != root || saved_identity != store_identity {
                return Err(Error::Config(
                    "legacy recovery requires the matching durable identity bundle".into(),
                ));
            }
            true
        } else {
            let record = store_guard.verify(&store_identity)?;
            matches!(
                record.phase,
                kv9_common::store_lifecycle::StorePhase::Active(_)
            )
        };
        persist_root_bundle(&data_dir, &root, &store_identity)?;
        if !legacy {
            store_guard.bind(&root, &store_identity)?;
            store_guard.fence_legacy_writers()?;
        }
        let voters: Vec<NodeId> = seeds.iter().map(|seed| seed.node_id).collect();
        let endpoint_recovery = EndpointRecovery::load(&data_dir, &store_identity)?;
        let endpoint_ready = Arc::new(AtomicBool::new(false));
        let voter_fp = voter_set_fingerprint(
            &seeds
                .iter()
                .map(|seed| (seed.node_id.0, seed.addr))
                .collect::<Vec<_>>(),
        );
        let voter_ids: Vec<u64> = voters.iter().map(|node| node.0).collect();
        let storage = if recover_only {
            DiskRaftStorage::recover(&data_dir.join("raft"))?
        } else {
            DiskRaftStorage::open(&data_dir.join("raft"), &voter_ids)?.0
        };
        let raft_io_metrics = storage.io_metrics();
        let remote = crate::remote_storage::prepare_remote(
            &data_dir,
            root.cluster_id.to_string(),
            &storage,
        )?;
        let catalog_path = data_dir.join("catalog.wal");
        if let Some(checkpoint) = WalEngine::checkpoint_reference(&catalog_path)? {
            let bytes = checkpoint.encode()?;
            if checkpoint.scope.cluster != root.cluster_id.to_string()
                || checkpoint.scope.region != META_REGION_0.0
                || !storage.has_committed_checkpoint(&bytes)?
            {
                return Err(Error::Engine(
                    "checkpoint is not certified by this cluster's committed Raft log".into(),
                ));
            }
        }
        let (engine, replay) = WalEngine::open_with_uploader(
            &catalog_path,
            remote.as_ref().map(|config| config.uploader.as_ref()),
        )?;
        // Upgrade the old in-band index exactly once, using the durable Raft log
        // for its term. Atomically rewrite legacy state into ONE v2 record.
        let legacy_key = b"\x00kv9\x00applied_index";
        if let Some(bytes) = engine.get(kv9_engine::ColumnFamily::Default, legacy_key)? {
            if engine.applied_position()? != kv9_engine::DurableAppliedPosition::AppliedNothing {
                return Err(Error::Engine(
                    "legacy applied marker coexists with a positioned WAL".into(),
                ));
            }
            let index = u64::from_be_bytes(
                bytes
                    .try_into()
                    .map_err(|_| Error::Engine("invalid legacy applied marker".into()))?,
            );
            let term = storage.committed_term(index)?;
            engine.upgrade_legacy_applied(legacy_key, AppliedPosition { term, index })?;
        }
        if let kv9_engine::DurableAppliedPosition::AppliedThrough(at) = engine.applied_position()? {
            if storage.committed_term(at.index)? != at.term {
                return Err(Error::Engine(
                    "engine applied term disagrees with committed Raft history".into(),
                ));
            }
        }
        // The store guard excludes other writers. Publish the segmented layout
        // only after all recovered progress has committed Raft authority, before
        // starting the peer or exposing Serving.
        engine.enable_segmentation()?;
        let peer = Arc::new(RaftPeer::with_storage(id, META_REGION_0, storage)?);
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
        if legacy {
            if local_identity.is_none() {
                return Err(Error::Config("legacy store lacks a committed root certificate; independent store preparation is required".into()));
            }
            store_guard.adopt_certified_recovery(&root, &store_identity)?;
        } else {
            store_guard.activate(&store_identity)?;
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
        // Durable Raft history alone is not a completed catalog. An exact
        // original store may resume first formation after an activation or
        // election crash. The lifecycle check above prevents replacement-disk
        // reuse; advance_initialization drains the current-term barrier and
        // checks the catalog under the planner mutex before a term-fenced
        // proposal. A retained init therefore wins over any new seed plan.
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
        let grpc_runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            // Poll socket readiness while public handlers keep workers busy.
            // Inbound peer streams still share the public RPC listener.
            .event_interval(8)
            .enable_all()
            .build()
            .map_err(|error| Error::Config(format!("create gRPC runtime: {error}")))?;
        // Outbound Raft tasks keep their original bounded queues and watchdogs,
        // but do not compete with public handlers in the same runnable queue.
        let peer_runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("kv9-raft-tx")
            .event_interval(8)
            .enable_all()
            .build()
            .map_err(|error| Error::Config(format!("create Raft transport runtime: {error}")))?;
        let transport = GrpcTransport::new(
            id,
            Some(auth.cluster_token.clone()),
            peer_runtime.handle().clone(),
            root.digest(),
        );
        for seed in &seeds {
            if seed.node_id != id {
                transport.register_peer(seed.node_id, seed.addr);
            }
        }
        // The fence adjudicator is installed BEFORE the driver starts pumping, which is the
        // only ordering that works: a committed `Command::Fenced` reaching a state machine
        // without one is a typed apply error by design, because whether an adjudicator is
        // installed is node-local configuration and must never become a verdict that
        // differently-configured replicas would not share. Installing it here means the
        // window in which that error is reachable does not exist on a live node.
        let mut state_machine = MemStateMachine::with_engine(engine)?;
        state_machine.set_fence_adjudicator(match overrides.adjudicator {
            Some(factory) => factory(node.clone()),
            None => Arc::new(CatalogFenceAdjudicator::new(node.clone())),
        });
        let driver = NodeDriver::new(peer, transport.clone(), state_machine)?;
        // Initial root voters retain the existing first-formation/stable-log
        // contract. A dynamic member must carry an exact local incarnation
        // binding or obtain its own registration receipt before Raft runs.
        let recovered_member = joining
            && local_identity.is_some()
            && local_member_is_active(&node, &driver, store_identity.store_incarnation)?;
        let authorized = !joining || recovered_member;
        let status_path = data_dir.join("status");

        let backend = Arc::new(RuntimeBackend {
            node: node.clone(),
            driver: driver.clone(),
            transport: transport.clone(),
            endpoint_ready: endpoint_ready.clone(),
            initial_voters: seeds.iter().map(|s| (s.node_id, s.addr)).collect(),
        });
        let client_authenticator = Arc::new(TokenAuthenticator::new(auth.client_tokens)?);
        let public_api = Kv9Grpc::with_limits(backend.clone(), public_limits)?;
        let public_admission = public_api.admission();
        #[cfg(feature = "rpc-experiment")]
        let experimental_rpc = match std::env::var("KV9_RPC_EXPERIMENT_ADDR") {
            Ok(address) => {
                let address = address
                    .parse()
                    .map_err(|_| Error::Config("invalid experimental RPC address".into()))?;
                Some(
                    grpc_runtime
                        .block_on(crate::rpc_experiment::start(
                            address,
                            public_api.clone(),
                            client_authenticator.clone(),
                        ))
                        .map_err(|error| {
                            Error::Config(format!("start experimental RPC listener: {error}"))
                        })?,
                )
            }
            Err(std::env::VarError::NotPresent) => None,
            Err(_) => {
                return Err(Error::Config(
                    "experimental RPC address is not Unicode".into(),
                ))
            }
        };
        #[cfg(feature = "rpc-experiment")]
        let streaming_rpc = match std::env::var("KV9_GRPC_STREAM_EXPERIMENT_ADDR") {
            Ok(address) => {
                let address = address
                    .parse()
                    .map_err(|_| Error::Config("invalid streaming RPC address".into()))?;
                Some(
                    grpc_runtime
                        .block_on(crate::point_stream::start(
                            address,
                            public_api.clone(),
                            client_authenticator.clone(),
                        ))
                        .map_err(|error| {
                            Error::Config(format!("start streaming RPC listener: {error}"))
                        })?,
                )
            }
            Err(std::env::VarError::NotPresent) => None,
            Err(_) => return Err(Error::Config("streaming RPC address is not Unicode".into())),
        };
        let point_shutdown = tokio_util::sync::CancellationToken::new();
        let point_service = crate::point_stream::service(
            public_api.clone(),
            client_authenticator.clone(),
            point_shutdown.clone(),
        );
        let public_service = public_api.authenticated_service(client_authenticator);
        let catchup_capability: Arc<std::sync::Mutex<Option<CatchupCapability>>> =
            Arc::new(std::sync::Mutex::new(None));
        let cluster_authenticator = Arc::new(ClusterAuthenticator {
            expected_token: Arc::from(auth.cluster_token.clone()),
            voters: Arc::new(voters.iter().copied().collect()),
            node: node.clone(),
            driver: driver.clone(),
            catchup: catchup_capability.clone(),
        });
        let raft_service = RaftGrpcService::new(id, transport.inbox_sender(), discovery.clone())
            .with_registration(backend as Arc<dyn RegistrationBackend>);
        let raft_service = tonic::service::interceptor::InterceptedService::new(
            Kv9RaftServer::new(raft_service),
            AuthInterceptor::new(cluster_authenticator),
        );
        // An injected listener is adopted, never rebound: it arrives already bound and its
        // ownership moves in here, so the port is never released. `from_std` needs a reactor
        // to register with, hence the runtime guard, and needs the socket non-blocking —
        // std's default is blocking, and a blocking socket handed to tonic stalls the
        // accept loop rather than failing, which is the kind of bug that looks like a
        // hanging test. See [`StartOverrides::listener`].
        let listener = match overrides.listener {
            Some(bound) => {
                bound.set_nonblocking(true).map_err(|error| {
                    Error::Config(format!("injected listener to non-blocking: {error}"))
                })?;
                let _guard = grpc_runtime.enter();
                tokio::net::TcpListener::from_std(bound)
                    .map_err(|error| Error::Config(format!("adopt injected listener: {error}")))?
            }
            None => grpc_runtime
                .block_on(tokio::net::TcpListener::bind(requested_addr))
                .map_err(|error| {
                    Error::Config(format!("bind gRPC listener {requested_addr}: {error}"))
                })?,
        };
        // Where we are ACTUALLY listening, which is not necessarily `config.addr` (@Tess).
        //
        // An adopted listener was bound by someone else, so its address is the kernel's answer
        // rather than the config's request — with `127.0.0.1:0` the config cannot even name it.
        // Keeping the config value here made the runtime report, and the status file publish,
        // an address nothing was listening on. Reading it back off the socket is the only
        // source that cannot disagree with reality.
        //
        // Deliberately NOT the advertised endpoint: a Pod binding `0.0.0.0` or a node behind
        // NAT legitimately advertises something else, and `observe_advertised_endpoint` probes
        // that independently. Bind address and advertised address stay separate.
        let addr = listener
            .local_addr()
            .map_err(|error| Error::Config(format!("read bound listener address: {error}")))?;
        let advertised_addr = advertised_override
            .or_else(|| root.voter(id).map(|voter| voter.addr))
            .unwrap_or(addr);
        crate::endpoints::socket(&advertised_addr.to_string())?;
        let incoming = grpc_incoming(listener);
        let (grpc_shutdown_tx, grpc_shutdown_rx) = tokio::sync::oneshot::channel();
        let grpc_server = grpc_runtime.spawn(
            tonic::transport::Server::builder()
                .add_service(public_service)
                .add_service(point_service)
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
        let advertised_endpoint_observation = Some(DiscoveryObservation::new(
            SeedPeer {
                node_id: id,
                addr: advertised_addr,
            },
            false,
        ));

        let remote_storage = remote
            .map(|uploader| {
                crate::remote_storage::RemoteStorage::start(node.clone(), driver.clone(), uploader)
            })
            .transpose()?;

        // No fallible startup work may follow owner creation. In particular,
        // a bind/auth/checkpoint setup failure must drop every store reference
        // before the local guard unlocks; dropping a JoinHandle detaches it.
        let driver_thread = if authorized {
            discovery.authorize_raft();
            #[cfg(test)]
            let defer_owner = overrides.defer_owner;
            #[cfg(not(test))]
            let defer_owner = false;
            if defer_owner {
                None
            } else {
                Some(driver.spawn(TICK)?)
            }
        } else {
            None
        };
        Ok(Self {
            node,
            driver,
            transport,
            discovery,
            driver_thread,
            remote_storage,
            grpc_runtime,
            _peer_runtime: peer_runtime,
            public_admission,
            raft_io_metrics,
            metrics_exporter: crate::observability::MetricsExporter::new(&data_dir, id.0),
            #[cfg(feature = "rpc-experiment")]
            experimental_rpc,
            #[cfg(feature = "rpc-experiment")]
            streaming_rpc,
            grpc_shutdown: Some(grpc_shutdown_tx),
            grpc_server: Some(grpc_server),
            point_shutdown,
            cluster_token: auth.cluster_token,
            voters,
            seeds,
            discovery_observations,
            advertised_endpoint_observation,
            advertised_addr,
            endpoint_ready,
            endpoint_recovery,
            registration_observation: RegistrationObservation::new(),
            registration_seed_cursor: RegistrationSeedCursor::default(),
            data_dir,
            status_path,
            addr,
            root,
            store_identity,
            join_ticket_sha256,
            campaign_started: false,
            initial_proposal: None,
            registration_receipt: None,
            catchup_capability,
            next_discovery: Instant::now(),
            next_advertised_endpoint_probe: Instant::now(),
            #[cfg(test)]
            route_snapshot_gate: std::sync::Mutex::new(None),
            _store_guard: store_guard,
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
        // Capture and installation share the planner lock with registration
        // and operator updates. A delayed snapshot cannot overwrite a route
        // installed by a newer committed catalog transaction.
        let _guard = self.node.meta_raft.lock_catalog_txn();
        let txn = self.node.meta_raft.store.begin()?;
        let rows = txn.scan(&NODES_DESC, MAX_PHASE1_NODES + 1)?;
        #[cfg(test)]
        if let Some((captured, resume)) = self.route_snapshot_gate.lock().unwrap().take() {
            captured.send(()).unwrap();
            resume.recv().unwrap();
        }
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
            let endpoint = kv9_meta::endpoint::node_endpoint(&txn, node)?
                .ok_or_else(|| Error::Config("scanned endpoint row disappeared".into()))?;
            self.transport
                .register_catalog_peer(node, endpoint.address, endpoint.generation)?;
        }
        Ok(())
    }

    /// The successful registration endpoint bootstraps a missing route only.
    /// Once this replica has an applied row, that row owns the route. Share
    /// the catalog lock so this fallback cannot race a newer local installer.
    fn install_registration_response_route(
        &self,
        via: (NodeId, std::net::SocketAddr),
    ) -> Result<()> {
        let _guard = self.node.meta_raft.lock_catalog_txn();
        let txn = self.node.meta_raft.store.begin()?;
        let applied_endpoint = kv9_meta::endpoint::node_endpoint(&txn, via.0)?;
        if let Some(endpoint) = applied_endpoint {
            self.transport
                .register_catalog_peer(via.0, endpoint.address, endpoint.generation)?;
        } else {
            self.transport.register_peer(via.0, via.1);
        }
        Ok(())
    }

    fn check_grpc_server(&mut self) -> Result<()> {
        #[cfg(feature = "rpc-experiment")]
        if self
            .streaming_rpc
            .as_ref()
            .is_some_and(|server| server.task.is_finished())
        {
            return Err(Error::Raft("streaming RPC listener stopped".into()));
        }
        #[cfg(feature = "rpc-experiment")]
        if self
            .experimental_rpc
            .as_ref()
            .is_some_and(|server| server.task.is_finished())
        {
            return Err(Error::Raft("experimental RPC listener stopped".into()));
        }

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
        self.advance_membership_bootstrap()?;
        let serving = self
            .node
            .meta
            .lock()
            .expect("meta poisoned")
            .bootstrap
            .is_serving();
        let ready = self.advance_endpoint_recovery()?;
        self.endpoint_ready
            .store(serving && ready, Ordering::Release);
        if serving && !ready {
            self.node
                .meta
                .lock()
                .expect("meta poisoned")
                .bootstrap
                .on_event(BootstrapEvent::EndpointUnconfirmed)?;
        }
        Ok(())
    }

    fn advance_membership_bootstrap(&mut self) -> Result<()> {
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
        let status = self.driver.status();
        if status.role != Role::Leader {
            // A lost-term proposal may still commit under the next leader.
            // Drop only the local waiter; a later attempt must first drain
            // prior terms and inspect the catalog before planning again.
            self.initial_proposal = None;
            self.node
                .meta
                .lock()
                .expect("meta poisoned")
                .bootstrap
                .on_event(BootstrapEvent::LostElection)?;
            return Ok(());
        }
        if let Some(cluster_id) = self.node.local_cluster_identity()? {
            return self.finish_initialization(cluster_id);
        }
        if self
            .initial_proposal
            .is_some_and(|(proposal, _)| proposal.term != status.term)
        {
            // Leadership can be lost and regained between lifecycle ticks.
            self.initial_proposal = None;
        }
        if self.initial_proposal.is_none() {
            // This applies to the FIRST election as well as takeover. The
            // current-term election no-op proves every earlier committed
            // initialization has applied before we read an empty catalog.
            if self
                .driver
                .driver_applied()
                .is_none_or(|at| at.term != status.term)
            {
                return Ok(());
            }
            let node = self.node.clone();
            let _guard = node.meta_raft.lock_catalog_txn();
            let confirm = self.driver.status();
            if confirm.role != Role::Leader || confirm.term != status.term {
                return Ok(());
            }
            if let Some(cluster_id) = node.local_cluster_identity()? {
                return self.finish_initialization(cluster_id);
            }
            // Creation authority existed before Raft opened. Election chooses
            // which provisioned voter may submit the root; it never mints a
            // new identity and therefore cannot fork creation after a retry.
            let cluster_id = self.root.cluster_id;
            let cmd = self
                .node
                .build_initial_metadata_command_for_root(&self.voters, &self.root)?;
            match self.driver.propose_in_term(&cmd, status.term) {
                Ok(at) => self.initial_proposal = Some((at, cluster_id)),
                // Both are pre-append refusals. A subsequent lifecycle tick
                // rechecks the new term and rebuilds against its applied state.
                Err(Error::NotLeader { .. } | Error::WriteConflict(_)) => {}
                Err(error) => return Err(error),
            }
            return Ok(());
        }
        let (proposal, cluster_id) = self.initial_proposal.expect("set above");
        match self.driver.wait_applied(proposal, Duration::from_millis(1)) {
            Ok(ApplyWaitOutcome::Manifest { at, .. }) => Err(Error::Raft(format!(
                "initial-metadata proposal received a manifest verdict at term {} \
                 index {} — receipt correlation broke",
                at.term, at.index
            ))),
            Ok(ApplyWaitOutcome::Applied(_)) => self.finish_initialization(cluster_id),
            Ok(ApplyWaitOutcome::Replaced) => {
                self.initial_proposal = None;
                Ok(())
            }
            // The init command is a CatalogTxn; a fence verdict for it would
            // mean the log carries something this node never proposed.
            Ok(ApplyWaitOutcome::FenceRejected { at, region }) => Err(Error::Raft(format!(
                "bootstrap proposal position ({}, {}) reported a fence verdict for \
                 region {} — the init command is never fenced",
                at.term, at.index, region.0
            ))),
            // A one-millisecond condition poll coming back unconfirmed means
            // "pending" — but a FAILED driver is not pending: silently spinning
            // on a poisoned node was only tolerable when the two were the same
            // stringly error (task #30 made them distinct states).
            Err(ApplyWaitError::Unconfirmed { .. }) => Ok(()),
            Err(ApplyWaitError::Failed(e)) => Err(e),
        }
    }

    fn finish_initialization(&mut self, cluster_id: ClusterId) -> Result<()> {
        self.verify_certified_root()?;
        if cluster_id != self.root.cluster_id {
            return Err(Error::MetaNotReady(
                "initialized cluster identity does not match the certified root".into(),
            ));
        }
        write_init_marker(&self.data_dir)?;
        self.discovery.set_cluster_id(cluster_id);
        self.node
            .meta
            .lock()
            .expect("meta poisoned")
            .bootstrap
            .on_event(BootstrapEvent::MetadataInitialized { cluster_id })?;
        self.initial_proposal = None;
        Ok(())
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
            let seeds: Vec<(NodeId, std::net::SocketAddr)> = self
                .seeds
                .iter()
                .map(|seed| (seed.node_id, seed.addr))
                .collect();
            let seeds = self.registration_seed_cursor.order(&seeds);
            let handle = self.grpc_runtime.handle().clone();
            let node_id = self.node.id;
            let my_addr = self.advertised_addr.to_string();
            let identity = JoinIdentity {
                cluster_id,
                ticket_sha256: ticket,
                store_incarnation: self.store_identity.store_incarnation,
            };
            let token = self.cluster_token.clone();
            // One absolute window for the whole pass. Each new pass rotates
            // the first seed, so a blackhole cannot consume every seed's turn.
            let pass_deadline = Instant::now() + REGISTRATION_PASS_TIMEOUT;
            match registration_walk(
                &seeds,
                &mut self.registration_observation,
                pass_deadline,
                Instant::now,
                |addr, remaining| {
                    grpc_register(
                        &handle,
                        node_id,
                        addr,
                        &my_addr,
                        identity,
                        // Each dial consumes exactly the window's remainder —
                        // no per-dial re-grant of any kind. A slow candidate
                        // can eat the rest of THIS pass (the next run-loop
                        // pass retries); it can never enlarge the window.
                        remaining,
                        Some(token.clone()),
                    )
                },
            ) {
                WalkOutcome::Registered { receipt, via } => {
                    // The transport half of the fresh-joiner catch-22: the
                    // leader that minted this receipt must be routable
                    // BEFORE the catalog can name it, or this node's raft
                    // responses are dropped and catch-up never starts. The
                    // endpoint is the one this node just successfully
                    // registered against (not an unvalidated hint), and
                    // the applied catalog overwrites it on first sync —
                    // mirror of the leader-side eager register_peer.
                    self.install_registration_response_route(via)?;
                    // The ONLY install site of the catch-up capability, and
                    // it is inside the typed Registered arm: InvalidTicket
                    // and every Unconfirmed reason are structurally unable
                    // to install one.
                    *self
                        .catchup_capability
                        .lock()
                        .expect("catchup capability slot poisoned") =
                        Some(CatchupCapability::from_receipt(&receipt));
                    self.registration_receipt = Some(receipt);
                    // The sender catch-up capability and receipt are published
                    // before opening ingress. Only this typed success starts
                    // an owner for a previously unauthorized dynamic member.
                    self.discovery.authorize_raft();
                    if self.driver_thread.is_none() {
                        self.driver_thread = Some(self.driver.spawn(TICK)?);
                    }
                }
                // Recorded in the observation (typed) and the next run-loop
                // pass retries; InvalidTicket keeps the loop alive only
                // because a corrected ticket arrives via restart, and the
                // status line must keep showing the rejection meanwhile.
                WalkOutcome::InvalidTicket | WalkOutcome::InvalidIncarnation => {}
                WalkOutcome::Unconfirmed(reason) => {
                    // The walk's return value and the observation it filed
                    // must never disagree — status renders the observation.
                    debug_assert_eq!(self.registration_observation.last_walk, Some(reason));
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
            Ok(ApplyWaitOutcome::Applied(_)) => {}
            Ok(ApplyWaitOutcome::Manifest { at, .. }) => {
                return Err(Error::Raft(format!(
                    "registration receipt at term {} index {} carries a manifest \
                     verdict — receipt correlation broke",
                    at.term, at.index
                )))
            }
            Ok(ApplyWaitOutcome::Replaced) => {
                return Err(Error::Raft(format!(
                    "registration receipt at term {} index {} was overwritten",
                    exact.term, exact.index.0
                )))
            }
            // Registration is a CatalogTxn; a fence verdict here is a
            // protocol violation, not a state to continue past.
            Ok(ApplyWaitOutcome::FenceRejected { at, region }) => {
                return Err(Error::Raft(format!(
                    "registration receipt position ({}, {}) reported a fence \
                     verdict for region {} — registration is never fenced",
                    at.term, at.index, region.0
                )))
            }
            Err(ApplyWaitError::Unconfirmed { .. }) => return Ok(()),
            Err(ApplyWaitError::Failed(e)) => return Err(e),
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
        local_member_is_active(
            &self.node,
            &self.driver,
            self.store_identity.store_incarnation,
        )
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
        self.metrics_exporter.export(raft.fatal.is_some(), || {
            crate::observability::capture(
                &self.public_admission,
                &self.driver,
                &self.raft_io_metrics,
                &self.node.store.engine.io_metrics(),
            )
        });
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
        let mut body = format!(
            "pid={}\nnode_id={}\ncluster_id={}\nbootstrap_generation={}\nroot_digest={}\nstore_incarnation={}\nleader_id={}\nrole={}\nmeta_voters={}\nmeta_learners={}\npending_admissions={}\nconf_index={}\nterm={}\nraft_committed={}\napplied_index={}\napplied_term={}\n{}bootstrap_state={:?}\nadvertised_endpoint={}\nregistration_attempts={}\nregistration_errors={}\nregistration_last={}\nregistration_last_walk={}\nregistration_last_hint={}\nregistration_receipt_term={}\nregistration_receipt_index={}\n{}fatal={}\n",
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
            // Typed walk verdict rendered from the enum — no string parsing
            // anywhere between the walk and this line.
            self.registration_observation
                .last_walk
                .map(WalkTerminal::as_status)
                .unwrap_or("none"),
            self.registration_observation
                .last_hint
                .as_deref()
                .unwrap_or("none"),
            self.registration_receipt
                .as_ref()
                .map_or_else(|| "none".into(), |r| r.applied_term.to_string()),
            self.registration_receipt
                .as_ref()
                .map_or_else(|| "none".into(), |r| r.applied_index.to_string()),
            discovery_status,
            raft.fatal.as_deref().unwrap_or(""),
        );
        body.push_str(&self.public_admission.snapshot().status_lines());
        let applies = self.driver.async_apply_snapshot();
        body.push_str(&format!("raft_async_apply_limit={}\nraft_async_apply_queued={}\nraft_async_apply_in_flight={}\nraft_async_apply_peak={}\nraft_async_apply_stopped={}\n",
            applies.limit, applies.queued, applies.in_flight, applies.peak, applies.stopped));
        let reads = self.driver.async_read_snapshot();
        body.push_str(&format!("raft_async_read_limit={}\nraft_async_read_queued={}\nraft_async_read_active={}\nraft_async_read_in_flight={}\nraft_async_read_peak={}\nraft_async_read_stopped={}\n",
            reads.limit, reads.queued, reads.active, reads.in_flight, reads.peak, reads.stopped));
        body.push_str(&format!("raft_async_read_active_groups={}\nraft_async_read_inspected={}\nraft_async_read_group_attempts={}\nraft_async_read_admitted_groups={}\nraft_async_read_admitted_members={}\nraft_async_read_max_admitted_group={}\n",
            reads.active_groups, reads.inspected, reads.group_attempts, reads.admitted_groups, reads.admitted_members, reads.max_admitted_group));
        body.push_str(&format!(
            "raft_receive_authorized={}\nraft_owner_started={}\nlisten_addr={}\n",
            self.discovery.raft_receive_allowed(),
            self.driver_thread.is_some(),
            self.addr,
        ));
        body.push_str(&self.metrics_exporter.status_lines());
        body.push_str(&format!(
            "endpoint_ready={}\nendpoint_recovery_attempts={}\nendpoint_recovery_last={}\nendpoint_confirmation_term={}\nendpoint_confirmation_index={}\n",
            self.endpoint_ready.load(Ordering::Acquire),
            self.endpoint_recovery.attempts,
            self.endpoint_recovery.last,
            self.endpoint_recovery.receipt.map_or_else(|| "none".into(), |at| at.term.to_string()),
            self.endpoint_recovery.receipt.map_or_else(|| "none".into(), |at| at.index.to_string()),
        ));
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
        self.point_shutdown.cancel();
        #[cfg(feature = "rpc-experiment")]
        self.experimental_rpc.take();
        #[cfg(feature = "rpc-experiment")]
        self.streaming_rpc.take();
        self.remote_storage.take();
        self.driver.stop();
        if let Some(handle) = self.driver_thread.take() {
            let _ = handle.join();
        }
        if let Some(shutdown) = self.grpc_shutdown.take() {
            let _ = shutdown.send(());
        }
        // Abort rather than await: graceful drain waits for open inbound
        // connections to CLOSE, and live raft peers hold their HTTP/2
        // streams open indefinitely — awaiting here deadlocks any drop
        // performed while peers are still up (the fatal-error exit path,
        // and every in-process multi-node test). Both durable logs fsync
        // before visibility, so nothing is lost by cancelling the acceptor.
        if let Some(server) = self.grpc_server.take() {
            server.abort();
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
fn prepare_test_store(directory: &Path, id: NodeId) -> StoreIncarnation {
    let mut guard = kv9_common::store_lifecycle::StoreGuard::lock(directory).unwrap();
    guard.prepare(id).unwrap().incarnation
}

#[cfg(test)]
mod tests {
    mod async_batch_read_tests;

    #[tokio::test]
    async fn accepted_grpc_sockets_disable_nagle_without_rebinding() {
        use tokio_stream::StreamExt;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut incoming = grpc_incoming(listener);
        assert_eq!(incoming.local_addr().unwrap(), address);
        assert!(matches!(
            std::net::TcpListener::bind(address),
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse
        ));
        // Read the option from actual accepted kernel sockets. A builder flag
        // alone is insufficient when tonic receives a custom incoming stream.
        for _ in 0..2 {
            let client = tokio::time::timeout(
                Duration::from_secs(5),
                tokio::net::TcpStream::connect(address),
            )
            .await
            .unwrap()
            .unwrap();
            let accepted = tokio::time::timeout(Duration::from_secs(5), incoming.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert!(
                accepted.nodelay().unwrap(),
                "accepted gRPC socket retained Nagle buffering"
            );
            drop(accepted);
            drop(client);
        }
    }

    #[test]
    fn logical_observer_keeps_one_sample_across_retries_and_preserves_unknowns() {
        use kv9_common::metrics::{Latency, Outcome};
        let metric = Latency::default();
        let mut proposals = 0;
        let mut waits = 0;
        let budget = Duration::from_secs(5);
        let got = observe_proposal_wait(
            &metric,
            || {
                proposals += 1;
                Ok(ProposedAt {
                    term: 3,
                    index: kv9_raft::LogIndex(proposals),
                })
            },
            |at, remaining| {
                waits += 1;
                assert!(remaining <= budget);
                Ok(if waits < 3 {
                    ApplyWaitOutcome::Replaced
                } else {
                    ApplyWaitOutcome::Applied(AppliedPosition {
                        term: at.term,
                        index: at.index.0,
                    })
                })
            },
            budget,
        )
        .unwrap();
        assert_eq!((proposals, waits, got.index), (3, 3, 3));
        assert_eq!(
            metric.snapshot().outcomes[Outcome::Success as usize].count,
            1
        );
        assert_eq!(
            metric.snapshot().outcomes[Outcome::Replaced as usize].count,
            0
        );
        let error = observe_proposal_wait(
            &metric,
            || {
                proposals += 1;
                Ok(ProposedAt {
                    term: 4,
                    index: kv9_raft::LogIndex(20),
                })
            },
            |at, _| {
                Err(ApplyWaitError::Unconfirmed {
                    index: at.index.0,
                    waited: Duration::ZERO,
                })
            },
            budget,
        )
        .unwrap_err();
        assert_eq!(proposals, 4, "unconfirmed attempt was retried");
        assert!(error.to_string().contains("unconfirmed"));
        assert_eq!(
            metric.snapshot().outcomes[Outcome::Unconfirmed as usize].count,
            1
        );
        assert_eq!(
            metric.snapshot().outcomes[Outcome::Success as usize].count,
            1
        );
    }
    use super::*;
    use kv9_common::RegionId;
    use std::cell::RefCell;

    // ---- task #30: the re-proposal control loop, every contract clause lit
    // through the scripted seam the production wrapper consumes. ----

    fn at(term: u64, index: u64) -> ProposedAt {
        ProposedAt {
            term,
            index: kv9_raft::LogIndex(index),
        }
    }

    #[tokio::test]
    async fn async_replacement_reuses_one_command_and_deadline_and_returns_the_receipt() {
        use kv9_common::metrics::Outcome;
        let original = Arc::new(Command::Put {
            cf: 0,
            key: b"same-key".to_vec(),
            value: b"same-value".to_vec(),
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        let receipt = AppliedPosition { term: 9, index: 27 };
        let mut attempts = 0;
        let (result, category) = finish_async_proposal_loop(
            original.clone(),
            (at(1, 5), Ok(ApplyWaitOutcome::Replaced)),
            deadline,
            std::future::ready,
            |command, remaining_deadline| {
                attempts += 1;
                assert!(
                    Arc::ptr_eq(&command, &original),
                    "replacement changed the original command"
                );
                assert_eq!(
                    remaining_deadline, deadline,
                    "replacement reset the original deadline"
                );
                std::future::ready(Ok((
                    at(2, 6),
                    Ok(if attempts == 1 {
                        ApplyWaitOutcome::Replaced
                    } else {
                        ApplyWaitOutcome::Applied(receipt)
                    }),
                )))
            },
        )
        .await;
        assert_eq!(attempts, 2, "known replacements were not retried");
        assert_eq!(
            result.unwrap(),
            receipt,
            "asynchronous completion echoed the proposal instead of its receipt"
        );
        assert_eq!(category, Outcome::Success);
    }

    #[tokio::test]
    async fn async_terminal_outcomes_and_expired_replacements_are_never_retried() {
        use kv9_common::metrics::Outcome;
        let position = AppliedPosition { term: 1, index: 5 };
        let cases = [
            (
                Err(ApplyWaitError::Unconfirmed {
                    index: 5,
                    waited: Duration::ZERO,
                }),
                false,
                Outcome::Unconfirmed,
                "unconfirmed",
            ),
            (
                Err(ApplyWaitError::Failed(Error::Raft("poisoned".into()))),
                false,
                Outcome::Error,
                "poisoned",
            ),
            (
                Ok(ApplyWaitOutcome::FenceRejected {
                    at: position,
                    region: RegionId(3),
                }),
                false,
                Outcome::Rejected,
                "epoch",
            ),
            (
                Ok(ApplyWaitOutcome::Manifest {
                    at: position,
                    verdict: kv9_raft::ManifestVerdict::AlreadyApplied {
                        region: RegionId(3),
                        generation: 8,
                    },
                }),
                false,
                Outcome::Error,
                "non-manifest proposal",
            ),
            (
                Ok(ApplyWaitOutcome::Replaced),
                true,
                Outcome::Replaced,
                "budget is exhausted",
            ),
        ];
        for (outcome, expired, expected_category, expected_error) in cases {
            let deadline = Instant::now()
                + if expired {
                    Duration::ZERO
                } else {
                    Duration::from_secs(5)
                };
            let mut proposals = 0;
            let (result, category) = finish_async_proposal_loop(
                Arc::new(Command::Noop),
                (at(1, 5), outcome),
                deadline,
                std::future::ready,
                |_, _| {
                    proposals += 1;
                    std::future::ready(Err(Error::Raft(
                        "FUSE: forbidden asynchronous retry".into(),
                    )))
                },
            )
            .await;
            assert_eq!(proposals, 0, "terminal asynchronous outcome was retried");
            assert_eq!(category, expected_category);
            let error = result.unwrap_err().to_string();
            assert!(
                error.contains(expected_error),
                "lost terminal outcome: {error}"
            );
        }
    }

    /// `Unconfirmed` returns immediately and is NEVER retried (an unknown
    /// outcome retried is how one write becomes two). Tess's mutant — turning
    /// the Unconfirmed arm into `continue` — must red here: the loop would
    /// spin until the deadline and the single-call assertions break.
    #[test]
    fn unconfirmed_is_never_retried() {
        let proposes = RefCell::new(0u32);
        let waits = RefCell::new(0u32);
        let err = propose_and_wait_loop(
            || {
                *proposes.borrow_mut() += 1;
                // Fuse: a mutated loop that retries Unconfirmed has no exit
                // path at all (the deadline check lives in the Replaced arm),
                // so give it a scripted way out that the assertions below
                // catch by NAME instead of hanging or overflowing a counter.
                if *proposes.borrow() > 3 {
                    return Err(Error::Raft(
                        "FUSE: the loop kept re-proposing an unknown outcome".into(),
                    ));
                }
                Ok(at(1, 5))
            },
            |_, _| {
                *waits.borrow_mut() += 1;
                Err(ApplyWaitError::Unconfirmed {
                    index: 5,
                    waited: Duration::from_millis(1),
                })
            },
            Duration::from_millis(50),
        )
        .expect_err("unconfirmed must surface");
        assert!(
            err.to_string().contains("unconfirmed"),
            "the typed Unconfirmed must survive recognizably: {err}"
        );
        assert_eq!(
            *proposes.borrow(),
            1,
            "an unknown outcome must not be re-proposed"
        );
        assert_eq!(
            *waits.borrow(),
            1,
            "an unknown outcome must not be re-waited"
        );
    }

    /// `Failed` (poisoned driver) returns immediately, never retried.
    #[test]
    fn failed_is_never_retried() {
        let proposes = RefCell::new(0u32);
        let err = propose_and_wait_loop(
            || {
                *proposes.borrow_mut() += 1;
                // Fuse: a mutated loop that retries Failed has no exit path
                // (the deadline check lives in the Replaced arm) — give it a
                // bounded way out the named assertions catch; the detector is
                // the assertion, never an outer job timeout.
                if *proposes.borrow() > 3 {
                    return Err(Error::Raft(
                        "FUSE: the loop kept re-proposing after a driver failure".into(),
                    ));
                }
                Ok(at(1, 5))
            },
            |_, _| Err(ApplyWaitError::Failed(Error::Raft("poisoned".into()))),
            Duration::from_millis(50),
        )
        .expect_err("a failed driver must surface");
        assert!(
            err.to_string().contains("poisoned"),
            "the driver's own failure must survive recognizably: {err}"
        );
        assert_eq!(
            *proposes.borrow(),
            1,
            "a driver failure must not be re-proposed"
        );
    }

    /// `Replaced` re-proposes — and the budget is the ORIGINAL deadline, not
    /// per-attempt: every wait receives a shrinking remaining slice, and the
    /// whole loop gives up once the original budget is spent (elapsed stays in
    /// the deadline's order of magnitude, not attempts × deadline).
    #[test]
    fn replaced_reproposes_within_the_original_budget_only() {
        let proposes = RefCell::new(0u32);
        let remainders: RefCell<Vec<Duration>> = RefCell::new(Vec::new());
        let deadline = Duration::from_millis(50);
        let started = std::time::Instant::now();
        let err = propose_and_wait_loop(
            || {
                *proposes.borrow_mut() += 1;
                Ok(at(1, 5))
            },
            |_, remaining| {
                remainders.borrow_mut().push(remaining);
                // Consume a little real time so the budget actually drains.
                std::thread::sleep(Duration::from_millis(5));
                Ok(ApplyWaitOutcome::Replaced)
            },
            deadline,
        )
        .expect_err("an always-replaced position must exhaust the budget");
        assert!(
            err.to_string().contains("budget is exhausted"),
            "exhaustion must be named: {err}"
        );
        assert!(*proposes.borrow() >= 2, "Replaced must actually re-propose");
        assert!(
            started.elapsed() < deadline * 4,
            "the budget must be the original deadline, never reset per attempt"
        );
        let rem = remainders.borrow();
        assert!(
            rem.windows(2).all(|w| w[1] <= w[0]),
            "each wait must receive a SHRINKING slice of the original budget: {rem:?}"
        );
    }

    /// The receipt returned after a successful re-proposal is the one the
    /// WAIT produced (the driver's ring-built AppliedPosition), not anything
    /// reconstructed from the proposal. Command identity across the retry is
    /// by construction in the production wrapper: the propose closure borrows
    /// the single `&Command`, so a retry has no second command to use.
    #[test]
    fn replaced_then_applied_returns_the_waits_receipt() {
        let proposes = RefCell::new(0u32);
        let receipt = AppliedPosition { term: 7, index: 9 };
        let got = propose_and_wait_loop(
            || {
                *proposes.borrow_mut() += 1;
                Ok(at(1, 5))
            },
            |_, _| {
                if *proposes.borrow() == 1 {
                    Ok(ApplyWaitOutcome::Replaced)
                } else {
                    Ok(ApplyWaitOutcome::Applied(receipt))
                }
            },
            Duration::from_secs(5),
        )
        .expect("second attempt applies");
        assert_eq!(*proposes.borrow(), 2);
        assert_eq!(
            got, receipt,
            "the receipt must be the wait's applied position, verbatim"
        );
    }

    // ---- the registration walk (the 972-not_leader master red) ----

    fn walk_receipt() -> RegistrationReceipt {
        RegistrationReceipt {
            applied_term: 2,
            applied_index: 9,
            voters: vec![1, 2, 3, 4],
            learners: vec![5],
        }
    }

    fn walk_addr(port: u16) -> std::net::SocketAddr {
        format!("127.0.0.1:{port}").parse().unwrap()
    }

    /// A deadline far enough away that time plays no role in the test.
    fn far_deadline() -> std::time::Instant {
        std::time::Instant::now() + Duration::from_secs(3600)
    }

    fn walk_hint(id: u64, addr: std::net::SocketAddr) -> RegisterOutcome {
        RegisterOutcome::NotLeader {
            leader: Some(LeaderHint {
                id: NodeId(id),
                addr: Some(addr),
            }),
        }
    }

    /// The exact master-red scene, deterministic: every seed answers
    /// NotLeader with a hint pointing at a NON-SEED leader; the walk must
    /// follow the hint and register there in the same pass. Precondition
    /// pinned by name: the hinted leader is not in the seed list — without
    /// that, the walk can succeed through a seed and the hint arm is never
    /// selected.
    ///
    /// Mutant contract: deleting the hint-follow (queue.push) reverts to the
    /// seeds-only loop — reds at "must register by following the hint".
    #[test]
    fn the_walk_follows_a_not_leader_hint_to_a_non_seed_leader() {
        let seeds = vec![
            (NodeId(1), walk_addr(24101)),
            (NodeId(2), walk_addr(24102)),
            (NodeId(3), walk_addr(24103)),
        ];
        let leader_addr = walk_addr(24104);
        assert!(
            !seeds.iter().any(|(_, a)| *a == leader_addr),
            "precondition: the hinted leader must NOT be a seed — otherwise \
             the seeds-only loop also succeeds and the hint arm is unselected"
        );
        let mut obs = RegistrationObservation::new();
        let mut dialed: Vec<std::net::SocketAddr> = Vec::new();
        let got = registration_walk(
            &seeds,
            &mut obs,
            far_deadline(),
            std::time::Instant::now,
            |addr, _remaining| {
                dialed.push(addr);
                if addr == leader_addr {
                    Ok(RegisterOutcome::Registered(walk_receipt()))
                } else {
                    Ok(walk_hint(4, leader_addr))
                }
            },
        );
        assert!(
            matches!(got, WalkOutcome::Registered { .. }),
            "must register by following the hint to the non-seed leader \
             (the seeds-only loop dies here with endless not_leader)"
        );
        assert_eq!(
            dialed.last(),
            Some(&leader_addr),
            "the successful dial must be the hinted endpoint"
        );
        assert_eq!(obs.last, RegistrationLastOutcome::Registered);
        assert_eq!(obs.last_walk, Some(WalkTerminal::Registered));
    }

    /// Cycles and stale chains terminate on the hop cap within ONE pass —
    /// the hop budget never resets. Every hint here is a fresh (id, addr)
    /// pair, the worst case for dedup; the walk must dial exactly seeds +
    /// cap, stop, and say WHY with the typed terminal.
    #[test]
    fn hint_chains_terminate_on_the_hop_cap_with_a_typed_terminal() {
        let seeds = vec![(NodeId(1), walk_addr(24201))];
        let mut obs = RegistrationObservation::new();
        let mut dial_count = 0u64;
        let got = registration_walk(
            &seeds,
            &mut obs,
            far_deadline(),
            std::time::Instant::now,
            |_, _| {
                dial_count += 1;
                // Always a NEW (id, addr) hint: an endless stale chain.
                Ok(walk_hint(
                    100 + dial_count,
                    walk_addr(25000 + dial_count as u16),
                ))
            },
        );
        assert_eq!(
            dial_count,
            1 + 4,
            "a fresh-hint chain must dial exactly seeds + hop cap, then stop \
             — the cap guards stale chains exactly as it guards cycles"
        );
        assert_eq!(obs.attempts, 1 + 4, "the pass hop budget must not reset");
        assert!(
            matches!(got, WalkOutcome::Unconfirmed(WalkTerminal::HopCapExhausted)),
            "a truncated live chain must end as HopCapExhausted, not a \
             silent generic failure: {got:?}"
        );
        assert_eq!(obs.last_walk, Some(WalkTerminal::HopCapExhausted));
    }

    /// The hint frontier closing on already-visited (id, addr) pairs is its
    /// own mutually exclusive verdict: the directory content was exhausted,
    /// nothing was truncated.
    #[test]
    fn a_hint_cycle_is_a_typed_terminal_distinct_from_the_cap() {
        let seeds = vec![(NodeId(1), walk_addr(24501))];
        let hop = walk_addr(24504);
        let mut obs = RegistrationObservation::new();
        let got = registration_walk(
            &seeds,
            &mut obs,
            far_deadline(),
            std::time::Instant::now,
            // Every answer names the same (id, addr): the seed's hint is
            // novel and followed; the hop's own answer points straight back
            // at the pair already visited — a dedup hit, i.e. the cycle.
            |_, _| Ok(walk_hint(4, hop)),
        );
        assert!(
            matches!(got, WalkOutcome::Unconfirmed(WalkTerminal::HintCycle)),
            "a frontier that closes on visited pairs must end as HintCycle: {got:?}"
        );
    }

    /// The three dial-error classes stay mutually exclusive all the way to
    /// the walk terminal — Timeout is NOT folded into a generic failure
    /// (isolating "the seed is slow" from "the seed refused" from "the seed
    /// is gone" is exactly what a wedged-join scene needs).
    #[test]
    fn dial_error_classes_survive_to_the_walk_terminal_unmerged() {
        for (error, expected) in [
            (
                RegisterError::Connect("refused".into()),
                WalkTerminal::ConnectFailed,
            ),
            (RegisterError::Timeout, WalkTerminal::Timeout),
            (RegisterError::Failed("boom".into()), WalkTerminal::Failed),
        ] {
            let seeds = vec![(NodeId(1), walk_addr(24601))];
            let mut obs = RegistrationObservation::new();
            let got = registration_walk(
                &seeds,
                &mut obs,
                far_deadline(),
                std::time::Instant::now,
                |_, _| Err(error.clone()),
            );
            assert!(
                matches!(&got, WalkOutcome::Unconfirmed(t) if *t == expected),
                "error {error:?} must terminate as {expected:?}, got {got:?}"
            );
            assert_eq!(obs.last_walk, Some(expected));
        }
    }

    /// Multi-cause pass: connect error + dedup cycle + truncation in ONE
    /// pass. The final verdict follows the fixed documented precedence
    /// (cap > cycle > unresolved > last dial error); without the truncation
    /// the same shape ends as the cycle. Both directions pinned so the
    /// precedence cannot silently reorder.
    #[test]
    fn walk_terminal_precedence_is_fixed() {
        // s1 errors, s2 and s3 hint the SAME pair (second is a dedup hit =
        // cycle), the chain from that pair keeps yielding novel hints until
        // the cap truncates it.
        let seeds = vec![
            (NodeId(1), walk_addr(24701)),
            (NodeId(2), walk_addr(24702)),
            (NodeId(3), walk_addr(24703)),
        ];
        let first_hop = walk_addr(24710);
        let mut obs = RegistrationObservation::new();
        let mut novel = 0u64;
        let got = registration_walk(
            &seeds,
            &mut obs,
            far_deadline(),
            std::time::Instant::now,
            |addr, _| {
                if addr == seeds[0].1 {
                    Err(RegisterError::Connect("refused".into()))
                } else if addr == seeds[1].1 || addr == seeds[2].1 {
                    Ok(walk_hint(10, first_hop))
                } else {
                    novel += 1;
                    Ok(walk_hint(10 + novel, walk_addr(24710 + novel as u16)))
                }
            },
        );
        assert!(
            matches!(got, WalkOutcome::Unconfirmed(WalkTerminal::HopCapExhausted)),
            "cap must outrank cycle and the dial error: {got:?}"
        );

        // Same shape minus truncation: the chain converges back onto the
        // visited pair instead of growing — now the verdict is the cycle.
        let mut obs = RegistrationObservation::new();
        let got = registration_walk(
            &seeds,
            &mut obs,
            far_deadline(),
            std::time::Instant::now,
            |addr, _| {
                if addr == seeds[0].1 {
                    Err(RegisterError::Connect("refused".into()))
                } else {
                    Ok(walk_hint(10, first_hop))
                }
            },
        );
        assert!(
            matches!(got, WalkOutcome::Unconfirmed(WalkTerminal::HintCycle)),
            "without truncation the closed frontier must read as HintCycle, \
             and the connect error must not mask it: {got:?}"
        );
    }

    /// A stale hint costs one hop, then the fresh hint from the real target
    /// wins (Tess's scripted control): stale A → typed NotLeader with B →
    /// B registers, all within the same pass.
    #[test]
    fn a_stale_hint_is_one_hop_then_the_fresh_hint_wins() {
        let seeds = vec![(NodeId(1), walk_addr(24301))];
        let stale = walk_addr(24399); // old leader: answers NotLeader with the real one
        let real = walk_addr(24304);
        let mut obs = RegistrationObservation::new();
        let mut dialed = Vec::new();
        let got = registration_walk(
            &seeds,
            &mut obs,
            far_deadline(),
            std::time::Instant::now,
            |addr, _| {
                dialed.push(addr);
                if addr == real {
                    Ok(RegisterOutcome::Registered(walk_receipt()))
                } else if addr == stale {
                    Ok(walk_hint(4, real))
                } else {
                    // Seed's lagging view points at the STALE leader.
                    Ok(walk_hint(9, stale))
                }
            },
        );
        assert!(
            matches!(got, WalkOutcome::Registered { .. }),
            "the fresh hint must win after one stale hop"
        );
        assert_eq!(dialed, vec![seeds[0].1, stale, real]);
    }

    /// An addr-less hint is recorded observably (id + why it could not be
    /// followed) and never silently swallowed — the 972-loop scene had no
    /// record of what the hints said. The typed terminal says the same
    /// thing machine-readably: UnresolvedHint, exclusive of cap/cycle.
    #[test]
    fn an_addressless_hint_is_recorded_not_swallowed() {
        let seeds = vec![(NodeId(1), walk_addr(24401))];
        let mut obs = RegistrationObservation::new();
        let got = registration_walk(
            &seeds,
            &mut obs,
            far_deadline(),
            std::time::Instant::now,
            |_, _| {
                Ok(RegisterOutcome::NotLeader {
                    leader: Some(LeaderHint {
                        id: NodeId(4),
                        addr: None,
                    }),
                })
            },
        );
        assert!(
            matches!(got, WalkOutcome::Unconfirmed(WalkTerminal::UnresolvedHint)),
            "an unfollowable redirect must end as UnresolvedHint: {got:?}"
        );
        assert_eq!(
            obs.last_hint.as_deref(),
            Some("leader=4 addr=unresolved"),
            "the status must show WHERE the client was pointed and why it \
             could not follow"
        );
        assert_eq!(obs.last_walk, Some(WalkTerminal::UnresolvedHint));

        // The id-less shape is the same verdict — and is NEVER followed
        // (the wire boundary already guarantees no addr can accompany it).
        let mut obs = RegistrationObservation::new();
        let got = registration_walk(
            &seeds,
            &mut obs,
            far_deadline(),
            std::time::Instant::now,
            |_, _| Ok(RegisterOutcome::NotLeader { leader: None }),
        );
        assert!(matches!(
            got,
            WalkOutcome::Unconfirmed(WalkTerminal::UnresolvedHint)
        ));
        assert_eq!(obs.attempts, 1, "an id-less redirect adds no candidate");
        assert_eq!(obs.last_hint.as_deref(), Some("leader=unknown"));
    }

    /// The whole pass has ONE absolute window: every dial receives exactly
    /// the window's remainder — monotone non-increasing, never above the
    /// original budget, EQUAL remainders legal when the clock does not move
    /// between cheap dials — and following hints never enlarges the pass
    /// (the scripted chain still has candidates when the window ends).
    ///
    /// Mutant contract: re-granting a fresh per-dial budget turns the
    /// recorded remainders non-monotone (or endless) and reds here.
    #[test]
    fn the_pass_budget_is_one_absolute_window_hints_never_enlarge_it() {
        let window = Duration::from_millis(50);
        let base = std::time::Instant::now();
        // Scripted clock: consecutive per-call advances. The two zeros pin
        // "equal remainders are legal" (Tess: a clock may not move between
        // cheap dials; strict decrease would red a correct implementation).
        let advances = [0u64, 0, 30, 10, 10, 0];
        let mut calls = 0usize;
        let now = move || {
            calls += 1;
            let elapsed: u64 = advances[..calls.min(advances.len())].iter().sum();
            base + Duration::from_millis(elapsed)
        };
        let seeds = vec![
            (NodeId(1), walk_addr(24801)),
            (NodeId(2), walk_addr(24802)),
            (NodeId(3), walk_addr(24803)),
        ];
        let mut obs = RegistrationObservation::new();
        let mut remainders: Vec<Duration> = Vec::new();
        let mut novel = 0u64;
        let got = registration_walk(&seeds, &mut obs, base + window, now, |_, remaining| {
            remainders.push(remaining);
            // An endless novel chain: candidates outlive the window.
            novel += 1;
            Ok(walk_hint(50 + novel, walk_addr(24810 + novel as u16)))
        });
        assert_eq!(
            remainders,
            vec![
                Duration::from_millis(50),
                Duration::from_millis(50),
                Duration::from_millis(20),
                Duration::from_millis(10),
            ],
            "each dial must receive exactly the one window's remainder"
        );
        assert!(
            remainders.windows(2).all(|w| w[1] <= w[0]),
            "remainders must be monotone non-increasing"
        );
        assert!(
            remainders.iter().all(|r| *r <= window),
            "no dial may ever see more than the original window"
        );
        assert!(
            matches!(
                got,
                WalkOutcome::Unconfirmed(WalkTerminal::DeadlineExhausted)
            ),
            "candidates remained but the window ended — the typed verdict \
             is DeadlineExhausted: {got:?}"
        );
        assert_eq!(
            obs.attempts, 4,
            "the pass must stop dialing when the window is spent, even \
             though the hint chain still had candidates"
        );
    }

    #[test]
    fn repeated_registration_passes_reach_a_healthy_seed_after_a_blackhole() {
        use std::cell::Cell;
        let seeds = vec![
            (NodeId(1), walk_addr(24911)),
            (NodeId(2), walk_addr(24912)),
            (NodeId(3), walk_addr(24913)),
        ];
        let window = REGISTRATION_PASS_TIMEOUT;
        let mut cursor = RegistrationSeedCursor::default();
        let mut obs = RegistrationObservation::new();
        let mut visited = Vec::new();
        for _ in 0..seeds.len() {
            let base = Instant::now();
            let elapsed = Cell::new(Duration::ZERO);
            let result = registration_walk(
                &cursor.order(&seeds),
                &mut obs,
                base + window,
                || base + elapsed.get(),
                |addr, remaining| {
                    visited.push(addr);
                    if addr == seeds[0].1 {
                        elapsed.set(elapsed.get() + remaining);
                        Err(RegisterError::Timeout)
                    } else {
                        Ok(RegisterOutcome::Registered(walk_receipt()))
                    }
                },
            );
            if matches!(result, WalkOutcome::Registered { .. }) {
                return;
            }
        }
        // Sensitivity: the same live seed succeeds with the same absolute
        // budget when it is selected first. No transport/quorum fault there.
        let base = Instant::now();
        let control = registration_walk(
            &seeds[1..],
            &mut obs,
            base + window,
            || base,
            |_, _| Ok(RegisterOutcome::Registered(walk_receipt())),
        );
        assert!(matches!(control, WalkOutcome::Registered { .. }));
        assert!(visited.contains(&seeds[1].1),
            "a blackholed first seed consumed every retry window; the healthy seed was never contacted: {visited:?}");
    }

    #[test]
    fn a_novel_leader_hint_precedes_a_blackholed_remaining_seed() {
        use std::cell::Cell;
        let seeds = vec![(NodeId(1), walk_addr(24921)), (NodeId(2), walk_addr(24922))];
        let leader = walk_addr(24924);
        let base = Instant::now();
        let elapsed = Cell::new(Duration::ZERO);
        let mut obs = RegistrationObservation::new();
        let mut visited = Vec::new();
        let result = registration_walk(
            &seeds,
            &mut obs,
            base + REGISTRATION_PASS_TIMEOUT,
            || base + elapsed.get(),
            |addr, remaining| {
                visited.push(addr);
                if addr == seeds[0].1 {
                    Ok(walk_hint(4, leader))
                } else if addr == leader {
                    Ok(RegisterOutcome::Registered(walk_receipt()))
                } else {
                    elapsed.set(elapsed.get() + remaining);
                    Err(RegisterError::Timeout)
                }
            },
        );
        assert!(
            matches!(result, WalkOutcome::Registered { .. }),
            "a pending blackhole prevented following an already received leader hint"
        );
        assert_eq!(visited, vec![seeds[0].1, leader]);
    }

    #[test]
    fn registration_budget_allows_durable_work_beyond_a_discovery_probe() {
        let seeds = vec![(NodeId(1), walk_addr(24931))];
        let base = Instant::now();
        let mut obs = RegistrationObservation::new();
        let result = registration_walk(
            &seeds,
            &mut obs,
            base + REGISTRATION_PASS_TIMEOUT,
            || base,
            |_, remaining| {
                if remaining >= Duration::from_millis(250) {
                    Ok(RegisterOutcome::Registered(walk_receipt()))
                } else {
                    Err(RegisterError::Timeout)
                }
            },
        );
        assert!(
            matches!(result, WalkOutcome::Registered { .. }),
            "a discovery-probe timeout cannot budget several durable consensus steps"
        );
    }

    #[test]
    fn registration_seed_rotation_covers_every_position_from_every_start() {
        for count in 1..=5 {
            let seeds: Vec<_> = (0..count)
                .map(|i| (NodeId(i as u64 + 1), walk_addr(24940 + i as u16)))
                .collect();
            for start in 0..count {
                let mut cursor = RegistrationSeedCursor { next: start };
                let mut firsts = HashSet::new();
                for _ in 0..count {
                    let order = cursor.order(&seeds);
                    assert_eq!(
                        order.iter().copied().collect::<HashSet<_>>(),
                        seeds.iter().copied().collect()
                    );
                    assert!(
                        firsts.insert(order[0]),
                        "a seed repeated before another got its turn"
                    );
                }
                assert_eq!(cursor.next, start);
            }
        }
        let mut empty = RegistrationSeedCursor::default();
        assert!(empty.order(&[]).is_empty());
    }

    /// A window already spent at entry means ZERO dials — the deadline is
    /// checked before every dial including the first.
    #[test]
    fn an_expired_window_ends_the_pass_before_any_dial() {
        let base = std::time::Instant::now();
        let seeds = vec![(NodeId(1), walk_addr(24901))];
        let mut obs = RegistrationObservation::new();
        let got = registration_walk(
            &seeds,
            &mut obs,
            base,
            move || base,
            |_, _| panic!("no dial may happen after the window is spent"),
        );
        assert!(matches!(
            got,
            WalkOutcome::Unconfirmed(WalkTerminal::DeadlineExhausted)
        ));
        assert_eq!(obs.attempts, 0);
        assert_eq!(obs.last_walk, Some(WalkTerminal::DeadlineExhausted));
    }

    /// `FenceRejected` maps to the typed `StaleEpoch { region }` IMMEDIATELY
    /// and is NEVER re-proposed: the expected epoch is authoritatively stale
    /// and will not come back — a retry re-proposes the same stale
    /// expectation (contrast Replaced, where re-proposing the identical
    /// command is safe and correct). Fuse for the retry mutant: a loop that
    /// retries a fence rejection has no exit path.
    #[test]
    fn fence_rejected_is_stale_epoch_immediately_and_never_retried() {
        let proposes = RefCell::new(0u32);
        let err = propose_and_wait_loop(
            || {
                *proposes.borrow_mut() += 1;
                if *proposes.borrow() > 3 {
                    return Err(Error::Raft(
                        "FUSE: the loop kept re-proposing a fence-rejected write".into(),
                    ));
                }
                Ok(at(1, 5))
            },
            |_, _| {
                Ok(ApplyWaitOutcome::FenceRejected {
                    at: AppliedPosition { term: 1, index: 5 },
                    region: kv9_common::RegionId(42),
                })
            },
            Duration::from_millis(50),
        )
        .expect_err("a fence rejection must surface");
        assert!(
            matches!(
                err,
                Error::StaleEpoch {
                    region: kv9_common::RegionId(42)
                }
            ),
            "the verdict must map to the typed StaleEpoch carrying the exact \
             rejected region: {err:?}"
        );
        assert_eq!(
            *proposes.borrow(),
            1,
            "a stale expectation must never be re-proposed"
        );
    }

    /// A propose error — the REAL typed `Error::NotLeader` with its hint,
    /// after leadership moved — surfaces with variant and hint structurally
    /// intact (review round: the first draft forged a string and only proved
    /// a string passes through); the client's own retry policy owns that case.
    #[test]
    fn a_propose_error_surfaces_verbatim() {
        let proposes = RefCell::new(0u32);
        let err = propose_and_wait_loop(
            || {
                *proposes.borrow_mut() += 1;
                if *proposes.borrow() == 1 {
                    Ok(at(1, 5))
                } else {
                    Err(Error::NotLeader {
                        leader: Some(NodeId(3)),
                    })
                }
            },
            |_, _| Ok(ApplyWaitOutcome::Replaced),
            Duration::from_secs(5),
        )
        .expect_err("the moved-leadership propose error must surface");
        assert!(
            matches!(
                err,
                Error::NotLeader {
                    leader: Some(NodeId(3))
                }
            ),
            "the typed NotLeader variant and its leader hint must pass through \
             structurally unchanged: {err:?}"
        );
        assert_eq!(*proposes.borrow(), 2);
    }

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
            .expect("drain token minted once per peer")
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
        )
        .expect("drain token minted once per peer");
        (
            RuntimeBackend {
                node,
                driver,
                transport,
                endpoint_ready: Arc::new(AtomicBool::new(false)),
                initial_voters: Vec::new(),
            },
            runtime,
            dir,
        )
    }

    #[test]
    fn catalog_planning_barrier_drains_an_earlier_unconfirmed_command() {
        let (backend, _runtime, dir) = pre_serving_runtime_backend();
        let driver = backend.driver.clone();
        driver.peer().campaign().unwrap();
        for _ in 0..20 {
            driver.tick_and_step().unwrap();
        }
        driver.pause_apply(true);
        let earlier = driver
            .propose(&kv9_raft::Command::Put {
                cf: 0,
                key: b"earlier-catalog-plan".to_vec(),
                value: b"committed".to_vec(),
            })
            .unwrap();
        driver.step().unwrap();
        assert!(backend
            .node
            .meta_raft
            .store
            .engine()
            .get(kv9_engine::ColumnFamily::Default, b"earlier-catalog-plan")
            .unwrap()
            .is_none());
        let committed_before = driver.status().raft_committed;
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let waiter = std::thread::spawn(move || {
            let _guard = backend.node.meta_raft.lock_catalog_txn();
            let term = backend.prepare_catalog().unwrap();
            let value = backend
                .node
                .meta_raft
                .store
                .engine()
                .get(kv9_engine::ColumnFamily::Default, b"earlier-catalog-plan")
                .unwrap();
            tx.send((term, value)).unwrap();
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while driver.status().raft_committed <= committed_before {
            assert!(
                std::time::Instant::now() < deadline,
                "planning barrier must be proposed"
            );
            driver.step().unwrap();
            std::thread::yield_now();
        }
        assert!(
            rx.try_recv().is_err(),
            "planner must wait for ordered apply"
        );
        driver.pause_apply(false);
        driver.step().unwrap();
        let (term, value) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(term, earlier.term);
        assert_eq!(
            value,
            Some(b"committed".to_vec()),
            "next plan sees earlier ambiguous write"
        );
        waiter.join().unwrap();
        drop(driver);
        let _ = std::fs::remove_dir_all(dir);
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
            origin: crate::api::RequestOrigin::from_transport("test-client"),
        };
        let transaction = kv9_txn::TxnDescriptor {
            keyspace: ctx.keyspace,
            id: kv9_txn::TxnId {
                txn_group: TxnGroupId(0),
                timeline: kv9_common::TimelineId(0),
                timeline_generation: kv9_txn::TimelineGeneration(1),
                start_ts: kv9_common::TimeStamp(1),
            },
            primary: kv9_txn::QualifiedKey {
                keyspace: ctx.keyspace,
                user_key: b"k".to_vec(),
            },
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

        assert_meta_not_ready(backend.kv_begin(&ctx, transaction.primary.clone()));
        assert_meta_not_ready(backend.kv_get(&ctx, b"k", &transaction));
        assert_meta_not_ready(backend.kv_batch_get(&ctx, &[b"k".to_vec()], &transaction));
        assert_meta_not_ready(backend.kv_scan(&ctx, b"", b"", 10, &transaction));
        assert_meta_not_ready(backend.kv_prewrite(
            &ctx,
            &[(b"k".to_vec(), Some(b"v".to_vec()))],
            &transaction,
        ));
        assert_meta_not_ready(backend.kv_commit(&ctx, &[b"k".to_vec()], &transaction));
        assert_meta_not_ready(backend.kv_pessimistic_lock(&ctx, &[b"k".to_vec()], &transaction));
        assert_meta_not_ready(backend.kv_pessimistic_rollback(
            &ctx,
            &[b"k".to_vec()],
            &transaction,
        ));
        assert_meta_not_ready(backend.kv_resolve_lock(&ctx, &transaction));
        assert_meta_not_ready(backend.kv_cleanup(&ctx, b"k", &transaction));
        assert_meta_not_ready(backend.kv_check_txn_status(&ctx, &transaction));

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
            KeySpan::BatchKeys(&[b"b".to_vec(), b"c".to_vec()]),
        )
        .expect("control: keys in one region are accepted");

        // b is in [a,m); z is in [m,) -- same epoch, different regions.
        assert!(
            matches!(
                check_context(
                    store,
                    keyspace,
                    &epoch(1, 1),
                    KeySpan::BatchKeys(&[b"b".to_vec(), b"z".to_vec()])
                ),
                Err(Error::RangeCrossesRegion)
            ),
            "a batch crossing regions must be refused, not silently split"
        );
    }

    /// Borrowing a request must preserve authorization and the first observable
    /// routing error. In particular, omitting the already-checked anchor must
    /// never omit the next position, even if it repeats the anchor's bytes.
    #[test]
    fn borrowed_batches_preserve_region_coverage_and_error_order() {
        let (node, keyspace) = two_region_keyspace();
        let store = &node.meta_raft.store;
        type GateCase<'a> = (&'a [&'a [u8]], u64, u64, (&'static str, u64));
        let cases: &[GateCase<'_>] = &[
            (&[], 1, 1, ("missing", 0)),
            (&[b"b"], 1, 1, ("ok", 300)),
            (&[b"b", b"b", b"c"], 1, 1, ("ok", 300)),
            (&[b"m", b"m", b"z"], 1, 1, ("ok", 301)),
            (&[b"b", b"m"], 1, 1, ("cross", 0)),
            (&[b"z", b"b"], 1, 1, ("cross", 0)),
            (&[b"b", b"b", b"z"], 1, 1, ("cross", 0)),
            (&[b"b", b""], 1, 1, ("missing", 0)),
            (&[b"b", b"z", b""], 1, 1, ("cross", 0)),
            (&[b"b", b"", b"z"], 1, 1, ("missing", 0)),
            (&[b"", b"b"], 1, 1, ("missing", 0)),
            (&[b"b", b"z"], 2, 1, ("stale", 300)),
            (&[b"z", b"b"], 1, 2, ("stale", 301)),
            (&[b"", b"b"], 2, 2, ("missing", 0)),
        ];
        for &(input, conf, ver, expected) in cases {
            let keys: Vec<UserKey> = input.iter().map(|key| key.to_vec()).collect();
            let pairs: Vec<(UserKey, Value)> = keys
                .iter()
                .enumerate()
                .map(|(i, key)| (key.clone(), vec![i as u8]))
                .collect();
            for span in [KeySpan::BatchKeys(&keys), KeySpan::BatchPairs(&pairs)] {
                let result = match check_context(store, keyspace, &epoch(conf, ver), span) {
                    Ok(fence) => {
                        let fence = fence.into_region_fence();
                        assert_eq!((fence.conf_ver, fence.version), (conf, ver));
                        ("ok", fence.region_id)
                    }
                    Err(Error::RegionNotFound) => ("missing", 0),
                    Err(Error::RangeCrossesRegion) => ("cross", 0),
                    Err(Error::StaleEpoch { region }) => ("stale", region.0),
                    Err(other) => panic!("unexpected batch gate error: {other:?}"),
                };
                assert_eq!(
                    result, expected,
                    "borrowed batch lost region coverage or error order for {input:?}"
                );
            }
        }
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

    /// A stand-in authorisation for the loop tests, which are about *control flow* — how
    /// many chunks run, what a mid-range failure reports — and not about fence content.
    ///
    /// Deliberately not derived from a catalog: these tests must stay runnable without one.
    /// The content assertion lives in `fence_firing_tests`, against a real node, precisely
    /// because a fixture-built fence can say nothing about what production builds
    /// (docs/TESTING.md rule 17).
    fn granted() -> ValidatedFence {
        ValidatedFence::for_loop_tests()
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
                Ok(granted())
            },
            planner(4),
            |_fence, batch| {
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
                Ok(granted())
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
            |_fence, batch| {
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
            |_fence, _batch| panic!("nothing may be committed for an empty range"),
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
            |_| Ok(granted()),
            planner(4),
            |_fence, batch| {
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
            |_| Ok(granted()),
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
            |_fence, batch| {
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
            |_| Ok(granted()),
            planner(4),
            |_fence, _batch| Err(Error::Engine("injected disk failure".into())),
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
            |_| Ok(granted()),
            planner(0),
            |_fence, _batch| panic!("nothing should be committed"),
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
            |_| Ok(granted()),
            planner(3),
            |_fence, batch| {
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
        let wal = Arc::new(wal);
        let peer = Arc::new(RaftPeer::new(NodeId(1), META_REGION_0, &[NodeId(1)]).unwrap());
        let node = Arc::new(
            Node::with_raft_and_engine(NodeId(1), Config::default(), peer.clone(), wal.clone())
                .unwrap(),
        );
        let hub = InProcHub::new();
        let driver = NodeDriver::new(
            peer,
            Arc::new(hub.endpoint(NodeId(1))) as Arc<dyn RaftTransport>,
            MemStateMachine::with_engine(wal).unwrap(),
        )
        .expect("drain token minted once per peer");
        let authenticator = ClusterAuthenticator {
            expected_token: Arc::from("secret"),
            voters: Arc::new([NodeId(1), NodeId(2)].into_iter().collect()),
            node,
            driver,
            catchup: Arc::new(std::sync::Mutex::new(None)),
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

    /// Authenticator + armable catalog for the catch-up-window cells. The
    /// capability content here is synthetic (the WINDOW TRANSITION on a
    /// production-minted receipt lives in the hint-follow product-chain
    /// test); these cells pin the authenticator's DECISION STRUCTURE:
    /// membership, explicit revocation, and the two catalog-failure arms.
    struct CatchupAuthHarness {
        authenticator: ClusterAuthenticator<kv9_raft::rawnode::MemStorage, FaultyEngine<MemEngine>>,
        engine: Arc<FaultyEngine<MemEngine>>,
        node: Arc<Node<FaultyEngine<MemEngine>>>,
    }

    fn catchup_auth_harness() -> CatchupAuthHarness {
        let engine = Arc::new(FaultyEngine::new(MemEngine::new()));
        let peer = Arc::new(RaftPeer::new(NodeId(1), META_REGION_0, &[NodeId(1)]).unwrap());
        let node = Arc::new(
            Node::with_raft_and_engine(NodeId(1), Config::default(), peer.clone(), engine.clone())
                .unwrap(),
        );
        let hub = InProcHub::new();
        let driver = NodeDriver::new(
            peer,
            Arc::new(hub.endpoint(NodeId(1))) as Arc<dyn RaftTransport>,
            MemStateMachine::with_engine(engine.clone()).unwrap(),
        )
        .expect("drain token minted once per peer");
        // A capability naming node 4, with a barrier far ahead of this
        // fresh driver's (empty) watermark: the window is OPEN.
        let catchup = Arc::new(std::sync::Mutex::new(Some(CatchupCapability {
            members: [NodeId(4)].into_iter().collect(),
            receipt_position: DriverAppliedPosition {
                term: 1,
                index: 1_000,
            },
        })));
        CatchupAuthHarness {
            authenticator: ClusterAuthenticator {
                expected_token: Arc::from("secret"),
                voters: Arc::new([NodeId(1), NodeId(2), NodeId(3)].into_iter().collect()),
                node: node.clone(),
                driver,
                catchup,
            },
            engine,
            node,
        }
    }

    fn cluster_metadata(sender: u64) -> MetadataMap {
        let mut metadata = MetadataMap::new();
        metadata.insert(CLUSTER_TOKEN_KEY, "secret".parse().unwrap());
        metadata.insert(NODE_ID_KEY, sender.to_string().parse().unwrap());
        metadata
    }

    /// Window membership is exact: a receipt member absent from the catalog
    /// passes while behind; a sender outside the receipt set is refused on
    /// the same catalog state, same watermark, same token.
    #[test]
    fn the_catchup_window_admits_receipt_members_and_only_receipt_members() {
        let harness = catchup_auth_harness();
        let admitted = harness.authenticator.authenticate(&cluster_metadata(4));
        assert!(
            admitted.is_ok(),
            "a receipt member absent from the empty catalog must be admitted \
             while behind the barrier: {admitted:?}"
        );
        assert_eq!(
            harness
                .authenticator
                .authenticate(&cluster_metadata(9))
                .unwrap_err()
                .code(),
            Code::PermissionDenied,
            "a sender outside the receipt member set must be refused even \
             with the window open"
        );
    }

    /// An explicit applied revocation beats BOTH the window and every
    /// positive membership arm. The scenario is the real decommission
    /// shape (Tess's blocker on 19adf3b): the node REGISTERED successfully
    /// — its NODES_DESC membership row is present — and its admission was
    /// revoked afterwards. With `registered => allow` evaluated before
    /// `revoked => reject`, the revoked arm is dead code for exactly the
    /// nodes it exists for; this test reds on that ordering.
    #[test]
    fn an_explicit_revocation_beats_the_catchup_window() {
        let harness = catchup_auth_harness();
        {
            let mut txn = harness.node.meta_raft.store.begin().unwrap();
            kv9_meta::admission::initialize_cluster(
                &mut txn,
                kv9_common::ClusterId::from_bytes([7; 16]),
                1,
            )
            .unwrap();
            kv9_meta::admission::admit_node(
                &mut txn,
                NodeId(4),
                "127.0.0.1:29999",
                kv9_meta::admission::AdmittedRole::Learner,
                u64::MAX,
            )
            .unwrap();
            // The node completed registration: a REAL membership row via
            // the production row builder (state 2 = active), exactly what
            // register() leaves behind.
            txn.insert(
                &NODES_DESC,
                &[memcmp_uint(4)],
                membership_node_row(
                    NodeId(4),
                    "127.0.0.1:29999",
                    2,
                    7,
                    StoreIncarnation::from_bytes([4; 16]),
                ),
            )
            .unwrap();
            // ... and was decommissioned afterwards.
            kv9_meta::admission::revoke_admission(&mut txn, NodeId(4)).unwrap();
            txn.commit().unwrap();
        }
        assert_eq!(
            harness
                .authenticator
                .authenticate(&cluster_metadata(4))
                .unwrap_err()
                .code(),
            Code::PermissionDenied,
            "a revoked node must be refused even though its membership row \
             is still present AND it is in the receipt member set — the \
             explicit verdict outranks both"
        );
    }

    #[test]
    fn superseded_ticket_preserves_only_existing_member_authentication() {
        use kv9_meta::admission::{self, AdmittedRole};
        let harness = catchup_auth_harness();
        let mut txn = harness.node.meta_raft.store.begin().unwrap();
        admission::initialize_cluster(&mut txn, ClusterId::from_bytes([7; 16]), 1).unwrap();
        admission::admit_node(
            &mut txn,
            NodeId(4),
            "127.0.0.1:29999",
            AdmittedRole::Learner,
            u64::MAX,
        )
        .unwrap();
        admission::supersede_admission(&mut txn, NodeId(4)).unwrap();
        txn.commit().unwrap();
        assert_eq!(
            harness
                .authenticator
                .authenticate(&cluster_metadata(4))
                .unwrap_err()
                .code(),
            Code::PermissionDenied,
            "superseded ticket granted membership or catch-up authority"
        );
        let mut txn = harness.node.meta_raft.store.begin().unwrap();
        txn.insert(
            &NODES_DESC,
            &[memcmp_uint(4)],
            membership_node_row(
                NodeId(4),
                "127.0.0.1:29999",
                2,
                7,
                StoreIncarnation::from_bytes([4; 16]),
            ),
        )
        .unwrap();
        txn.commit().unwrap();
        assert!(
            harness
                .authenticator
                .authenticate(&cluster_metadata(4))
                .is_ok(),
            "endpoint migration decommissioned the existing member"
        );
        let mut txn = harness.node.meta_raft.store.begin().unwrap();
        admission::revoke_admission(&mut txn, NodeId(4)).unwrap();
        admission::supersede_admission(&mut txn, NodeId(4)).unwrap();
        txn.commit().unwrap();
        assert_eq!(
            harness
                .authenticator
                .authenticate(&cluster_metadata(4))
                .unwrap_err()
                .code(),
            Code::PermissionDenied,
            "endpoint supersession lifted explicit decommission"
        );
    }

    /// Both catalog-failure arms reject WITHOUT consulting the window — a
    /// failed read is a failed read, never "not caught up yet". The two
    /// switches are armed separately (Ren's boundary: one switch makes
    /// `begin()` fail first and the read-through arm is never reached).
    #[test]
    fn a_failed_catalog_read_rejects_and_never_falls_back_to_the_window() {
        // Arm 1: snapshot open (`begin`) fails.
        let harness = catchup_auth_harness();
        harness.engine.start_failing_snapshots();
        assert_eq!(
            harness
                .authenticator
                .authenticate(&cluster_metadata(4))
                .unwrap_err()
                .code(),
            Code::Unavailable,
            "a failed catalog snapshot-open must reject; the open window \
             (sender IS a receipt member) must not be consulted"
        );

        // Arm 2: snapshot opens, the read-through fails.
        let harness = catchup_auth_harness();
        harness.engine.start_failing_reads();
        assert_eq!(
            harness
                .authenticator
                .authenticate(&cluster_metadata(4))
                .unwrap_err()
                .code(),
            Code::Unavailable,
            "a failed catalog read-through must reject; the open window \
             (sender IS a receipt member) must not be consulted"
        );
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
        )
        .expect("drain token minted once per peer");
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
            if matches!(
                driver.wait_applied(healthy, Duration::from_millis(1)),
                Ok(ApplyWaitOutcome::Applied(_))
            ) {
                break;
            }
        }
        assert!(matches!(
            driver
                .wait_applied(healthy, Duration::from_millis(5))
                .unwrap(),
            ApplyWaitOutcome::Applied(_)
        ));
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

    /// Full-runtime in-process harness helpers for the deterministic
    /// product-chain registration scene (Tess's blocker 3 on `6c32536`).
    fn bound_listener_for_e2e() -> std::net::TcpListener {
        // Retain ownership until start_core adopts the listener. Returning an
        // address and rebinding let another runtime/outbound connection steal
        // the port during the persistence-matrix workspace run.
        std::net::TcpListener::bind("127.0.0.1:0").unwrap()
    }

    fn backend_view(rt: &NodeRuntime, root: &RootDescriptor) -> RuntimeBackend {
        RuntimeBackend {
            node: rt.node.clone(),
            driver: rt.driver.clone(),
            transport: rt.transport.clone(),
            endpoint_ready: rt.endpoint_ready.clone(),
            initial_voters: root
                .voters
                .iter()
                .map(|voter| (voter.node_id, voter.addr))
                .collect(),
        }
    }

    /// One faithful `run()` step per runtime (minus status-file churn), then
    /// a short real-time yield for the driver/gRPC threads.
    fn step_cluster(rts: &mut [NodeRuntime]) {
        for rt in rts.iter_mut() {
            if let Some(fatal) = rt.driver.status().fatal {
                panic!("node {} went fatal: {fatal}", rt.node.id.0);
            }
            rt.sync_registered_peers().unwrap();
            rt.advance_bootstrap().unwrap();
            // Status files are the wedge diagnostics if a phase never
            // completes — same surface the shell E2E preserves.
            rt.write_status().unwrap();
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    fn wait_for(
        rts: &mut [NodeRuntime],
        secs: u64,
        what: &str,
        mut cond: impl FnMut(&[NodeRuntime]) -> bool,
    ) {
        eprintln!("[hint-follow e2e] waiting: {what}");
        let deadline = std::time::Instant::now() + Duration::from_secs(secs);
        loop {
            step_cluster(rts);
            if cond(rts) {
                eprintln!("[hint-follow e2e] reached: {what}");
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for: {what}"
            );
        }
    }

    fn cluster_leader(rts: &[NodeRuntime]) -> Option<usize> {
        rts.iter()
            .position(|rt| rt.driver.status().role == Role::Leader)
    }

    /// The master-red product scene, rebuilt deterministic and in-process:
    /// the 972-not_leader loop happened because a real joiner's seed set is
    /// FOREVER the root descriptor's initial voters, while the leader had
    /// become an admitted dynamic member outside that set.
    ///
    /// Construction: three initial voters bootstrap for real (disk raft +
    /// catalog WAL + real gRPC on loopback); n4 is admitted and promoted
    /// through the production admin path; then the TEST-ONLY raft transfer
    /// seam (`transfer_leader_for_tests`, raft-rs MsgTransferLeader →
    /// MsgTimeoutNow) makes n4 leader — the one deterministic election
    /// construction under pre_vote/check_quorum, and the ONLY scripted
    /// piece: registration, resolver, wire metadata, and runtime below it
    /// are all production code.
    ///
    /// Pinned preconditions, in order, BEFORE the joiner exists:
    ///   1. the leader is an admitted NON-SEED (not in the root voter set);
    ///   2. a seed FOLLOWER resolves the leader's canonical endpoint from
    ///      its own APPLIED catalog via the production resolver.
    ///
    /// Only then does n5 join with the unchanged seed set {n1,n2,n3}: every
    /// seed answers NotLeader + wire hint, and registration must follow the
    /// hint to the non-seed leader within one pass.
    ///
    /// Mutant contract: deleting the hint-follow (the walk's `queue.push`)
    /// reverts the client to the seeds-only loop — this test then reds at
    /// "n5 must register by following the wire hint", strictly AFTER both
    /// pinned preconditions passed.
    #[test]
    fn a_real_joiner_follows_the_wire_hint_to_a_non_seed_leader_end_to_end() {
        let base = std::env::temp_dir().join(format!(
            "kv9-hint-follow-e2e-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let listeners: Vec<_> = (0..5).map(|_| bound_listener_for_e2e()).collect();
        let addrs: Vec<_> = listeners.iter().map(|l| l.local_addr().unwrap()).collect();
        let mut listeners = listeners.into_iter();
        let voters = (1..=3u64)
            .map(|id| {
                Ok(kv9_common::RootVoter {
                    node_id: NodeId(id),
                    addr: addrs[(id - 1) as usize],
                    store_incarnation: prepare_test_store(&base.join(format!("n{id}")), NodeId(id)),
                })
            })
            .collect::<kv9_common::Result<Vec<_>>>()
            .unwrap();
        let root = RootDescriptor::new(
            kv9_common::ClusterId::mint().unwrap(),
            kv9_common::BootstrapGeneration::mint().unwrap(),
            voters,
            b"hint-follow-bootstrap-credential",
        )
        .unwrap();
        let auth = || RuntimeAuth {
            cluster_token: "hint-follow-cluster-token".into(),
            client_tokens: vec![("acceptance".into(), "hint-follow-client-token".into())],
        };
        let config_for = |id: u64| Config {
            advertise_addr: None,
            addr: addrs[(id - 1) as usize].to_string(),
            data_dir: base.join(format!("n{id}")).to_string_lossy().into_owned(),
            join: Vec::new(),
            wal_streams: 1,
            replication_factor: 3,
        };

        let mut rts: Vec<NodeRuntime> = (1..=3u64)
            .map(|id| {
                NodeRuntime::start_core(
                    NodeId(id),
                    config_for(id),
                    auth(),
                    root.clone(),
                    StoreIdentity::for_voter(&root, NodeId(id)).unwrap(),
                    None,
                    StartOverrides {
                        listener: Some(listeners.next().unwrap()),
                        ..Default::default()
                    },
                )
                .unwrap()
            })
            .collect();
        wait_for(&mut rts, 60, "three initial voters Serving", |rts| {
            rts.iter()
                .all(|rt| rt.node.meta.lock().unwrap().bootstrap.is_serving())
        });

        // Admit + start + promote n4 through the production admin path.
        let leader = cluster_leader(&rts).expect("a serving cluster has a leader");
        let admit4 = backend_view(&rts[leader], &root)
            .admit_node("acceptance", NodeId(4), &addrs[3].to_string(), 600)
            .unwrap();
        let ticket4 = admit4.join_ticket.expect("admission mints a ticket");
        rts.push(
            NodeRuntime::start_core(
                NodeId(4),
                config_for(4),
                auth(),
                root.clone(),
                StoreIdentity::for_joiner(
                    &root,
                    NodeId(4),
                    prepare_test_store(&base.join("n4"), NodeId(4)),
                )
                .unwrap(),
                Some(&ticket4),
                StartOverrides {
                    listener: Some(listeners.next().unwrap()),
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        wait_for(
            &mut rts,
            60,
            "n4 registered and Serving as learner",
            |rts| rts[3].node.meta.lock().unwrap().bootstrap.is_serving(),
        );
        let leader = cluster_leader(&rts).expect("leader");
        backend_view(&rts[leader], &root)
            .promote_node("acceptance", NodeId(4))
            .unwrap();
        wait_for(&mut rts, 60, "n4 a voter on every replica", |rts| {
            rts.iter().all(|rt| rt.driver.status().voters.contains(&4))
        });

        // Election seam (the only scripted piece): hand leadership to n4.
        let leader = cluster_leader(&rts).expect("leader");
        rts[leader]
            .driver
            .peer()
            .transfer_leader_for_tests(NodeId(4));
        wait_for(&mut rts, 60, "n4 leads and every replica knows it", |rts| {
            rts[3].driver.status().role == Role::Leader
                && rts
                    .iter()
                    .all(|rt| rt.driver.status().leader_id == Some(NodeId(4)))
        });

        // PINNED PRECONDITION 1: the leader is NOT a seed — otherwise the
        // seeds-only loop also registers the joiner and the hint arm is
        // never selected (the exact reason the master red was invisible to
        // the previous E2E pass).
        assert!(
            root.voters.iter().all(|voter| voter.node_id != NodeId(4)),
            "precondition: the leader must be an admitted non-seed"
        );
        // PINNED PRECONDITION 2: a seed FOLLOWER resolves the leader's
        // canonical endpoint from its own APPLIED catalog — the production
        // resolver over the real replicated row, no scripting.
        let follower = (0..3)
            .find(|i| rts[*i].driver.status().role != Role::Leader)
            .unwrap();
        assert_eq!(
            backend_view(&rts[follower], &root).resolve_registration_endpoint(NodeId(4)),
            Some(addrs[3].to_string()),
            "precondition: a seed follower must authoritatively resolve the \
             non-seed leader's endpoint from its applied catalog"
        );

        wait_for(
            &mut rts,
            60,
            "every seed publishes the non-seed registration hint",
            |rts| {
                rts[..3].iter().all(|rt| matches!(
                backend_view(rt, &root).register(NodeId(5), &addrs[4].to_string(), root.cluster_id, &[], StoreIncarnation::mint().unwrap()),
                Err(RegistrationError::NotLeader { leader: Some(NodeId(4)), leader_addr: Some(addr) }) if addr == addrs[3].to_string()
            ))
            },
        );

        // The real joiner: seed set is FOREVER {n1,n2,n3} (root descriptor),
        // which excludes the leader — the master-red scene, now pinned.
        let admit5 = backend_view(&rts[3], &root)
            .admit_node("acceptance", NodeId(5), &addrs[4].to_string(), 600)
            .unwrap();
        let ticket5 = admit5.join_ticket.expect("admission mints a ticket");
        rts.push(
            NodeRuntime::start_core(
                NodeId(5),
                config_for(5),
                auth(),
                root.clone(),
                StoreIdentity::for_joiner(
                    &root,
                    NodeId(5),
                    prepare_test_store(&base.join("n5"), NodeId(5)),
                )
                .unwrap(),
                Some(&ticket5),
                StartOverrides {
                    listener: Some(listeners.next().unwrap()),
                    ..Default::default()
                },
            )
            .unwrap(),
        );

        // Freeze n5's APPLY half (raft and registration keep running) so the
        // catch-up window can be probed deterministically on BOTH sides of
        // its production-minted barrier: first while provably behind, then
        // after the exact receipt command has applied.
        rts[4].driver.pause_apply(true);
        wait_for(
            &mut rts,
            60,
            "n5 must register by following the wire hint to the non-seed \
             leader (the seeds-only loop dies here in endless not_leader — \
             the 972-loop master red)",
            |rts| rts[4].registration_receipt.is_some(),
        );
        let capability = rts[4]
            .catchup_capability
            .lock()
            .unwrap()
            .clone()
            .expect("the typed Registered outcome installs the capability");
        let watermark = rts[4].driver.driver_applied();
        assert!(
            watermark.is_none_or(|wm| wm.index < capability.receipt_position.index),
            "precondition: with apply frozen the joiner is still BEHIND the \
             receipt barrier"
        );
        assert!(
            capability.admits(NodeId(4), watermark),
            "behind the production receipt barrier, the non-seed leader — a \
             receipt member with no row in the joiner's catalog — must be \
             admitted: without this window the joiner can never receive the \
             very stream that fills its catalog (the catch-up deadlock)"
        );
        assert!(
            !capability.admits(NodeId(9), watermark),
            "a sender outside the receipt member set is refused even while \
             the window is open"
        );
        rts[4].driver.pause_apply(false);
        // Timeout margin (Cindy's measurement on 19adf3b): the whole test
        // completes in 1.67/1.67/1.82s over three runs against these 60s
        // phase deadlines — a ~36x margin. A red here is NOT "the machine
        // was slow" unless it was 36x slow; treat it as a real regression.
        wait_for(
            &mut rts,
            60,
            "n5 catches up from the non-seed leader and reaches Serving",
            |rts| rts[4].node.meta.lock().unwrap().bootstrap.is_serving(),
        );

        let status = fs::read_to_string(&rts[4].status_path).unwrap();
        assert!(status.lines().any(|line| line
            == format!(
                "registration_receipt_term={}",
                capability.receipt_position.term
            )));
        assert!(status.lines().any(|line| line
            == format!(
                "registration_receipt_index={}",
                capability.receipt_position.index
            )));

        // The window transition on the SAME production capability: Serving
        // implies the exact receipt command applied, and from that instant
        // the fallback answers no to the very sender it admitted above.
        // Deleting the barrier comparison in `admits` reds exactly here.
        let watermark = rts[4].driver.driver_applied();
        assert!(
            watermark.is_some_and(|wm| wm.index >= capability.receipt_position.index),
            "precondition: Serving implies the exact receipt command applied"
        );
        assert!(
            !capability.admits(NodeId(4), watermark),
            "at and after the exact receipt command the window is closed — \
             the sender it existed for is refused, only the catalog speaks"
        );

        // The receipt chain proves HOW it registered: the walk's typed
        // verdict, and the last wire hint it followed — the seed's NotLeader
        // metadata naming the non-seed leader's catalog endpoint.
        let obs = &rts[4].registration_observation;
        assert_eq!(obs.last, RegistrationLastOutcome::Registered);
        assert_eq!(obs.last_walk, Some(WalkTerminal::Registered));
        assert_eq!(
            obs.last_hint.as_deref(),
            Some(format!("leader=4 addr={}", addrs[3]).as_str()),
            "the followed hint must be the seed's wire metadata carrying \
             the leader's catalog endpoint"
        );

        assert!(rts[4].discovery.raft_receive_allowed());
        assert!(rts[4].driver_thread.is_some());
        assert!(rts[4].local_membership_is_active().unwrap());
        assert!(
            local_member_is_active(
                &rts[4].node,
                &rts[4].driver,
                StoreIncarnation::mint().unwrap()
            )
            .is_err(),
            "a different store accepted the durable local membership binding"
        );
        let saved_identity = rts[4].store_identity;
        drop(rts.pop().unwrap());
        fs::remove_file(base.join("n5").join(kv9_meta::bootstrap::INIT_MARKER_FILE)).unwrap();
        let recovered = NodeRuntime::start_core(
            NodeId(5),
            config_for(5),
            auth(),
            root.clone(),
            saved_identity,
            None,
            StartOverrides::default(),
        )
        .unwrap();
        assert!(
            recovered.discovery.raft_receive_allowed(),
            "durable member required a registration response"
        );
        assert!(
            recovered.driver_thread.is_some(),
            "durable member did not start its owner"
        );
        assert_eq!(recovered.registration_observation.attempts, 0);
        assert!(recovered.registration_receipt.is_none());
        rts.push(recovered);
        wait_for(
            &mut rts,
            60,
            "same-store recovery without marker or ticket",
            |rts| rts[4].node.meta.lock().unwrap().bootstrap.is_serving(),
        );
        assert_eq!(
            rts[4].registration_observation.attempts, 0,
            "same-store recovery depended on a registration leader"
        );
        drop(rts);
        let _ = fs::remove_dir_all(&base);
    }

    fn active_store_restart_endpoint_scene(change_address: bool) {
        let (mut rts, root, _, base) = serving_trio("active-endpoint-restart");
        let member = NodeId(4);
        let listener = bound_listener_for_e2e();
        let original_addr = listener.local_addr().unwrap();
        let original_identity =
            StoreIdentity::for_joiner(&root, member, prepare_test_store(&base.join("n4"), member))
                .unwrap();
        let config = |addr: std::net::SocketAddr| Config {
            addr: addr.to_string(),
            advertise_addr: None,
            data_dir: base.join("n4").to_string_lossy().into_owned(),
            join: Vec::new(),
            wal_streams: 1,
            replication_factor: 3,
        };
        let auth = || RuntimeAuth {
            cluster_token: "establishing-read-cluster-token".into(),
            client_tokens: vec![("acceptance".into(), "establishing-read-token".into())],
        };
        let leader = cluster_leader(&rts).unwrap();
        let ticket = backend_view(&rts[leader], &root)
            .admit_node("acceptance", member, &original_addr.to_string(), 600)
            .unwrap()
            .join_ticket
            .unwrap();
        rts.push(
            NodeRuntime::start_core(
                member,
                config(original_addr),
                auth(),
                root.clone(),
                original_identity,
                Some(&ticket),
                StartOverrides {
                    listener: Some(listener),
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        wait_for(
            &mut rts,
            60,
            "original endpoint is registered and applied",
            |rts| {
                rts[3].node.meta.lock().unwrap().bootstrap.is_serving()
                    && rts.iter().all(|rt| {
                        backend_view(rt, &root).resolve_registration_endpoint(member)
                            == Some(original_addr.to_string())
                    })
            },
        );
        assert!(rts[3].registration_receipt.is_some());
        assert!(rts[3].local_membership_is_active().unwrap());
        assert!(init_marker_exists(&base.join("n4")));
        drop(rts.pop().unwrap());

        // Keep the old port allocated during the changed-endpoint scene so
        // the new listener cannot accidentally reuse it. This guard runs no
        // Raft service. The unchanged scene reopens that same real endpoint.
        let old_port = std::net::TcpListener::bind(original_addr).unwrap();
        let listener = if change_address {
            bound_listener_for_e2e()
        } else {
            old_port.try_clone().unwrap()
        };
        let restarted_addr = listener.local_addr().unwrap();
        assert_eq!(restarted_addr != original_addr, change_address);
        let restarted = NodeRuntime::start_core(
            member,
            config(restarted_addr),
            auth(),
            root.clone(),
            original_identity,
            None,
            StartOverrides {
                listener: Some(listener),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            restarted.store_identity.store_incarnation,
            original_identity.store_incarnation
        );
        assert!(restarted.local_membership_is_active().unwrap());
        assert_eq!(restarted.addr, restarted_addr);
        rts.push(restarted);
        for _ in 0..20 {
            step_cluster(&mut rts);
        }
        // No new admission or catalog update was submitted after the first
        // registration. Every durable directory still names the old endpoint.
        assert!(rts.iter().all(|rt| {
            backend_view(rt, &root).resolve_registration_endpoint(member)
                == Some(original_addr.to_string())
        }));
        let restarted = &rts[3];
        let serving = restarted.node.meta.lock().unwrap().bootstrap.is_serving();
        eprintln!(
            "active endpoint restart: original={original_addr} restarted={restarted_addr} \
             serving={serving} registration_attempts={} receipt={:?}",
            restarted.registration_observation.attempts, restarted.registration_receipt
        );
        if change_address {
            assert!(
                !serving,
                "changed endpoint reused old Active membership to enter Serving without a committed route transition"
            );
            assert!(!restarted.endpoint_ready.load(Ordering::Acquire));
            assert!(backend_view(restarted, &root).ensure_serving().is_err());
            assert!(restarted.discovery.raft_receive_allowed());
            assert!(restarted.driver_thread.is_some());

            // Authorization is a public CAS. First apply the changed catalog,
            // then freeze before the separate fresh confirmation. The catalog
            // predicate alone must not substitute for that exact receipt.
            let leader = cluster_leader(&rts).unwrap();
            let mut client = crate::endpoints::EndpointClient::connect(
                &rts[leader].addr.to_string(),
                "acceptance",
            )
            .unwrap();
            let request = crate::api::EndpointChange {
                cluster: root.cluster_id,
                node: member,
                incarnation: original_identity.store_incarnation,
                expected_address: original_addr,
                expected_generation: 0,
                new_address: restarted_addr,
            };
            let mutation = match client.change(request).unwrap() {
                crate::api::EndpointUpdateResult::Changed { applied, .. } => applied,
                other => panic!("migration did not return a mutation receipt: {other:?}"),
            };
            for rt in &rts {
                rt.sync_registered_peers().unwrap();
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            while rts[3].driver.status().applied_index < mutation.index && Instant::now() < deadline
            {
                std::thread::sleep(TICK);
            }
            assert!(
                rts[3].driver.status().applied_index >= mutation.index,
                "changed catalog did not apply before confirmation freeze"
            );
            rts[3].driver.pause_apply(true);
            wait_for(
                &mut rts,
                20,
                "fresh confirmation before local application",
                |rts| rts[3].endpoint_recovery.receipt.is_some(),
            );
            let confirmation = rts[3].endpoint_recovery.receipt.unwrap();
            assert!(confirmation.index > mutation.index);
            for _ in 0..3 {
                step_cluster(&mut rts);
                assert!(
                    !rts[3].endpoint_ready.load(Ordering::Acquire),
                    "changed endpoint served before its exact confirmation applied"
                );
            }
            assert!(
                !rts[3].endpoint_ready.load(Ordering::Acquire),
                "changed endpoint served before its exact confirmation applied"
            );
            assert!(!rts[3].node.meta.lock().unwrap().bootstrap.is_serving());
            assert!(backend_view(&rts[3], &root).ensure_serving().is_err());
            rts[3].driver.pause_apply(false);
            wait_for(
                &mut rts,
                30,
                "authorized changed endpoint serves after exact application",
                |rts| {
                    rts[3].endpoint_ready.load(Ordering::Acquire)
                        && rts[3].node.meta.lock().unwrap().bootstrap.is_serving()
                },
            );
            assert!(matches!(rts[3].driver.wait_applied(ProposedAt {
                term: confirmation.term, index: kv9_raft::LogIndex(confirmation.index),
            }, Duration::from_secs(1)), Ok(ApplyWaitOutcome::Applied(at)) if at == confirmation));
            assert_eq!(client.get(member).unwrap().endpoint.unwrap().generation, 1);
            assert!(rts[3].registration_receipt.is_none());
            assert_eq!(rts[3].registration_observation.attempts, 0);
            assert_eq!(
                kv9_common::load_root_bundle(&base.join("n4")).unwrap(),
                (root.clone(), original_identity)
            );
        } else {
            assert!(serving, "stable endpoint lost coordinator-free recovery");
            assert_eq!(restarted.registration_observation.attempts, 0);
            assert!(restarted.registration_receipt.is_none());
        }
        drop(rts);
        // Both original and migrated stable endpoints recover locally with
        // every other database process stopped. The saved receipt is durable;
        // neither a registration coordinator nor a fresh leader is needed.
        let listener = if change_address {
            std::net::TcpListener::bind(restarted_addr).unwrap()
        } else {
            old_port.try_clone().unwrap()
        };
        let mut stable = NodeRuntime::start_core(
            member,
            config(restarted_addr),
            auth(),
            root.clone(),
            original_identity,
            None,
            StartOverrides {
                listener: Some(listener),
                ..Default::default()
            },
        )
        .unwrap();
        for _ in 0..3 {
            stable.advance_bootstrap().unwrap();
        }
        assert!(
            stable.endpoint_ready.load(Ordering::Acquire),
            "stable migrated endpoint lost its durable serving authority"
        );
        assert!(stable.node.meta.lock().unwrap().bootstrap.is_serving());
        assert_eq!(stable.endpoint_recovery.attempts, 0);
        assert_eq!(stable.registration_observation.attempts, 0);
        assert!(stable.endpoint_recovery.receipt.is_none());
        drop(stable);
        drop(old_port);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn active_store_restart_at_the_same_endpoint_needs_no_registration() {
        active_store_restart_endpoint_scene(false);
    }

    #[test]
    fn active_store_restart_at_a_changed_endpoint_waits_for_committed_route() {
        active_store_restart_endpoint_scene(true);
    }

    #[test]
    fn fresh_joiner_rejects_stale_heartbeat_before_raft_owner_starts() {
        use kv9_raft::grpc::pb;
        use protobuf::Message as _;
        let (rts, root, _, base) = serving_trio("receive-authority");
        let listener = bound_listener_for_e2e();
        let addr = listener.local_addr().unwrap();
        let joiner = NodeRuntime::start_core(
            NodeId(4),
            Config {
                advertise_addr: None,
                addr: addr.to_string(),
                data_dir: base.join("n4").to_string_lossy().into_owned(),
                join: Vec::new(),
                wal_streams: 1,
                replication_factor: 3,
            },
            RuntimeAuth {
                cluster_token: "establishing-read-cluster-token".into(),
                client_tokens: vec![("acceptance".into(), "establishing-read-token".into())],
            },
            root.clone(),
            StoreIdentity::for_joiner(
                &root,
                NodeId(4),
                prepare_test_store(&base.join("n4"), NodeId(4)),
            )
            .unwrap(),
            None,
            StartOverrides {
                listener: Some(listener),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(joiner.driver_thread.is_none(), "unregistered owner started");
        assert!(
            !joiner.discovery.raft_receive_allowed(),
            "unregistered receive authority granted"
        );
        let heartbeat = raft::eraftpb::Message {
            from: 1,
            to: 4,
            term: 2,
            commit: 40,
            msg_type: raft::eraftpb::MessageType::MsgHeartbeat,
            ..Default::default()
        };
        let status = joiner.grpc_runtime.block_on(async {
            let mut client = pb::kv9_raft_client::Kv9RaftClient::connect(format!("http://{addr}"))
                .await
                .unwrap();
            let mut request = tonic::Request::new(tokio_stream::iter(vec![pb::BatchRaftMessage {
                msgs: vec![pb::RaftEnvelope {
                    from_node: 1,
                    to_node: 4,
                    raft_message: heartbeat.write_to_bytes().unwrap(),
                    ..Default::default()
                }],
                root_digest: root.digest().as_bytes().to_vec(),
                ..Default::default()
            }]));
            request.metadata_mut().insert(
                CLUSTER_TOKEN_KEY,
                "establishing-read-cluster-token".parse().unwrap(),
            );
            request
                .metadata_mut()
                .insert(NODE_ID_KEY, "1".parse().unwrap());
            client
                .batch_raft(request)
                .await
                .expect_err("stale heartbeat entered an unauthorized replica")
        });
        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
        assert_eq!(
            status.message(),
            "local replica has no Raft receive authority"
        );
        assert!(
            joiner.transport.drain().is_empty(),
            "stale heartbeat reached the Raft inbox"
        );
        assert_eq!(joiner.driver.status().raft_committed, 0);
        assert!(joiner.driver_thread.is_none());
        drop(joiner);
        drop(rts);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn registration_refusals_preserve_routes_and_renewed_tickets_cannot_rebind() {
        let (rts, root, _, base) = serving_trio("registration-binding");
        let leader = cluster_leader(&rts).unwrap();
        let backend = backend_view(&rts[leader], &root);
        let member = NodeId(4);
        let listener = bound_listener_for_e2e();
        let addr = listener.local_addr().unwrap();
        let original = StoreIncarnation::mint().unwrap();
        let replacement = StoreIncarnation::mint().unwrap();
        let admission = backend
            .admit_node("acceptance", member, &addr.to_string(), 600)
            .unwrap();
        let ticket = RootDigest::sha256(admission.join_ticket.unwrap().as_bytes());
        assert_eq!(rts[leader].transport.peer_address_for_tests(member), None);
        assert!(matches!(
            backend.register(
                member,
                &addr.to_string(),
                root.cluster_id,
                &[0; 32],
                original
            ),
            Err(RegistrationError::InvalidTicket)
        ));
        assert_eq!(
            rts[leader].transport.peer_address_for_tests(member),
            None,
            "invalid ticket installed a transport route"
        );
        let receipt = backend
            .register(
                member,
                &addr.to_string(),
                root.cluster_id,
                ticket.as_bytes(),
                original,
            )
            .unwrap();
        assert!(receipt.applied_index > 0);
        assert_eq!(
            rts[leader].transport.peer_address_for_tests(member),
            Some(addr)
        );
        let changed_addr = bound_listener_for_e2e().local_addr().unwrap();
        assert!(matches!(
            backend.register(
                member,
                &changed_addr.to_string(),
                root.cluster_id,
                ticket.as_bytes(),
                replacement
            ),
            Err(RegistrationError::InvalidIncarnation)
        ));
        assert_eq!(
            rts[leader].transport.peer_address_for_tests(member),
            Some(addr),
            "replacement incarnation changed an existing transport route"
        );
        // A replicated revocation and a new valid ticket do not erase the
        // immutable binding retained in NODES for this numeric replica id.
        {
            let _guard = backend.node.meta_raft.lock_catalog_txn();
            let term = backend.prepare_catalog().unwrap();
            let mut txn = backend.node.meta_raft.store.begin().unwrap();
            kv9_meta::admission::revoke_admission(&mut txn, member).unwrap();
            backend
                .commit_catalog(&kv9_raft::Command::from_batch(&txn.into_batch()), term)
                .unwrap();
        }
        let renewed = backend
            .admit_node("acceptance", member, &changed_addr.to_string(), 600)
            .unwrap();
        let renewed_ticket = RootDigest::sha256(renewed.join_ticket.unwrap().as_bytes());
        let replacement_result = backend.register(
            member,
            &changed_addr.to_string(),
            root.cluster_id,
            renewed_ticket.as_bytes(),
            replacement,
        );
        assert_eq!(
            rts[leader].transport.peer_address_for_tests(member),
            Some(addr),
            "renewed replacement changed the existing transport route"
        );
        let txn = backend.node.meta_raft.store.begin().unwrap();
        assert_eq!(
            kv9_meta::admission::admission(&txn, member)
                .unwrap()
                .unwrap()
                .state,
            kv9_meta::admission::AdmissionState::Pending,
            "rejected replacement consumed the renewed ticket"
        );
        drop(txn);
        // The endpoint writer also validates the immutable store binding.
        // Check the public refusal separately from those side-effect guards.
        assert!(
            matches!(
                replacement_result,
                Err(RegistrationError::InvalidIncarnation)
            ),
            "renewed replacement did not receive a typed incarnation refusal: {replacement_result:?}"
        );
        // The original store can complete the same new admission, including
        // its new canonical address. Rejecting every renewed ticket is wrong.
        backend
            .register(
                member,
                &changed_addr.to_string(),
                root.cluster_id,
                renewed_ticket.as_bytes(),
                original,
            )
            .unwrap();
        let endpoint = kv9_meta::endpoint::node_endpoint(
            &backend.node.meta_raft.store.begin().unwrap(),
            member,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            endpoint.generation, 1,
            "renewed registration bypassed endpoint versioning"
        );
        assert_eq!(endpoint.previous_address, Some(addr));
        assert_eq!(
            rts[leader].transport.peer_address_for_tests(member),
            Some(changed_addr)
        );
        drop(backend);
        drop(rts);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn public_endpoint_cas_returns_distinct_exact_mutation_and_confirmation_receipts() {
        use crate::api::{EndpointChange, EndpointRefusal, EndpointUpdateResult};
        use crate::endpoints::{EndpointClient, EndpointRpcError};
        let (rts, root, addrs, base) = serving_trio("public-endpoint-cas");
        let leader = cluster_leader(&rts).unwrap();
        let backend = backend_view(&rts[leader], &root);
        let member = NodeId(4);
        let original_listener = bound_listener_for_e2e();
        let next_listener = bound_listener_for_e2e();
        let original = original_listener.local_addr().unwrap();
        let next = next_listener.local_addr().unwrap();
        let incarnation = StoreIncarnation::mint().unwrap();
        let admission = backend
            .admit_node("admin", member, &original.to_string(), 600)
            .unwrap();
        let ticket = RootDigest::sha256(admission.join_ticket.unwrap().as_bytes());
        backend
            .register(
                member,
                &original.to_string(),
                root.cluster_id,
                ticket.as_bytes(),
                incarnation,
            )
            .unwrap();
        let request = EndpointChange {
            cluster: root.cluster_id,
            node: member,
            incarnation,
            expected_address: original,
            expected_generation: 0,
            new_address: next,
        };
        let mut rejected =
            EndpointClient::connect(&addrs[leader].to_string(), "wrong-token").unwrap();
        assert!(matches!(
            rejected.change(request),
            Err(EndpointRpcError::Unconfirmed(_))
        ));
        let follower = (leader + 1) % 3;
        let mut wrong_leader =
            EndpointClient::connect(&addrs[follower].to_string(), "acceptance").unwrap();
        let refused = wrong_leader.change(request);
        assert!(
            matches!(refused, Err(EndpointRpcError::NotLeader { .. })),
            "follower lost its typed refusal: {refused:?}"
        );
        let mut client = EndpointClient::connect(&addrs[leader].to_string(), "acceptance").unwrap();
        let before = client.get(member).unwrap();
        assert_eq!(before.cluster, root.cluster_id);
        assert_eq!(before.endpoint.unwrap().generation, 0);
        let changed = match client.change(request).unwrap() {
            EndpointUpdateResult::Changed { endpoint, applied } => {
                assert_eq!((endpoint.address, endpoint.generation), (next, 1));
                applied
            }
            other => panic!("initial endpoint CAS did not return its mutation receipt: {other:?}"),
        };
        assert!(matches!(rts[leader].driver.wait_applied(
            ProposedAt { term: changed.term, index: kv9_raft::LogIndex(changed.index) }, Duration::from_secs(1)),
            Ok(ApplyWaitOutcome::Applied(at)) if at == changed));
        let confirmation = match client.change(request).unwrap() {
            EndpointUpdateResult::Confirmed {
                endpoint,
                confirmation,
            } => {
                assert_eq!((endpoint.address, endpoint.generation), (next, 1));
                confirmation
            }
            other => {
                panic!("duplicate endpoint CAS did not return a fresh confirmation: {other:?}")
            }
        };
        assert!(
            confirmation.index > changed.index,
            "confirmation reused the original mutation receipt"
        );
        assert_eq!(client.get(member).unwrap().endpoint.unwrap().generation, 1);
        assert_eq!(
            client
                .change(EndpointChange {
                    expected_address: next,
                    ..request
                })
                .unwrap(),
            EndpointUpdateResult::Refused(EndpointRefusal::Conflict)
        );
        assert_eq!(
            client
                .change(EndpointChange {
                    incarnation: StoreIncarnation::mint().unwrap(),
                    ..request
                })
                .unwrap(),
            EndpointUpdateResult::Refused(EndpointRefusal::InvalidIncarnation)
        );
        assert_eq!(
            rts[leader].transport.peer_address_for_tests(member),
            Some(next)
        );
        assert_eq!(rts[leader].driver.status().learners, vec![4]);
        drop(client);
        drop(wrong_leader);
        drop(rejected);
        drop(backend);
        drop(rts);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn committed_endpoint_change_revokes_registration_and_rejects_its_retry() {
        let (rts, root, _, base) = serving_trio("endpoint-revoked-retry");
        let leader = cluster_leader(&rts).unwrap();
        let backend = backend_view(&rts[leader], &root);
        let member = NodeId(4);
        let original_listener = bound_listener_for_e2e();
        let replacement_listener = bound_listener_for_e2e();
        let original_addr = original_listener.local_addr().unwrap();
        let replacement_addr = replacement_listener.local_addr().unwrap();
        let incarnation = StoreIncarnation::mint().unwrap();
        let granted = backend
            .admit_node("admin", member, &original_addr.to_string(), 600)
            .unwrap();
        let ticket = RootDigest::sha256(granted.join_ticket.unwrap().as_bytes());
        backend
            .register(
                member,
                &original_addr.to_string(),
                root.cluster_id,
                ticket.as_bytes(),
                incarnation,
            )
            .unwrap();
        let applied = {
            let _guard = backend.node.meta_raft.lock_catalog_txn();
            let term = backend.prepare_catalog().unwrap();
            let mut txn = backend.node.meta_raft.store.begin().unwrap();
            assert!(matches!(
                kv9_meta::endpoint::change_endpoint(
                    &mut txn,
                    kv9_meta::endpoint::EndpointChange {
                        cluster: root.cluster_id,
                        node: member,
                        incarnation,
                        expected_address: original_addr,
                        expected_generation: 0,
                        new_address: replacement_addr,
                    }
                )
                .unwrap(),
                kv9_meta::endpoint::EndpointChangeOutcome::Changed(_)
            ));
            backend
                .commit_catalog(&kv9_raft::Command::from_batch(&txn.into_batch()), term)
                .unwrap()
        };
        assert!(applied.index > 0);
        rts[leader].sync_registered_peers().unwrap();
        assert_eq!(
            rts[leader].transport.peer_address_for_tests(member),
            Some(replacement_addr)
        );
        assert!(
            backend
                .register(
                    member,
                    &original_addr.to_string(),
                    root.cluster_id,
                    ticket.as_bytes(),
                    incarnation
                )
                .is_err(),
            "old registration was accepted after its endpoint authority was revoked"
        );
        assert_eq!(
            rts[leader].transport.peer_address_for_tests(member),
            Some(replacement_addr),
            "old registration restored a superseded transport endpoint"
        );
        let txn = backend.node.meta_raft.store.begin().unwrap();
        assert_eq!(
            kv9_meta::admission::admission(&txn, member)
                .unwrap()
                .unwrap()
                .state,
            kv9_meta::admission::AdmissionState::Superseded
        );
        let endpoint = kv9_meta::endpoint::node_endpoint(&txn, member)
            .unwrap()
            .unwrap();
        assert_eq!(
            (endpoint.generation, endpoint.address),
            (1, replacement_addr)
        );
        drop(txn);
        // Recreate an obsolete consumed admission left by a writer that did
        // not revoke it. Address validation must independently refuse this
        // retry before its eager transport update, even with a valid ticket.
        {
            let _guard = backend.node.meta_raft.lock_catalog_txn();
            let term = backend.prepare_catalog().unwrap();
            let mut txn = backend.node.meta_raft.store.begin().unwrap();
            txn.update(
                &kv9_meta::schema::NODE_ADMISSIONS_DESC,
                &[memcmp_uint(member.0)],
                vec![(
                    ColumnId(5),
                    ColumnValue::Uint(kv9_meta::admission::AdmissionState::Consumed as u64),
                )],
            )
            .unwrap();
            backend
                .commit_catalog(&kv9_raft::Command::from_batch(&txn.into_batch()), term)
                .unwrap();
        }
        assert!(
            backend
                .register(
                    member,
                    &original_addr.to_string(),
                    root.cluster_id,
                    ticket.as_bytes(),
                    incarnation
                )
                .is_err(),
            "obsolete consumed admission bypassed current endpoint validation"
        );
        assert_eq!(
            rts[leader].transport.peer_address_for_tests(member),
            Some(replacement_addr)
        );
        drop(backend);
        drop(rts);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn delayed_catalog_snapshot_cannot_restore_an_older_live_route() {
        use kv9_raft::transport::RaftTransport;
        let (rts, root, _, base) = serving_trio("endpoint-snapshot-order");
        let leader = cluster_leader(&rts).unwrap();
        let runtime = &rts[leader];
        let backend = backend_view(runtime, &root);
        let member = NodeId(4);
        let mut addresses = Vec::new();
        let mut receivers = Vec::new();
        for _ in 0..2 {
            let listener = bound_listener_for_e2e();
            addresses.push(listener.local_addr().unwrap());
            listener.set_nonblocking(true).unwrap();
            let incoming = {
                let _entered = runtime.grpc_runtime.enter();
                tokio_stream::wrappers::TcpListenerStream::new(
                    tokio::net::TcpListener::from_std(listener).unwrap(),
                )
            };
            let receiver = GrpcTransport::new(
                member,
                None,
                runtime.grpc_runtime.handle().clone(),
                root.digest(),
            );
            let tx = receiver.inbox_sender();
            receivers.push(receiver);
            let discovery = Arc::new(RuntimeDiscovery::new(
                member,
                false,
                0,
                RootWireIdentity {
                    bootstrap_generation: root.bootstrap_generation,
                    root_digest: root.digest(),
                },
            ));
            discovery.authorize_raft();
            let service = RaftGrpcService::new(member, tx, discovery);
            let token = runtime.cluster_token.clone();
            runtime.grpc_runtime.spawn(async move {
                tonic::transport::Server::builder()
                    .add_service(
                        kv9_raft::grpc::pb::kv9_raft_server::Kv9RaftServer::with_interceptor(
                            service,
                            kv9_raft::grpc::cluster_token_interceptor(token),
                        ),
                    )
                    .serve_with_incoming(incoming)
                    .await
                    .unwrap();
            });
        }
        let incarnation = StoreIncarnation::mint().unwrap();
        let granted = backend
            .admit_node("admin", member, &addresses[0].to_string(), 600)
            .unwrap();
        let ticket = RootDigest::sha256(granted.join_ticket.unwrap().as_bytes());
        backend
            .register(
                member,
                &addresses[0].to_string(),
                root.cluster_id,
                ticket.as_bytes(),
                incarnation,
            )
            .unwrap();
        let send = |context: &[u8]| {
            runtime.transport.send(
                member,
                raft::eraftpb::Message {
                    from: runtime.node.id.0,
                    to: member.0,
                    context: context.to_vec().into(),
                    ..Default::default()
                },
            )
        };
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            send(b"before-snapshot");
            if receivers[0]
                .drain()
                .into_iter()
                .any(|message| message.context.as_ref() == b"before-snapshot")
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "original endpoint never received the control message"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let (captured, captured_rx) = std::sync::mpsc::channel();
        let (resume, resumed) = std::sync::mpsc::channel();
        *runtime.route_snapshot_gate.lock().unwrap() = Some((captured, resumed));
        let write = || {
            let term = backend.prepare_catalog().unwrap();
            let mut txn = backend.node.meta_raft.store.begin().unwrap();
            assert!(matches!(
                kv9_meta::endpoint::change_endpoint(
                    &mut txn,
                    kv9_meta::endpoint::EndpointChange {
                        cluster: root.cluster_id,
                        node: member,
                        incarnation,
                        expected_address: addresses[0],
                        expected_generation: 0,
                        new_address: addresses[1],
                    }
                )
                .unwrap(),
                kv9_meta::endpoint::EndpointChangeOutcome::Changed(_)
            ));
            backend
                .commit_catalog(&kv9_raft::Command::from_batch(&txn.into_batch()), term)
                .unwrap();
            backend
                .transport
                .register_catalog_peer(member, addresses[1], 1)
                .unwrap();
        };
        std::thread::scope(|scope| {
            let sync = scope.spawn(|| runtime.sync_registered_peers());
            captured_rx.recv_timeout(Duration::from_secs(3)).unwrap();
            // Try the real planner mutex at the captured-snapshot cut. If a
            // mutant omitted the lock, the writer commits first and the stale
            // sync subsequently overwrites it. Otherwise the writer follows
            // the completed sync. Neither order depends on a scheduling sleep.
            let wrote_early = if let Some(_guard) = backend.node.meta_raft.try_lock_catalog_txn() {
                write();
                true
            } else {
                false
            };
            resume.send(()).unwrap();
            sync.join().unwrap().unwrap();
            if !wrote_early {
                let _guard = backend.node.meta_raft.lock_catalog_txn();
                write();
            }
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut old_deliveries = 0;
        loop {
            send(b"after-snapshot");
            for message in receivers[0].drain() {
                old_deliveries += usize::from(message.context.as_ref() == b"after-snapshot");
            }
            if receivers[1]
                .drain()
                .into_iter()
                .any(|message| message.context.as_ref() == b"after-snapshot")
            {
                break;
            }
            assert!(Instant::now() < deadline,
                "stale catalog snapshot restored the old live endpoint: old deliveries={old_deliveries}");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(old_deliveries, 0);
        drop(backend);
        drop(rts);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn registration_response_fallback_defers_to_the_applied_directory() {
        let (rts, root, _, base) = serving_trio("endpoint-response-route");
        let runtime = &rts[cluster_leader(&rts).unwrap()];
        let peer = root
            .voters
            .iter()
            .find(|voter| voter.node_id != runtime.node.id)
            .unwrap();
        let stale_listener = bound_listener_for_e2e();
        let stale = stale_listener.local_addr().unwrap();
        assert_ne!(stale, peer.addr);
        runtime
            .install_registration_response_route((peer.node_id, stale))
            .unwrap();
        assert_eq!(
            runtime.transport.peer_address_for_tests(peer.node_id),
            Some(peer.addr),
            "registration response overwrote an applied endpoint with its dial address"
        );
        // A fresh joiner still needs the successfully contacted leader before
        // its own catalog catches up; absence must preserve that bootstrap path.
        runtime
            .install_registration_response_route((NodeId(100), stale))
            .unwrap();
        assert_eq!(
            runtime.transport.peer_address_for_tests(NodeId(100)),
            Some(stale)
        );
        drop(rts);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn failed_listener_bind_releases_every_store_owner_before_unlocking() {
        let (rts, root, addrs, base) = unformed_trio("failed-start-owner", true);
        drop(rts);
        let held = std::net::TcpListener::bind(addrs[0]).unwrap();
        let observed = Arc::new(std::sync::Mutex::new(None));
        let captured = observed.clone();
        let refused = NodeRuntime::start_core(
            NodeId(1),
            Config {
                addr: addrs[0].to_string(),
                data_dir: base.join("n1").to_string_lossy().into_owned(),
                ..Default::default()
            },
            RuntimeAuth {
                cluster_token: "establishing-read-cluster-token".into(),
                client_tokens: vec![("acceptance".into(), "establishing-read-token".into())],
            },
            root.clone(),
            StoreIdentity::for_voter(&root, NodeId(1)).unwrap(),
            None,
            StartOverrides {
                adjudicator: Some(Box::new(move |node| {
                    *captured.lock().unwrap() = Some(Arc::downgrade(&node));
                    Arc::new(CatalogFenceAdjudicator::new(node))
                })),
                ..Default::default()
            },
        );
        assert!(
            matches!(refused, Err(Error::Config(ref reason)) if reason.contains("bind gRPC listener"))
        );
        assert!(
            observed
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .upgrade()
                .is_none(),
            "failed startup left a detached Raft owner using an unlocked store"
        );
        let guard = kv9_common::store_lifecycle::StoreGuard::lock(&base.join("n1")).unwrap();
        drop(guard);
        drop(held);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn root_voter_rejects_another_disk_and_missing_activated_log_before_starting() {
        let (mut rts, root, addrs, base) = serving_trio("root-store-recovery");
        let identity = rts[0].store_identity;
        assert_eq!(
            rts[0]._store_guard.record().unwrap().phase,
            kv9_common::store_lifecycle::StorePhase::Active(root.digest()),
            "Raft owner started before durable store activation"
        );
        let auth = || RuntimeAuth {
            cluster_token: "establishing-read-cluster-token".into(),
            client_tokens: vec![("acceptance".into(), "establishing-read-token".into())],
        };
        let config = |directory: &Path| Config {
            advertise_addr: None,
            addr: addrs[0].to_string(),
            data_dir: directory.to_string_lossy().into_owned(),
            join: Vec::new(),
            wal_streams: 1,
            replication_factor: 3,
        };
        let replacement = base.join("replacement");
        assert_ne!(
            prepare_test_store(&replacement, NodeId(1)),
            identity.store_incarnation
        );
        let refused = NodeRuntime::start_core(
            NodeId(1),
            config(&replacement),
            auth(),
            root.clone(),
            identity,
            None,
            StartOverrides::default(),
        );
        assert!(
            matches!(refused, Err(Error::Config(ref reason)) if reason == "prepared store identity does not match root/store identity"),
            "a new disk acquired an initial root voter's identity"
        );
        assert!(
            !replacement.join("raft").exists(),
            "rejected root replacement opened Raft storage"
        );
        let duplicate = NodeRuntime::start_core(
            NodeId(1),
            config(&base.join("n1")),
            auth(),
            root.clone(),
            identity,
            None,
            StartOverrides::default(),
        );
        assert!(
            matches!(duplicate, Err(Error::Config(ref reason)) if reason.contains("already owned")),
            "two runtimes concurrently opened one durable store"
        );
        drop(rts.remove(0));
        let path = base.join("n1/raft/raft.log");
        let saved = base.join("original-raft.log");
        fs::rename(&path, &saved).unwrap();
        for _ in 0..2 {
            let refused = NodeRuntime::start_core(
                NodeId(1),
                config(&base.join("n1")),
                auth(),
                root.clone(),
                identity,
                None,
                StartOverrides::default(),
            );
            assert!(
                matches!(refused, Err(Error::Raft(_))),
                "missing activated log was recreated for an old voter"
            );
            assert!(
                !path.exists(),
                "a rejected recovery created an empty replacement log"
            );
        }
        fs::rename(saved, path).unwrap();
        rts.push(
            NodeRuntime::start_core(
                NodeId(1),
                config(&base.join("n1")),
                auth(),
                root.clone(),
                identity,
                None,
                StartOverrides::default(),
            )
            .unwrap(),
        );
        wait_for(
            &mut rts,
            60,
            "original root voter recovers retained log",
            |rts| {
                rts.iter()
                    .all(|rt| rt.node.meta.lock().unwrap().bootstrap.is_serving())
            },
        );
        // A verified legacy store can acquire the lifecycle record from its
        // exact committed root certificate; an identity bundle alone cannot.
        drop(rts.pop().unwrap());
        fs::remove_file(
            base.join("n1")
                .join(kv9_common::store_lifecycle::STORE_LIFECYCLE_FILE),
        )
        .unwrap();
        let recovered = NodeRuntime::start_core(
            NodeId(1),
            config(&base.join("n1")),
            auth(),
            root.clone(),
            identity,
            None,
            StartOverrides::default(),
        )
        .unwrap();
        assert_eq!(
            recovered._store_guard.record().unwrap().phase,
            kv9_common::store_lifecycle::StorePhase::Active(root.digest())
        );
        assert!(recovered.discovery.raft_receive_allowed());
        drop(recovered);
        drop(rts);
        fs::remove_dir_all(base).unwrap();
    }

    /// A Serving 3-voter cluster over real disks and real gRPC — the same
    /// construction the hint-follow product-chain test opens with, extracted
    /// for the establishing-read cells (that test's copy stays inline: it is
    /// a reviewed object and its scene continues past the trio).
    fn unformed_trio(
        tag: &str,
        defer_owner: bool,
    ) -> (
        Vec<NodeRuntime>,
        RootDescriptor,
        Vec<std::net::SocketAddr>,
        PathBuf,
    ) {
        let base = std::env::temp_dir().join(format!(
            "kv9-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let listeners: Vec<_> = (0..3).map(|_| bound_listener_for_e2e()).collect();
        let addrs: Vec<_> = listeners.iter().map(|l| l.local_addr().unwrap()).collect();
        let mut listeners = listeners.into_iter();
        let voters = (1..=3u64)
            .map(|id| {
                Ok(kv9_common::RootVoter {
                    node_id: NodeId(id),
                    addr: addrs[(id - 1) as usize],
                    store_incarnation: prepare_test_store(&base.join(format!("n{id}")), NodeId(id)),
                })
            })
            .collect::<kv9_common::Result<Vec<_>>>()
            .unwrap();
        let root = RootDescriptor::new(
            kv9_common::ClusterId::mint().unwrap(),
            kv9_common::BootstrapGeneration::mint().unwrap(),
            voters,
            b"establishing-read-bootstrap-credential",
        )
        .unwrap();
        let rts: Vec<NodeRuntime> = (1..=3u64)
            .map(|id| {
                NodeRuntime::start_core(
                    NodeId(id),
                    Config {
                        advertise_addr: None,
                        addr: addrs[(id - 1) as usize].to_string(),
                        data_dir: base.join(format!("n{id}")).to_string_lossy().into_owned(),
                        join: Vec::new(),
                        wal_streams: 1,
                        replication_factor: 3,
                    },
                    RuntimeAuth {
                        cluster_token: "establishing-read-cluster-token".into(),
                        client_tokens: vec![(
                            "acceptance".into(),
                            "establishing-read-token".into(),
                        )],
                    },
                    root.clone(),
                    StoreIdentity::for_voter(&root, NodeId(id)).unwrap(),
                    None,
                    StartOverrides {
                        listener: Some(listeners.next().unwrap()),
                        defer_owner,
                        ..Default::default()
                    },
                )
                .unwrap()
            })
            .collect();
        (rts, root, addrs, base)
    }

    fn serving_trio(
        tag: &str,
    ) -> (
        Vec<NodeRuntime>,
        RootDescriptor,
        Vec<std::net::SocketAddr>,
        PathBuf,
    ) {
        let (mut rts, root, addrs, base) = unformed_trio(tag, false);
        wait_for(&mut rts, 60, "establishing-read trio Serving", |rts| {
            rts.iter()
                .all(|rt| rt.node.meta.lock().unwrap().bootstrap.is_serving())
        });
        (rts, root, addrs, base)
    }

    #[test]
    fn original_stores_resume_formation_after_each_pre_catalog_crash_cut() {
        fn wait_driver(rts: &[NodeRuntime], predicate: impl Fn(&[NodeRuntime]) -> bool) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !predicate(rts) {
                assert!(rts.iter().all(|rt| rt.driver.status().fatal.is_none()));
                assert!(
                    Instant::now() < deadline,
                    "pre-catalog Raft cut was not reached"
                );
                std::thread::sleep(TICK);
            }
        }
        fn reopen(root: &RootDescriptor, base: &Path) -> Vec<NodeRuntime> {
            root.voters
                .iter()
                .map(|voter| {
                    NodeRuntime::start_core(
                        voter.node_id,
                        Config {
                            advertise_addr: None,
                            addr: voter.addr.to_string(),
                            data_dir: base
                                .join(format!("n{}", voter.node_id.0))
                                .to_string_lossy()
                                .into_owned(),
                            join: Vec::new(),
                            wal_streams: 1,
                            replication_factor: 3,
                        },
                        RuntimeAuth {
                            cluster_token: "establishing-read-cluster-token".into(),
                            client_tokens: vec![(
                                "acceptance".into(),
                                "establishing-read-token".into(),
                            )],
                        },
                        root.clone(),
                        StoreIdentity::for_voter(root, voter.node_id).unwrap(),
                        None,
                        StartOverrides::default(),
                    )
                    .unwrap()
                })
                .collect()
        }
        for cut in [
            "initial-confstate",
            "election-only",
            "committed-unapplied-catalog",
        ] {
            let (mut rts, root, _, base) = unformed_trio(cut, cut == "initial-confstate");
            assert!(rts
                .iter()
                .all(|rt| rt.node.local_cluster_identity().unwrap().is_none()));
            if cut == "initial-confstate" {
                assert!(rts.iter().all(|rt| rt.driver_thread.is_none()
                    && rt.driver.status().term == 0
                    && rt.driver.status().raft_committed == 0));
            } else {
                wait_driver(&rts, |rts| {
                    cluster_leader(rts).is_some_and(|leader| {
                        let status = rts[leader].driver.status();
                        rts.iter().all(|rt| {
                            rt.driver
                                .driver_applied()
                                .is_some_and(|at| at.term == status.term)
                        })
                    })
                });
                if cut == "committed-unapplied-catalog" {
                    for rt in &rts {
                        rt.driver.pause_apply(true);
                    }
                    for rt in &mut rts {
                        rt.advance_discovery().unwrap();
                        rt.advance_election().unwrap();
                    }
                    let leader = cluster_leader(&rts).unwrap();
                    rts[leader].advance_initialization().unwrap();
                    let (at, _) = rts[leader]
                        .initial_proposal
                        .expect("real initialization must append a catalog command");
                    wait_driver(&rts, |rts| {
                        rts.iter()
                            .all(|rt| rt.driver.status().raft_committed >= at.index.0)
                    });
                    assert!(rts
                        .iter()
                        .all(|rt| rt.driver.driver_applied().unwrap().index < at.index.0));
                }
            }
            for rt in &rts {
                assert!(!init_marker_exists(&rt.data_dir));
                assert!(rt.node.local_cluster_identity().unwrap().is_none());
                assert_eq!(
                    rt._store_guard.record().unwrap().phase,
                    kv9_common::store_lifecycle::StorePhase::Active(root.digest())
                );
            }
            drop(rts);
            rts = reopen(&root, &base);
            wait_for(
                &mut rts,
                10,
                "original root formation resumes after a pre-catalog crash",
                |rts| {
                    rts.iter()
                        .all(|rt| rt.node.meta.lock().unwrap().bootstrap.is_serving())
                },
            );
            for rt in &rts {
                rt.verify_certified_root().unwrap();
                assert_eq!(
                    rt.node.local_cluster_identity().unwrap(),
                    Some(root.cluster_id)
                );
            }
            wait_driver(&rts, |rts| {
                cluster_leader(rts).is_some_and(|leader| {
                    let status = rts[leader].driver.status();
                    rts.iter().all(|rt| {
                        rt.driver
                            .driver_applied()
                            .is_some_and(|at| at.term == status.term)
                    })
                })
            });
            let leader = cluster_leader(&rts).unwrap();
            let created = backend_view(&rts[leader], &root)
                .create_keyspace(
                    "acceptance",
                    "formation-survives",
                    TenantId::DEFAULT,
                    ApiType::Raw,
                    TxnGroupId(0),
                )
                .unwrap();
            wait_driver(&rts, |rts| {
                rts.iter().all(|rt| {
                    rt.driver
                        .driver_applied()
                        .is_some_and(|at| at.index >= created.proposed.unwrap().index)
                })
            });
            drop(rts);
            for voter in &root.voters {
                fs::remove_file(
                    base.join(format!("n{}", voter.node_id.0))
                        .join(kv9_meta::bootstrap::INIT_MARKER_FILE),
                )
                .unwrap();
            }
            rts = reopen(&root, &base);
            wait_for(
                &mut rts,
                10,
                "committed catalog survives marker-loss restart",
                |rts| {
                    rts.iter()
                        .all(|rt| rt.node.meta.lock().unwrap().bootstrap.is_serving())
                },
            );
            wait_driver(&rts, |rts| {
                cluster_leader(rts).is_some_and(|leader| {
                    let status = rts[leader].driver.status();
                    rts.iter().all(|rt| {
                        rt.driver
                            .driver_applied()
                            .is_some_and(|at| at.term == status.term)
                    })
                })
            });
            let leader = cluster_leader(&rts).unwrap();
            let recovered = backend_view(&rts[leader], &root)
                .list_keyspaces("acceptance")
                .unwrap();
            assert!(
                recovered.iter().any(|keyspace| {
                    keyspace.name == "formation-survives" && keyspace.id == created.keyspace
                }),
                "formation retry lost an acknowledged catalog row"
            );
            let next = backend_view(&rts[leader], &root)
                .create_keyspace(
                    "acceptance",
                    "formation-next",
                    TenantId::DEFAULT,
                    ApiType::Raw,
                    TxnGroupId(0),
                )
                .unwrap();
            assert!(
                next.keyspace > created.keyspace,
                "formation retry reset the catalog allocator"
            );
            for rt in &rts {
                rt.verify_certified_root().unwrap();
            }
            drop(rts);
            fs::remove_dir_all(base).unwrap();
            eprintln!("PASS: original root formation recovered at {cut}");
        }
    }

    /// The committed-but-unapplied window, end to end on the PRODUCT read
    /// path (card cell c + the interface ruling's behavioral layer 2):
    ///
    /// 1. `v1` applied; then the leader's APPLY half is frozen and `v2`
    ///    COMMITS under it — pinned by name as committed-but-unapplied.
    /// 2. A `raw_get` starts and provably enters its read barrier while the
    ///    freeze holds (pinned via the barrier mint counter) — so the key
    ///    transitions committed→applied strictly INSIDE the barrier window,
    ///    never before it (Cindy's precondition: without this, a stale view
    ///    could miss `v2` for reasons other than snapshot order).
    /// 3. BYPASS CONTROL, same instant: a read built the pre-establishing
    ///    way (leadership check + direct engine snapshot, no barrier) serves
    ///    the STALE view — it misses the committed `v2`. This is the exact
    ///    defect the establishing seam closes, demonstrated in the same
    ///    scene that proves the seam closes it. (Its direct `.snapshot()`
    ///    call is the ONE deliberate non-seam use in this crate — listed in
    ///    the seam doc inventory.)
    /// 4. Unfreeze: the established read returns and MUST serve `v2`.
    ///    Moving the production snapshot before the barrier — or bypassing
    ///    the credential — serves `v1` here and reds at the named assert.
    #[test]
    fn an_established_read_serves_the_committed_but_unapplied_write() {
        established_read_observes_committed_apply(false);
    }

    #[test]
    fn an_async_prepared_read_serves_the_committed_but_unapplied_write() {
        established_read_observes_committed_apply(true);
    }

    fn established_read_observes_committed_apply(prepared: bool) {
        let (rts, root, _addrs, base) = serving_trio(if prepared {
            "async-established-read"
        } else {
            "established-read"
        });
        let leader = cluster_leader(&rts).expect("a serving trio has a leader");
        let backend = backend_view(&rts[leader], &root);
        let created = backend
            .create_keyspace(
                "acceptance",
                "lin",
                TenantId::DEFAULT,
                ApiType::Raw,
                TxnGroupId(0),
            )
            .unwrap();
        let location = backend
            .get_region("acceptance", created.keyspace, b"k")
            .unwrap();
        let ctx = RequestContext {
            keyspace: created.keyspace,
            region_epoch: location.epoch,
            origin: crate::api::RequestOrigin::from_transport("acceptance"),
        };
        backend
            .raw_put(&ctx, b"k".to_vec(), b"v1".to_vec())
            .unwrap();

        let driver = rts[leader].driver.clone();
        driver.pause_apply(true);
        std::thread::scope(|scope| {
            let put_backend = backend_view(&rts[leader], &root);
            let put_ctx = ctx.clone();
            let put =
                scope.spawn(move || put_backend.raw_put(&put_ctx, b"k".to_vec(), b"v2".to_vec()));

            // NAMED PRECONDITION 1: v2 is committed-but-unapplied — the raft
            // log accepted it while the frozen apply half has not.
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            loop {
                let status = driver.status();
                let applied = driver.driver_applied().map_or(0, |wm| wm.index);
                if status.raft_committed > applied {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "precondition: v2 must COMMIT while apply is frozen \
                     (freeze stops apply, not commits)"
                );
                std::thread::sleep(Duration::from_millis(2));
            }

            let mints_before = driver.read_barriers_minted();
            let get_backend = backend_view(&rts[leader], &root);
            let get_ctx = ctx.clone();
            let get = scope.spawn(move || {
                if prepared {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_time()
                        .build()
                        .unwrap();
                    let job = runtime
                        .block_on(Arc::new(get_backend).prepare_raw_get(get_ctx, b"k".to_vec()))?;
                    job.run()
                } else {
                    get_backend.raw_get(&get_ctx, b"k")
                }
            });

            // NAMED PRECONDITION 2: the read has minted its barrier while
            // the freeze still holds — v2 becomes applied strictly INSIDE
            // the barrier window. Without this pin, "the stale view misses
            // v2" could hold for timing reasons unrelated to snapshot order.
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while driver.read_barriers_minted() == mints_before {
                assert!(
                    std::time::Instant::now() < deadline,
                    "precondition: the established read must mint its barrier \
                     while apply is still frozen"
                );
                std::thread::sleep(Duration::from_millis(1));
            }

            // BYPASS CONTROL (card cell c): the pre-establishing read shape —
            // leadership believed, snapshot taken directly, NO barrier. It
            // serves the stale view: v2, though committed, is invisible.
            let stale_view = rts[leader]
                .node
                .meta_raft
                .store
                .engine()
                .snapshot()
                .unwrap();
            let stale_read = LeaderRead::new(stale_view.as_ref(), true, None).unwrap();
            let stale = RawExecutor.get(&stale_read, ctx.keyspace, b"k").unwrap();
            assert_eq!(
                stale.as_deref(),
                Some(b"v1".as_slice()),
                "bypass control: a read skipping the establishing seam must \
                 serve the STALE view during the window — it misses the \
                 committed-but-unapplied v2 (this is the defect the barrier \
                 closes; if this ever sees v2 the control lost its teeth)"
            );

            driver.pause_apply(false);
            put.join().unwrap().unwrap();
            let got = get.join().unwrap().unwrap();
            assert_eq!(
                got.as_deref(),
                Some(b"v2".as_slice()),
                "the established read must serve every write committed before \
                 its barrier — a production snapshot taken before the barrier \
                 (or bypassing the credential) serves v1 and reds exactly here"
            );
            if prepared {
                assert_eq!(
                    driver.async_read_snapshot().peak,
                    1,
                    "prepared read fell back to the synchronous barrier"
                );
                assert_eq!(driver.async_read_snapshot().in_flight, 0);
            }
        });
        drop(rts);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn a_contended_prepared_read_checks_the_epoch_when_its_engine_job_runs() {
        prepared_read_epoch_case(true);
    }

    #[test]
    fn a_resident_prepared_read_finishes_on_one_authorized_version() {
        prepared_read_epoch_case(false);
    }

    fn prepared_read_epoch_case(force_blocking: bool) {
        use kv9_meta::codec::{memcmp_uint, ColumnValue};
        use kv9_meta::schema::{ColumnId, REGIONS_DESC};

        let (rts, root, _addrs, base) = serving_trio("async-read-epoch");
        let leader = cluster_leader(&rts).expect("a serving trio has a leader");
        let backend = Arc::new(backend_view(&rts[leader], &root));
        let executor = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        for (column, name) in [(5, "async-conf-epoch"), (6, "async-version-epoch")] {
            let created = backend
                .create_keyspace(
                    "acceptance",
                    name,
                    TenantId::DEFAULT,
                    ApiType::Raw,
                    TxnGroupId(0),
                )
                .unwrap();
            let location = backend
                .get_region("acceptance", created.keyspace, b"k")
                .unwrap();
            let ctx = RequestContext {
                keyspace: created.keyspace,
                region_epoch: location.epoch,
                origin: crate::api::RequestOrigin::from_transport("acceptance"),
            };
            backend
                .raw_put(&ctx, b"k".to_vec(), b"before".to_vec())
                .unwrap();
            let barrier =
                executor.block_on(backend.driver.read_barrier_async(READ_BARRIER_DEADLINE));
            // Hold the lifecycle lock after quorum completion to deterministically
            // select the real contention fallback without blocking the Raft owner.
            let held = force_blocking.then(|| backend.node.meta.lock().unwrap());
            let mut prepared = backend
                .clone()
                .finish_prepared_get(ctx.clone(), b"k".to_vec(), barrier)
                .unwrap();
            drop(held);
            if force_blocking {
                assert!(
                    matches!(prepared, crate::api::RawReadJob::Blocking(_)),
                    "a contended lifecycle check must defer the engine job"
                );
            } else {
                // Background lifecycle publication may briefly contend. Repeat
                // only reads until the actual inline branch is observed.
                let until = std::time::Instant::now() + Duration::from_secs(5);
                while matches!(prepared, crate::api::RawReadJob::Blocking(_)) {
                    assert!(
                        std::time::Instant::now() < until,
                        "quiescent resident read never completed without a blocking job"
                    );
                    prepared = executor
                        .block_on(backend.clone().prepare_raw_get(ctx.clone(), b"k".to_vec()))
                        .unwrap();
                }
            }
            assert!(
                backend.driver.async_read_snapshot().peak > 0,
                "epoch test must use asynchronous preparation"
            );

            // A blocking job has not read the engine. Completed already read
            // the old authorized version. Commit an epoch change before consuming
            // either return value; each must reflect its own execution cut.
            let region = Tables::new(&backend.node.meta_raft.store)
                .region_for_key(ctx.keyspace, b"k")
                .unwrap()
                .unwrap();
            let mut next_ctx = ctx.clone();
            let next = if column == 5 {
                next_ctx.region_epoch.conf_ver += 1;
                next_ctx.region_epoch.conf_ver
            } else {
                next_ctx.region_epoch.version += 1;
                next_ctx.region_epoch.version
            };
            let term = backend.prepare_catalog().unwrap();
            let mut change = backend.node.meta_raft.store.begin().unwrap();
            change
                .update(
                    &REGIONS_DESC,
                    &[memcmp_uint(region.id.0)],
                    vec![(ColumnId(column), ColumnValue::Uint(next))],
                )
                .unwrap();
            backend
                .commit_catalog(&Command::from_batch(&change.into_batch()), term)
                .unwrap();
            backend
                .raw_put(&next_ctx, b"k".to_vec(), b"after".to_vec())
                .unwrap();

            let result = prepared.run();
            if force_blocking {
                assert!(
                    matches!(result, Err(Error::StaleEpoch { region: id }) if id == region.id),
                    "prepared job served data under an obsolete epoch: {result:?}"
                );
            } else {
                assert_eq!(
                    result.unwrap().as_deref(),
                    Some(b"before".as_slice()),
                    "completed resident read was reevaluated on a later version"
                );
            }
            let stale = executor
                .block_on(backend.clone().prepare_raw_get(ctx.clone(), b"k".to_vec()))
                .and_then(crate::api::RawReadJob::run);
            assert!(
                matches!(stale, Err(Error::StaleEpoch { region: id }) if id == region.id),
                "a fresh prepared read accepted the obsolete epoch: {stale:?}"
            );
            let current = executor
                .block_on(backend.clone().prepare_raw_get(next_ctx, b"k".to_vec()))
                .unwrap()
                .run()
                .unwrap();
            assert_eq!(
                current.as_deref(),
                Some(b"after".as_slice()),
                "current context must observe the post-change value"
            );
        }
        assert_eq!(backend.driver.async_read_snapshot().in_flight, 0);
        drop(backend);
        drop(rts);
        let _ = fs::remove_dir_all(&base);
    }

    /// The re-route half on the product path: a FOLLOWER refuses an
    /// establishing read with the TYPED NotLeader + hint (the post-deposition
    /// family of the card's phase split — phase-independent on a follower, so
    /// no deposition construction is needed to pin it).
    /// A context these cells never actually gate on.
    ///
    /// `established_read` is barrier-first: `read_barrier(..)?` is its FIRST statement and
    /// `check_context_in(..)?` its fourth, so every cell below — follower, isolated leader,
    /// deposed leader — returns from the barrier and the context is never read. A real
    /// keyspace here would suggest the gate is part of what these cells prove; it is not.
    ///
    /// The migration these calls come from (@Tess): they used to call `leader_read()`, a
    /// wrapper that lost its last production caller when the point reads moved here and
    /// delete-range moved to `established_view`. A test-only wrapper is where a mutation aimed
    /// at the old address stays green forever — @Cindy hit exactly that near-miss one head
    /// earlier. These now enter through the same function production does.
    fn unreachable_gate_ctx() -> RequestContext {
        RequestContext {
            keyspace: KeyspaceId(0),
            region_epoch: kv9_region::RegionEpoch {
                conf_ver: 0,
                version: 0,
            },
            origin: crate::api::RequestOrigin::from_transport("establishing-read-cell"),
        }
    }

    #[test]
    fn a_follower_refuses_an_establishing_read_with_a_typed_hint() {
        let (rts, root, _addrs, base) = serving_trio("follower-read");
        let leader = cluster_leader(&rts).expect("leader");
        let leader_id = NodeId(leader as u64 + 1);
        let follower = (0..3).find(|i| *i != leader).unwrap();
        let err = match backend_view(&rts[follower], &root)
            .established_read(&unreachable_gate_ctx(), KeySpan::Point(b"k"))
        {
            Err(err) => err,
            Ok(_) => panic!("a follower must refuse an establishing read"),
        };
        assert!(
            matches!(err, Error::NotLeader { leader: Some(id) } if id == leader_id),
            "the refusal must be the TYPED NotLeader carrying the live leader \
             hint (never a string, never a transport error): {err:?}"
        );
        let backend = backend_view(&rts[follower], &root);
        for result in [
            backend.admit_node("admin", NodeId(8), "127.0.0.1:20168", 120),
            backend.promote_node("admin", NodeId(8)),
        ] {
            assert!(
                matches!(result, Err(Error::NotLeader { leader: Some(id) }) if id == leader_id),
                "membership follower refusal must be typed before any mutation: {result:?}"
            );
        }
        assert!(
            matches!(
                backend.list_keyspaces("reader"),
                Err(Error::NotLeader { .. })
            ),
            "catalog listing must use a quorum barrier"
        );
        assert!(
            matches!(
                backend.get_region("reader", KeyspaceId(0), b"k"),
                Err(Error::NotLeader { .. })
            ),
            "routing must use a quorum barrier"
        );
        assert!(
            matches!(backend.cluster_info("reader"), Err(Error::NotLeader { .. })),
            "catalog counts must use a quorum barrier"
        );
        drop(rts);
        let _ = fs::remove_dir_all(&base);
    }

    /// The isolated self-believed leader, phase-split and typed at the SERVER
    /// seam (card cell a; the driver-level halves live in kv9-raft):
    ///
    /// Phase 1 — pre-deposition, pinned: peers go silent under manual tick
    /// control, the node still believes it leads (asserted immediately before
    /// the read). The establishing read must fail as the TYPED
    /// `ReadUnconfirmed { quorum_confirmed: false }` — a transport error
    /// cannot satisfy this (the read path never touches the transport on
    /// this node), and stale data is never served.
    ///
    /// Phase 2 — post-deposition: check_quorum's own discipline deposes the
    /// silent-peer leader; the same call now fails typed NotLeader.
    #[test]
    fn an_isolated_leader_fails_establishing_typed_in_both_phases() {
        use kv9_raft::transport::{InProcHub, RaftTransport};
        let dir = std::env::temp_dir().join(format!(
            "kv9-isolated-establishing-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let hub = InProcHub::new();
        let ids = [NodeId(1), NodeId(2), NodeId(3)];
        let mk = |id: NodeId| {
            let (storage, _) =
                DiskRaftStorage::open(&dir.join(format!("raft{}", id.0)), &[1, 2, 3]).unwrap();
            let peer = Arc::new(RaftPeer::with_storage(id, META_REGION_0, storage).unwrap());
            let (wal, _) = WalEngine::open(dir.join(format!("wal{}", id.0))).unwrap();
            let wal = Arc::new(wal);
            (
                NodeDriver::new(
                    peer.clone(),
                    Arc::new(hub.endpoint(id)) as Arc<dyn RaftTransport>,
                    MemStateMachine::with_engine(wal.clone()).unwrap(),
                )
                .expect("drain token minted once per peer"),
                peer,
                wal,
            )
        };
        let (d1, p1, wal1) = mk(NodeId(1));
        let (d2, _p2, _w2) = mk(NodeId(2));
        let (d3, _p3, _w3) = mk(NodeId(3));
        let _ = ids;
        d1.peer().campaign().unwrap();
        for _ in 0..300 {
            d1.tick_and_step().unwrap();
            d2.tick_and_step().unwrap();
            d3.tick_and_step().unwrap();
            if d1.status().role == Role::Leader {
                break;
            }
        }
        assert_eq!(d1.status().role, Role::Leader);
        // Precondition (same pin as the driver-level isolation cell): the
        // current-term barrier must be applied — live quorum contact has
        // demonstrably happened — before the peers go silent.
        for _ in 0..500 {
            d1.tick_and_step().unwrap();
            d2.tick_and_step().unwrap();
            d3.tick_and_step().unwrap();
            let s1 = d1.status();
            if d1.driver_applied().is_some_and(|wm| wm.term == s1.term) {
                break;
            }
        }
        {
            let s1 = d1.status();
            assert!(
                d1.driver_applied().is_some_and(|wm| wm.term == s1.term),
                "precondition: current-term barrier applied before the cut"
            );
        }
        let node =
            Arc::new(Node::with_raft_and_engine(NodeId(1), Config::default(), p1, wal1).unwrap());
        let tokio_rt = tokio::runtime::Runtime::new().unwrap();
        let backend = RuntimeBackend {
            node,
            driver: d1.clone(),
            endpoint_ready: Arc::new(AtomicBool::new(false)),
            transport: GrpcTransport::new(
                NodeId(1),
                None,
                tokio_rt.handle().clone(),
                kv9_common::RootDigest::from_bytes([0; 32]),
            ),
            initial_voters: Vec::new(),
        };

        // Peers silent from here; only d1 pumps (real-time thread).
        let _pump = d1.spawn(Duration::from_millis(2)).unwrap();

        // PHASE 1, pinned: still the self-believed leader at call entry.
        assert_eq!(d1.status().role, Role::Leader, "phase pin: not yet deposed");
        let err = match backend.established_read(&unreachable_gate_ctx(), KeySpan::Point(b"k")) {
            Err(err) => err,
            Ok(_) => panic!("an isolated leader must not serve an establishing read"),
        };
        assert!(
            matches!(
                err,
                Error::ReadUnconfirmed {
                    phase: kv9_common::ReadBarrierPhase::QuorumConfirmation
                }
            ),
            "pre-deposition the failure must be the TYPED quorum-unconfirmed \
             variant — not a transport error, not NotLeader, and never stale \
             data: {err:?}"
        );

        // PHASE 2: check_quorum deposes the silent-peer leader on its own.
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while d1.status().role == Role::Leader {
            assert!(
                std::time::Instant::now() < deadline,
                "check_quorum must depose a leader with no quorum contact"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let err = match backend.established_read(&unreachable_gate_ctx(), KeySpan::Point(b"k")) {
            Err(err) => err,
            Ok(_) => panic!("a deposed leader must refuse the establishing read"),
        };
        assert!(
            matches!(err, Error::NotLeader { .. }),
            "post-deposition the failure must be the TYPED NotLeader: {err:?}"
        );
        d1.stop();
        d2.stop();
        d3.stop();
        let _ = fs::remove_dir_all(&dir);
    }
}

/// Does the production write path actually hand the fence the values the gate resolved —
/// and does it do so **per chunk**?
///
/// These live here, apart from the adjudicator's own tests in `fence.rs`, and the split is
/// semantic rather than an accident of visibility (@Tess): `fence.rs` guards the
/// adjudicator's verdicts — fresh, stale, read-failure — while this guards whether
/// `validated_context → commit_batch → driver` emits a `Command::Fenced` carrying the
/// *right content*. The seam under test, the start overrides and the private fields all live
/// in this module.
///
/// Every test here drives a **real** `NodeRuntime` to Serving and calls a **real** endpoint.
/// Building a `Command::Fenced` in the fixture and asserting on it is what the first attempt
/// did, and reverting `commit_batch` to `Command::write_from_batch` — the original defect —
/// left it green (docs/TESTING.md rule 17).
#[cfg(test)]
mod fence_firing_tests {
    use super::*;
    use kv9_common::{ApiType, BootstrapGeneration, ClusterId, RootVoter, TenantId};
    use kv9_raft::{FenceAdjudicator, RegionFence};
    use std::sync::Mutex as StdMutex;

    /// Records every fence it is asked about, then returns a fixed verdict.
    ///
    /// The recording is what makes fence *content* observable, and content is the whole of
    /// this layer: an always-refusing adjudicator plus a "nothing was written" assertion is
    /// insensitive to the fields, because a `Command::Fenced` carrying entirely wrong values
    /// is refused just the same (@Tess).
    ///
    /// The verdict is a parameter because the two properties need opposite ones. Refusal
    /// makes absence-of-data an observable signal; freshness lets a multi-chunk delete
    /// actually run to completion so its chunks can be counted.
    struct RecordingAdjudicator {
        seen: Arc<StdMutex<Vec<RegionFence>>>,
        verdict: bool,
    }

    impl FenceAdjudicator for RecordingAdjudicator {
        fn is_fresh(&self, fence: &RegionFence) -> Result<bool> {
            self.seen.lock().expect("recorder poisoned").push(*fence);
            Ok(self.verdict)
        }
    }

    /// A real `NodeRuntime` driven to Serving through the production bootstrap path.
    ///
    /// The listener is bound HERE, on `127.0.0.1:0`, and **held** until its ownership moves
    /// into `start_core`. Two rules of this repo meet at that line:
    ///
    /// 1. The port comes from the kernel, never from arithmetic. A hand-computed port both
    ///    collides under parallel runs and can leave the legal range: `24000 + (pid % 3000) * 8`
    ///    exceeds 32768 for roughly half of all pids, which violated our own `<32768` rule
    ///    intermittently and passed every single time it was run by hand.
    /// 2. The listener is never dropped to "reserve" the port. `bind → local_addr → drop →
    ///    rebind` looks equivalent and is not: another process can take the port inside that
    ///    window, so the failure appears only under load. That is why this is an ownership
    ///    transfer and not a port lookup — see [`StartOverrides::listener`].
    ///
    /// It must NOT use `Node::bootstrap`/`create_keyspace`/`propose_apply` to reach Serving:
    /// those reach raft through `MetaRaft::drive_apply`, a second destructive `take_ready`
    /// consumer alongside the driver pump. With the pump running that races, and the harness
    /// would be intermittently red for reasons that look like fence bugs (@Rafa's analysis;
    /// @Tess's diagnostic: needing those entry points means you are on the old single-node
    /// path).
    fn serving_runtime(
        seen: Arc<StdMutex<Vec<RegionFence>>>,
        verdict: bool,
    ) -> (NodeRuntime, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "kv9-fence-firing-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        // Bound now, held across everything below, moved into `start_core` at the end.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("kernel assigns a port");
        let addr = listener
            .local_addr()
            .expect("a bound listener has an address");

        let root = RootDescriptor::new(
            ClusterId::from_bytes([7; 16]),
            BootstrapGeneration::mint().unwrap(),
            vec![RootVoter {
                node_id: NodeId(1),
                addr,
                store_incarnation: prepare_test_store(&dir, NodeId(1)),
            }],
            b"fence-firing-credential",
        )
        .unwrap();
        let identity = StoreIdentity::for_voter(&root, NodeId(1)).unwrap();

        let config = Config {
            addr: addr.to_string(),
            data_dir: dir.to_string_lossy().into_owned(),
            ..Default::default()
        };
        let auth = RuntimeAuth {
            cluster_token: "fence-firing-cluster".into(),
            client_tokens: vec![("admin".into(), "fence-firing-client".into())],
        };

        let runtime = NodeRuntime::start_core(
            NodeId(1),
            config,
            auth,
            root,
            identity,
            None,
            StartOverrides {
                adjudicator: Some(Box::new(move |_node| {
                    Arc::new(RecordingAdjudicator { seen, verdict })
                })),
                listener: Some(listener),
                ..Default::default()
            },
        )
        .expect("single-voter runtime starts");
        (runtime, dir)
    }

    /// **The runtime must record where it is ACTUALLY listening, not what the config asked
    /// for.**
    ///
    /// @Tess's blocker on the first layer-3 head: `start_core` adopted the injected listener
    /// and then stored `config.addr` anyway, so a runtime told to listen on one address while
    /// handed a socket bound to another would report — and publish into its status file — an
    /// address nothing was listening on. Every other test in this module derives its config
    /// from the listener, so the two agreed by construction and none of them could see it.
    ///
    /// Both addresses are real kernel-assigned ports and both listeners are **held** for the
    /// duration, so "the config's address" names something that genuinely exists and is
    /// genuinely not ours. Asserting against an invented port would let this pass for the
    /// wrong reason if the code fell back to a default.
    ///
    /// Mutation (must red, and did): drop the `local_addr()` read after the adoption and keep
    /// `requested_addr`. Only this test reds — 61 passed, 1 failed.
    #[test]
    fn an_adopted_listener_decides_the_recorded_address_not_the_config() {
        let dir = std::env::temp_dir().join(format!(
            "kv9-adopted-addr-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        // The one we hand over.
        let adopted = std::net::TcpListener::bind("127.0.0.1:0").expect("kernel assigns a port");
        let adopted_addr = adopted.local_addr().expect("bound");
        // A different real port, held so nothing else can occupy it and so the two can never
        // coincide.
        let decoy = std::net::TcpListener::bind("127.0.0.1:0").expect("kernel assigns a port");
        let decoy_addr = decoy.local_addr().expect("bound");
        assert_ne!(
            adopted_addr, decoy_addr,
            "two independent :0 binds must differ, or this test proves nothing"
        );

        let root = RootDescriptor::new(
            ClusterId::from_bytes([9; 16]),
            BootstrapGeneration::mint().unwrap(),
            vec![RootVoter {
                node_id: NodeId(1),
                addr: decoy_addr,
                store_incarnation: prepare_test_store(&dir, NodeId(1)),
            }],
            b"adopted-addr-credential",
        )
        .unwrap();
        let identity = StoreIdentity::for_voter(&root, NodeId(1)).unwrap();

        let runtime = NodeRuntime::start_core(
            NodeId(1),
            Config {
                // Deliberately the DECOY: the config asks for one address while the runtime is
                // handed a socket bound to another.
                addr: decoy_addr.to_string(),
                data_dir: dir.to_string_lossy().into_owned(),
                ..Default::default()
            },
            RuntimeAuth {
                cluster_token: "adopted-addr-cluster".into(),
                client_tokens: vec![("admin".into(), "adopted-addr-client".into())],
            },
            root,
            identity,
            None,
            StartOverrides {
                adjudicator: None,
                listener: Some(adopted),
                ..Default::default()
            },
        )
        .expect("runtime starts on the adopted listener");

        assert_eq!(
            runtime.addr, adopted_addr,
            "the runtime must record the adopted listener's address ({adopted_addr}), \
             not the config's ({decoy_addr})"
        );

        drop(runtime);
        drop(decoy);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Drive the runtime to Serving using the same method `run()` calls.
    fn drive_to_initializing(runtime: &mut NodeRuntime) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            runtime.advance_bootstrap().unwrap();
            if matches!(
                runtime.node.meta.lock().unwrap().bootstrap.state(),
                BootstrapState::Initializing { .. }
            ) {
                return;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(TICK);
        }
    }

    #[test]
    fn initialization_waits_for_apply_before_planning_a_seed() {
        let (mut runtime, dir) = serving_runtime(Arc::new(StdMutex::new(Vec::new())), true);
        runtime.driver.pause_apply(true);
        drive_to_initializing(&mut runtime);
        let command = runtime
            .node
            .build_initial_metadata_command_for_root(&runtime.voters, &runtime.root)
            .unwrap();
        let prior = runtime.driver.propose(&command).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while runtime.driver.status().raft_committed < prior.index.0 {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(TICK);
        }
        assert!(runtime.node.local_cluster_identity().unwrap().is_none());
        assert!(runtime.driver.driver_applied().is_none());
        runtime.advance_initialization().unwrap();
        assert!(
            runtime.initial_proposal.is_none(),
            "initialization must not plan another seed behind unapplied committed metadata"
        );
        let next = runtime.driver.propose(&kv9_raft::Command::Noop).unwrap();
        assert_eq!(
            next.index.0,
            prior.index.0 + 1,
            "no duplicate seed entered Raft"
        );
        runtime.driver.pause_apply(false);
        drive_to_serving(&mut runtime);
        runtime.verify_certified_root().unwrap();
        drop(runtime);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn initialization_adopts_a_committed_catalog_without_its_original_receipt() {
        let (mut runtime, dir) = serving_runtime(Arc::new(StdMutex::new(Vec::new())), true);
        drive_to_initializing(&mut runtime);
        let command = runtime
            .node
            .build_initial_metadata_command_for_root(&runtime.voters, &runtime.root)
            .unwrap();
        let prior = propose_and_wait(&runtime.driver, &command, Duration::from_secs(5)).unwrap();
        assert!(runtime.initial_proposal.is_none());
        assert_eq!(
            runtime.node.local_cluster_identity().unwrap(),
            Some(runtime.root.cluster_id)
        );
        runtime.advance_initialization().expect(
            "already committed metadata must be adopted without rebuilding duplicate seed rows",
        );
        assert!(runtime.node.meta.lock().unwrap().bootstrap.is_serving());
        let next = runtime.driver.propose(&kv9_raft::Command::Noop).unwrap();
        assert_eq!(
            next.index.0,
            prior.index + 1,
            "adoption must not append a seed"
        );
        drop(runtime);
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn drive_to_serving(runtime: &mut NodeRuntime) {
        for _ in 0..600 {
            runtime.advance_bootstrap().expect("bootstrap advances");
            if matches!(
                runtime
                    .node
                    .meta
                    .lock()
                    .expect("meta poisoned")
                    .bootstrap
                    .state(),
                BootstrapState::Serving { .. }
            ) {
                return;
            }
            std::thread::sleep(TICK);
        }
        // Margin, so a future red here is not misread as "the machine was slow".
        //
        // Measured 0.46-0.48s across 3 runs on a40ef35 for this whole module -- which
        // includes TWO full bootstraps -- against the 12s bound below, i.e. a conservative
        // margin of >=25x. Conservative because it charges the entire module's wall time to a
        // single wait; the true per-wait margin is larger, and is deliberately not quoted
        // because it was never measured on its own.
        //
        // Range + the head it was measured on + a conservative bound, all three (@Cindy): a
        // bare figure with no head reads as a regression the moment someone measures a
        // different value on a different head, and a bare bound invites "did it only just
        // pass?".
        panic!("single-voter runtime did not reach Serving within 12s");
    }

    fn backend_of(runtime: &NodeRuntime) -> RuntimeBackend {
        RuntimeBackend {
            node: runtime.node.clone(),
            driver: runtime.driver.clone(),
            transport: runtime.transport.clone(),
            endpoint_ready: runtime.endpoint_ready.clone(),
            // From the same source `start_core` uses, not an empty placeholder: these tests
            // never register, so a `Vec::new()` would compile and then silently diverge from
            // production rather than failing. (Integration with the registration-follow-hint
            // work, which added this field.)
            initial_voters: runtime.seeds.iter().map(|s| (s.node_id, s.addr)).collect(),
        }
    }

    /// The keyspace, plus the context a client would send for it — epoch read from the
    /// catalog rather than invented, so the assertions compare production against production.
    fn keyspace_and_ctx(
        runtime: &NodeRuntime,
        backend: &RuntimeBackend,
        anchor: &[u8],
    ) -> (KeyspaceId, RequestContext, kv9_meta::tables::Region) {
        let keyspace = backend
            .create_keyspace(
                "t",
                "fenced",
                TenantId::DEFAULT,
                ApiType::Raw,
                TxnGroupId(0),
            )
            .expect("create_keyspace on a Serving node")
            .keyspace;
        let region = Tables::new(&runtime.node.meta_raft.store)
            .region_for_key(keyspace, anchor)
            .expect("catalog readable")
            .expect("CreateKeyspace creates the initial region");
        let ctx = RequestContext {
            keyspace,
            region_epoch: kv9_region::RegionEpoch {
                conf_ver: region.epoch_conf,
                version: region.epoch_ver,
            },
            origin: crate::api::RequestOrigin::from_transport("fence-firing"),
        };
        (keyspace, ctx, region)
    }

    /// A real `raw_put` hands the fence exactly the region and epoch the gate resolved, the
    /// refusal writes nothing, **and the caller is told**.
    ///
    /// Three assertions and they are not redundant. The second is the one an always-false
    /// adjudicator cannot make on its own: a `Fenced` command carrying wrong fields is
    /// refused identically, so "no data landed" says nothing about content. The third is the
    /// silent-lost-write property — before the receipt seam existed, `commit_batch` returned
    /// `Ok` for a write that was refused and stored nothing.
    ///
    /// Mutation (must red): `commit_batch` back to `Command::write_from_batch`. The
    /// adjudicator is then never consulted and assertion 1 fails on `0 != 1`.
    #[test]
    fn a_real_raw_put_fences_with_the_region_and_epoch_the_gate_resolved() {
        let seen: Arc<StdMutex<Vec<RegionFence>>> = Arc::new(StdMutex::new(Vec::new()));
        let (mut runtime, dir) = serving_runtime(seen.clone(), false);
        drive_to_serving(&mut runtime);
        let backend = backend_of(&runtime);
        let (keyspace, ctx, region) = keyspace_and_ctx(&runtime, &backend, b"k");

        let outcome = backend.raw_put(&ctx, b"k".to_vec(), b"v".to_vec());

        // 1. the adjudicator was consulted exactly once for this user write
        let recorded = seen.lock().expect("recorder poisoned").clone();
        assert_eq!(
            recorded.len(),
            1,
            "a fenced user write must reach the adjudicator exactly once; got {recorded:?}"
        );

        // 2. it received EXACTLY what the gate resolved — region id and both epoch halves
        assert_eq!(
            recorded[0],
            RegionFence {
                region_id: region.id.0,
                conf_ver: region.epoch_conf,
                version: region.epoch_ver,
            },
            "the fence must carry the region the gate validated and its full epoch, \
             not a value re-derived at commit time"
        );

        // 3. the caller is told, and nothing was written
        assert!(
            matches!(outcome, Err(Error::StaleEpoch { region: r }) if r == region.id),
            "a refused fence must surface as StaleEpoch naming the region, got {outcome:?}"
        );
        // Composed from the two production pieces rather than kept alive via the old
        // wrapper: this cell only needs *an established view* to assert against.
        let barrier = runtime
            .driver
            .read_barrier(READ_BARRIER_DEADLINE)
            .expect("barrier");
        let view = backend.established_view(barrier).expect("established view");
        let hint = runtime.driver.status().leader_id;
        let read = LeaderRead::new(view.as_ref(), true, hint).expect("leader");
        assert_eq!(
            RawExecutor.get(&read, keyspace, b"k").expect("get"),
            None,
            "a write refused at apply must leave nothing behind"
        );

        drop(runtime);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prepared_point_and_batch_writes_preserve_apply_fences_and_effects() {
        for accepted in [true, false] {
            let seen = Arc::new(StdMutex::new(Vec::new()));
            let (mut runtime, dir) = serving_runtime(seen.clone(), accepted);
            drive_to_serving(&mut runtime);
            let backend = Arc::new(backend_of(&runtime));
            let (_, ctx, region) = keyspace_and_ctx(&runtime, &backend, b"k");
            seen.lock().unwrap().clear();
            let executor = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .max_blocking_threads(1)
                .build()
                .unwrap();
            for operation in [
                crate::api::RawWrite::Put {
                    key: b"k".to_vec(),
                    value: b"v1".to_vec(),
                },
                crate::api::RawWrite::BatchPut(vec![
                    (b"k".to_vec(), b"v2".to_vec()),
                    (b"b".to_vec(), b"v3".to_vec()),
                ]),
                crate::api::RawWrite::Delete { key: b"k".to_vec() },
            ] {
                let preparation = backend.clone().prepare_raw_write(ctx.clone(), operation);
                let result = executor.block_on(async {
                    let completion = tokio::task::spawn_blocking(preparation).await.unwrap()?;
                    completion.await
                });
                if accepted {
                    assert!(result.is_ok(), "accepted prepared write failed: {result:?}");
                    let at = result.unwrap();
                    assert!(at.term > 0 && at.index > 0);
                } else {
                    assert!(
                        matches!(result, Err(Error::StaleEpoch { region: got }) if got == region.id),
                        "prepared write hid an ordered fence rejection: {result:?}"
                    );
                }
            }
            let fences = seen.lock().unwrap().clone();
            assert_eq!(
                fences.len(),
                3,
                "prepared writes bypassed or retried the fence gate"
            );
            for fence in fences {
                assert_eq!(
                    fence,
                    RegionFence {
                        region_id: region.id.0,
                        conf_ver: region.epoch_conf,
                        version: region.epoch_ver
                    }
                );
            }
            assert_eq!(backend.raw_get(&ctx, b"k").unwrap(), None);
            assert_eq!(
                backend.raw_get(&ctx, b"b").unwrap(),
                accepted.then(|| b"v3".to_vec())
            );
            assert_eq!(backend.driver.async_apply_snapshot().in_flight, 0);
            assert!(
                backend.driver.async_apply_snapshot().peak > 0,
                "prepared writes did not use async apply waits"
            );
            drop(executor);
            drop(backend);
            drop(runtime);
            fs::remove_dir_all(dir).unwrap();
        }
    }

    /// **Exactly what this proves, and no more (@Tess): under a static catalog, a delete range
    /// longer than one chunk emits one correctly-populated fence per committed chunk and
    /// completes the delete.**
    ///
    /// It has to exceed `RAW_DELETE_RANGE_CHUNK` because at or below it the loop runs once and
    /// a single fence says nothing about chunking at all.
    ///
    /// **It does NOT prove each fence came from that chunk's own freshly minted
    /// authorisation.** Under a static catalog every chunk's revalidation resolves the same
    /// region and epoch, so a version that reused chunk 1's capability records exactly these
    /// fences and passes exactly these assertions — measured, not supposed: with
    /// `ValidatedFence` made `Copy` that merge compiles and this test is green. The type and
    /// the `NOT_CLONE_OR_COPY` guard beside it are what forbid it.
    ///
    /// So what reds here is a regression to a single up-front commit, an unfenced chunk, or a
    /// fence carrying the wrong values — not a re-introduction of the stale-authorisation
    /// shape.
    #[test]
    fn a_multi_chunk_delete_range_emits_one_fence_per_chunk_with_the_gate_resolved_values() {
        const KEYS: usize = RAW_DELETE_RANGE_CHUNK + 200;

        let seen: Arc<StdMutex<Vec<RegionFence>>> = Arc::new(StdMutex::new(Vec::new()));
        // Fresh, not stale: the delete must actually run to completion so its chunks exist
        // to be counted.
        let (mut runtime, dir) = serving_runtime(seen.clone(), true);
        drive_to_serving(&mut runtime);
        let backend = backend_of(&runtime);
        let (keyspace, ctx, region) = keyspace_and_ctx(&runtime, &backend, b"");

        // Seed through the real endpoint, so the keys are physically encoded the same way
        // the delete planner will encode them.
        let pairs: Vec<(UserKey, Value)> = (0..KEYS)
            .map(|i| (format!("k{i:06}").into_bytes(), b"v".to_vec()))
            .collect();
        for chunk in pairs.chunks(256) {
            backend.raw_batch_put(&ctx, chunk).expect("seed");
        }
        let seeding_fences = seen.lock().expect("recorder poisoned").len();
        let barriers_before = runtime.driver.read_barriers_minted();

        let receipt = backend
            .raw_delete_range(&ctx, b"", b"")
            .expect("the range delete completes");

        assert!(
            receipt.committed_chunks >= 2,
            "{KEYS} keys at a chunk size of {RAW_DELETE_RANGE_CHUNK} must take more than one \
             chunk, got {}",
            receipt.committed_chunks
        );

        // ONE barrier for the whole request, however many chunks it took.
        //
        // This is the cell that discriminates @Tess's ruling from the shape it replaced. The
        // fence assertions below cannot: a per-chunk-barrier version records exactly the same
        // fences and deletes exactly the same keys. Only the count separates them, and it is
        // meaningful *because* `committed_chunks >= 2` above already established that the loop
        // really ran more than once — otherwise "1 barrier for 1 chunk" would prove nothing.
        //
        // Mutation (must red): move the barrier back inside the planner closure. This becomes
        // `receipt.committed_chunks` barriers instead of 1.
        let barriers_used = runtime.driver.read_barriers_minted() - barriers_before;
        assert_eq!(
            barriers_used, 1,
            "a delete-range request must establish exactly one quorum barrier, not one per \
             chunk; the request committed {} chunks and minted {barriers_used} barriers",
            receipt.committed_chunks
        );

        // One fence per committed chunk, each carrying this request's authorisation.
        let delete_fences: Vec<RegionFence> =
            seen.lock().expect("recorder poisoned")[seeding_fences..].to_vec();
        assert_eq!(
            delete_fences.len() as u64,
            receipt.committed_chunks,
            "every committed chunk must have been fenced exactly once; \
             {} fences for {} chunks",
            delete_fences.len(),
            receipt.committed_chunks
        );
        let expected = RegionFence {
            region_id: region.id.0,
            conf_ver: region.epoch_conf,
            version: region.epoch_ver,
        };
        assert!(
            delete_fences.iter().all(|f| *f == expected),
            // Deliberately NOT "its own revalidation minted": comparing recorded fields
            // against a static catalog's expected values cannot establish provenance, and
            // claiming it here would contradict this test's own "does NOT prove" doc (@Tess).
            "each chunk must carry the gate-resolved region and full epoch; \
             got {delete_fences:?}, expected every entry to be {expected:?}"
        );

        // And the range is genuinely empty afterwards — the fences did not come at the cost
        // of the delete.
        // Composed from the two production pieces rather than kept alive via the old
        // wrapper: this cell only needs *an established view* to assert against.
        let barrier = runtime
            .driver
            .read_barrier(READ_BARRIER_DEADLINE)
            .expect("barrier");
        let view = backend.established_view(barrier).expect("established view");
        let hint = runtime.driver.status().leader_id;
        let read = LeaderRead::new(view.as_ref(), true, hint).expect("leader");
        let left = RawExecutor
            .scan(&read, keyspace, b"", b"", 16)
            .expect("scan");
        assert!(
            left.is_empty(),
            "the whole range must be gone, {left:?} remains"
        );

        drop(runtime);
        let _ = fs::remove_dir_all(&dir);
    }
}
