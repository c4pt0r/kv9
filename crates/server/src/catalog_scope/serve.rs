//! The full-node side of a scope stream (docs/META-REPLICA-MODES.md §3b).
//!
//! One thread per subscriber. It snapshots the subscriber's scope at one position
//! (the ring's lock pins the engine view and the position together), then follows
//! the ring: every op of every entry is routed against the subscriber's closure.
//! Ops are coalesced into one delta naming the position it applies on; positions
//! with nothing for the subscriber are sent as an idle delta every
//! `KV9_SCOPE_IDLE_MS`. An op that widens the closure, a full-set change or a
//! position the ring no longer holds sends a fresh snapshot instead. An undeclared
//! key ends the stream with an error (M3).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kv9_common::{AppliedPosition, Error, Result};
use kv9_engine::ReplicatedEngine;
use kv9_meta::scope::{self, Closure, Resolver, Route};
use kv9_raft::grpc::catalog_scope::{FrameReceiver, KIND_DELTA, KIND_SNAPSHOT_END, KIND_SNAPSHOT_PART, MAX_FRAME_BYTES};
use kv9_raft::grpc::pb;
use kv9_raft::{cf_code, KvOp};
use tonic::Status;

use super::ring::ChangeRing;

/// The full nodes a subscriber may use, with addresses, and the leader hint.
pub trait FullSet: Send + Sync + 'static {
    /// `(node, address)` of every metadata voter and full learner, sorted by node.
    fn servers(&self) -> Vec<(u64, String)>;
    fn leader(&self) -> u64;
}

/// What a full node has served, for its status document, and its snapshot limiter.
#[derive(Default)]
pub struct ServeStats {
    pub streams: std::sync::atomic::AtomicU64,
    pub frames: std::sync::atomic::AtomicU64,
    pub bytes: std::sync::atomic::AtomicU64,
    pub snapshots: std::sync::atomic::AtomicU64,
    /// Snapshots being computed now, and the most ever at once.
    pub snapshots_active: std::sync::Mutex<u64>,
    pub snapshots_peak: std::sync::atomic::AtomicU64,
    pub snapshots_waited: std::sync::atomic::AtomicU64,
    snapshot_turn: std::sync::Condvar,
    /// Per live subscriber: the index last sent and when (unix ms). What `cluster health
    /// --full-only` reads instead of asking every scoped node (R7).
    pub subscribers: std::sync::Mutex<std::collections::BTreeMap<u64, (u64, u64)>>,
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// How many scope snapshots one full node computes at once (R4); more wait their turn.
pub fn snapshot_parallel() -> u64 {
    std::env::var("KV9_SCOPE_SNAPSHOT_PARALLEL")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(4)
}

struct SnapshotPermit<'a>(&'a ServeStats);

impl ServeStats {
    fn snapshot_permit(&self) -> SnapshotPermit<'_> {
        use std::sync::atomic::Ordering::Relaxed;
        let limit = snapshot_parallel();
        let mut active = self.snapshots_active.lock().expect("snapshot limiter poisoned");
        if *active >= limit {
            self.snapshots_waited.fetch_add(1, Relaxed);
        }
        while *active >= limit {
            active = self.snapshot_turn.wait(active).expect("snapshot limiter poisoned");
        }
        *active += 1;
        self.snapshots_peak.fetch_max(*active, Relaxed);
        SnapshotPermit(self)
    }
}

impl Drop for SnapshotPermit<'_> {
    fn drop(&mut self) {
        *self.0.snapshots_active.lock().expect("snapshot limiter poisoned") -= 1;
        self.0.snapshot_turn.notify_one();
    }
}

pub fn idle_interval() -> Duration {
    Duration::from_millis(
        std::env::var("KV9_SCOPE_IDLE_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(1000),
    )
}

struct Subscriber<E: ReplicatedEngine> {
    engine: Arc<E>,
    ring: Arc<ChangeRing>,
    full: Arc<dyn FullSet>,
    node: u64,
    tx: tokio::sync::mpsc::Sender<std::result::Result<pb::CatalogScopeMessage, Status>>,
    stats: Arc<ServeStats>,
    /// What the subscriber holds: the last position sent and the keys it has.
    sent: Option<AppliedPosition>,
    /// How far the ring has been routed into `pending`.
    routed: Option<AppliedPosition>,
    pending: Vec<pb::ScopeOp>,
    pending_bytes: usize,
    held: BTreeSet<Vec<u8>>,
    closure: Option<Closure>,
    servers: Vec<(u64, String)>,
}

fn put(key: &[u8], value: &[u8]) -> pb::ScopeOp {
    pb::ScopeOp { cf: 0, key: key.to_vec(), value: value.to_vec(), delete: false }
}
fn delete(key: &[u8]) -> pb::ScopeOp {
    pb::ScopeOp { cf: 0, key: key.to_vec(), value: Vec::new(), delete: true }
}
fn op_size(op: &pb::ScopeOp) -> usize {
    op.key.len() + op.value.len() + 8
}

enum Next {
    Continue,
    Snapshot,
}

impl<E: ReplicatedEngine + 'static> Subscriber<E> {
    fn send(&self, message: pb::CatalogScopeMessage) -> Result<()> {
        use std::sync::atomic::Ordering::Relaxed;
        self.stats.frames.fetch_add(1, Relaxed);
        self.stats
            .bytes
            .fetch_add(message.ops.iter().map(|op| (op.key.len() + op.value.len()) as u64).sum(), Relaxed);
        if message.kind == KIND_SNAPSHOT_END {
            self.stats.snapshots.fetch_add(1, Relaxed);
        }
        if message.kind != KIND_SNAPSHOT_PART {
            self.stats
                .subscribers
                .lock()
                .expect("subscriber table poisoned")
                .insert(self.node, (message.index, unix_ms()));
        }
        self.tx
            .blocking_send(Ok(message))
            .map_err(|_| Error::MetaNotReady("catalog scope subscriber left".into()))
    }

    fn fail(&self, error: &Error) {
        let _ = self.tx.blocking_send(Err(Status::failed_precondition(error.to_string())));
    }

    fn servers_message(&self) -> Vec<pb::ScopeServer> {
        self.servers
            .iter()
            .map(|(node_id, addr)| pb::ScopeServer { node_id: *node_id, addr: addr.clone() })
            .collect()
    }

    fn snapshot(&mut self) -> Result<()> {
        let stats = self.stats.clone();
        let _permit = stats.snapshot_permit();
        self.servers = self.full.servers();
        let full: Vec<u64> = self.servers.iter().map(|(n, _)| *n).collect();
        let engine = self.engine.clone();
        let frozen = self
            .ring
            .frozen(|at| engine.snapshot().map(|view| (view, at)))
            .ok_or_else(|| Error::MetaNotReady("change ring has no position yet".into()))??;
        let (view, at) = frozen;
        let closure = scope::closure(view.as_ref(), self.node, &full)?;
        let rows = scope::snapshot_rows(view.as_ref(), &closure)?;
        drop(view);
        let mut part = Vec::new();
        let mut bytes = 0;
        let mut held = BTreeSet::new();
        for (key, value) in rows {
            let op = put(&key, &value);
            held.insert(key);
            if bytes + op_size(&op) > MAX_FRAME_BYTES && !part.is_empty() {
                self.send(pb::CatalogScopeMessage {
                    kind: KIND_SNAPSHOT_PART,
                    ops: std::mem::take(&mut part),
                    ..Default::default()
                })?;
                bytes = 0;
            }
            bytes += op_size(&op);
            part.push(op);
        }
        self.send(pb::CatalogScopeMessage {
            kind: KIND_SNAPSHOT_END,
            term: at.term,
            index: at.index,
            ops: part,
            regions: closure.regions.iter().copied().collect(),
            servers: self.servers_message(),
            leader: self.full.leader(),
            ..Default::default()
        })?;
        self.held = held;
        self.closure = Some(closure);
        self.sent = Some(at);
        self.routed = Some(at);
        Ok(())
    }

    /// Route the ring's entries after `routed` into the pending delta, and send it
    /// when it holds ops or the idle interval is due.
    fn follow(&mut self, idle_due: bool) -> Result<Next> {
        let (Some(sent), Some(routed)) = (self.sent, self.routed) else { return Ok(Next::Snapshot) };
        if self.full.servers() != self.servers {
            return Ok(Next::Snapshot);
        }
        let entries = match self.ring.after(routed, 512) {
            Ok(entries) => entries,
            Err(super::ring::Behind::Snapshot) => return Ok(Next::Snapshot),
        };
        if !entries.is_empty() {
            let closure = self.closure.clone().expect("a sent position has a closure");
            let engine = self.engine.clone();
            let view = engine.snapshot()?;
            let mut resolver = Resolver::new(view.as_ref());
            for entry in &entries {
                for op in entry.ops.iter() {
                    let (cf, key, value) = match op {
                        KvOp::Put { cf, key, value } => (*cf, key, Some(value.as_slice())),
                        KvOp::Delete { cf, key } => (*cf, key, None),
                    };
                    if cf != cf_code(kv9_engine::ColumnFamily::Default) {
                        return Err(Error::MalformedKey(
                            "catalog scope: a catalog op outside the default column family".into(),
                        ));
                    }
                    match scope::route(&closure, &mut resolver, key, value)? {
                        Route::Deliver => match value {
                            Some(v) => {
                                self.held.insert(key.clone());
                                self.pending_bytes += key.len() + v.len() + 8;
                                self.pending.push(put(key, v));
                            }
                            None => {
                                if self.held.remove(key) {
                                    self.pending_bytes += key.len() + 8;
                                    self.pending.push(delete(key));
                                }
                            }
                        },
                        // A row that left the scope, or a delete of a row whose subject
                        // went with its value, is removed where it is held.
                        Route::Skip | Route::IfHeld => {
                            if self.held.remove(key) {
                                self.pending_bytes += key.len() + 8;
                                self.pending.push(delete(key));
                            }
                        }
                        Route::Resnapshot => return Ok(Next::Snapshot),
                        Route::Undeclared => {
                            return Err(Error::MalformedKey(format!(
                                "catalog scope: undeclared metadata key at index {}",
                                entry.at.index
                            )))
                        }
                    }
                }
                self.routed = Some(entry.at);
                if self.pending_bytes > MAX_FRAME_BYTES {
                    break;
                }
            }
        }
        if !self.pending.is_empty() || idle_due {
            let at = self.routed.expect("routed is set with sent");
            let ops = std::mem::take(&mut self.pending);
            self.pending_bytes = 0;
            self.send(pb::CatalogScopeMessage {
                kind: KIND_DELTA,
                prev_term: sent.term,
                prev_index: sent.index,
                term: at.term,
                index: at.index,
                ops,
                leader: self.full.leader(),
                ..Default::default()
            })?;
            self.sent = Some(at);
        }
        Ok(Next::Continue)
    }

    fn run(&mut self) {
        let idle = idle_interval();
        let mut last_send = Instant::now();
        let mut snapshot_due = true;
        loop {
            let outcome = if snapshot_due {
                self.pending.clear();
                self.pending_bytes = 0;
                self.snapshot().map(|()| Next::Continue)
            } else {
                let due = last_send.elapsed() >= idle;
                let before = self.sent;
                let outcome = self.follow(due);
                if self.sent != before || due {
                    last_send = Instant::now();
                }
                outcome
            };
            match outcome {
                Ok(Next::Continue) => snapshot_due = false,
                Ok(Next::Snapshot) => {
                    snapshot_due = true;
                    continue;
                }
                Err(Error::MetaNotReady(_)) if self.tx.is_closed() => return,
                Err(error) => {
                    self.fail(&error);
                    return;
                }
            }
            if self.tx.is_closed() {
                return;
            }
            let wait = idle.saturating_sub(last_send.elapsed()).max(Duration::from_millis(1));
            self.ring.wait_past(self.routed, wait);
        }
    }
}

/// Start serving `node`'s scope after `from` (unused beyond validation: a caller
/// position is honoured only when the ring holds it, otherwise a snapshot comes first).
pub fn open<E: ReplicatedEngine + 'static>(
    engine: Arc<E>,
    ring: Arc<ChangeRing>,
    full: Arc<dyn FullSet>,
    node: u64,
    from: Option<AppliedPosition>,
    stats: Arc<ServeStats>,
) -> Result<FrameReceiver> {
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    let resumable = from.filter(|p| ring.after(*p, 1).is_ok());
    let mut subscriber = Subscriber {
        engine,
        ring,
        full,
        node,
        tx,
        stats: stats.clone(),
        sent: None,
        routed: None,
        pending: Vec::new(),
        pending_bytes: 0,
        held: BTreeSet::new(),
        closure: None,
        servers: Vec::new(),
    };
    // Resuming needs the subscriber's held set and closure, which only a snapshot
    // establishes on this server; positions are kept so an unchanged slice is
    // replaced by an identical one at the same or a later position.
    let _ = resumable;
    stats.streams.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::thread::Builder::new()
        .name(format!("scope-serve-{node}"))
        .spawn(move || {
            subscriber.run();
            stats.streams.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            stats.subscribers.lock().expect("subscriber table poisoned").remove(&node);
        })
        .map_err(|e| Error::Config(format!("spawn catalog scope server: {e}")))?;
    Ok(rx)
}
