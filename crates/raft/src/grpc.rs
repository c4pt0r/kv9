//! gRPC node-to-node transport (task #19): the TiKV/CSE shape on tonic.
//!
//! Layering contract (verified against the local CSE source — its
//! `RaftStoreRouter` is a fully synchronous trait):
//! - **The core stays synchronous.** This module's async code lives on a tokio
//!   runtime owned/handed in at the edge; the only crossing into the core is a
//!   channel send. Nothing here requires `NodeDriver` or the state machine to
//!   become async.
//! - **Streams, not unary calls**: each peer pair keeps one long-lived
//!   client-stream carrying [`pb::BatchRaftMessage`] — batching by count and
//!   bytes with a short flush window is what makes gRPC viable at raft message
//!   rates (CSE's `batch_raft`).
//! - **Best-effort delivery**: raft tolerates loss; a full queue or a dead
//!   connection drops messages and raft retransmits. Reconnection backs off.
//! - Discovery keeps its fencing semantics verbatim: silence is a transport
//!   error (never an answer), answers bind to the responder's declared
//!   voter-set fingerprint, and the CALLER counts only exact matches.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use protobuf::Message as PbCodec;
use raft::prelude::Message;
use tokio::sync::mpsc;
use tonic::{Request, Response, Status, Streaming};

use kv9_common::{ClusterId, Error, NodeId, Result};

use crate::transport::RaftTransport;

/// Generated protobuf/tonic types for `proto/kv9_raft.proto`.
pub mod pb {
    tonic::include_proto!("kv9.raft");
}

use pb::kv9_raft_client::Kv9RaftClient;
use pb::kv9_raft_server::Kv9Raft;

/// Metadata key carrying the shared cluster token (EdHuang's ruling: token
/// auth ships with the gRPC rewrite). Threat boundary, stated where it will
/// be read: the token gates **unauthorized processes joining the control
/// plane** — it does not protect against a wire adversary (that is TLS,
/// deferred until cross-machine deployment) and is a different layer from the
/// voter fingerprint (config-accident detection). Three layers, three jobs.
pub const CLUSTER_TOKEN_KEY: &str = "kv9-cluster-token";
/// Authenticated node identity declared in transport metadata. The server
/// interceptor validates and turns this into its trusted `AuthContext`; service
/// handlers never infer an identity from the protobuf body.
pub const NODE_ID_KEY: &str = "kv9-node-id";

/// Server-side interceptor enforcing the shared cluster token. The server
/// crate wraps registered services with this (or with the richer
/// `AuthContext` authenticator from the external-API work — same contract:
/// handlers never trust caller identity from the body).
pub fn cluster_token_interceptor(
    expected: String,
) -> impl FnMut(Request<()>) -> std::result::Result<Request<()>, Status> + Clone {
    move |mut req: Request<()>| {
        match req.metadata().get(CLUSTER_TOKEN_KEY) {
            Some(v) if v.to_str().map(|s| s == expected).unwrap_or(false) => {}
            Some(_) => return Err(Status::unauthenticated("cluster token mismatch")),
            None => return Err(Status::unauthenticated("cluster token required")),
        }
        let node = req
            .metadata()
            .get(NODE_ID_KEY)
            .ok_or_else(|| Status::unauthenticated("declared node identity required"))?
            .to_str()
            .map_err(|_| Status::unauthenticated("invalid node identity metadata"))?
            .parse::<u64>()
            .map(NodeId)
            .map_err(|_| Status::unauthenticated("invalid node identity"))?;
        req.extensions_mut().insert(node);
        Ok(req)
    }
}

fn attach_auth<T>(req: &mut Request<T>, token: &Option<String>, node: NodeId) {
    if let Some(t) = token {
        if let Ok(v) = t.parse() {
            req.metadata_mut().insert(CLUSTER_TOKEN_KEY, v);
        }
    }
    if let Ok(value) = node.0.to_string().parse() {
        req.metadata_mut().insert(NODE_ID_KEY, value);
    }
}

/// Batching knobs (CSE defaults are config-driven; Phase 1 fixes sane values).
const MAX_BATCH_MSGS: usize = 128;
const MAX_BATCH_BYTES: usize = 1024 * 1024;
const FLUSH_WINDOW: Duration = Duration::from_millis(2);
/// Per-peer outbound queue; overflow drops (raft retransmits).
const PEER_QUEUE: usize = 4096;
const RECONNECT_MIN: Duration = Duration::from_millis(100);
const RECONNECT_MAX: Duration = Duration::from_secs(2);

/// Answers discovery for this node (same contract as the TCP transport's
/// `DiscoveryState`): `(node id, initialized?, declared voter-set fingerprint)`.
pub trait GrpcDiscoveryState: Send + Sync + 'static {
    fn answer(&self) -> (NodeId, bool, u64);

    /// The cluster identity, once initialized (task #24, gate 2). The
    /// CONTRACT couples this to `answer().1`: whenever `initialized` is
    /// true this MUST return `Some` — the service refuses to publish an
    /// initialized answer that cannot name its cluster (a protocol error on
    /// our own side beats an unverifiable claim on the wire).
    fn cluster_id(&self) -> Option<ClusterId> {
        None
    }
}

/// One committed registration (task #24, gate 3): the receipt the server-side
/// backend returns after the admission consume + membership upsert applied at
/// an exact position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrationReceipt {
    pub applied_term: u64,
    pub applied_index: u64,
    pub voters: Vec<u64>,
    pub learners: Vec<u64>,
}

/// Why a registration failed — TYPED, because the wire contract depends on
/// the distinction: a follower's refusal is retryable with a redirect and
/// carries a machine-readable marker; an ordinary precondition failure is
/// not, and must never be mistaken for one (clients check code AND marker,
/// never parse strings — the same double-condition rule as the raw path).
///
/// CONVERGENCE OBLIGATION (Cindy/Tess, 2026-08-28): this is a branch-local
/// typed result, kept independent only so the seam did not wait on another
/// lane. The common target ALREADY EXISTS on the raw line (`d1731dc`:
/// `kv9_common::Error::NotLeader { leader: Option<u64> }` with the identical
/// `kv9-not-leader=true` / optional `kv9-leader-node-id` wire convention —
/// this type reuses those key strings and semantics VERBATIM). At the
/// raw+membership combination this enum must collapse into that Error
/// variant; Tess owns the combination and the single client path. Payload is
/// `Option<NodeId>` so the collapse is a promotion, not a translation.
#[derive(Debug)]
pub enum RegistrationError {
    /// This node is not the leader; retry against `leader` if known. Maps to
    /// FAILED_PRECONDITION + `kv9-not-leader: true` (+ optional leader id).
    NotLeader { leader: Option<NodeId> },
    /// Any other refusal (admission missing/expired/wrong cluster, …). Same
    /// gRPC code, NO marker — machine-distinguishable from NotLeader.
    Failed(Error),
}

/// gRPC metadata key marking a machine-readable not-leader refusal. The
/// marker's VALUE must be ASCII `true`; presence alone is not enough
/// (a `false` asserts the opposite).
pub const NOT_LEADER_KEY: &str = "kv9-not-leader";
/// gRPC metadata key carrying the redirect hint (decimal node id). ABSENT
/// when the leader is unknown — never "0" (clients must distinguish
/// "retry node 7" from "rediscover").
pub const LEADER_NODE_ID_KEY: &str = "kv9-leader-node-id";

/// The seam the server injects (trait here, implementation there — the
/// division that keeps this crate free of catalog/runtime knowledge): consume
/// the caller's admission and commit its membership registration, returning
/// the exact applied receipt. Implementations run on the LEADER's committed
/// path only; a follower returns `RegistrationError::NotLeader` so the
/// handler can emit the machine-readable redirect.
pub trait RegistrationBackend: Send + Sync + 'static {
    fn register(
        &self,
        node: NodeId,
        addr: &str,
        cluster_id: ClusterId,
    ) -> std::result::Result<RegistrationReceipt, RegistrationError>;
}

/// The inbound half: implements the generated service. Holds ONLY a channel
/// sender into the synchronous core — no listener, no runtime, no port. The
/// server crate registers this on its single shared `tonic` server.
pub struct RaftGrpcService {
    me: NodeId,
    inbox: mpsc::UnboundedSender<Message>,
    discovery: Arc<dyn GrpcDiscoveryState>,
    /// The registration seam (None = this node serves no registration, e.g.
    /// tests or a build wired before the server injects it — callers get
    /// UNIMPLEMENTED, loudly, never a silent fake success).
    registration: Option<Arc<dyn RegistrationBackend>>,
    /// Envelopes rejected for a wrong destination (diagnostic mirror of the
    /// TCP transport's step-error counter: growth = misconfiguration).
    misrouted: AtomicU64,
}

impl RaftGrpcService {
    pub fn new(
        me: NodeId,
        inbox: mpsc::UnboundedSender<Message>,
        discovery: Arc<dyn GrpcDiscoveryState>,
    ) -> RaftGrpcService {
        RaftGrpcService {
            me,
            inbox,
            discovery,
            registration: None,
            misrouted: AtomicU64::new(0),
        }
    }

    /// Inject the server-side registration backend (builder style, called at
    /// service assembly in the server crate).
    pub fn with_registration(mut self, backend: Arc<dyn RegistrationBackend>) -> Self {
        self.registration = Some(backend);
        self
    }

    pub fn misrouted(&self) -> u64 {
        self.misrouted.load(Ordering::Relaxed)
    }
}

#[tonic::async_trait]
impl Kv9Raft for RaftGrpcService {
    async fn batch_raft(
        &self,
        request: Request<Streaming<pb::BatchRaftMessage>>,
    ) -> std::result::Result<Response<pb::Done>, Status> {
        let authenticated = *request
            .extensions()
            .get::<NodeId>()
            .ok_or_else(|| Status::unauthenticated("authenticated node identity missing"))?;
        let mut stream = request.into_inner();
        loop {
            let batch = match stream.message().await {
                Ok(Some(b)) => b,
                Ok(None) => return Ok(Response::new(pb::Done {})), // clean end
                Err(status) => return Err(status),
            };
            for env in batch.msgs {
                if env.from_node != authenticated.0 {
                    return Err(Status::permission_denied(
                        "envelope sender does not match authenticated node",
                    ));
                }
                // CSE's StoreNotMatch, at our node granularity: a wrong
                // destination is a config error — kill the stream loudly
                // rather than silently applying someone else's traffic.
                if env.to_node != self.me.0 {
                    self.misrouted.fetch_add(1, Ordering::Relaxed);
                    return Err(Status::invalid_argument(format!(
                        "envelope for node {} delivered to node {}",
                        env.to_node, self.me.0
                    )));
                }
                // A malformed raft payload poisons only this stream.
                let msg = Message::parse_from_bytes(&env.raft_message)
                    .map_err(|e| Status::invalid_argument(format!("bad raft message: {e}")))?;
                if msg.from != authenticated.0 {
                    return Err(Status::permission_denied(
                        "raft sender does not match authenticated node",
                    ));
                }
                // Unbounded send into the sync core never blocks the runtime;
                // a closed core (shutdown) just ends the stream.
                if self.inbox.send(msg).is_err() {
                    return Err(Status::unavailable("node is shutting down"));
                }
            }
        }
    }

    async fn discover(
        &self,
        request: Request<pb::DiscoverRequest>,
    ) -> std::result::Result<Response<pb::DiscoverResponse>, Status> {
        let authenticated = *request
            .extensions()
            .get::<NodeId>()
            .ok_or_else(|| Status::unauthenticated("authenticated node identity missing"))?;
        if request.get_ref().from_node != authenticated.0 {
            return Err(Status::permission_denied(
                "discovery sender does not match authenticated node",
            ));
        }
        let (node, initialized, fp) = self.discovery.answer();
        let cluster_id = match (initialized, self.discovery.cluster_id()) {
            (true, Some(id)) => id.as_bytes().to_vec(),
            (true, None) => {
                // Refuse to publish an initialized answer that cannot name
                // its cluster: post-init authority IS the identity (gate 2),
                // and a nameless "initialized" would push joiners back onto
                // the retired fingerprint.
                return Err(Status::internal(
                    "initialized but no cluster identity available",
                ));
            }
            (false, _) => Vec::new(),
        };
        Ok(Response::new(pb::DiscoverResponse {
            node_id: node.0,
            initialized,
            voter_fingerprint: fp,
            cluster_id,
        }))
    }

    async fn register(
        &self,
        request: Request<pb::RegisterRequest>,
    ) -> std::result::Result<Response<pb::RegisterReceipt>, Status> {
        let authenticated = *request
            .extensions()
            .get::<NodeId>()
            .ok_or_else(|| Status::unauthenticated("authenticated node identity missing"))?;
        let req = request.get_ref();
        // Bodies never self-report identity: the interceptor's NodeId is the
        // caller, and a mismatch is rejected before any catalog access.
        if req.node_id != authenticated.0 {
            return Err(Status::permission_denied(
                "registration sender does not match authenticated node",
            ));
        }
        let bytes: [u8; 16] = req.cluster_id.as_slice().try_into().map_err(|_| {
            Status::invalid_argument("cluster_id must be exactly 16 bytes")
        })?;
        let cluster_id = ClusterId::from_bytes(bytes);
        let Some(backend) = &self.registration else {
            // Loud stub discipline: absence of the seam is UNIMPLEMENTED,
            // never a fabricated success.
            return Err(Status::unimplemented(
                "node registration is not served by this build",
            ));
        };
        let receipt = match backend.register(authenticated, &req.addr, cluster_id) {
            Ok(r) => r,
            Err(RegistrationError::NotLeader { leader }) => {
                let mut status = Status::failed_precondition("not the leader");
                status
                    .metadata_mut()
                    .insert(NOT_LEADER_KEY, "true".parse().expect("ascii"));
                if let Some(leader) = leader {
                    status.metadata_mut().insert(
                        LEADER_NODE_ID_KEY,
                        leader
                            .0
                            .to_string()
                            .parse()
                            .expect("a decimal u64 is valid ASCII metadata"),
                    );
                }
                return Err(status);
            }
            // Ordinary refusal: same code, NO marker — never mistakable for
            // a redirect.
            Err(RegistrationError::Failed(e)) => {
                return Err(Status::failed_precondition(e.to_string()))
            }
        };
        Ok(Response::new(pb::RegisterReceipt {
            applied_term: receipt.applied_term,
            applied_index: receipt.applied_index,
            voters: receipt.voters,
            learners: receipt.learners,
        }))
    }
}

/// One-shot discovery over gRPC (fencing rule a). Blocking wrapper for the
/// synchronous bootstrap path; `handle` names the runtime the call runs on.
/// Silence/connect failure/timeout are `Err` — never an answer.
pub fn grpc_discover(
    handle: &tokio::runtime::Handle,
    from: NodeId,
    addr: SocketAddr,
    timeout: Duration,
    token: Option<String>,
) -> Result<DiscoverAnswer> {
    let url = format!("http://{addr}");
    handle.block_on(async move {
        let fut = async {
            let mut client = Kv9RaftClient::connect(url)
                .await
                .map_err(|e| Error::Raft(format!("discovery connect {addr}: {e}")))?;
            let mut req = Request::new(pb::DiscoverRequest {
                from_node: from.0,
                voter_fingerprint: 0,
            });
            attach_auth(&mut req, &token, from);
            let resp = client
                .discover(req)
                .await
                .map_err(|e| Error::Raft(format!("discovery rpc {addr}: {e}")))?
                .into_inner();
            // Contract: an initialized answer MUST name its cluster; an
            // uninitialized one must not. Anything else is a protocol error
            // — treated like a malformed frame, never a lenient default
            // (a nameless "initialized" would push joiners back onto the
            // retired fingerprint).
            let cluster_id = match (resp.initialized, resp.cluster_id.len()) {
                (true, 16) => {
                    let bytes: [u8; 16] =
                        resp.cluster_id.as_slice().try_into().expect("len checked");
                    Some(ClusterId::from_bytes(bytes))
                }
                (true, n) => {
                    return Err(Error::Raft(format!(
                        "initialized discovery answer from {addr} carries a \
                         {n}-byte cluster id (need 16)"
                    )))
                }
                (false, 0) => None,
                (false, _) => {
                    return Err(Error::Raft(format!(
                        "uninitialized discovery answer from {addr} carries a \
                         cluster id"
                    )))
                }
            };
            Ok::<_, Error>(DiscoverAnswer {
                node: NodeId(resp.node_id),
                initialized: resp.initialized,
                voter_fingerprint: resp.voter_fingerprint,
                cluster_id,
            })
        };
        tokio::time::timeout(timeout, fut)
            .await
            .map_err(|_| Error::Raft(format!("discovery timeout {addr}")))?
    })
}

/// A registration attempt's machine-readable outcome. `NotLeader` is decoded
/// from code + marker (BOTH required — other FAILED_PRECONDITION refusals
/// share the code and must surface as plain errors, never as redirects).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterOutcome {
    Registered(RegistrationReceipt),
    NotLeader { leader: Option<NodeId> },
}

/// One-shot registration call (blocking wrapper, like [`grpc_discover`]).
/// Decodes the not-leader redirect strictly: marker must be ASCII `true`;
/// a present-but-garbled leader id is a protocol error, never a silent
/// `None` (guessing "rediscover" when the server named a leader hides a
/// half-broken deployment).
pub fn grpc_register(
    handle: &tokio::runtime::Handle,
    me: NodeId,
    addr: SocketAddr,
    listen_addr: &str,
    cluster_id: ClusterId,
    timeout: Duration,
    token: Option<String>,
) -> Result<RegisterOutcome> {
    let url = format!("http://{addr}");
    let listen_addr = listen_addr.to_string();
    handle.block_on(async move {
        let fut = async {
            let mut client = Kv9RaftClient::connect(url)
                .await
                .map_err(|e| Error::Raft(format!("register connect {addr}: {e}")))?;
            let mut req = Request::new(pb::RegisterRequest {
                node_id: me.0,
                addr: listen_addr,
                cluster_id: cluster_id.as_bytes().to_vec(),
            });
            attach_auth(&mut req, &token, me);
            match client.register(req).await {
                Ok(resp) => {
                    let r = resp.into_inner();
                    Ok(RegisterOutcome::Registered(RegistrationReceipt {
                        applied_term: r.applied_term,
                        applied_index: r.applied_index,
                        voters: r.voters,
                        learners: r.learners,
                    }))
                }
                Err(status) => {
                    let marker_true = status
                        .metadata()
                        .get(NOT_LEADER_KEY)
                        .and_then(|v| v.to_str().ok())
                        == Some("true");
                    if status.code() == tonic::Code::FailedPrecondition && marker_true {
                        let leader = match status.metadata().get(LEADER_NODE_ID_KEY) {
                            None => None,
                            Some(v) => {
                                let id = v
                                    .to_str()
                                    .ok()
                                    .and_then(|s| s.parse::<u64>().ok())
                                    .ok_or_else(|| {
                                        Error::Raft(
                                            "not-leader answer carries an unreadable \
                                             leader id"
                                                .into(),
                                        )
                                    })?;
                                Some(NodeId(id))
                            }
                        };
                        Ok(RegisterOutcome::NotLeader { leader })
                    } else {
                        Err(Error::Raft(format!("register rpc {addr}: {status}")))
                    }
                }
            }
        };
        tokio::time::timeout(timeout, fut)
            .await
            .map_err(|_| Error::Raft(format!("register timeout {addr}")))?
    })
}

/// One discovery answer, decoded and contract-checked ((initialized ⇔ named)
/// is enforced at decode; callers never see a nameless initialized answer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiscoverAnswer {
    pub node: NodeId,
    pub initialized: bool,
    /// Bootstrap-era authority only (uninitialized cross-endorsement fence).
    pub voter_fingerprint: u64,
    /// Post-initialization authority (present ⇔ initialized).
    pub cluster_id: Option<ClusterId>,
}

/// The outbound half + inbox drain: a [`RaftTransport`] carried by gRPC.
///
/// `send` enqueues to a per-peer worker (spawned on the provided runtime
/// handle) that owns one long-lived `BatchRaft` client-stream and flushes
/// batches by count/bytes/window; `drain` empties the inbox the service side
/// fills. The synchronous `NodeDriver` uses both without knowing tonic exists.
pub struct GrpcTransport {
    me: NodeId,
    token: Option<String>,
    handle: tokio::runtime::Handle,
    peers: Mutex<HashMap<u64, mpsc::Sender<pb::RaftEnvelope>>>,
    addrs: Mutex<HashMap<u64, SocketAddr>>,
    inbox_rx: Mutex<mpsc::UnboundedReceiver<Message>>,
    inbox_tx: mpsc::UnboundedSender<Message>,
}

impl GrpcTransport {
    /// Build the transport. `handle` is the runtime the per-peer workers run
    /// on (the server's runtime in production; a test runtime in tests).
    pub fn new(
        me: NodeId,
        token: Option<String>,
        handle: tokio::runtime::Handle,
    ) -> Arc<GrpcTransport> {
        let (inbox_tx, inbox_rx) = mpsc::unbounded_channel();
        Arc::new(GrpcTransport {
            me,
            token,
            handle,
            peers: Mutex::new(HashMap::new()),
            addrs: Mutex::new(HashMap::new()),
            inbox_rx: Mutex::new(inbox_rx),
            inbox_tx,
        })
    }

    /// The sender the service side pushes inbound messages into (register it
    /// with [`RaftGrpcService::new`] on the shared server).
    pub fn inbox_sender(&self) -> mpsc::UnboundedSender<Message> {
        self.inbox_tx.clone()
    }

    /// Declare/replace a peer's address (from the declared `id@addr` set).
    pub fn register_peer(&self, id: NodeId, addr: SocketAddr) {
        self.addrs
            .lock()
            .expect("addrs poisoned")
            .insert(id.0, addr);
    }

    fn peer_sender(&self, to: NodeId) -> Option<mpsc::Sender<pb::RaftEnvelope>> {
        {
            let peers = self.peers.lock().expect("peers poisoned");
            if let Some(s) = peers.get(&to.0) {
                return Some(s.clone());
            }
        }
        let addr = *self.addrs.lock().expect("addrs poisoned").get(&to.0)?;
        let mut peers = self.peers.lock().expect("peers poisoned");
        Some(
            peers
                .entry(to.0)
                .or_insert_with(|| {
                    let (tx, rx) = mpsc::channel(PEER_QUEUE);
                    self.handle
                        .spawn(peer_worker(self.me, addr, self.token.clone(), rx));
                    tx
                })
                .clone(),
        )
    }
}

impl RaftTransport for GrpcTransport {
    fn send(&self, to: NodeId, msg: Message) {
        let Some(sender) = self.peer_sender(to) else {
            return; // unknown peer: drop (raft retransmits after registration)
        };
        let Ok(bytes) = msg.write_to_bytes() else {
            return;
        };
        let env = pb::RaftEnvelope {
            region_id: 0, // META_REGION_0 in Phase 1-final
            from_node: self.me.0,
            to_node: to.0,
            raft_message: bytes,
            epoch_conf_ver: 0,
            epoch_version: 0,
        };
        // Full queue = backpressure by dropping (best-effort, raft recovers).
        let _ = sender.try_send(env);
    }

    fn drain(&self) -> Vec<Message> {
        let mut rx = self.inbox_rx.lock().expect("inbox poisoned");
        let mut out = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            out.push(msg);
        }
        out
    }
}

/// Per-peer outbound worker: batch by count/bytes with a short flush window,
/// one long-lived client-stream per connection, reconnect with backoff.
async fn peer_worker(
    me: NodeId,
    addr: SocketAddr,
    token: Option<String>,
    mut rx: mpsc::Receiver<pb::RaftEnvelope>,
) {
    let url = format!("http://{addr}");
    let mut backoff = RECONNECT_MIN;
    loop {
        // (Re)connect.
        let mut client = match Kv9RaftClient::connect(url.clone()).await {
            Ok(c) => {
                backoff = RECONNECT_MIN;
                c
            }
            Err(_) => {
                // Drain whatever queued during the outage (drop: best-effort),
                // then back off before retrying.
                while rx.try_recv().is_ok() {}
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(RECONNECT_MAX);
                continue;
            }
        };

        // One stream per connection; the receiver task of `outbound` feeds it.
        let (batch_tx, batch_rx) = mpsc::channel::<pb::BatchRaftMessage>(16);
        let stream = tokio_stream::wrappers::ReceiverStream::new(batch_rx);
        let mut stream_req = Request::new(stream);
        attach_auth(&mut stream_req, &token, me);
        let rpc = client.batch_raft(stream_req);
        tokio::pin!(rpc);

        // Batch loop: runs until the peer connection dies or we shut down.
        'batching: loop {
            let first = tokio::select! {
                m = rx.recv() => match m {
                    Some(m) => m,
                    None => return, // transport dropped: shut down worker
                },
                // The RPC resolving means the server closed our stream.
                _ = &mut rpc => break 'batching,
            };
            let mut batch = pb::BatchRaftMessage {
                msgs: vec![first],
                flushed_unix_nanos: 0,
            };
            let mut bytes: usize = batch.msgs[0].raft_message.len();
            let window = tokio::time::sleep(FLUSH_WINDOW);
            tokio::pin!(window);
            while batch.msgs.len() < MAX_BATCH_MSGS && bytes < MAX_BATCH_BYTES {
                tokio::select! {
                    m = rx.recv() => match m {
                        Some(m) => {
                            bytes += m.raft_message.len();
                            batch.msgs.push(m);
                        }
                        None => break,
                    },
                    _ = &mut window => break,
                }
            }
            if batch_tx.send(batch).await.is_err() {
                break 'batching; // stream side gone: reconnect
            }
        }
        drop(batch_tx);
        // Loop back to reconnect.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::NodeDriver;
    use crate::rawnode::RaftPeer;
    use crate::{Command, MemStateMachine, RaftGroup, Role};
    use kv9_common::RegionId;
    use pb::kv9_raft_server::Kv9RaftServer;

    struct StaticDiscovery(NodeId, bool, u64);
    impl GrpcDiscoveryState for StaticDiscovery {
        fn answer(&self) -> (NodeId, bool, u64) {
            (self.0, self.1, self.2)
        }
    }

    /// Initialized discovery with an identity (the coupled contract's
    /// positive half).
    struct NamedDiscovery(NodeId, ClusterId);
    impl GrpcDiscoveryState for NamedDiscovery {
        fn answer(&self) -> (NodeId, bool, u64) {
            (self.0, true, 0)
        }
        fn cluster_id(&self) -> Option<ClusterId> {
            Some(self.1)
        }
    }

    /// The broken responder: claims initialized, names nothing. The service
    /// must refuse to publish this answer.
    struct NamelessInitialized(NodeId);
    impl GrpcDiscoveryState for NamelessInitialized {
        fn answer(&self) -> (NodeId, bool, u64) {
            (self.0, true, 0)
        }
    }

    /// Stub registration backend recording the call it served; the response
    /// is scripted per call so one server exercises every outcome.
    struct StubRegistration {
        calls: std::sync::Mutex<Vec<(NodeId, String, ClusterId)>>,
        script: std::sync::Mutex<Vec<std::result::Result<(), RegistrationError>>>,
    }
    impl StubRegistration {
        fn ok_only() -> Self {
            StubRegistration {
                calls: std::sync::Mutex::new(Vec::new()),
                script: std::sync::Mutex::new(Vec::new()),
            }
        }
    }
    impl RegistrationBackend for StubRegistration {
        fn register(
            &self,
            node: NodeId,
            addr: &str,
            cluster_id: ClusterId,
        ) -> std::result::Result<RegistrationReceipt, RegistrationError> {
            self.calls
                .lock()
                .unwrap()
                .push((node, addr.to_string(), cluster_id));
            if let Some(step) = self.script.lock().unwrap().pop() {
                step?;
            }
            Ok(RegistrationReceipt {
                applied_term: 3,
                applied_index: 17,
                voters: vec![1, 2, 3],
                learners: vec![node.0],
            })
        }
    }

    fn test_cid() -> ClusterId {
        ClusterId::from_bytes([0xAB; 16])
    }

    fn free_addr() -> SocketAddr {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
    }

    /// Serve one node's RaftGrpcService on `addr` (the server crate does this
    /// on its shared tonic server in production; tests build a minimal one).
    fn serve(
        handle: &tokio::runtime::Handle,
        me: NodeId,
        addr: SocketAddr,
        inbox: mpsc::UnboundedSender<Message>,
        fp: u64,
    ) {
        let svc = RaftGrpcService::new(me, inbox, Arc::new(StaticDiscovery(me, false, fp)));
        handle.spawn(async move {
            tonic::transport::Server::builder()
                .add_service(Kv9RaftServer::with_interceptor(
                    svc,
                    cluster_token_interceptor("test-cluster-token".into()),
                ))
                .serve(addr)
                .await
                .ok();
        });
    }

    /// The full #19 path: three nodes, raft over REAL gRPC streams (tonic
    /// server + client-streaming batches), election, replication verified by
    /// (term, index) on every node, live leader failover — the same
    /// correctness surface the TCP transport passed, on the new carrier.
    #[test]
    fn three_nodes_over_grpc_elect_replicate_failover() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let handle = rt.handle().clone();
        let region = RegionId(1);
        let ids = [NodeId(1), NodeId(2), NodeId(3)];
        let addrs: Vec<SocketAddr> = ids.iter().map(|_| free_addr()).collect();

        let mut drivers = Vec::new();
        for (i, &id) in ids.iter().enumerate() {
            let transport =
                GrpcTransport::new(id, Some("test-cluster-token".into()), handle.clone());
            for (j, &peer) in ids.iter().enumerate() {
                if peer != id {
                    transport.register_peer(peer, addrs[j]);
                }
            }
            serve(&handle, id, addrs[i], transport.inbox_sender(), 42);
            let peer = Arc::new(RaftPeer::new(id, region, &ids).unwrap());
            drivers.push(NodeDriver::new(
                peer,
                transport as Arc<dyn RaftTransport>,
                MemStateMachine::new(),
            ));
        }
        // Give the listeners a beat to come up, then run production cadence.
        std::thread::sleep(Duration::from_millis(100));
        let handles: Vec<_> = drivers
            .iter()
            .map(|d| d.spawn(Duration::from_millis(10)))
            .collect();

        drivers[0].peer().campaign().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let leader_idx = loop {
            if let Some(i) = drivers.iter().position(|d| d.status().role == Role::Leader) {
                break i;
            }
            assert!(std::time::Instant::now() < deadline, "no leader over gRPC");
            std::thread::sleep(Duration::from_millis(5));
        };

        let cmd = Command::Put {
            cf: 0,
            key: b"grpc".to_vec(),
            value: b"carried".to_vec(),
        };
        let at = drivers[leader_idx].propose(&cmd).unwrap();
        for d in &drivers {
            assert!(
                d.wait_applied(at, Duration::from_secs(20)).unwrap(),
                "exact (term,index) must apply on every node over gRPC"
            );
            assert_eq!(
                d.get(kv9_engine::ColumnFamily::Default, b"grpc").unwrap(),
                Some(b"carried".to_vec())
            );
        }

        // Live failover: stop the leader's pump; survivors re-elect and commit.
        drivers[leader_idx].stop();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let new_leader = loop {
            if let Some(i) = drivers
                .iter()
                .enumerate()
                .position(|(i, d)| i != leader_idx && d.status().role == Role::Leader)
            {
                break i;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "no re-election over gRPC"
            );
            std::thread::sleep(Duration::from_millis(5));
        };
        let cmd2 = Command::Put {
            cf: 0,
            key: b"after".to_vec(),
            value: b"failover".to_vec(),
        };
        let at2 = drivers[new_leader].propose(&cmd2).unwrap();
        assert!(at2.term > at.term);
        for (i, d) in drivers.iter().enumerate() {
            if i == leader_idx {
                continue;
            }
            assert!(d.wait_applied(at2, Duration::from_secs(20)).unwrap());
        }

        for d in &drivers {
            d.stop();
        }
        for h in handles {
            let _ = h.join();
        }
    }

    /// Discovery over gRPC keeps the fencing semantics: a live peer answers
    /// with its identity + declared-set fingerprint; silence (unbound addr)
    /// is an Err — never an answer.
    #[test]
    fn grpc_discovery_answers_and_silence_is_err() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let handle = rt.handle().clone();
        let addr = free_addr();
        let (tx, _rx) = mpsc::unbounded_channel();
        serve(&handle, NodeId(7), addr, tx, 0xFEED);
        std::thread::sleep(Duration::from_millis(100));

        let a = grpc_discover(
            &handle,
            NodeId(1),
            addr,
            Duration::from_secs(2),
            Some("test-cluster-token".into()),
        )
        .unwrap();
        assert_eq!(a.node, NodeId(7));
        assert!(!a.initialized);
        assert_eq!(
            a.voter_fingerprint, 0xFEED,
            "answer must carry the responder's declaration"
        );
        assert_eq!(a.cluster_id, None, "uninitialized answers name nothing");

        assert!(grpc_discover(
            &handle,
            NodeId(1),
            free_addr(),
            Duration::from_millis(300),
            Some("test-cluster-token".into())
        )
        .is_err());
    }

    /// Token auth (EdHuang: ships with the rewrite): wrong or missing token is
    /// Unauthenticated; the right token passes. Control both ways — an
    /// interceptor that rejects everything would pass the first two asserts.
    #[test]
    fn cluster_token_gates_the_service() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let handle = rt.handle().clone();
        let addr = free_addr();
        let (tx, _rx) = mpsc::unbounded_channel();
        let svc = RaftGrpcService::new(
            NodeId(5),
            tx,
            Arc::new(StaticDiscovery(NodeId(5), false, 1)),
        );
        handle.spawn(async move {
            tonic::transport::Server::builder()
                .add_service(pb::kv9_raft_server::Kv9RaftServer::with_interceptor(
                    svc,
                    cluster_token_interceptor("sesame".into()),
                ))
                .serve(addr)
                .await
                .ok();
        });
        std::thread::sleep(Duration::from_millis(100));

        // Missing token: refused.
        assert!(grpc_discover(&handle, NodeId(1), addr, Duration::from_secs(2), None).is_err());
        // Wrong token: refused.
        assert!(grpc_discover(
            &handle,
            NodeId(1),
            addr,
            Duration::from_secs(2),
            Some("wrong".into())
        )
        .is_err());
        // Right token: answered (control — the gate opens for the key).
        let a = grpc_discover(
            &handle,
            NodeId(1),
            addr,
            Duration::from_secs(2),
            Some("sesame".into()),
        )
        .unwrap();
        assert_eq!(a.node, NodeId(5));
    }

    /// A misrouted envelope (wrong to_node) kills the stream with a loud error
    /// — CSE's StoreNotMatch shape — and is counted, never silently applied.
    #[test]
    fn misrouted_envelope_errors_the_stream() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let handle = rt.handle().clone();
        let addr = free_addr();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let svc = Arc::new(RaftGrpcService::new(
            NodeId(1),
            tx,
            Arc::new(StaticDiscovery(NodeId(1), false, 0)),
        ));
        {
            let svc = Arc::clone(&svc);
            handle.spawn(async move {
                // Arc<RaftGrpcService> can't be moved into the server twice;
                // build a thin forwarding service for the test.
                struct Fwd(Arc<RaftGrpcService>);
                #[tonic::async_trait]
                impl Kv9Raft for Fwd {
                    async fn batch_raft(
                        &self,
                        r: Request<Streaming<pb::BatchRaftMessage>>,
                    ) -> std::result::Result<Response<pb::Done>, Status> {
                        self.0.batch_raft(r).await
                    }
                    async fn discover(
                        &self,
                        r: Request<pb::DiscoverRequest>,
                    ) -> std::result::Result<Response<pb::DiscoverResponse>, Status>
                    {
                        self.0.discover(r).await
                    }
                    async fn register(
                        &self,
                        r: Request<pb::RegisterRequest>,
                    ) -> std::result::Result<Response<pb::RegisterReceipt>, Status>
                    {
                        self.0.register(r).await
                    }
                }
                tonic::transport::Server::builder()
                    .add_service(Kv9RaftServer::with_interceptor(
                        Fwd(svc),
                        cluster_token_interceptor("test-cluster-token".into()),
                    ))
                    .serve(addr)
                    .await
                    .ok();
            });
        }
        std::thread::sleep(Duration::from_millis(100));

        let sender_status = handle.block_on(async {
            let mut client = Kv9RaftClient::connect(format!("http://{addr}"))
                .await
                .unwrap();
            let batch = pb::BatchRaftMessage {
                msgs: vec![pb::RaftEnvelope {
                    region_id: 0,
                    from_node: 8, // body tries to impersonate another voter
                    to_node: 1,
                    raft_message: Message::default().write_to_bytes().unwrap(),
                    epoch_conf_ver: 0,
                    epoch_version: 0,
                }],
                flushed_unix_nanos: 0,
            };
            let mut request = Request::new(tokio_stream::iter(vec![batch]));
            attach_auth(&mut request, &Some("test-cluster-token".into()), NodeId(9));
            client.batch_raft(request).await.unwrap_err()
        });
        assert_eq!(sender_status.code(), tonic::Code::PermissionDenied);
        assert_eq!(svc.misrouted(), 0, "sender spoofing is not a routing error");

        let status = handle.block_on(async {
            let mut client = Kv9RaftClient::connect(format!("http://{addr}"))
                .await
                .unwrap();
            let env = pb::RaftEnvelope {
                region_id: 0,
                from_node: 9,
                to_node: 99, // wrong destination
                raft_message: Message::default().write_to_bytes().unwrap(),
                epoch_conf_ver: 0,
                epoch_version: 0,
            };
            let batch = pb::BatchRaftMessage {
                msgs: vec![env],
                flushed_unix_nanos: 0,
            };
            let stream = tokio_stream::iter(vec![batch]);
            let mut request = Request::new(stream);
            attach_auth(&mut request, &Some("test-cluster-token".into()), NodeId(9));
            client.batch_raft(request).await
        });
        assert!(status.is_err(), "misroute must error the stream");
        assert_eq!(svc.misrouted(), 1);
        // Control (sensitivity): nothing reached the core inbox.
        assert!(rx.try_recv().is_err());
    }


    /// The register seam end-to-end over a real wire: authenticated identity
    /// must match the body, absence of a backend is UNIMPLEMENTED (loud stub
    /// discipline — never a fabricated success), and the happy path returns
    /// the backend's exact receipt with the call recorded.
    #[test]
    fn register_enforces_identity_and_backend_presence() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let handle = rt.handle().clone();

        // Server WITHOUT a backend: UNIMPLEMENTED.
        let bare = free_addr();
        {
            let (tx, _rx) = mpsc::unbounded_channel();
            let svc = RaftGrpcService::new(
                NodeId(1),
                tx,
                Arc::new(NamedDiscovery(NodeId(1), test_cid())),
            );
            handle.spawn(async move {
                tonic::transport::Server::builder()
                    .add_service(Kv9RaftServer::with_interceptor(
                        svc,
                        cluster_token_interceptor("test-cluster-token".into()),
                    ))
                    .serve(bare)
                    .await
                    .ok();
            });
        }
        // Server WITH a stub backend.
        let addr = free_addr();
        let backend = Arc::new(StubRegistration::ok_only());
        {
            let (tx, _rx) = mpsc::unbounded_channel();
            let svc = RaftGrpcService::new(
                NodeId(1),
                tx,
                Arc::new(NamedDiscovery(NodeId(1), test_cid())),
            )
            .with_registration(backend.clone() as Arc<dyn RegistrationBackend>);
            handle.spawn(async move {
                tonic::transport::Server::builder()
                    .add_service(Kv9RaftServer::with_interceptor(
                        svc,
                        cluster_token_interceptor("test-cluster-token".into()),
                    ))
                    .serve(addr)
                    .await
                    .ok();
            });
        }
        std::thread::sleep(Duration::from_millis(100));

        let register = |target: SocketAddr, as_node: u64, body_node: u64| {
            handle.block_on(async move {
                let mut client = Kv9RaftClient::connect(format!("http://{target}"))
                    .await
                    .unwrap();
                let mut req = Request::new(pb::RegisterRequest {
                    node_id: body_node,
                    addr: "127.0.0.1:9009".into(),
                    cluster_id: test_cid().as_bytes().to_vec(),
                });
                attach_auth(&mut req, &Some("test-cluster-token".into()), NodeId(as_node));
                client.register(req).await
            })
        };

        // No backend: loudly unimplemented.
        assert_eq!(
            register(bare, 4, 4).unwrap_err().code(),
            tonic::Code::Unimplemented
        );
        // Body identity != authenticated identity: refused before any backend
        // call (the stub records nothing).
        assert_eq!(
            register(addr, 4, 9).unwrap_err().code(),
            tonic::Code::PermissionDenied
        );
        assert!(backend.calls.lock().unwrap().is_empty());
        // Happy path: the backend's exact receipt comes back, call recorded.
        let receipt = register(addr, 4, 4).unwrap().into_inner();
        assert_eq!((receipt.applied_term, receipt.applied_index), (3, 17));
        assert_eq!(receipt.voters, vec![1, 2, 3]);
        assert_eq!(receipt.learners, vec![4]);
        assert_eq!(
            backend.calls.lock().unwrap().as_slice(),
            &[(NodeId(4), "127.0.0.1:9009".to_string(), test_cid())]
        );
    }

    /// The coupled discovery contract, both halves: an initialized answer
    /// carries the identity (and a joiner can read it), and a responder that
    /// claims initialized but names nothing is refused BY THE SERVICE — the
    /// broken answer never reaches the wire.
    #[test]
    fn initialized_discovery_must_name_its_cluster() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let handle = rt.handle().clone();

        let named = free_addr();
        {
            let (tx, _rx) = mpsc::unbounded_channel();
            let svc = RaftGrpcService::new(
                NodeId(2),
                tx,
                Arc::new(NamedDiscovery(NodeId(2), test_cid())),
            );
            handle.spawn(async move {
                tonic::transport::Server::builder()
                    .add_service(Kv9RaftServer::with_interceptor(
                        svc,
                        cluster_token_interceptor("test-cluster-token".into()),
                    ))
                    .serve(named)
                    .await
                    .ok();
            });
        }
        let nameless = free_addr();
        {
            let (tx, _rx) = mpsc::unbounded_channel();
            let svc =
                RaftGrpcService::new(NodeId(3), tx, Arc::new(NamelessInitialized(NodeId(3))));
            handle.spawn(async move {
                tonic::transport::Server::builder()
                    .add_service(Kv9RaftServer::with_interceptor(
                        svc,
                        cluster_token_interceptor("test-cluster-token".into()),
                    ))
                    .serve(nameless)
                    .await
                    .ok();
            });
        }
        std::thread::sleep(Duration::from_millis(100));

        let a = grpc_discover(
            &handle,
            NodeId(1),
            named,
            Duration::from_secs(2),
            Some("test-cluster-token".into()),
        )
        .unwrap();
        assert!(a.initialized);
        assert_eq!(a.cluster_id, Some(test_cid()));
        // Post-init the fingerprint has retired: nothing meaningful travels.
        assert_eq!(a.voter_fingerprint, 0);

        // The nameless-initialized responder is a server-side error — the
        // client sees a failed RPC, never a lenient nameless answer.
        assert!(grpc_discover(
            &handle,
            NodeId(1),
            nameless,
            Duration::from_secs(2),
            Some("test-cluster-token".into()),
        )
        .is_err());
    }


    /// The machine-readable not-leader contract, both directions (Tess's
    /// seam blocker on dd69bcd): a follower's refusal decodes to a TYPED
    /// redirect — with and without a leader hint — while an ordinary
    /// precondition refusal (same gRPC code, no marker) surfaces as a plain
    /// error. Sensitivity is the third call: if the client keyed on the code
    /// alone, that call would decode as NotLeader and the assert would fail.
    #[test]
    fn register_not_leader_redirect_is_machine_readable() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let handle = rt.handle().clone();
        let addr = free_addr();
        let backend = Arc::new(StubRegistration::ok_only());
        // Script (popped in reverse order): hinted redirect, hintless
        // redirect, ordinary failure.
        *backend.script.lock().unwrap() = vec![
            Err(RegistrationError::Failed(Error::Config(
                "admission expired".into(),
            ))),
            Err(RegistrationError::NotLeader { leader: None }),
            Err(RegistrationError::NotLeader {
                leader: Some(NodeId(7)),
            }),
        ];
        {
            let backend = backend.clone() as Arc<dyn RegistrationBackend>;
            let (tx, _rx) = mpsc::unbounded_channel();
            let svc = RaftGrpcService::new(
                NodeId(1),
                tx,
                Arc::new(NamedDiscovery(NodeId(1), test_cid())),
            )
            .with_registration(backend);
            handle.spawn(async move {
                tonic::transport::Server::builder()
                    .add_service(Kv9RaftServer::with_interceptor(
                        svc,
                        cluster_token_interceptor("test-cluster-token".into()),
                    ))
                    .serve(addr)
                    .await
                    .ok();
            });
        }
        std::thread::sleep(Duration::from_millis(100));

        let call = || {
            grpc_register(
                &handle,
                NodeId(4),
                addr,
                "127.0.0.1:9009",
                test_cid(),
                Duration::from_secs(2),
                Some("test-cluster-token".into()),
            )
        };

        // Hinted redirect: typed, with the leader id.
        assert_eq!(
            call().unwrap(),
            RegisterOutcome::NotLeader {
                leader: Some(NodeId(7))
            }
        );
        // Hintless redirect: typed, leader None (absent key, never "0").
        assert_eq!(call().unwrap(), RegisterOutcome::NotLeader { leader: None });
        // Ordinary precondition refusal — same code, NO marker: a plain
        // error, not a redirect (code-only decoding would fail here).
        let err = call().unwrap_err().to_string();
        assert!(
            err.contains("admission expired"),
            "ordinary refusal lost its cause: {err}"
        );
        // Control: after the script drains, the same wire registers fine.
        assert!(matches!(call().unwrap(), RegisterOutcome::Registered(_)));
    }
}
