//! Real Phase-1 metadata-node runtime.
//!
//! This is the process boundary missing from the earlier deterministic harness:
//! fixed seed identities, real TCP discovery/Raft traffic, durable Raft state,
//! durable catalog apply, election-first bootstrap, and a machine-readable status
//! file for external acceptance. The status file is evidence; log timing is not.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use kv9_common::{Config, Error, NodeId, Result, SeedPeer, META_REGION_0};
use kv9_engine::WalEngine;
use kv9_meta::bootstrap::{init_marker_exists, write_init_marker};
use kv9_meta::codec::memcmp_uint;
use kv9_meta::schema::SCHEMA_VERSION_DESC;
use kv9_meta::{Bootstrap, BootstrapEvent, BootstrapState};
use kv9_raft::driver::NodeDriver;
use kv9_raft::storage::DiskRaftStorage;
use kv9_raft::transport::{DiscoveryState, TcpTransport};
use kv9_raft::{MemStateMachine, ProposedAt, RaftGroup, RaftPeer, Role};

use crate::Node;

const TICK: Duration = Duration::from_millis(20);
const DISCOVERY_INTERVAL: Duration = Duration::from_millis(200);
const DISCOVERY_TIMEOUT: Duration = Duration::from_millis(50);

#[derive(Debug)]
struct RuntimeDiscovery {
    node: NodeId,
    initialized: AtomicBool,
}

impl RuntimeDiscovery {
    fn new(node: NodeId, initialized: bool) -> Self {
        Self {
            node,
            initialized: AtomicBool::new(initialized),
        }
    }

    fn set_initialized(&self) {
        self.initialized.store(true, Ordering::Release);
    }
}

impl DiscoveryState for RuntimeDiscovery {
    fn answer(&self) -> (NodeId, bool) {
        (self.node, self.initialized.load(Ordering::Acquire))
    }
}

/// A running real-process metadata member.
pub struct NodeRuntime {
    node: Arc<Node<WalEngine>>,
    driver: Arc<NodeDriver<DiskRaftStorage, WalEngine>>,
    transport: Arc<TcpTransport>,
    discovery: Arc<RuntimeDiscovery>,
    driver_thread: Option<std::thread::JoinHandle<()>>,
    voters: Vec<NodeId>,
    seeds: Vec<SeedPeer>,
    data_dir: PathBuf,
    status_path: PathBuf,
    campaign_started: bool,
    initial_proposal: Option<ProposedAt>,
    next_discovery: Instant,
}

impl NodeRuntime {
    /// Assemble and start the TCP listener + Raft pump. Bootstrap advances in
    /// [`Self::run`], after every process is already able to answer discovery.
    pub fn start(id: NodeId, config: Config) -> Result<Self> {
        config.validate()?;
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
        ));
        let peers = seeds
            .iter()
            .filter(|seed| seed.node_id != id)
            .map(|seed| (seed.node_id.0, seed.addr))
            .collect::<HashMap<_, _>>();
        let transport = TcpTransport::bind(id, addr, peers, discovery.clone())?;
        let driver = NodeDriver::new(
            peer,
            transport.clone(),
            MemStateMachine::with_engine(engine)?,
        );
        let driver_thread = Some(driver.spawn(TICK));
        let status_path = data_dir.join("status");

        Ok(Self {
            node,
            driver,
            transport,
            discovery,
            driver_thread,
            voters,
            seeds,
            data_dir,
            status_path,
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
            if let Some(fatal) = self.driver.status().fatal {
                self.write_status()?;
                return Err(Error::Raft(fatal));
            }
            self.advance_bootstrap()?;
            self.write_status()?;
            std::thread::sleep(TICK);
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
            BootstrapState::Discovering => self.advance_discovery(),
            BootstrapState::BootstrapElection => self.advance_election(),
            BootstrapState::Initializing => self.advance_initialization(),
            BootstrapState::WaitForBootstrap | BootstrapState::Joining => self.advance_joining(),
            BootstrapState::Serving => Ok(()),
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
        if locally_fenced {
            self.node
                .meta
                .lock()
                .expect("meta poisoned")
                .bootstrap
                .on_event(BootstrapEvent::FoundInitialized)?;
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
            if let Ok((answer_id, initialized)) =
                TcpTransport::discover(self.node.id, seed.addr, DISCOVERY_TIMEOUT)
            {
                // The address is authoritative for one declared identity. A
                // different answer is configuration error/outsider, never a vote.
                if answer_id != seed.node_id {
                    continue;
                }
                if initialized {
                    found_initialized = true;
                } else {
                    uninitialized.push(answer_id);
                }
            }
        }
        let mut meta = self.node.meta.lock().expect("meta poisoned");
        if found_initialized {
            meta.bootstrap.on_event(BootstrapEvent::FoundInitialized)?;
            return Ok(());
        }
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
        if self.driver.status().role != Role::Leader {
            return Err(Error::MetaNotReady(
                "bootstrap initializer lost leadership before catalog commit".into(),
            ));
        }
        if self.initial_proposal.is_none() {
            let cmd = self.node.build_initial_metadata_command_for(&self.voters)?;
            self.initial_proposal = Some(self.driver.propose(&cmd)?);
            return Ok(());
        }
        let proposal = self.initial_proposal.expect("set above");
        match self.driver.wait_applied(proposal, Duration::from_millis(1)) {
            Ok(true) => {
                write_init_marker(&self.data_dir)?;
                self.discovery.set_initialized();
                self.node
                    .meta
                    .lock()
                    .expect("meta poisoned")
                    .bootstrap
                    .on_event(BootstrapEvent::MetadataInitialized)?;
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
        if !catalog_initialized(&self.node)? {
            return Ok(());
        }
        write_init_marker(&self.data_dir)?;
        self.discovery.set_initialized();
        let mut meta = self.node.meta.lock().expect("meta poisoned");
        match meta.bootstrap.state() {
            BootstrapState::WaitForBootstrap => {
                meta.bootstrap
                    .on_event(BootstrapEvent::MetadataInitialized)?;
                meta.bootstrap.on_event(BootstrapEvent::Registered)?;
            }
            BootstrapState::Joining => {
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
        };
        let body = format!(
            "pid={}\nnode_id={}\nleader_id={}\nrole={}\nterm={}\nraft_committed={}\napplied_index={}\nbootstrap_state={:?}\nfatal={}\n",
            std::process::id(),
            raft.node_id.0,
            raft.leader_id.map_or(0, |id| id.0),
            role,
            raft.term,
            raft.raft_committed,
            raft.applied_index,
            bootstrap,
            raft.fatal.as_deref().unwrap_or(""),
        );
        let tmp = self.data_dir.join("status.tmp");
        fs::write(&tmp, body)
            .and_then(|_| fs::rename(&tmp, &self.status_path))
            .map_err(|e| Error::Config(format!("write {}: {e}", self.status_path.display())))
    }
}

impl Drop for NodeRuntime {
    fn drop(&mut self) {
        self.driver.stop();
        self.transport.shutdown();
        if let Some(handle) = self.driver_thread.take() {
            let _ = handle.join();
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
