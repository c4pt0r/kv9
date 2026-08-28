//! raft-rs adapter (ROADMAP §Dependency decisions): `RawNode`/`Ready` behind the
//! pull-model [`RaftGroup`] trait.
//!
//! Boundaries fixed by design review:
//! - raft-rs [`raft::Storage`] is the **raft-log** layer (Phase-1: `MemStorage`;
//!   later the LogStore/WalStream of DESIGN "6.4 Raft log vs. WAL stream"). It is
//!   NOT the state-machine engine — `kv9_engine::Engine` appears only downstream of
//!   apply (committed entry → `Command` → `WriteBatch`).
//! - The Ready loop **persists entries + hardstate before sending messages** — a
//!   raft safety requirement that is vacuous under `MemStorage` but structural here
//!   so the Phase-2 real log slots in without reshaping the loop.
//! - `propose` reports the **locally assigned** `(term, index)` ([`ProposedAt`]) —
//!   a position claim, not a commit promise. After a leader change the same index
//!   can be overwritten by another leader's entry, so callers correlate a committed
//!   entry by **term + index** (or payload/context), never by position alone.

use std::sync::{Arc, Mutex, MutexGuard};

use raft::prelude::{ConfState, Entry, HardState, Message};
use raft::storage::MemStorage;
use raft::{Config, RawNode, StateRole};
use slog::{o, Discard, Logger};

use kv9_common::{Error, NodeId, RegionId, Result};

use crate::{CommittedEntry, LogIndex, RaftGroup, Role};

/// A raft-rs [`raft::Storage`] that can also **persist** what the Ready loop
/// hands it: log entries and the HardState (term + vote + commit).
///
/// The raft safety contract lives here: **a vote must be durable before the
/// reply leaves the node** — the Ready loop calls `append`/`set_hardstate`
/// *before* messages are handed to the transport. `MemStorage` implements this
/// trait volatilely (in-process clusters, tests); `DiskRaftStorage` implements
/// it durably (real processes). A restarted node on volatile storage forgets
/// its vote and can vote twice in one term — two leaders — which is why the
/// cross-process path must use the durable impl.
pub trait PersistentRaftStorage: raft::Storage + Send + Sync + 'static {
    fn append(&self, entries: &[Entry]) -> Result<()>;
    fn set_hardstate(&self, hs: &HardState) -> Result<()>;
}

impl PersistentRaftStorage for MemStorage {
    fn append(&self, entries: &[Entry]) -> Result<()> {
        self.wl().append(entries).map_err(raft_err)
    }

    fn set_hardstate(&self, hs: &HardState) -> Result<()> {
        self.wl().set_hardstate(hs.clone());
        Ok(())
    }
}

/// The locally assigned position of an accepted proposal: a claim that entry
/// `(term, index)` is *this* proposal — committed only once a committed entry with
/// the **same term** appears at that index. If a later leader overwrites the index,
/// the term differs and the proposal must be reported failed, never silently
/// "reached" (observed in the selection spike: a failover's no-op barrier shifts
/// indexes; correlation by position alone returns someone else's command).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProposedAt {
    pub term: u64,
    pub index: LogIndex,
}

/// One member of a raft-rs group on one node, driven by an external pump
/// ([`InProcessCluster::round`] in Phase-1 tests/harness).
pub struct RaftPeer<S: PersistentRaftStorage = MemStorage> {
    node: NodeId,
    region: RegionId,
    inner: Mutex<PeerInner<S>>,
}

struct PeerInner<S: PersistentRaftStorage> {
    raw: RawNode<S>,
    /// Committed, non-empty entries not yet drained by [`RaftGroup::take_ready`].
    ready: Vec<CommittedEntry>,
    /// Outgoing raft messages awaiting delivery by the cluster pump.
    outbox: Vec<Message>,
    /// Harness kill switch: a dead peer neither ticks nor receives messages.
    alive: bool,
}

impl RaftPeer<MemStorage> {
    /// Build a peer for `region` on `node`, with the (fixed, Phase-1) voter set,
    /// on volatile in-memory storage (tests / in-process clusters).
    pub fn new(node: NodeId, region: RegionId, voters: &[NodeId]) -> Result<RaftPeer> {
        let ids: Vec<u64> = voters.iter().map(|n| n.0).collect();
        let storage = MemStorage::new_with_conf_state(ConfState::from((ids, vec![])));
        RaftPeer::with_storage(node, region, storage)
    }
}

impl<S: PersistentRaftStorage> RaftPeer<S> {
    /// Build a peer over an explicit storage — durable
    /// ([`crate::storage::DiskRaftStorage`]) for real processes; the storage's
    /// initial state carries the voter set and any surviving HardState/log.
    pub fn with_storage(node: NodeId, region: RegionId, storage: S) -> Result<RaftPeer<S>> {
        let cfg = Config {
            id: node.0,
            election_tick: 10,
            heartbeat_tick: 3,
            // DESIGN "5.3 MetaLeader election": disturbance protection + the
            // gray-failure discipline (leader steps down without an active quorum).
            pre_vote: true,
            check_quorum: true,
            ..Default::default()
        };
        cfg.validate().map_err(raft_err)?;
        let logger = Logger::root(Discard, o!());
        let raw = RawNode::new(&cfg, storage, &logger).map_err(raft_err)?;
        Ok(RaftPeer {
            node,
            region,
            inner: Mutex::new(PeerInner {
                raw,
                ready: Vec::new(),
                outbox: Vec::new(),
                alive: true,
            }),
        })
    }

    pub fn node_id(&self) -> NodeId {
        self.node
    }

    fn lock(&self) -> MutexGuard<'_, PeerInner<S>> {
        self.inner.lock().expect("raft peer poisoned")
    }

    /// Propose on the leader, returning the locally assigned [`ProposedAt`].
    /// `propose` and the `last_index` read happen under one lock, so the pair is
    /// exact; it is still only a position claim (see [`ProposedAt`]).
    pub fn propose_traced(&self, data: Vec<u8>) -> Result<ProposedAt> {
        let mut g = self.lock();
        if g.raw.raft.state != StateRole::Leader {
            return Err(Error::Raft(format!(
                "node {} is not the leader of region {}",
                self.node.0, self.region.0
            )));
        }
        let term = g.raw.raft.term;
        g.raw.propose(Vec::new(), data).map_err(raft_err)?;
        let index = g.raw.raft.raft_log.last_index();
        Ok(ProposedAt {
            term,
            index: LogIndex(index),
        })
    }

    /// This node's current view of the group leader, if any.
    pub fn leader_hint(&self) -> Option<NodeId> {
        let g = self.lock();
        let id = g.raw.raft.leader_id;
        (id != raft::INVALID_ID).then_some(NodeId(id))
    }

    /// The current raft term at this peer.
    pub fn term(&self) -> u64 {
        self.lock().raw.raft.term
    }

    /// The highest applied-position handed out via `take_ready` so far comes from
    /// the drained entries themselves; the raft-committed watermark is this.
    pub fn raft_committed(&self) -> LogIndex {
        LogIndex(self.lock().raw.raft.raft_log.committed)
    }

    fn tick(&self) {
        let mut g = self.lock();
        if g.alive {
            g.raw.tick();
        }
    }

    fn step(&self, msg: Message) {
        let mut g = self.lock();
        if g.alive {
            let _ = g.raw.step(msg);
        }
    }

    /// Drain this peer's `Ready`: **persist entries + hardstate first**, then queue
    /// messages, then stash committed entries for `take_ready`, then advance.
    fn process_ready(&self) {
        let mut g = self.lock();
        if !g.alive || !g.raw.has_ready() {
            return;
        }
        let mut ready = g.raw.ready();
        let mut msgs = ready.take_messages();
        // 1. Persist raft-log entries and hardstate (the safety point: durable
        //    BEFORE any message leaves this node — a vote must never outrun its
        //    own persistence).
        if !ready.entries().is_empty() {
            g.raw
                .store()
                .append(ready.entries())
                .expect("raft storage append");
        }
        if let Some(hs) = ready.hs() {
            g.raw
                .store()
                .set_hardstate(hs)
                .expect("raft storage hardstate");
        }
        // 2. Only now hand messages to the transport.
        msgs.extend(ready.take_persisted_messages());
        // 3. Committed entries → the take_ready queue (skip no-op barriers).
        let mut committed: Vec<CommittedEntry> = Vec::new();
        for e in ready.take_committed_entries() {
            if !e.data.is_empty() {
                committed.push(CommittedEntry {
                    index: LogIndex(e.index),
                    term: e.term,
                    data: e.data.to_vec(),
                });
            }
        }
        let mut light = g.raw.advance(ready);
        msgs.extend(light.take_messages());
        for e in light.take_committed_entries() {
            if !e.data.is_empty() {
                committed.push(CommittedEntry {
                    index: LogIndex(e.index),
                    term: e.term,
                    data: e.data.to_vec(),
                });
            }
        }
        g.raw.advance_apply();
        g.ready.extend(committed);
        g.outbox.extend(msgs);
    }

    /// Feed one raft message from the transport into this peer.
    pub fn step_message(&self, msg: Message) {
        self.step(msg);
    }

    /// Advance one tick (called by the driver on its real-time cadence).
    pub fn tick_once(&self) {
        self.tick();
    }

    /// Process pending Ready state and take the outgoing messages for the
    /// transport. Persistence happens inside, before messages are returned.
    pub fn pump(&self) -> Vec<Message> {
        self.process_ready();
        std::mem::take(&mut self.lock().outbox)
    }
}

impl<S: PersistentRaftStorage> RaftGroup for RaftPeer<S> {
    fn region_id(&self) -> RegionId {
        self.region
    }

    fn role(&self) -> Role {
        match self.lock().raw.raft.state {
            StateRole::Leader => Role::Leader,
            StateRole::Follower => Role::Follower,
            StateRole::Candidate | StateRole::PreCandidate => Role::Candidate,
        }
    }

    fn propose(&self, data: Vec<u8>) -> Result<LogIndex> {
        Ok(self.propose_traced(data)?.index)
    }

    fn take_ready(&self) -> Result<Vec<CommittedEntry>> {
        Ok(std::mem::take(&mut self.lock().ready))
    }

    fn committed_index(&self) -> LogIndex {
        self.raft_committed()
    }

    fn campaign(&self) -> Result<()> {
        self.lock().raw.campaign().map_err(raft_err)
    }
}

/// A deterministic in-process cluster of [`RaftPeer`]s for one region: explicit
/// `round()` pumping (tick → process readies → deliver messages), no threads, no
/// timers, no sleeps — the shape the Phase-1 acceptance harness drives.
pub struct InProcessCluster {
    region: RegionId,
    peers: Vec<Arc<RaftPeer<MemStorage>>>,
}

impl InProcessCluster {
    pub fn new(region: RegionId, voters: &[NodeId]) -> Result<InProcessCluster> {
        let peers = voters
            .iter()
            .map(|&n| RaftPeer::new(n, region, voters).map(Arc::new))
            .collect::<Result<Vec<_>>>()?;
        Ok(InProcessCluster { region, peers })
    }

    pub fn region_id(&self) -> RegionId {
        self.region
    }

    pub fn peers(&self) -> &[Arc<RaftPeer<MemStorage>>] {
        &self.peers
    }

    pub fn peer(&self, node: NodeId) -> Option<&Arc<RaftPeer<MemStorage>>> {
        self.peers.iter().find(|p| p.node == node)
    }

    /// Harness kill switch: a dead peer stops ticking and drops traffic both ways.
    pub fn set_alive(&self, node: NodeId, alive: bool) {
        if let Some(p) = self.peer(node) {
            p.lock().alive = alive;
        }
    }

    /// The current leader, if exactly one *alive* peer believes it is leader.
    pub fn leader(&self) -> Option<NodeId> {
        let mut leaders = self
            .peers
            .iter()
            .filter(|p| {
                let g = p.lock();
                g.alive && g.raw.raft.state == StateRole::Leader
            })
            .map(|p| p.node);
        match (leaders.next(), leaders.next()) {
            (Some(l), None) => Some(l),
            _ => None,
        }
    }

    /// One deterministic pump round: tick every alive peer, then settle multi-hop
    /// message exchanges (process readies → deliver → repeat a bounded number of
    /// passes). No wall-clock anywhere.
    pub fn round(&self) {
        for p in &self.peers {
            p.tick();
        }
        for _ in 0..10 {
            for p in &self.peers {
                p.process_ready();
            }
            let mut in_flight: Vec<Message> = Vec::new();
            for p in &self.peers {
                let mut g = p.lock();
                if g.alive {
                    in_flight.append(&mut g.outbox);
                } else {
                    g.outbox.clear();
                }
            }
            if in_flight.is_empty() {
                continue;
            }
            for msg in in_flight {
                if let Some(target) = self.peers.iter().find(|p| p.node.0 == msg.to) {
                    target.step(msg);
                }
            }
        }
    }

    /// Pump until `cond` holds, at most `max_rounds` rounds. Deterministic: fails
    /// with a typed error instead of hanging or sleeping.
    pub fn run_until<F: Fn(&InProcessCluster) -> bool>(
        &self,
        max_rounds: usize,
        what: &str,
        cond: F,
    ) -> Result<()> {
        for _ in 0..max_rounds {
            if cond(self) {
                return Ok(());
            }
            self.round();
        }
        Err(Error::Raft(format!(
            "condition not reached within {max_rounds} rounds: {what}"
        )))
    }
}

fn raft_err(e: raft::Error) -> Error {
    Error::Raft(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{drive_apply, Command, MemStateMachine, RaftGroup, StateMachine};
    use kv9_engine::ColumnFamily;

    const R: RegionId = RegionId(1);
    const N1: NodeId = NodeId(1);
    const N2: NodeId = NodeId(2);
    const N3: NodeId = NodeId(3);

    fn put(key: &[u8], value: &[u8]) -> Vec<u8> {
        Command::Put {
            cf: 0,
            key: key.to_vec(),
            value: value.to_vec(),
        }
        .encode()
    }

    /// Pump + apply every alive peer's drained entries into its own state machine.
    fn drive(cluster: &InProcessCluster, sms: &mut [MemStateMachine]) {
        cluster.round();
        for (p, sm) in cluster.peers().iter().zip(sms.iter_mut()) {
            let _ = drive_apply(p.as_ref(), sm);
        }
    }

    fn run<F: Fn(&InProcessCluster, &[MemStateMachine]) -> bool>(
        cluster: &InProcessCluster,
        sms: &mut [MemStateMachine],
        what: &str,
        cond: F,
    ) {
        for _ in 0..500 {
            if cond(cluster, sms) {
                return;
            }
            drive(cluster, sms);
        }
        panic!("not reached in 500 rounds: {what}");
    }

    /// Real consensus single-node: campaign → propose → committed entry drained via
    /// take_ready → applied → readable. The full pull-model path over raft-rs.
    #[test]
    fn single_node_propose_apply_get() {
        let cluster = InProcessCluster::new(R, &[N1]).unwrap();
        let mut sms = vec![MemStateMachine::new()];
        cluster.peers()[0].campaign().unwrap();
        run(&cluster, &mut sms, "self-elect", |c, _| {
            c.leader().is_some()
        });

        let at = cluster.peers()[0].propose_traced(put(b"k", b"v")).unwrap();
        run(&cluster, &mut sms, "applied", |_, sms| {
            sms[0].applied_index() >= at.index
        });
        assert_eq!(
            sms[0].get(ColumnFamily::Default, b"k").unwrap(),
            Some(b"v".to_vec())
        );
    }

    /// 3 nodes: elect, replicate, every replica applies the entry with the
    /// proposer's (term, index) — the acceptance sync criterion, no sleeps.
    #[test]
    fn three_node_replication_and_failover() {
        let cluster = InProcessCluster::new(R, &[N1, N2, N3]).unwrap();
        let mut sms = vec![
            MemStateMachine::new(),
            MemStateMachine::new(),
            MemStateMachine::new(),
        ];
        cluster.peers()[0].campaign().unwrap();
        run(&cluster, &mut sms, "elect", |c, _| c.leader().is_some());
        let leader = cluster.leader().unwrap();

        let at = cluster
            .peer(leader)
            .unwrap()
            .propose_traced(put(b"a", b"1"))
            .unwrap();
        run(&cluster, &mut sms, "all applied", |_, sms| {
            sms.iter().all(|sm| sm.applied_index() >= at.index)
        });
        for sm in &sms {
            assert_eq!(
                sm.get(ColumnFamily::Default, b"a").unwrap(),
                Some(b"1".to_vec())
            );
        }

        // Live leader failover: kill the leader, survivors re-elect + commit.
        cluster.set_alive(leader, false);
        run(&cluster, &mut sms, "re-elect", |c, _| {
            c.leader().map(|l| l != leader).unwrap_or(false)
        });
        let leader2 = cluster.leader().unwrap();
        let at2 = cluster
            .peer(leader2)
            .unwrap()
            .propose_traced(put(b"b", b"2"))
            .unwrap();
        // The new leader's no-op barrier consumed at least one index.
        assert!(at2.index > at.index);
        let survivors: Vec<usize> = cluster
            .peers()
            .iter()
            .enumerate()
            .filter(|(_, p)| p.node_id() != leader)
            .map(|(i, _)| i)
            .collect();
        run(&cluster, &mut sms, "survivors applied", |_, sms| {
            survivors
                .iter()
                .all(|&i| sms[i].applied_index() >= at2.index)
        });
        for &i in &survivors {
            assert_eq!(
                sms[i].get(ColumnFamily::Default, b"b").unwrap(),
                Some(b"2".to_vec())
            );
        }
    }

    /// The overwrite hazard (contract item 13): a proposal accepted locally but
    /// never committed is overwritten after a failover. Its position is passed by
    /// the new leader's log, yet the proposal's payload never commits anywhere —
    /// so "position reached" MUST NOT be reported as success; only a matching
    /// (term, index) / payload may.
    #[test]
    fn uncommitted_proposal_is_overwritten_not_committed() {
        let cluster = InProcessCluster::new(R, &[N1, N2, N3]).unwrap();
        let mut sms = vec![
            MemStateMachine::new(),
            MemStateMachine::new(),
            MemStateMachine::new(),
        ];
        cluster.peers()[0].campaign().unwrap();
        run(&cluster, &mut sms, "elect", |c, _| c.leader().is_some());
        let old_leader = cluster.leader().unwrap();

        // Isolate the leader, then let it accept a proposal it can never commit.
        cluster.set_alive(old_leader, false);
        let orphan = cluster
            .peer(old_leader)
            .unwrap()
            .propose_traced(put(b"orphan", b"lost"))
            .unwrap();

        // Survivors elect a new leader (higher term) and write.
        run(&cluster, &mut sms, "re-elect", |c, _| {
            c.leader().map(|l| l != old_leader).unwrap_or(false)
        });
        let new_leader = cluster.leader().unwrap();
        let at2 = cluster
            .peer(new_leader)
            .unwrap()
            .propose_traced(put(b"live", b"yes"))
            .unwrap();
        assert!(at2.term > orphan.term, "new leader must have a higher term");
        run(
            &cluster,
            &mut sms,
            "survivors applied live write",
            |c, sms| {
                c.peers()
                    .iter()
                    .zip(sms.iter())
                    .filter(|(p, _)| p.node_id() != old_leader)
                    .all(|(_, sm)| sm.applied_index() >= at2.index)
            },
        );

        // Rejoin the old leader; its orphan entry is truncated away.
        cluster.set_alive(old_leader, true);
        run(&cluster, &mut sms, "old leader catches up", |c, sms| {
            c.peers()
                .iter()
                .zip(sms.iter())
                .all(|(_, sm)| sm.applied_index() >= at2.index)
        });

        // The orphan's position was passed on every replica…
        for p in cluster.peers() {
            assert!(p.raft_committed() >= orphan.index);
        }
        // …but its payload committed nowhere: position reached ≠ proposal reached.
        for sm in &sms {
            assert_eq!(sm.get(ColumnFamily::Default, b"orphan").unwrap(), None);
            assert_eq!(
                sm.get(ColumnFamily::Default, b"live").unwrap(),
                Some(b"yes".to_vec())
            );
        }
    }

    /// A follower must refuse proposals (routing sends clients to the leader).
    #[test]
    fn follower_rejects_propose() {
        let cluster = InProcessCluster::new(R, &[N1, N2, N3]).unwrap();
        let mut sms = vec![
            MemStateMachine::new(),
            MemStateMachine::new(),
            MemStateMachine::new(),
        ];
        cluster.peers()[0].campaign().unwrap();
        run(&cluster, &mut sms, "elect", |c, _| c.leader().is_some());
        let leader = cluster.leader().unwrap();
        let follower = cluster
            .peers()
            .iter()
            .find(|p| p.node_id() != leader)
            .unwrap();
        assert!(follower.propose_traced(put(b"x", b"y")).is_err());
    }
}
