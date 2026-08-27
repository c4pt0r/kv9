//! Election-first bootstrap state machine (DESIGN §5.2).
//!
//! A node joining an *uninitialized* cluster does not assume any pre-assigned role. The
//! joining nodes first **elect the metadata server** (a plain Raft leader election over
//! the well-known, empty `META_REGION_0` log), and the elected node then performs
//! metadata initialization and self-bootstrap.
//!
//! ```text
//!   Discovering ──initialized──▶ Joining ─────────────────┐
//!        │                                                 │
//!    uninitialized                                         │
//!        ▼                                                 │
//!   BootstrapElection ──elected──▶ Initializing ───────────┤
//!        │                                                 ▼
//!        └──not elected──▶ WaitForBootstrap ────────────▶ Serving
//! ```

use kv9_common::{Error, NodeId, Result};

/// The node lifecycle states during bootstrap (DESIGN §5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapState {
    /// Contact the join-set, ask "is the cluster initialized?" (DESIGN §5.2).
    Discovering,
    /// Cluster is already initialized: this node just joins and registers.
    Joining,
    /// Uninitialized: run one Raft election over `META_REGION_0` (DESIGN §5.2).
    BootstrapElection,
    /// This node won: it writes the initial metadata as the first committed entries
    /// (system keyspace, default tenant, `META_REGION_0` record, TSO window).
    Initializing,
    /// This node lost: wait until the leader wrote the catalog, then register self.
    WaitForBootstrap,
    /// Data-driven from here on (DESIGN §5.2).
    Serving,
}

/// The event that drives a transition (DESIGN §5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapEvent {
    /// Discovery found the cluster already initialized.
    FoundInitialized,
    /// Discovery found the cluster uninitialized.
    FoundUninitialized,
    /// This node won the bootstrap election.
    WonElection,
    /// This node lost the bootstrap election.
    LostElection,
    /// The winner finished writing the initial metadata / catalog exists.
    MetadataInitialized,
    /// This node has registered itself into membership.
    Registered,
}

/// Election-first bootstrap driver (DESIGN §5.2). Crash-safe & idempotent because the
/// initialization steps are ordinary Raft-committed entries: a crashed initializer just
/// re-elects and continues.
#[derive(Debug)]
pub struct Bootstrap {
    node: NodeId,
    state: BootstrapState,
}

impl Bootstrap {
    /// Start a node in the `Discovering` state (DESIGN §5.2).
    pub fn new(node: NodeId) -> Self {
        Bootstrap {
            node,
            state: BootstrapState::Discovering,
        }
    }

    pub fn node(&self) -> NodeId {
        self.node
    }

    pub fn state(&self) -> BootstrapState {
        self.state
    }

    /// Apply an event, returning the new state or an error on an illegal transition
    /// (DESIGN §5.2).
    pub fn on_event(&mut self, event: BootstrapEvent) -> Result<BootstrapState> {
        use BootstrapEvent::*;
        use BootstrapState::*;
        let next = match (self.state, event) {
            (Discovering, FoundInitialized) => Joining,
            (Discovering, FoundUninitialized) => BootstrapElection,
            (BootstrapElection, WonElection) => Initializing,
            (BootstrapElection, LostElection) => WaitForBootstrap,
            (Initializing, MetadataInitialized) => Serving,
            (WaitForBootstrap, MetadataInitialized) => WaitForBootstrap, // catalog exists, now register
            (WaitForBootstrap, Registered) => Serving,
            (Joining, Registered) => Serving,
            (state, ev) => {
                return Err(Error::MetaNotReady(format!(
                    "illegal bootstrap transition: {state:?} on {ev:?}"
                )))
            }
        };
        self.state = next;
        Ok(next)
    }

    pub fn is_serving(&self) -> bool {
        self.state == BootstrapState::Serving
    }
}
