//! The scoped node's side (docs/META-REPLICA-MODES.md §3b, S3).
//!
//! One thread holds one stream to one full node, chosen by rendezvous hash of
//! `(this node, server)` so subscribers spread across the full set (R4). Frames
//! apply to the local metadata engine at the stream's own positions, through the
//! same positioned write the metadata state machine uses, so every local catalog
//! reader sees a consistent slice at one position:
//!
//! - a snapshot replaces the whole slice; one older than the local position is
//!   refused and the subscriber moves to another server;
//! - a delta applies only on the exact position it names; any other position
//!   ends the stream and resubscribes from the local position (proof:
//!   `an_out_of_position_message_is_ignored`).
//!
//! A stream that is silent for three idle intervals is abandoned.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use kv9_common::{AppliedPosition, Error, NodeId, Result, RootDigest};
use kv9_engine::{ColumnFamily, DurableAppliedPosition, ReplicatedEngine, WriteBatch};
use kv9_raft::grpc::catalog_scope::{subscribe_on_channel, KIND_DELTA, KIND_SNAPSHOT_END, KIND_SNAPSHOT_PART};
use kv9_raft::grpc::pb;

/// What `status` and the registration gate read.
#[derive(Debug, Clone, Default)]
pub struct ScopeState {
    pub position: Option<AppliedPosition>,
    pub regions: Vec<u64>,
    pub servers: Vec<(u64, String)>,
    pub leader: u64,
    pub server: Option<u64>,
    pub last_frame_unix_ms: u64,
    pub snapshots: u64,
    pub deltas: u64,
    pub refused: u64,
    /// Rows the local slice holds (exact: counted at a snapshot, adjusted per op).
    pub rows: u64,
    /// Bytes of ops received (snapshots and deltas).
    pub bytes: u64,
    pub error: Option<String>,
}

pub struct Subscription {
    pub state: Arc<Mutex<ScopeState>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub struct Config {
    pub node: NodeId,
    pub store: [u8; 16],
    pub root: RootDigest,
    pub token: Option<String>,
    /// Servers to try before the first snapshot names the full set.
    pub seeds: Vec<(u64, String)>,
    /// Advanced after every applied frame.
    pub watch: Arc<super::SliceWatch>,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Order servers for `node` by rendezvous hash (highest first).
pub fn rendezvous(node: u64, servers: &[(u64, String)]) -> Vec<(u64, String)> {
    let mut scored: Vec<_> = servers
        .iter()
        .map(|(id, addr)| {
            let mut bytes = node.to_be_bytes().to_vec();
            bytes.extend(id.to_be_bytes());
            let digest = RootDigest::sha256(&bytes);
            let score = u64::from_be_bytes(digest.as_bytes()[..8].try_into().expect("8 bytes"));
            (score, (*id, addr.clone()))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().map(|(_, s)| s).collect()
}

fn local_position<E: ReplicatedEngine>(engine: &E) -> Result<Option<AppliedPosition>> {
    Ok(match engine.applied_position()? {
        DurableAppliedPosition::AppliedThrough(at) => Some(at),
        _ => None,
    })
}

/// Replace the whole local slice with `rows` at `at`. At the local position itself
/// (a re-snapshot after the full set changed) the slice is replaced without moving
/// the position.
fn install_snapshot<E: ReplicatedEngine>(
    engine: &E,
    rows: Vec<pb::ScopeOp>,
    at: AppliedPosition,
    same_position: bool,
) -> Result<()> {
    let keep: BTreeSet<&[u8]> = rows.iter().map(|op| op.key.as_slice()).collect();
    let mut batch = WriteBatch::new();
    let view = engine.snapshot()?;
    kv9_meta::scope::for_each(view.as_ref(), kv9_meta::scope::ALL_START, kv9_meta::scope::ALL_END, |key, _| {
        if !keep.contains(key) {
            batch.delete(ColumnFamily::Default, key.to_vec());
        }
        Ok(())
    })?;
    drop(view);
    for op in &rows {
        batch.put(ColumnFamily::Default, op.key.clone(), op.value.clone());
    }
    if same_position {
        engine.write(batch)
    } else {
        engine.write_applied(batch, at)
    }
}

/// Apply one delta; returns the change in held rows.
fn apply_delta<E: ReplicatedEngine>(engine: &E, ops: Vec<pb::ScopeOp>, at: AppliedPosition) -> Result<i64> {
    let mut batch = WriteBatch::new();
    let mut rows = 0i64;
    let mut seen: std::collections::BTreeMap<Vec<u8>, bool> = std::collections::BTreeMap::new();
    for op in &ops {
        let present = match seen.get(&op.key) {
            Some(p) => *p,
            None => engine.get(ColumnFamily::Default, &op.key)?.is_some(),
        };
        match (present, op.delete) {
            (false, false) => rows += 1,
            (true, true) => rows -= 1,
            _ => {}
        }
        seen.insert(op.key.clone(), !op.delete);
    }
    for op in ops {
        if op.cf != 0 {
            return Err(Error::Raft("catalog scope op outside the default column family".into()));
        }
        if op.delete {
            batch.delete(ColumnFamily::Default, op.key);
        } else {
            batch.put(ColumnFamily::Default, op.key, op.value);
        }
    }
    engine.write_applied(batch, at)?;
    Ok(rows)
}

enum End {
    /// Try the next server.
    Next,
    Stop,
}

struct Runner<E: ReplicatedEngine> {
    engine: Arc<E>,
    config: Config,
    state: Arc<Mutex<ScopeState>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    runtime: tokio::runtime::Runtime,
}

impl<E: ReplicatedEngine + 'static> Runner<E> {
    fn stopped(&self) -> bool {
        self.stop.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn note_error(&self, error: impl Into<String>) {
        let mut state = self.state.lock().expect("scope state poisoned");
        state.error = Some(error.into());
        state.server = None;
    }

    fn stream_from(&self, server: u64, addr: &str) -> End {
        let idle = super::serve::idle_interval();
        let silence = idle * 3 + Duration::from_secs(2);
        let channel = match tonic::transport::Endpoint::from_shared(format!("http://{addr}"))
            .map(|e| e.connect_timeout(Duration::from_secs(3)))
        {
            Ok(endpoint) => match self.runtime.block_on(endpoint.connect()) {
                Ok(channel) => channel,
                Err(e) => {
                    self.note_error(format!("connect node {server}: {e}"));
                    return End::Next;
                }
            },
            Err(e) => {
                self.note_error(format!("server address {addr}: {e}"));
                return End::Next;
            }
        };
        let from = match local_position(self.engine.as_ref()) {
            Ok(p) => p,
            Err(e) => {
                self.note_error(format!("local position: {e}"));
                return End::Stop;
            }
        };
        let mut stream = match self.runtime.block_on(subscribe_on_channel(
            channel,
            self.config.root,
            self.config.store,
            from,
            self.config.node,
            self.config.token.clone(),
            Duration::from_secs(5),
        )) {
            Ok(stream) => stream,
            Err(e) => {
                self.note_error(format!("subscribe at node {server}: {e}"));
                return End::Next;
            }
        };
        self.state.lock().expect("scope state poisoned").server = Some(server);
        let mut parts: Vec<pb::ScopeOp> = Vec::new();
        loop {
            if self.stopped() {
                return End::Stop;
            }
            let next = self
                .runtime
                .block_on(async { tokio::time::timeout(Duration::from_millis(500), stream.message()).await });
            let message = match next {
                Err(_) => {
                    let last = self.state.lock().expect("scope state poisoned").last_frame_unix_ms;
                    if now_ms().saturating_sub(last) > silence.as_millis() as u64 {
                        self.note_error(format!("node {server} went silent"));
                        return End::Next;
                    }
                    continue;
                }
                Ok(Ok(Some(message))) => message,
                Ok(Ok(None)) => {
                    self.note_error(format!("node {server} ended the stream"));
                    return End::Next;
                }
                Ok(Err(status)) => {
                    self.note_error(format!("node {server}: {}", status.message()));
                    return End::Next;
                }
            };
            if let Err(error) = self.apply(message, &mut parts) {
                self.note_error(error.to_string());
                return End::Next;
            }
        }
    }

    fn apply(&self, m: pb::CatalogScopeMessage, parts: &mut Vec<pb::ScopeOp>) -> Result<()> {
        let at = AppliedPosition { term: m.term, index: m.index };
        let local = local_position(self.engine.as_ref())?;
        match m.kind {
            KIND_SNAPSHOT_PART => {
                parts.extend(m.ops);
                return Ok(());
            }
            KIND_SNAPSHOT_END => {
                let mut rows = std::mem::take(parts);
                rows.extend(m.ops);
                let count = rows.len() as u64;
                let bytes: u64 = rows.iter().map(|op| (op.key.len() + op.value.len()) as u64).sum();
                match local {
                    Some(local) if at.index < local.index => {
                        self.state.lock().expect("scope state poisoned").refused += 1;
                        return Err(Error::MetaNotReady(format!(
                            "a snapshot at index {} is older than the local slice at {}",
                            at.index, local.index
                        )));
                    }
                    Some(local) if local == at => install_snapshot(self.engine.as_ref(), rows, at, true)?,
                    _ => install_snapshot(self.engine.as_ref(), rows, at, false)?,
                }
                let mut state = self.state.lock().expect("scope state poisoned");
                state.regions = m.regions;
                if !m.servers.is_empty() {
                    state.servers = m.servers.into_iter().map(|s| (s.node_id, s.addr)).collect();
                }
                state.snapshots += 1;
                state.rows = count;
                state.bytes += bytes;
                state.position = Some(at);
                self.config.watch.advance(at);
                state.leader = m.leader;
                state.last_frame_unix_ms = now_ms();
                state.error = None;
            }
            KIND_DELTA => {
                let prev = AppliedPosition { term: m.prev_term, index: m.prev_index };
                if local != Some(prev) {
                    self.state.lock().expect("scope state poisoned").refused += 1;
                    return Err(Error::MetaNotReady(format!(
                        "a delta on index {} does not apply on the local slice at {:?}",
                        prev.index,
                        local.map(|p| p.index)
                    )));
                }
                let bytes: u64 = m.ops.iter().map(|op| (op.key.len() + op.value.len()) as u64).sum();
                let changed = if at != prev { apply_delta(self.engine.as_ref(), m.ops, at)? } else { 0 };
                let mut state = self.state.lock().expect("scope state poisoned");
                state.rows = state.rows.saturating_add_signed(changed);
                state.bytes += bytes;
                state.deltas += 1;
                state.position = Some(at);
                self.config.watch.advance(at);
                state.leader = m.leader;
                state.last_frame_unix_ms = now_ms();
                state.error = None;
            }
            other => return Err(Error::Raft(format!("unknown catalog scope frame kind {other}"))),
        }
        Ok(())
    }

    fn run(self) {
        let mut attempt: u64 = 0;
        while !self.stopped() {
            let known = self.state.lock().expect("scope state poisoned").servers.clone();
            let candidates = if known.is_empty() { self.config.seeds.clone() } else { known };
            let ordered = rendezvous(self.config.node.0, &candidates);
            let mut ended = false;
            for (server, addr) in ordered {
                if self.stopped() {
                    return;
                }
                if server == self.config.node.0 {
                    continue;
                }
                match self.stream_from(server, &addr) {
                    End::Stop => return,
                    End::Next => {
                        ended = true;
                        // Jittered pause before the next server (R4).
                        let jitter = (self.config.node.0.wrapping_mul(2654435761) ^ attempt) % 400;
                        std::thread::sleep(Duration::from_millis(100 + jitter));
                        attempt += 1;
                    }
                }
            }
            if !ended {
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }
}

/// Start the subscriber. Dropping the returned handle stops it.
pub fn start<E: ReplicatedEngine + 'static>(engine: Arc<E>, config: Config) -> Result<Subscription> {
    let position = local_position(engine.as_ref())?;
    config.watch.mark_scoped();
    if let Some(at) = position {
        config.watch.advance(at);
    }
    let state = Arc::new(Mutex::new(ScopeState {
        position,
        ..ScopeState::default()
    }));
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::Config(format!("catalog scope runtime: {e}")))?;
    let runner = Runner { engine, config, state: state.clone(), stop: stop.clone(), runtime };
    let thread = std::thread::Builder::new()
        .name("scope-subscribe".into())
        .spawn(move || runner.run())
        .map_err(|e| Error::Config(format!("spawn catalog scope subscriber: {e}")))?;
    Ok(Subscription { state, stop, thread: Some(thread) })
}

