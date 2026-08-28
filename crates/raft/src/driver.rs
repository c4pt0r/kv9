//! Node runtime driver (Phase 1-final): the loop that turns a [`RaftPeer`],
//! a [`RaftTransport`] and a state machine into a running meta node.
//!
//! The server owns process residency (signals, lifecycle); this type owns the
//! pump: drain transport → step raft → persist/send via `pump()` → apply
//! committed entries → expose queryable [`NodeStatus`]. Acceptance criteria
//! read `status()` — never logs, never sleeps-as-proof.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kv9_engine::{Engine, MemEngine};
use raft::storage::MemStorage;

use kv9_common::{Error, NodeId, Result};

use crate::rawnode::{PersistentRaftStorage, ProposedAt, RaftPeer};
use crate::transport::RaftTransport;
use crate::{Command, MemStateMachine, RaftGroup, Role, StateMachine};

/// Queryable node state (the server's `status` surface, agreed seam with the
/// acceptance harness: success is judged on these fields, not on log text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeStatus {
    pub node_id: NodeId,
    pub leader_id: Option<NodeId>,
    pub role: Role,
    pub term: u64,
    pub raft_committed: u64,
    pub applied_index: u64,
}

/// How many recently applied `(index, term)` pairs are retained for proposal
/// verification (correlation is by term+index, never position alone).
const APPLIED_RING: usize = 1024;

pub struct NodeDriver<S: PersistentRaftStorage = MemStorage, E: Engine = MemEngine> {
    peer: Arc<RaftPeer<S>>,
    transport: Arc<dyn RaftTransport>,
    /// Generic over the engine so the durable `WalEngine` (or any other) sits
    /// directly in the apply downstream — the server passes a state machine
    /// sharing ONE engine instance with its MetaStore, or catalog reads and
    /// applied writes would land in different engines.
    sm: Mutex<MemStateMachine<E>>,
    /// Recently applied (index, term) pairs, for (term, index) verification.
    applied: Mutex<Vec<(u64, u64)>>,
    stop: AtomicBool,
}

impl<S: PersistentRaftStorage, E: Engine + 'static> NodeDriver<S, E> {
    pub fn new(
        peer: Arc<RaftPeer<S>>,
        transport: Arc<dyn RaftTransport>,
        sm: MemStateMachine<E>,
    ) -> Arc<NodeDriver<S, E>> {
        Arc::new(NodeDriver {
            peer,
            transport,
            sm: Mutex::new(sm),
            applied: Mutex::new(Vec::new()),
            stop: AtomicBool::new(false),
        })
    }

    pub fn peer(&self) -> &Arc<RaftPeer<S>> {
        &self.peer
    }

    /// One pump iteration WITHOUT a tick: deliver inbound, persist+send
    /// outbound, apply committed entries.
    pub fn step(&self) {
        for msg in self.transport.drain() {
            self.peer.step_message(msg);
        }
        for msg in self.peer.pump() {
            let to = NodeId(msg.to);
            self.transport.send(to, msg);
        }
        let entries = self.peer.take_ready().unwrap_or_default();
        if entries.is_empty() {
            return;
        }
        let mut sm = self.sm.lock().expect("sm poisoned");
        let mut applied = self.applied.lock().expect("applied poisoned");
        for entry in entries {
            if let Ok(cmd) = Command::decode(&entry.data) {
                let _ = sm.apply_command(entry.index, &cmd);
            }
            // Undecodable entries still advance the applied record — their
            // position is consumed either way, and correlation-by-term will
            // (correctly) fail any proposal that thought it owned the slot.
            applied.push((entry.index.0, entry.term));
            let len = applied.len();
            if len > APPLIED_RING {
                applied.drain(..len - APPLIED_RING);
            }
        }
    }

    /// Tick + step (the driver loop body).
    pub fn tick_and_step(&self) {
        self.peer.tick_once();
        self.step();
    }

    /// Propose a command on this node (must currently be leader).
    pub fn propose(&self, cmd: &Command) -> Result<ProposedAt> {
        self.peer.propose_traced(cmd.encode())
    }

    /// Wait until this node has applied `at` — verified by **term + index**,
    /// never position alone. Pure condition-poll: the pump must be running
    /// (via [`Self::spawn`] or a caller-driven loop). Returns:
    /// - `Ok(true)`  — the exact proposal applied here;
    /// - `Ok(false)` — the position was passed by a DIFFERENT entry (overwritten
    ///   after a leader change): the proposal must be reported failed;
    /// - `Err(_)`    — deadline reached. Success is judged on applied state,
    ///   never on elapsed time.
    pub fn wait_applied(&self, at: ProposedAt, deadline: Duration) -> Result<bool> {
        let start = Instant::now();
        loop {
            {
                let applied = self.applied.lock().expect("applied poisoned");
                if let Some(&(_, term)) = applied.iter().find(|(i, _)| *i == at.index.0) {
                    return Ok(term == at.term);
                }
                // Position passed without this index ever being recorded: the
                // slot was consumed by an entry that never reached the command
                // path (e.g. a no-op barrier) — not ours.
                if self.sm.lock().expect("sm poisoned").applied_index() >= at.index {
                    return Ok(false);
                }
            }
            if start.elapsed() > deadline {
                return Err(Error::Raft(format!(
                    "wait_applied deadline: index {} not reached",
                    at.index.0
                )));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// The queryable status surface.
    pub fn status(&self) -> NodeStatus {
        NodeStatus {
            node_id: self.peer.node_id(),
            leader_id: self.peer.leader_hint(),
            role: self.peer.role(),
            term: self.peer.term(),
            raft_committed: self.peer.raft_committed().0,
            applied_index: self.sm.lock().expect("sm poisoned").applied_index().0,
        }
    }

    /// Read a key from this node's applied state machine (harness verification).
    pub fn get(&self, cf: kv9_engine::ColumnFamily, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.sm.lock().expect("sm poisoned").get(cf, key)
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Background pump at `tick_every` cadence until [`Self::stop`].
    pub fn spawn(self: &Arc<Self>, tick_every: Duration) -> std::thread::JoinHandle<()> {
        let driver = Arc::clone(self);
        std::thread::spawn(move || {
            while !driver.stop.load(Ordering::Relaxed) {
                driver.tick_and_step();
                std::thread::sleep(tick_every);
            }
        })
    }
}
