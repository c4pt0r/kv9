//! Legacy framed-TCP transport retained as a codec/compatibility harness.
//!
//! Design boundaries:
//! Production node processes use the gRPC streaming transport in [`crate::grpc`].
//! This module still exercises framing corruption and the transport abstraction
//! independently; it is not the server runtime's listener.
//! - Delivery is **best-effort**: raft tolerates loss, duplication, and
//!   reordering. A send failure drops the message (and the connection); raft's
//!   own retransmission recovers.
//! - A frame-level decode error **kills the connection** — once framing is
//!   suspect, every subsequent byte on that stream is untrusted. Typed errors,
//!   never panics (DESIGN principle "never panic on the unknown").
//! - **No frame checksum — deliberately.** TCP already checksums in transit and
//!   nothing here is stored: a corrupt stream is handled by dropping the
//!   connection. The disk WAL's CRC exists for silent long-term media
//!   corruption — a different threat model; do not "fix" the asymmetry.
//!
//! Frame format v1 (all integers big-endian):
//!
//! ```text
//! magic   u16 = 0x4B39 ("K9")
//! version u8  = 1
//! kind    u8  : 1 = raft Message (rust-protobuf bytes)
//!               2 = discovery request  : ver u8=1 | from_node u64
//!               3 = discovery response : ver u8=1 | node u64 | initialized u8
//! len     u32   (payload length, capped at 16 MiB)
//! payload len bytes
//! ```

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use protobuf::Message as PbMessage;
use raft::prelude::Message;

use kv9_common::{Error, NodeId, Result};

/// Frame header magic: "K9".
pub const FRAME_MAGIC: u16 = 0x4B39;
/// Frame format version this binary speaks.
pub const FRAME_VERSION: u8 = 1;
/// Hard cap for raft-message payloads (snapshots); larger is corrupt.
pub const MAX_FRAME_LEN: u32 = 16 * 1024 * 1024;
/// Discovery payloads are FIXED length; anything else is invalid by definition
/// and must be rejected before any allocation happens.
const DISCOVERY_REQ_LEN: u32 = 17; // ver(1) + from_node(8) + voter_fp(8)
const DISCOVERY_RESP_LEN: u32 = 18; // ver(1) + node(8) + initialized(1) + voter_fp(8)

/// Canonical fingerprint of a declared voter set: FNV-1a-64 over the entries
/// sorted by node id, each encoded as `id u64 BE ++ addr-string ++ 0x00`.
///
/// Purpose (bootstrap fencing): a discovery answer must be bound to WHICH
/// declaration it endorses. Two nodes with divergent `--join` sets (e.g.
/// `{1,2,3}` vs `{1,2,9}`) would otherwise count each other's "uninitialized"
/// as a positive vote and assemble groups with different ConfStates. The
/// runtime counts only answers whose fingerprint equals its own. This detects
/// misconfiguration (a copy-paste-edited seed list, a wrong id) — it is NOT a
/// cryptographic commitment and must not be "upgraded" to one: the threat is
/// configuration accidents, not adversaries, and 64-bit FNV is exactly enough
/// for that. An adversarial peer defeats any hash here equally, because it can
/// simply echo whatever fingerprint it is asked for. If discovery ever has to
/// face untrusted peers, the correct move is control-plane authentication
/// (DESIGN principle 9, "Auth on the control/management plane from day one") —
/// not a wider hash.
pub fn voter_set_fingerprint(declared: &[(u64, SocketAddr)]) -> u64 {
    let mut entries: Vec<(u64, String)> = declared
        .iter()
        .map(|(id, a)| (*id, a.to_string()))
        .collect();
    entries.sort();
    let mut hash: u64 = 0xcbf29ce484222325;
    let mut eat = |b: u8| {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    };
    for (id, addr) in entries {
        for b in id.to_be_bytes() {
            eat(b);
        }
        for b in addr.as_bytes() {
            eat(*b);
        }
        eat(0x00);
    }
    hash
}

const KIND_RAFT: u8 = 1;
const KIND_DISCOVERY_REQ: u8 = 2;
const KIND_DISCOVERY_RESP: u8 = 3;

/// One decoded frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// A raft protocol message (protobuf bytes, decoded by the caller).
    Raft(Vec<u8>),
    /// "Is your cluster initialized?" (bootstrap discovery, fencing rule a).
    /// Carries the asker's declared voter-set fingerprint.
    DiscoveryReq { from: NodeId, voter_fp: u64 },
    /// The positive, attributable answer discovery quorums are built from.
    /// `voter_fp` binds the answer to the RESPONDER's declaration: the caller
    /// counts it only if the fingerprints are identical.
    DiscoveryResp {
        node: NodeId,
        initialized: bool,
        voter_fp: u64,
    },
}

/// Encode a frame (header + payload).
pub fn encode_frame(frame: &Frame) -> Vec<u8> {
    let (kind, payload): (u8, Vec<u8>) = match frame {
        Frame::Raft(bytes) => (KIND_RAFT, bytes.clone()),
        Frame::DiscoveryReq { from, voter_fp } => {
            let mut p = vec![1u8];
            p.extend_from_slice(&from.0.to_be_bytes());
            p.extend_from_slice(&voter_fp.to_be_bytes());
            (KIND_DISCOVERY_REQ, p)
        }
        Frame::DiscoveryResp {
            node,
            initialized,
            voter_fp,
        } => {
            let mut p = vec![1u8];
            p.extend_from_slice(&node.0.to_be_bytes());
            p.push(u8::from(*initialized));
            p.extend_from_slice(&voter_fp.to_be_bytes());
            (KIND_DISCOVERY_RESP, p)
        }
    };
    let mut out = Vec::with_capacity(8 + payload.len());
    out.extend_from_slice(&FRAME_MAGIC.to_be_bytes());
    out.push(FRAME_VERSION);
    out.push(kind);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(&payload);
    out
}

/// Read one frame from a stream. Every failure is a typed error; the caller
/// must treat any error as fatal for the connection.
pub fn read_frame(stream: &mut impl Read) -> Result<Frame> {
    let mut header = [0u8; 8];
    stream
        .read_exact(&mut header)
        .map_err(|e| Error::Raft(format!("frame header read: {e}")))?;
    let magic = u16::from_be_bytes([header[0], header[1]]);
    if magic != FRAME_MAGIC {
        return Err(Error::Raft(format!("bad frame magic {magic:#06x}")));
    }
    if header[2] != FRAME_VERSION {
        return Err(Error::Raft(format!("unknown frame version {}", header[2])));
    }
    let kind = header[3];
    let len = u32::from_be_bytes([header[4], header[5], header[6], header[7]]);
    // Per-kind length validation BEFORE any allocation: fixed-size kinds must
    // be exactly their size (a 16 MiB "discovery" frame is invalid by
    // definition, not merely oversized), and only raft messages get MiBs.
    let valid = match kind {
        KIND_RAFT => len <= MAX_FRAME_LEN,
        KIND_DISCOVERY_REQ => len == DISCOVERY_REQ_LEN,
        KIND_DISCOVERY_RESP => len == DISCOVERY_RESP_LEN,
        other => return Err(Error::Raft(format!("unknown frame kind {other}"))),
    };
    if !valid {
        return Err(Error::Raft(format!(
            "invalid length {len} for frame kind {kind}"
        )));
    }
    let mut payload = vec![0u8; len as usize];
    stream
        .read_exact(&mut payload)
        .map_err(|e| Error::Raft(format!("frame payload read (len {len}): {e}")))?;
    decode_payload(kind, payload)
}

fn decode_payload(kind: u8, payload: Vec<u8>) -> Result<Frame> {
    match kind {
        KIND_RAFT => Ok(Frame::Raft(payload)),
        KIND_DISCOVERY_REQ => {
            if payload.len() != DISCOVERY_REQ_LEN as usize || payload[0] != 1 {
                return Err(Error::Raft("malformed discovery request".into()));
            }
            let mut b = [0u8; 8];
            b.copy_from_slice(&payload[1..9]);
            let mut f = [0u8; 8];
            f.copy_from_slice(&payload[9..17]);
            Ok(Frame::DiscoveryReq {
                from: NodeId(u64::from_be_bytes(b)),
                voter_fp: u64::from_be_bytes(f),
            })
        }
        KIND_DISCOVERY_RESP => {
            if payload.len() != DISCOVERY_RESP_LEN as usize || payload[0] != 1 {
                return Err(Error::Raft("malformed discovery response".into()));
            }
            let mut b = [0u8; 8];
            b.copy_from_slice(&payload[1..9]);
            let node = NodeId(u64::from_be_bytes(b));
            // This bit feeds bootstrap fencing: only 0 and 1 are legal. 2..=255
            // must NOT coerce to "initialized" (nor to "uninitialized") — a
            // garbled byte is an error, not an answer.
            let initialized = match payload[9] {
                0 => false,
                1 => true,
                other => {
                    return Err(Error::Raft(format!(
                        "invalid initialized byte {other} (0 or 1)"
                    )))
                }
            };
            let mut f = [0u8; 8];
            f.copy_from_slice(&payload[10..18]);
            Ok(Frame::DiscoveryResp {
                node,
                initialized,
                voter_fp: u64::from_be_bytes(f),
            })
        }
        other => Err(Error::Raft(format!("unknown frame kind {other}"))),
    }
}

/// Answers this node's discovery state (bootstrap fencing rule a): the id it
/// answers as, and whether its cluster/data-dir is initialized.
pub trait DiscoveryState: Send + Sync {
    /// `(this node's id, initialized?, this node's declared voter-set fingerprint)`.
    fn answer(&self) -> (NodeId, bool, u64);
}

/// Message transport between the peers of one raft group.
///
/// `send` is non-blocking best-effort; `drain` returns messages delivered to
/// this node since the last drain, in arrival order.
pub trait RaftTransport: Send + Sync {
    fn send(&self, to: NodeId, msg: Message);
    fn drain(&self) -> Vec<Message>;
}

// ---------------------------------------------------------------------------
// In-process transport (tests / single-process clusters).
// ---------------------------------------------------------------------------

type Inboxes = Mutex<HashMap<u64, Vec<Message>>>;

/// Shared hub connecting in-process endpoints; the deterministic counterpart
/// of the TCP transport (same trait, no sockets, no threads).
#[derive(Default)]
pub struct InProcHub {
    inboxes: Inboxes,
}

impl InProcHub {
    pub fn new() -> Arc<InProcHub> {
        Arc::new(InProcHub::default())
    }

    pub fn endpoint(self: &Arc<Self>, me: NodeId) -> InProcEndpoint {
        self.inboxes
            .lock()
            .expect("hub poisoned")
            .entry(me.0)
            .or_default();
        InProcEndpoint {
            hub: Arc::clone(self),
            me,
        }
    }
}

/// One node's handle on an [`InProcHub`].
pub struct InProcEndpoint {
    hub: Arc<InProcHub>,
    me: NodeId,
}

impl RaftTransport for InProcEndpoint {
    fn send(&self, to: NodeId, msg: Message) {
        let mut inboxes = self.hub.inboxes.lock().expect("hub poisoned");
        if let Some(inbox) = inboxes.get_mut(&to.0) {
            inbox.push(msg);
        } // unknown/dead peer: drop, like the network would
    }

    fn drain(&self) -> Vec<Message> {
        let mut inboxes = self.hub.inboxes.lock().expect("hub poisoned");
        inboxes
            .get_mut(&self.me.0)
            .map(std::mem::take)
            .unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// TCP transport (real processes).
// ---------------------------------------------------------------------------

/// Framed-TCP transport: one listener for inbound frames (raft messages feed
/// the inbox; discovery requests are answered inline from [`DiscoveryState`]),
/// lazily-connected outbound streams per peer (dropped on any error; the next
/// send reconnects).
pub struct TcpTransport {
    me: NodeId,
    /// node id → address; grows via [`Self::register_peer`] as discovery
    /// resolves seed ADDRESSES into node IDS (three fresh processes must all
    /// listen before any can know its peers' ids — so the map cannot be
    /// required up front).
    peers: std::sync::RwLock<HashMap<u64, SocketAddr>>,
    inbox: Arc<Mutex<Vec<Message>>>,
    conns: Mutex<HashMap<u64, TcpStream>>,
    stop: Arc<AtomicBool>,
    local_addr: SocketAddr,
}

impl TcpTransport {
    /// Bind `addr` and start the listener. `peers` maps node id → address for
    /// outbound sends. `discovery` answers inbound discovery requests.
    pub fn bind(
        me: NodeId,
        addr: SocketAddr,
        peers: HashMap<u64, SocketAddr>,
        discovery: Arc<dyn DiscoveryState>,
    ) -> Result<Arc<TcpTransport>> {
        let listener =
            TcpListener::bind(addr).map_err(|e| Error::Raft(format!("bind {addr}: {e}")))?;
        let local_addr = listener
            .local_addr()
            .map_err(|e| Error::Raft(format!("local_addr: {e}")))?;
        let inbox: Arc<Mutex<Vec<Message>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        {
            let inbox = Arc::clone(&inbox);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                // The listener thread exits when `stop` is set and one more
                // connection attempt arrives (or the process ends; accept has
                // no portable timeout and this is Phase-1 scaffolding).
                for conn in listener.incoming() {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let Ok(stream) = conn else { continue };
                    let inbox = Arc::clone(&inbox);
                    let discovery = Arc::clone(&discovery);
                    let stop = Arc::clone(&stop);
                    std::thread::spawn(move || serve_conn(stream, inbox, discovery, stop));
                }
            });
        }
        Ok(Arc::new(TcpTransport {
            me,
            peers: std::sync::RwLock::new(peers),
            inbox,
            conns: Mutex::new(HashMap::new()),
            stop,
            local_addr,
        }))
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Add/replace a peer's address (typically after a discovery response
    /// resolved a seed address into a node id).
    pub fn register_peer(&self, id: NodeId, addr: SocketAddr) {
        self.peers
            .write()
            .expect("peers poisoned")
            .insert(id.0, addr);
    }

    /// The node this transport answers as.
    pub fn me(&self) -> NodeId {
        self.me
    }

    /// Stop accepting/reading. Existing outbound connections are dropped.
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.conns.lock().expect("conns poisoned").clear();
        // Nudge the listener out of accept().
        let _ = TcpStream::connect(self.local_addr);
    }

    /// One-shot discovery call to `addr` (fencing rule a): returns the peer's
    /// positive answer `(node, initialized, its voter-set fingerprint)`, or a
    /// typed error on connect failure/timeout/bad frame. Silence is an `Err`,
    /// never an answer. The caller counts the answer ONLY if the returned
    /// fingerprint equals its own declared one — an answer endorsing a
    /// different declaration is a misconfiguration, not a vote.
    pub fn discover(
        from: NodeId,
        voter_fp: u64,
        addr: SocketAddr,
        timeout: Duration,
    ) -> Result<(NodeId, bool, u64)> {
        let mut stream = TcpStream::connect_timeout(&addr, timeout)
            .map_err(|e| Error::Raft(format!("discovery connect {addr}: {e}")))?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|e| Error::Raft(format!("set timeout: {e}")))?;
        stream
            .write_all(&encode_frame(&Frame::DiscoveryReq { from, voter_fp }))
            .map_err(|e| Error::Raft(format!("discovery send: {e}")))?;
        match read_frame(&mut stream)? {
            Frame::DiscoveryResp {
                node,
                initialized,
                voter_fp,
            } => Ok((node, initialized, voter_fp)),
            other => Err(Error::Raft(format!(
                "unexpected discovery reply frame: {other:?}"
            ))),
        }
    }
}

fn serve_conn(
    mut stream: TcpStream,
    inbox: Arc<Mutex<Vec<Message>>>,
    discovery: Arc<dyn DiscoveryState>,
    stop: Arc<AtomicBool>,
) {
    while !stop.load(Ordering::Relaxed) {
        match read_frame(&mut stream) {
            Ok(Frame::Raft(bytes)) => {
                // A malformed protobuf payload poisons only this message.
                match Message::parse_from_bytes(&bytes) {
                    Ok(msg) => inbox.lock().expect("inbox poisoned").push(msg),
                    Err(_) => return, // corrupt payload: kill the connection
                }
            }
            Ok(Frame::DiscoveryReq { .. }) => {
                let (node, initialized, voter_fp) = discovery.answer();
                if stream
                    .write_all(&encode_frame(&Frame::DiscoveryResp {
                        node,
                        initialized,
                        voter_fp,
                    }))
                    .is_err()
                {
                    return;
                }
            }
            Ok(Frame::DiscoveryResp { .. }) => return, // unsolicited: protocol error
            Err(_) => return,                          // framing error or EOF: connection is done
        }
    }
}

impl RaftTransport for TcpTransport {
    fn send(&self, to: NodeId, msg: Message) {
        let addr = {
            let peers = self.peers.read().expect("peers poisoned");
            let Some(&addr) = peers.get(&to.0) else {
                return; // unknown peer: drop (raft retransmits after registration)
            };
            addr
        };
        let Ok(bytes) = msg.write_to_bytes() else {
            return;
        };
        let frame = encode_frame(&Frame::Raft(bytes));
        let mut conns = self.conns.lock().expect("conns poisoned");
        // Get-or-connect; on any write error drop the stream (reconnect next send).
        if let std::collections::hash_map::Entry::Vacant(slot) = conns.entry(to.0) {
            match TcpStream::connect_timeout(&addr, Duration::from_millis(200)) {
                Ok(s) => {
                    slot.insert(s);
                }
                Err(_) => return, // peer down: best-effort drop
            }
        }
        if let Some(stream) = conns.get_mut(&to.0) {
            if stream.write_all(&frame).is_err() {
                conns.remove(&to.0);
            }
        }
    }

    fn drain(&self) -> Vec<Message> {
        std::mem::take(&mut self.inbox.lock().expect("inbox poisoned"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::NodeDriver;
    use crate::rawnode::RaftPeer;
    use crate::{Command, MemStateMachine, RaftGroup, Role};
    use kv9_common::RegionId;
    use std::io::Cursor;

    #[test]
    fn frame_roundtrip_all_kinds() {
        for f in [
            Frame::Raft(vec![1, 2, 3]),
            Frame::Raft(Vec::new()),
            Frame::DiscoveryReq {
                from: NodeId(7),
                voter_fp: 0xDEAD_BEEF,
            },
            Frame::DiscoveryResp {
                node: NodeId(9),
                initialized: true,
                voter_fp: u64::MAX,
            },
            Frame::DiscoveryResp {
                node: NodeId(0),
                initialized: false,
                voter_fp: 0,
            },
        ] {
            let bytes = encode_frame(&f);
            let decoded = read_frame(&mut Cursor::new(bytes)).unwrap();
            assert_eq!(decoded, f);
        }
    }

    /// Basic negative coverage (Ren owns the thorough suite): bad magic, bad
    /// version, unknown kind, oversized length, truncation, lying length.
    #[test]
    fn frame_decode_rejects_bad_input() {
        let good = encode_frame(&Frame::DiscoveryReq {
            from: NodeId(1),
            voter_fp: 1,
        });
        // Bad magic.
        let mut b = good.clone();
        b[0] = 0xFF;
        assert!(read_frame(&mut Cursor::new(b)).is_err());
        // Unknown version.
        let mut b = good.clone();
        b[2] = FRAME_VERSION + 1;
        assert!(read_frame(&mut Cursor::new(b)).is_err());
        // Unknown kind (checksummed sanity is Ren's layer; framing rejects).
        let mut b = good.clone();
        b[3] = 0xEE;
        assert!(read_frame(&mut Cursor::new(b)).is_err());
        // Oversized declared length.
        let mut b = good.clone();
        b[4..8].copy_from_slice(&(MAX_FRAME_LEN + 1).to_be_bytes());
        assert!(read_frame(&mut Cursor::new(b)).is_err());
        // Truncation at every prefix.
        for cut in 0..good.len() {
            assert!(read_frame(&mut Cursor::new(good[..cut].to_vec())).is_err());
        }
        // Lying length: header claims more payload than the stream carries.
        let mut b = good.clone();
        b[4..8].copy_from_slice(&64u32.to_be_bytes());
        assert!(read_frame(&mut Cursor::new(b)).is_err());
        // Fixed-size kinds reject any other length BEFORE allocation — a
        // "16 MiB discovery request" is invalid by definition.
        let mut b = good.clone();
        b[4..8].copy_from_slice(&MAX_FRAME_LEN.to_be_bytes());
        assert!(read_frame(&mut Cursor::new(b)).is_err());
    }

    /// The initialized byte feeds bootstrap fencing: 0 and 1 only; a garbled
    /// byte is an error, never an answer in either direction.
    #[test]
    fn initialized_byte_rejects_non_boolean() {
        let mut bytes = encode_frame(&Frame::DiscoveryResp {
            node: NodeId(3),
            initialized: true,
            voter_fp: 5,
        });
        // The initialized byte sits before the trailing 8-byte fingerprint.
        let pos = bytes.len() - 9;
        bytes[pos] = 0x37;
        assert!(read_frame(&mut Cursor::new(bytes.clone())).is_err());
        bytes[pos] = 1;
        assert!(matches!(
            read_frame(&mut Cursor::new(bytes)).unwrap(),
            Frame::DiscoveryResp {
                initialized: true,
                ..
            }
        ));
    }

    struct StaticDiscovery(NodeId, bool, u64);
    impl DiscoveryState for StaticDiscovery {
        fn answer(&self) -> (NodeId, bool, u64) {
            (self.0, self.1, self.2)
        }
    }

    /// The fingerprint is order-independent over the declaration and sensitive
    /// to any change in it (a different member OR a different address).
    #[test]
    fn voter_fingerprint_binds_the_declaration() {
        let a1: SocketAddr = "127.0.0.1:1001".parse().unwrap();
        let a2: SocketAddr = "127.0.0.1:1002".parse().unwrap();
        let a3: SocketAddr = "127.0.0.1:1003".parse().unwrap();
        let fp = voter_set_fingerprint(&[(1, a1), (2, a2), (3, a3)]);
        // Order-independent: same declaration, any order.
        assert_eq!(fp, voter_set_fingerprint(&[(3, a3), (1, a1), (2, a2)]));
        // Different member set → different fingerprint (Tess's {1,2,9} case).
        assert_ne!(fp, voter_set_fingerprint(&[(1, a1), (2, a2), (9, a3)]));
        // Same ids, different address → different fingerprint.
        assert_ne!(fp, voter_set_fingerprint(&[(1, a1), (2, a2), (3, a1)]));
    }

    fn free_addr() -> SocketAddr {
        // Bind port 0 to reserve an ephemeral port, then release it.
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    }

    /// The full Phase 1-final library path over REAL sockets: three peers on
    /// localhost TCP elect a leader, replicate a command, every node applies
    /// it (verified by term+index), and discovery answers quorum questions.
    /// Condition-based deadline waits only — no fixed sleeps as proof.
    #[test]
    fn three_nodes_over_real_tcp_elect_replicate_and_discover() {
        let region = RegionId(1);
        let ids = [NodeId(1), NodeId(2), NodeId(3)];
        let addrs: Vec<SocketAddr> = ids.iter().map(|_| free_addr()).collect();
        let peers_map = |_me: u64| -> HashMap<u64, SocketAddr> {
            ids.iter().zip(&addrs).map(|(n, a)| (n.0, *a)).collect()
        };

        let mut drivers = Vec::new();
        let mut transports = Vec::new();
        for (i, &id) in ids.iter().enumerate() {
            let transport = TcpTransport::bind(
                id,
                addrs[i],
                peers_map(id.0),
                Arc::new(StaticDiscovery(id, false, 42)),
            )
            .unwrap();
            let peer = Arc::new(RaftPeer::new(id, region, &ids).unwrap());
            let driver = NodeDriver::new(
                peer,
                transport.clone() as Arc<dyn RaftTransport>,
                MemStateMachine::new(),
            );
            transports.push(transport);
            drivers.push(driver);
        }

        // Discovery over the wire: each seed answers positively; an unbound
        // address is silence — an Err, never an answer.
        let (node, initialized, fp) =
            TcpTransport::discover(NodeId(9), 42, addrs[0], Duration::from_millis(500)).unwrap();
        assert_eq!(node, NodeId(1));
        assert!(!initialized);
        assert_eq!(fp, 42, "the answer must echo the responder's declaration");
        assert!(
            TcpTransport::discover(NodeId(9), 42, free_addr(), Duration::from_millis(200)).is_err()
        );

        // Run every driver on its production cadence (background pump threads);
        // the test only observes status — the same shape the server uses.
        let handles: Vec<_> = drivers
            .iter()
            .map(|d| d.spawn(Duration::from_millis(10)))
            .collect();

        // Elect: campaign node 1, wait (condition-based) for a leader.
        drivers[0].peer().campaign().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        let leader_idx = loop {
            if let Some(i) = drivers.iter().position(|d| d.status().role == Role::Leader) {
                break i;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "no leader within deadline"
            );
            std::thread::sleep(Duration::from_millis(5));
        };

        // Replicate a command; every node must apply the exact (term, index).
        let cmd = Command::Put {
            cf: 0,
            key: b"wire".to_vec(),
            value: b"works".to_vec(),
        };
        let at = drivers[leader_idx].propose(&cmd).unwrap();
        for d in &drivers {
            assert!(
                d.wait_applied(at, Duration::from_secs(15)).unwrap(),
                "proposal must apply verbatim on every node"
            );
            assert_eq!(
                d.get(kv9_engine::ColumnFamily::Default, b"wire").unwrap(),
                Some(b"works".to_vec())
            );
        }

        // Status surface agrees across the cluster.
        let leader_id = drivers[leader_idx].status().node_id;
        for d in &drivers {
            assert_eq!(d.status().leader_id, Some(leader_id));
        }

        for d in &drivers {
            d.stop();
        }
        for t in &transports {
            t.shutdown();
        }
        for h in handles {
            let _ = h.join();
        }
    }
}
