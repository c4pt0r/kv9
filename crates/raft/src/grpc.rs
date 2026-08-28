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

use kv9_common::{Error, NodeId, Result};

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

/// Server-side interceptor enforcing the shared cluster token. The server
/// crate wraps registered services with this (or with the richer
/// `AuthContext` authenticator from the external-API work — same contract:
/// handlers never trust caller identity from the body).
pub fn cluster_token_interceptor(
    expected: String,
) -> impl FnMut(Request<()>) -> std::result::Result<Request<()>, Status> + Clone {
    move |req: Request<()>| match req.metadata().get(CLUSTER_TOKEN_KEY) {
        Some(v) if v.to_str().map(|s| s == expected).unwrap_or(false) => Ok(req),
        Some(_) => Err(Status::unauthenticated("cluster token mismatch")),
        None => Err(Status::unauthenticated("cluster token required")),
    }
}

fn attach_token<T>(req: &mut Request<T>, token: &Option<String>) {
    if let Some(t) = token {
        if let Ok(v) = t.parse() {
            req.metadata_mut().insert(CLUSTER_TOKEN_KEY, v);
        }
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
}

/// The inbound half: implements the generated service. Holds ONLY a channel
/// sender into the synchronous core — no listener, no runtime, no port. The
/// server crate registers this on its single shared `tonic` server.
pub struct RaftGrpcService {
    me: NodeId,
    inbox: mpsc::UnboundedSender<Message>,
    discovery: Arc<dyn GrpcDiscoveryState>,
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
            misrouted: AtomicU64::new(0),
        }
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
        let mut stream = request.into_inner();
        loop {
            let batch = match stream.message().await {
                Ok(Some(b)) => b,
                Ok(None) => return Ok(Response::new(pb::Done {})), // clean end
                Err(status) => return Err(status),
            };
            for env in batch.msgs {
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
        _request: Request<pb::DiscoverRequest>,
    ) -> std::result::Result<Response<pb::DiscoverResponse>, Status> {
        let (node, initialized, fp) = self.discovery.answer();
        Ok(Response::new(pb::DiscoverResponse {
            node_id: node.0,
            initialized,
            voter_fingerprint: fp,
        }))
    }
}

/// One-shot discovery over gRPC (fencing rule a). Blocking wrapper for the
/// synchronous bootstrap path; `handle` names the runtime the call runs on.
/// Silence/connect failure/timeout are `Err` — never an answer.
pub fn grpc_discover(
    handle: &tokio::runtime::Handle,
    addr: SocketAddr,
    timeout: Duration,
    token: Option<String>,
) -> Result<(NodeId, bool, u64)> {
    let url = format!("http://{addr}");
    handle.block_on(async move {
        let fut = async {
            let mut client = Kv9RaftClient::connect(url)
                .await
                .map_err(|e| Error::Raft(format!("discovery connect {addr}: {e}")))?;
            let mut req = Request::new(pb::DiscoverRequest {
                from_node: 0,
                voter_fingerprint: 0,
            });
            attach_token(&mut req, &token);
            let resp = client
                .discover(req)
                .await
                .map_err(|e| Error::Raft(format!("discovery rpc {addr}: {e}")))?
                .into_inner();
            Ok::<_, Error>((NodeId(resp.node_id), resp.initialized, resp.voter_fingerprint))
        };
        tokio::time::timeout(timeout, fut)
            .await
            .map_err(|_| Error::Raft(format!("discovery timeout {addr}")))?
    })
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
        self.addrs.lock().expect("addrs poisoned").insert(id.0, addr);
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
                    self.handle.spawn(peer_worker(addr, self.token.clone(), rx));
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
        attach_token(&mut stream_req, &token);
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
                .add_service(Kv9RaftServer::new(svc))
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
            let transport = GrpcTransport::new(id, None, handle.clone());
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
            if let Some(i) = drivers.iter().enumerate().position(|(i, d)| {
                i != leader_idx && d.status().role == Role::Leader
            }) {
                break i;
            }
            assert!(std::time::Instant::now() < deadline, "no re-election over gRPC");
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

        let (node, initialized, fp) =
            grpc_discover(&handle, addr, Duration::from_secs(2), None).unwrap();
        assert_eq!(node, NodeId(7));
        assert!(!initialized);
        assert_eq!(fp, 0xFEED, "answer must carry the responder's declaration");

        assert!(grpc_discover(&handle, free_addr(), Duration::from_millis(300), None).is_err());
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
        let svc = RaftGrpcService::new(NodeId(5), tx, Arc::new(StaticDiscovery(NodeId(5), false, 1)));
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
        assert!(grpc_discover(&handle, addr, Duration::from_secs(2), None).is_err());
        // Wrong token: refused.
        assert!(
            grpc_discover(&handle, addr, Duration::from_secs(2), Some("wrong".into())).is_err()
        );
        // Right token: answered (control — the gate opens for the key).
        let (node, _, _) =
            grpc_discover(&handle, addr, Duration::from_secs(2), Some("sesame".into())).unwrap();
        assert_eq!(node, NodeId(5));
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
                    ) -> std::result::Result<Response<pb::DiscoverResponse>, Status> {
                        self.0.discover(r).await
                    }
                }
                tonic::transport::Server::builder()
                    .add_service(Kv9RaftServer::new(Fwd(svc)))
                    .serve(addr)
                    .await
                    .ok();
            });
        }
        std::thread::sleep(Duration::from_millis(100));

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
            client.batch_raft(Request::new(stream)).await
        });
        assert!(status.is_err(), "misroute must error the stream");
        assert_eq!(svc.misrouted(), 1);
        // Control (sensitivity): nothing reached the core inbox.
        assert!(rx.try_recv().is_err());
    }
}
