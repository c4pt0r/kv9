//! Election-first bootstrap state machine (DESIGN §5.2), with fencing.
//!
//! A node joining an *uninitialized* cluster does not assume any pre-assigned role. The
//! joining nodes first **elect the metadata server** (a plain Raft leader election over
//! the well-known, empty `META_REGION_0` log), and the elected node then performs
//! metadata initialization and self-bootstrap.
//!
//! ```text
//!   Discovering ──initialized──▶ Joining ─────────────────┐
//!        │                                                 │
//!    uninitialized (quorum-attested)                       │
//!        ▼                                                 │
//!   BootstrapElection ──elected──▶ Initializing ───────────┤
//!        │                                                 ▼
//!        └──not elected──▶ WaitForBootstrap ────────────▶ Serving
//! ```
//!
//! **Fencing — bootstrap must be unforkable** (design review, task #6):
//! - (a) *unreachable ≠ uninitialized*: entering `BootstrapElection` requires a
//!   **positive "uninitialized" answer from a quorum of the declared seed set**
//!   ([`Bootstrap::discovered_uninitialized`]); silence or timeouts never qualify.
//! - (b) the bootstrap election counts votes only within the declared seed set and
//!   needs a majority of it — enforced by the raft group itself, whose voter set *is*
//!   the seed set ([`crate::…`]/`kv9_raft`); two disjoint seed lists can never both
//!   assemble a quorum.
//! - (c) initialization is once-per-lifetime: a node whose data-dir carries an
//!   initialized marker ([`Bootstrap::mark_data_dir_initialized`]) or non-pristine
//!   raft state refuses to re-enter `BootstrapElection` and rejoins via `Joining`;
//!   a wiped node is a *new* node. (raft-rs `initialize()` requires a pristine node,
//!   enforcing this at the library layer too.)

use std::path::Path;

use kv9_common::{Error, NodeId, Result};

/// Marker file recording that this data-dir belongs to an initialized cluster
/// (fencing rule c). Written when initialization commits / the catalog is first
/// observed; read at startup. A wiped dir = a new node.
pub const INIT_MARKER_FILE: &str = "kv9-initialized";

/// Does `data_dir` carry the initialized marker?
pub fn init_marker_exists(data_dir: &Path) -> bool {
    data_dir.join(INIT_MARKER_FILE).is_file()
}

/// Durably write the initialized marker (write temp + fsync + rename, so a
/// crash mid-write never leaves a half-marker that reads as initialized).
pub fn write_init_marker(data_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(data_dir)
        .map_err(|e| Error::MetaNotReady(format!("create {}: {e}", data_dir.display())))?;
    let tmp = data_dir.join(format!("{INIT_MARKER_FILE}.tmp"));
    let path = data_dir.join(INIT_MARKER_FILE);
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)
            .map_err(|e| Error::MetaNotReady(format!("marker create: {e}")))?;
        f.write_all(b"initialized\n")
            .and_then(|_| f.sync_all())
            .map_err(|e| Error::MetaNotReady(format!("marker write: {e}")))?;
    }
    std::fs::rename(&tmp, &path)
        .map_err(|e| Error::MetaNotReady(format!("marker rename: {e}")))
}

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
    /// Discovery found the cluster uninitialized. Fenced: accepted only when this
    /// node **alone** is a quorum of the declared seed set (single-node bootstrap);
    /// multi-node seed sets must present quorum evidence via
    /// [`Bootstrap::discovered_uninitialized`].
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
    /// The declared seed set (join-set). Always contains this node.
    seeds: Vec<NodeId>,
    /// Fencing rule (c): this data-dir has already been part of an initialized
    /// cluster — re-initialization is forbidden for the lifetime of the dir.
    data_dir_initialized: bool,
}

impl Bootstrap {
    /// Start a seedless (single-node) bootstrap: the seed set is `{node}`, so this
    /// node alone is its quorum (DESIGN §5.2's trivial case).
    pub fn new(node: NodeId) -> Self {
        Bootstrap::with_seeds(node, Vec::new())
    }

    /// [`Self::with_seeds`], additionally reading the durable initialized marker
    /// from `data_dir` (fencing rule c across real process restarts): a marked
    /// dir starts with re-initialization permanently forbidden.
    pub fn with_seeds_at(node: NodeId, seeds: Vec<NodeId>, data_dir: &Path) -> Self {
        let mut b = Bootstrap::with_seeds(node, seeds);
        if init_marker_exists(data_dir) {
            b.mark_data_dir_initialized();
        }
        b
    }

    /// Start with the declared seed set from `--join`. This node is always counted
    /// as a member of its own seed set.
    pub fn with_seeds(node: NodeId, mut seeds: Vec<NodeId>) -> Self {
        if !seeds.contains(&node) {
            seeds.push(node);
        }
        Bootstrap {
            node,
            state: BootstrapState::Discovering,
            seeds,
            data_dir_initialized: false,
        }
    }

    pub fn node(&self) -> NodeId {
        self.node
    }

    pub fn state(&self) -> BootstrapState {
        self.state
    }

    pub fn seeds(&self) -> &[NodeId] {
        &self.seeds
    }

    /// Votes needed for a majority of the declared seed set (fencing rules a/b).
    pub fn quorum(&self) -> usize {
        self.seeds.len() / 2 + 1
    }

    /// Record that this data-dir belongs to an initialized cluster (fencing rule c).
    /// From here on, this `Bootstrap` refuses `BootstrapElection` permanently; a
    /// restart rejoins via `Joining`. Called when initialization commits, when the
    /// catalog is first observed, or on startup when the marker is found on disk.
    pub fn mark_data_dir_initialized(&mut self) {
        self.data_dir_initialized = true;
    }

    pub fn data_dir_initialized(&self) -> bool {
        self.data_dir_initialized
    }

    /// Fenced entry into `BootstrapElection` (rule a): `answered` are the seed nodes
    /// that **positively** reported "uninitialized" (include this node; silence and
    /// timeouts must not appear here). Requires answers from a quorum of the declared
    /// seed set; anything less keeps the node `Discovering` with a typed error —
    /// *unreachable is not uninitialized*.
    pub fn discovered_uninitialized(&mut self, answered: &[NodeId]) -> Result<BootstrapState> {
        if self.state != BootstrapState::Discovering {
            return Err(Error::MetaNotReady(format!(
                "illegal bootstrap transition: {:?} on quorum discovery",
                self.state
            )));
        }
        if self.data_dir_initialized {
            return Err(Error::MetaNotReady(
                "this data-dir already belongs to an initialized cluster; \
                 re-initialization is forbidden (rejoin via Joining, or wipe to be a new node)"
                    .into(),
            ));
        }
        // Count only *declared* seeds, deduplicated — answers from nodes outside the
        // seed set carry no authority (two disjoint seed lists must not cross-attest).
        let mut counted: Vec<NodeId> = Vec::new();
        for n in answered {
            if self.seeds.contains(n) && !counted.contains(n) {
                counted.push(*n);
            }
        }
        if counted.len() < self.quorum() {
            return Err(Error::MetaNotReady(format!(
                "only {}/{} seed nodes positively reported uninitialized (quorum {}); \
                 unreachable is not uninitialized — staying in Discovering",
                counted.len(),
                self.seeds.len(),
                self.quorum()
            )));
        }
        self.state = BootstrapState::BootstrapElection;
        Ok(self.state)
    }

    /// Apply an event, returning the new state or an error on an illegal transition
    /// (DESIGN §5.2).
    pub fn on_event(&mut self, event: BootstrapEvent) -> Result<BootstrapState> {
        use BootstrapEvent::*;
        use BootstrapState::*;
        let next = match (self.state, event) {
            (Discovering, FoundInitialized) => Joining,
            // The bare event is only quorum-evidence-free in the single-node case;
            // multi-node seed sets must go through `discovered_uninitialized`.
            (Discovering, FoundUninitialized) => {
                return self.discovered_uninitialized(&[self.node]);
            }
            (BootstrapElection, WonElection) => Initializing,
            (BootstrapElection, LostElection) => WaitForBootstrap,
            (Initializing, MetadataInitialized) => {
                self.data_dir_initialized = true;
                Serving
            }
            (WaitForBootstrap, MetadataInitialized) => {
                self.data_dir_initialized = true;
                WaitForBootstrap // catalog exists, now register
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    const N1: NodeId = NodeId(1);
    const N2: NodeId = NodeId(2);
    const N3: NodeId = NodeId(3);
    const OUTSIDER: NodeId = NodeId(9);

    /// The seedless single-node path (server's current bootstrap) still works with
    /// the bare event: a one-node seed set is its own quorum.
    #[test]
    fn single_node_bare_event_is_its_own_quorum() {
        let mut b = Bootstrap::new(N1);
        assert_eq!(
            b.on_event(BootstrapEvent::FoundUninitialized).unwrap(),
            BootstrapState::BootstrapElection
        );
    }

    /// Fencing rule (a): in a 3-seed cluster, this node's own observation alone —
    /// i.e. the other seeds are merely unreachable — must NOT open the election.
    #[test]
    fn silence_is_not_uninitialized() {
        let mut b = Bootstrap::with_seeds(N1, vec![N1, N2, N3]);
        assert!(b.on_event(BootstrapEvent::FoundUninitialized).is_err());
        assert!(b.discovered_uninitialized(&[N1]).is_err());
        assert_eq!(b.state(), BootstrapState::Discovering);
        // A quorum of positive answers does.
        assert_eq!(
            b.discovered_uninitialized(&[N1, N3]).unwrap(),
            BootstrapState::BootstrapElection
        );
    }

    /// Fencing rule (b) at the attestation layer: answers from nodes outside the
    /// declared seed set carry no authority, and duplicates don't double-count —
    /// two disjoint seed lists cannot cross-attest each other into existence.
    ///
    /// Self-contained sensitivity: the final positive assertion proves the same
    /// gate opens for a genuine quorum from the same state, so the rejections
    /// above cannot be a gate that never opens (a tautological "never happens").
    #[test]
    fn outsiders_and_duplicates_do_not_count() {
        let mut b = Bootstrap::with_seeds(N1, vec![N1, N2, N3]);
        assert!(b
            .discovered_uninitialized(&[N1, OUTSIDER, OUTSIDER])
            .is_err());
        assert!(b.discovered_uninitialized(&[N1, N1, N1]).is_err());
        assert_eq!(b.state(), BootstrapState::Discovering);
        // Sensitivity control: same node, same state — a real quorum DOES open
        // the election, even alongside an outsider and a duplicate in the answer.
        assert_eq!(
            b.discovered_uninitialized(&[N1, N2, N2, OUTSIDER]).unwrap(),
            BootstrapState::BootstrapElection
        );
    }

    /// Fencing rule (c): a data-dir that was ever part of an initialized cluster
    /// can never re-initialize — even if discovery (wrongly) attests a quorum.
    #[test]
    fn initialized_data_dir_refuses_reinit() {
        let mut b = Bootstrap::with_seeds(N1, vec![N1, N2, N3]);
        b.mark_data_dir_initialized();
        assert!(b.discovered_uninitialized(&[N1, N2, N3]).is_err());
        assert_eq!(b.state(), BootstrapState::Discovering);
        // The Joining path stays open.
        assert_eq!(
            b.on_event(BootstrapEvent::FoundInitialized).unwrap(),
            BootstrapState::Joining
        );
    }

    /// The winner's `MetadataInitialized` sets the marker, so a crashed-and-restarted
    /// initializer that kept its data-dir cannot fork a second cluster.
    #[test]
    fn initialization_sets_the_marker() {
        let mut b = Bootstrap::new(N1);
        b.on_event(BootstrapEvent::FoundUninitialized).unwrap();
        b.on_event(BootstrapEvent::WonElection).unwrap();
        b.on_event(BootstrapEvent::MetadataInitialized).unwrap();
        assert!(b.is_serving());
        assert!(b.data_dir_initialized());
    }

    /// The marker survives a real process restart (file, not memory): a
    /// re-created FSM over the same data-dir refuses re-initialization and
    /// takes the Joining path. Control: a wiped dir is a new node again.
    #[test]
    fn init_marker_persists_across_restart() {
        let dir = std::env::temp_dir().join(format!(
            "kv9-marker-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);

        // First incarnation: fresh dir, bootstrap allowed, init commits → marker.
        let mut b = Bootstrap::with_seeds_at(N1, vec![N1, N2, N3], &dir);
        assert!(!b.data_dir_initialized());
        b.discovered_uninitialized(&[N1, N2]).unwrap();
        b.on_event(BootstrapEvent::WonElection).unwrap();
        b.on_event(BootstrapEvent::MetadataInitialized).unwrap();
        write_init_marker(&dir).unwrap();

        // "Restart": a new FSM over the same dir must refuse re-init…
        let mut restarted = Bootstrap::with_seeds_at(N1, vec![N1, N2, N3], &dir);
        assert!(restarted.data_dir_initialized());
        assert!(restarted.discovered_uninitialized(&[N1, N2, N3]).is_err());
        // …but joins normally.
        assert_eq!(
            restarted.on_event(BootstrapEvent::FoundInitialized).unwrap(),
            BootstrapState::Joining
        );

        // Control (sensitivity): a wiped dir is a NEW node — bootstrap opens again.
        let _ = std::fs::remove_dir_all(&dir);
        let mut wiped = Bootstrap::with_seeds_at(N1, vec![N1, N2, N3], &dir);
        assert!(!wiped.data_dir_initialized());
        assert_eq!(
            wiped.discovered_uninitialized(&[N1, N2]).unwrap(),
            BootstrapState::BootstrapElection
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Loser path: wait for the catalog, register, serve — marker set on observing
    /// the initialized catalog.
    #[test]
    fn loser_waits_then_registers() {
        let mut b = Bootstrap::with_seeds(N2, vec![N1, N2, N3]);
        b.discovered_uninitialized(&[N1, N2, N3]).unwrap();
        b.on_event(BootstrapEvent::LostElection).unwrap();
        b.on_event(BootstrapEvent::MetadataInitialized).unwrap();
        assert!(b.data_dir_initialized());
        assert_eq!(
            b.on_event(BootstrapEvent::Registered).unwrap(),
            BootstrapState::Serving
        );
    }
}
