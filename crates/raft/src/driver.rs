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
    /// A fatal apply-path failure (undecodable committed entry / engine apply
    /// error). Once set, the pump has stopped: continuing past a hole would
    /// silently diverge this replica from the group. The server surfaces this
    /// and exits non-zero.
    pub fatal: Option<String>,
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
    /// ONLY successfully applied entries enter this ring — a failed decode or
    /// apply must never be reported as success by `wait_applied`.
    applied: Mutex<Vec<(u64, u64)>>,
    /// First fatal apply-path error; poisons the driver (pump stops).
    fatal: Mutex<Option<String>>,
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
            fatal: Mutex::new(None),
            stop: AtomicBool::new(false),
        })
    }

    pub fn peer(&self) -> &Arc<RaftPeer<S>> {
        &self.peer
    }

    /// One pump iteration WITHOUT a tick: deliver inbound, persist+send
    /// outbound, apply committed entries.
    ///
    /// A committed entry that fails to decode or apply is **fatal**: every
    /// replica must apply the same committed sequence, so skipping one and
    /// continuing would silently diverge this node from the group. On error
    /// the driver poisons itself (pump stops, `status().fatal` set) and the
    /// failed entry never enters the success-correlation ring.
    pub fn step(&self) -> Result<()> {
        if let Some(f) = self.fatal.lock().expect("fatal poisoned").as_ref() {
            return Err(Error::Raft(f.clone()));
        }
        for msg in self.transport.drain() {
            self.peer.step_message(msg);
        }
        for msg in self.peer.pump() {
            let to = NodeId(msg.to);
            self.transport.send(to, msg);
        }
        let entries = self.peer.take_ready().unwrap_or_default();
        if entries.is_empty() {
            return Ok(());
        }
        let mut sm = self.sm.lock().expect("sm poisoned");
        let mut applied = self.applied.lock().expect("applied poisoned");
        for entry in entries {
            let outcome = Command::decode(&entry.data)
                .and_then(|cmd| sm.apply_command(entry.index, &cmd));
            if let Err(e) = outcome {
                let msg = format!(
                    "fatal at committed entry (term {}, index {}): {e}",
                    entry.term, entry.index.0
                );
                *self.fatal.lock().expect("fatal poisoned") = Some(msg.clone());
                self.stop.store(true, Ordering::Relaxed);
                return Err(Error::Raft(msg));
            }
            applied.push((entry.index.0, entry.term));
            let len = applied.len();
            if len > APPLIED_RING {
                applied.drain(..len - APPLIED_RING);
            }
        }
        Ok(())
    }

    /// Tick + step (the driver loop body).
    pub fn tick_and_step(&self) -> Result<()> {
        self.peer.tick_once();
        self.step()
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
            if let Some(f) = self.fatal.lock().expect("fatal poisoned").as_ref() {
                return Err(Error::Raft(format!("driver is poisoned: {f}")));
            }
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
            fatal: self.fatal.lock().expect("fatal poisoned").clone(),
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
                if driver.tick_and_step().is_err() {
                    break; // poisoned: fatal is recorded, status carries it
                }
                std::thread::sleep(tick_every);
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rawnode::RaftPeer;
    use crate::transport::{InProcHub, RaftTransport};
    use crate::RaftGroup;
    use kv9_common::{NodeId, RegionId};
    use kv9_engine::{ColumnFamily, Mutation, ReadView, ScanEntry, WriteBatch};

    fn single_node_driver() -> Arc<NodeDriver> {
        let hub = InProcHub::new();
        let peer = Arc::new(RaftPeer::new(NodeId(1), RegionId(1), &[NodeId(1)]).unwrap());
        let endpoint = hub.endpoint(NodeId(1));
        let driver = NodeDriver::new(
            peer,
            Arc::new(endpoint) as Arc<dyn RaftTransport>,
            MemStateMachine::new(),
        );
        driver.peer().campaign().unwrap();
        for _ in 0..50 {
            driver.tick_and_step().unwrap();
            if driver.status().role == Role::Leader {
                break;
            }
        }
        assert_eq!(driver.status().role, Role::Leader);
        driver
    }

    /// Negative 1 (Tess's review): an UNDECODABLE committed entry is fatal —
    /// it must never enter the success ring, wait_applied must error (not
    /// report success), the pump stops, and status carries the poison.
    #[test]
    fn undecodable_committed_entry_poisons_the_driver() {
        let driver = single_node_driver();
        // Propose raw garbage bytes directly (below the Command layer).
        let at = driver.peer().propose_traced(vec![0xFF, 0xEE, 0xDD]).unwrap();
        let mut poisoned = false;
        for _ in 0..50 {
            if driver.tick_and_step().is_err() {
                poisoned = true;
                break;
            }
        }
        assert!(poisoned, "a bad committed entry must fail the pump");
        let status = driver.status();
        assert!(status.fatal.is_some(), "status must surface the poison");
        // The failed entry is NOT reported as success.
        assert!(driver.wait_applied(at, Duration::from_millis(50)).is_err());
        // The watermark did not advance over the hole.
        assert!(status.applied_index < at.index.0);
    }

    /// An engine whose writes always fail (apply-path fault injection).
    struct FailingEngine;
    impl kv9_engine::Engine for FailingEngine {
        fn get(&self, _: ColumnFamily, _: &[u8]) -> kv9_common::Result<Option<Vec<u8>>> {
            Ok(None) // watermark recovery read: pretend empty
        }
        fn write(&self, _: WriteBatch) -> kv9_common::Result<()> {
            Err(Error::Engine("injected write failure".into()))
        }
        fn scan(
            &self,
            _: ColumnFamily,
            _: &[u8],
            _: &[u8],
            _: usize,
        ) -> kv9_common::Result<Vec<ScanEntry>> {
            Ok(Vec::new())
        }
        fn delete_range(&self, _: ColumnFamily, _: &[u8], _: &[u8]) -> kv9_common::Result<()> {
            Err(Error::Engine("injected".into()))
        }
        fn checksum(&self, _: ColumnFamily, _: &[u8], _: &[u8]) -> kv9_common::Result<u64> {
            Ok(0)
        }
        fn snapshot(&self) -> kv9_common::Result<Box<dyn ReadView + '_>> {
            Err(Error::Engine("injected".into()))
        }
    }
    // Silence unused-variant lint on Mutation import in some cfgs.
    const _: Option<Mutation> = None;

    /// Negative 2 (Tess's review): a VALID command whose engine apply fails is
    /// equally fatal — decode success must not mask apply failure.
    #[test]
    fn failed_apply_poisons_the_driver() {
        let hub = InProcHub::new();
        let peer = Arc::new(RaftPeer::new(NodeId(1), RegionId(1), &[NodeId(1)]).unwrap());
        let endpoint = hub.endpoint(NodeId(1));
        let driver = NodeDriver::new(
            peer,
            Arc::new(endpoint) as Arc<dyn RaftTransport>,
            MemStateMachine::with_engine(Arc::new(FailingEngine)).unwrap(),
        );
        driver.peer().campaign().unwrap();
        for _ in 0..50 {
            if driver.tick_and_step().is_err() {
                break;
            }
            if driver.status().role == Role::Leader {
                break;
            }
        }
        let at = driver
            .propose(&Command::Put {
                cf: 0,
                key: b"k".to_vec(),
                value: b"v".to_vec(),
            })
            .unwrap();
        let mut poisoned = false;
        for _ in 0..50 {
            if driver.tick_and_step().is_err() {
                poisoned = true;
                break;
            }
        }
        assert!(poisoned, "an apply failure must fail the pump");
        assert!(driver.status().fatal.is_some());
        assert!(driver.wait_applied(at, Duration::from_millis(50)).is_err());
    }

    /// Sensitivity control: on a HEALTHY driver the same shapes succeed — the
    /// poison path is reachable only through real failures, not always-on.
    #[test]
    fn control_healthy_driver_applies_and_reports_success() {
        let driver = single_node_driver();
        let at = driver
            .propose(&Command::Put {
                cf: 0,
                key: b"ok".to_vec(),
                value: b"yes".to_vec(),
            })
            .unwrap();
        for _ in 0..50 {
            driver.tick_and_step().unwrap();
            if driver.status().applied_index >= at.index.0 {
                break;
            }
        }
        assert!(driver.wait_applied(at, Duration::from_millis(100)).unwrap());
        assert!(driver.status().fatal.is_none());
    }
}
