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

use kv9_common::{ClusterId, Error, NodeId, Result};

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
///
/// **Structural fingerprint retirement (task #24, gate contract):** the
/// bootstrap voter-set fingerprint exists ONLY to keep two *uninitialized*
/// seed sets from cross-endorsing — so it lives as data inside the
/// pre-initialization states and NOWHERE else. Once a cluster identity
/// exists, the states carry the [`ClusterId`] instead; code that wants the
/// fingerprint after initialization has nothing to reach for
/// ([`Bootstrap::bootstrap_fingerprint`] returns `None`), rather than a
/// stale-but-readable field waiting to be misused as identity.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BootstrapState {
    /// Contact the join-set, ask "is the cluster initialized?" (DESIGN §5.2).
    Discovering { fp: u64 },
    /// Cluster is already initialized: this node just joins and registers.
    /// Carries the verified cluster identity — the steady-state authority.
    Joining { cluster_id: ClusterId },
    /// Uninitialized: run one Raft election over `META_REGION_0` (DESIGN §5.2).
    BootstrapElection { fp: u64 },
    /// This node won: it writes the initial metadata as the first committed entries
    /// (system keyspace, default tenant, `META_REGION_0` record, TSO window),
    /// including the minted [`ClusterId`].
    Initializing { fp: u64 },
    /// This node lost: wait until the leader wrote the catalog.
    WaitForBootstrap { fp: u64 },
    /// Data-driven from here on (DESIGN §5.2).
    Serving { cluster_id: ClusterId },
}

impl BootstrapState {
    /// The bare state name — the STABLE external form (status files print
    /// this; scripts grep `bootstrap_state=Serving`). Variant payloads are
    /// internal and must never leak into that surface.
    pub fn name(&self) -> &'static str {
        match self {
            BootstrapState::Discovering { .. } => "Discovering",
            BootstrapState::Joining { .. } => "Joining",
            BootstrapState::BootstrapElection { .. } => "BootstrapElection",
            BootstrapState::Initializing { .. } => "Initializing",
            BootstrapState::WaitForBootstrap { .. } => "WaitForBootstrap",
            BootstrapState::Serving { .. } => "Serving",
        }
    }
}

/// Prints the bare variant name only, so `{:?}` consumers (status files,
/// error strings, scripts) keep the exact pre-#24 wire form.
impl std::fmt::Debug for BootstrapState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// The event that drives a transition (DESIGN §5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapEvent {
    /// Discovery found the cluster already initialized. Carries the cluster
    /// identity learned from the answer (or the local marker/catalog) —
    /// initialized answers without an identity are a protocol error upstream,
    /// never a bare flag here.
    FoundInitialized { cluster_id: ClusterId },
    /// Discovery found the cluster uninitialized. Fenced: accepted only when this
    /// node **alone** is a quorum of the declared seed set (single-node bootstrap);
    /// multi-node seed sets must present quorum evidence via
    /// [`Bootstrap::discovered_uninitialized`]. FORBIDDEN in join-existing mode.
    FoundUninitialized,
    /// This node won the bootstrap election.
    WonElection,
    /// This node lost the bootstrap election.
    LostElection,
    /// The winner finished writing the initial metadata / the catalog now
    /// exists locally — carrying the identity it minted / recorded.
    MetadataInitialized { cluster_id: ClusterId },
    /// This node has registered itself into membership.
    Registered,
}

/// Which of the two bootstrap modes this node runs (task #24).
///
/// The mode is decided by DATA SHAPE, not a flag: a node whose id is in the
/// declared voter set is initial-bootstrap; a node whose id is absent is
/// join-existing (and must additionally present the expected [`ClusterId`]).
/// Forgetting to configure something therefore fails closed instead of
/// silently picking the wrong path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Self is a declared voter: may attest bootstrap quorum, campaign, init.
    InitialBootstrap,
    /// Self joins an existing cluster: may NEVER attest an uninitialized
    /// quorum or campaign; only a matching initialized answer admits it.
    JoinExisting { expected: ClusterId },
}

/// Election-first bootstrap driver (DESIGN §5.2). Crash-safe & idempotent because the
/// initialization steps are ordinary Raft-committed entries: a crashed initializer just
/// re-elects and continues.
#[derive(Debug)]
pub struct Bootstrap {
    node: NodeId,
    state: BootstrapState,
    /// The declared voter set. Initial-bootstrap mode: contains this node.
    /// Join-existing mode: does NOT (that asymmetry IS the mode marker).
    seeds: Vec<NodeId>,
    mode: Mode,
    /// Fencing rule (c): this data-dir has already been part of an initialized
    /// cluster — re-initialization is forbidden for the lifetime of the dir.
    data_dir_initialized: bool,
}

impl Bootstrap {
    /// Start a seedless (single-node) bootstrap: the seed set is `{node}`, so this
    /// node alone is its quorum (DESIGN §5.2's trivial case). Fingerprint 0 —
    /// a single declared node has no peer to cross-check against.
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

    /// [`Self::with_seeds`] with an explicit voter-set fingerprint (computed
    /// by the runtime over the declared `(node_id, address)` pairs — the FSM
    /// itself never sees addresses). The fingerprint is carried by the
    /// pre-initialization states and retires structurally at initialization.
    pub fn with_seeds_fp(node: NodeId, mut seeds: Vec<NodeId>, fp: u64) -> Self {
        if !seeds.contains(&node) {
            seeds.push(node);
        }
        Bootstrap {
            node,
            state: BootstrapState::Discovering { fp },
            seeds,
            mode: Mode::InitialBootstrap,
            data_dir_initialized: false,
        }
    }

    /// Start with the declared seed set from `--join`. This node is always counted
    /// as a member of its own seed set. (Fingerprint 0: callers that have one
    /// use [`Self::with_seeds_fp`].)
    pub fn with_seeds(node: NodeId, seeds: Vec<NodeId>) -> Self {
        Bootstrap::with_seeds_fp(node, seeds, 0)
    }

    /// **Join-existing mode** (task #24): this node is NOT in the declared
    /// voter set and presents the cluster identity it expects to join. It can
    /// never attest a bootstrap quorum, never campaign, and only a discovery
    /// answer carrying the EXPECTED identity moves it forward — a node
    /// pointed at the wrong environment stalls with typed errors instead of
    /// silently registering into it.
    pub fn join_existing_at(
        node: NodeId,
        bootstrap_voters: Vec<NodeId>,
        expected: ClusterId,
        fp: u64,
        data_dir: &Path,
    ) -> Result<Self> {
        if bootstrap_voters.is_empty() {
            return Err(Error::MetaNotReady(
                "join-existing requires the cluster's declared voter set".into(),
            ));
        }
        if bootstrap_voters.contains(&node) {
            return Err(Error::MetaNotReady(format!(
                "join-existing mode: node {} must NOT be in the declared voter \
                 set (a declared voter restarts in initial-bootstrap mode)",
                node.0
            )));
        }
        let mut b = Bootstrap {
            node,
            state: BootstrapState::Discovering { fp },
            seeds: bootstrap_voters,
            mode: Mode::JoinExisting { expected },
            data_dir_initialized: false,
        };
        if init_marker_exists(data_dir) {
            b.mark_data_dir_initialized();
        }
        Ok(b)
    }

    pub fn node(&self) -> NodeId {
        self.node
    }

    pub fn state(&self) -> BootstrapState {
        self.state
    }

    /// The bootstrap voter-set fingerprint — available ONLY before
    /// initialization (`None` from `Joining`/`Serving` onward). The post-init
    /// identity is [`Self::cluster_id`]; this accessor's `None` is the
    /// structural retirement, not a convention.
    pub fn bootstrap_fingerprint(&self) -> Option<u64> {
        match self.state {
            BootstrapState::Discovering { fp }
            | BootstrapState::BootstrapElection { fp }
            | BootstrapState::Initializing { fp }
            | BootstrapState::WaitForBootstrap { fp } => Some(fp),
            BootstrapState::Joining { .. } | BootstrapState::Serving { .. } => None,
        }
    }

    /// The cluster identity — available once this node has learned/minted it
    /// (`Joining`/`Serving`).
    pub fn cluster_id(&self) -> Option<ClusterId> {
        match self.state {
            BootstrapState::Joining { cluster_id } | BootstrapState::Serving { cluster_id } => {
                Some(cluster_id)
            }
            _ => None,
        }
    }

    /// Whether this node runs in join-existing mode (self outside the
    /// declared voter set).
    pub fn is_joining_mode(&self) -> bool {
        matches!(self.mode, Mode::JoinExisting { .. })
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

    /// LOCAL CATALOG FIRST: if this node's own durable catalog already names
    /// a cluster, the cluster exists — no missing marker, no peer answer, and
    /// no "uninitialized quorum" may override that. Returns `Ok(true)` when
    /// it advanced `Discovering → Joining` with the local identity
    /// (join-existing mode still verifies the expected id first).
    ///
    /// Scope, stated precisely (corrected after Tess's re-review): the
    /// crash-window restart (seed txn applied, marker unwritten) is ALREADY
    /// repaired by the runtime constructor's catalog preflight at open — this
    /// method is the second, identity-carrying line for the Discovering tick
    /// itself: it moves on the CLUSTER ID (not a schema row), covers any path
    /// that reaches Discovering without that preflight conclusion, and is the
    /// hook where join-existing verifies the expected identity. Rule (c)
    /// engages here too, so a node whose catalog names a cluster can never
    /// attest an uninitialized quorum regardless of marker state.
    pub fn observe_local_identity(&mut self, id: Option<ClusterId>) -> Result<bool> {
        let BootstrapState::Discovering { .. } = self.state else {
            return Ok(false);
        };
        let Some(cluster_id) = id else {
            return Ok(false);
        };
        if let Mode::JoinExisting { expected } = self.mode {
            if cluster_id != expected {
                return Err(Error::MetaNotReady(format!(
                    "local catalog names cluster {cluster_id}, expected {expected}; \
                     refusing to proceed"
                )));
            }
        }
        // The catalog names a cluster this data-dir belongs to: rule (c)
        // engages even if the marker write was lost.
        self.data_dir_initialized = true;
        self.state = BootstrapState::Joining { cluster_id };
        Ok(true)
    }

    /// Fenced entry into `BootstrapElection` (rule a): `answered` are the seed nodes
    /// that **positively** reported "uninitialized" (include this node; silence and
    /// timeouts must not appear here). Requires answers from a quorum of the declared
    /// seed set; anything less keeps the node `Discovering` with a typed error —
    /// *unreachable is not uninitialized*.
    pub fn discovered_uninitialized(&mut self, answered: &[NodeId]) -> Result<BootstrapState> {
        // Fail closed in join-existing mode: a joiner has no authority to
        // attest a bootstrap quorum — accepting would let a misconfigured
        // joiner co-found a second cluster (the exact fork the gates exist
        // to prevent).
        if let Mode::JoinExisting { .. } = self.mode {
            return Err(Error::MetaNotReady(
                "a joining node can never attest bootstrap quorum".into(),
            ));
        }
        let BootstrapState::Discovering { fp } = self.state else {
            return Err(Error::MetaNotReady(format!(
                "illegal bootstrap transition: {:?} on quorum discovery",
                self.state
            )));
        };
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
        self.state = BootstrapState::BootstrapElection { fp };
        Ok(self.state)
    }

    /// Apply an event, returning the new state or an error on an illegal transition
    /// (DESIGN §5.2).
    pub fn on_event(&mut self, event: BootstrapEvent) -> Result<BootstrapState> {
        use BootstrapEvent::*;
        use BootstrapState::*;
        let next = match (self.state, event) {
            (Discovering { .. }, FoundInitialized { cluster_id }) => {
                // Join-existing: the answer's identity must be the EXPECTED
                // one — an initialized answer from the wrong environment is a
                // typed error, not an admission (three-gate contract, gate 2).
                if let Mode::JoinExisting { expected } = self.mode {
                    if cluster_id != expected {
                        return Err(Error::MetaNotReady(format!(
                            "discovered cluster identity {cluster_id} does not match \
                             the expected {expected}; refusing to join"
                        )));
                    }
                }
                Joining { cluster_id }
            }
            // The bare event is only quorum-evidence-free in the single-node case;
            // multi-node seed sets must go through `discovered_uninitialized`.
            (Discovering { .. }, FoundUninitialized) => {
                return self.discovered_uninitialized(&[self.node]);
            }
            (BootstrapElection { fp }, WonElection) => Initializing { fp },
            (BootstrapElection { fp }, LostElection) => WaitForBootstrap { fp },
            (Initializing { .. }, MetadataInitialized { cluster_id }) => {
                self.data_dir_initialized = true;
                Serving { cluster_id }
            }
            // The catalog now exists locally (the winner's init applied here):
            // the fingerprint retires and the register path begins — same as
            // a discovered join.
            (WaitForBootstrap { .. }, MetadataInitialized { cluster_id }) => {
                self.data_dir_initialized = true;
                Joining { cluster_id }
            }
            (Joining { cluster_id }, Registered) => Serving { cluster_id },
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
        matches!(self.state, BootstrapState::Serving { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    const N1: NodeId = NodeId(1);
    const N2: NodeId = NodeId(2);
    const N3: NodeId = NodeId(3);
    const OUTSIDER: NodeId = NodeId(9);

    fn cid(hex_byte: &str) -> ClusterId {
        ClusterId::from_str(&hex_byte.repeat(32)).unwrap()
    }

    fn found(b: &str) -> BootstrapEvent {
        BootstrapEvent::FoundInitialized { cluster_id: cid(b) }
    }

    fn minted(b: &str) -> BootstrapEvent {
        BootstrapEvent::MetadataInitialized { cluster_id: cid(b) }
    }

    /// The seedless single-node path (server's current bootstrap) still works with
    /// the bare event: a one-node seed set is its own quorum.
    #[test]
    fn single_node_bare_event_is_its_own_quorum() {
        let mut b = Bootstrap::new(N1);
        assert_eq!(
            b.on_event(BootstrapEvent::FoundUninitialized).unwrap().name(),
            "BootstrapElection"
        );
    }

    /// Fencing rule (a): in a 3-seed cluster, this node's own observation alone —
    /// i.e. the other seeds are merely unreachable — must NOT open the election.
    #[test]
    fn silence_is_not_uninitialized() {
        let mut b = Bootstrap::with_seeds(N1, vec![N1, N2, N3]);
        assert!(b.on_event(BootstrapEvent::FoundUninitialized).is_err());
        assert!(b.discovered_uninitialized(&[N1]).is_err());
        assert_eq!(b.state().name(), "Discovering");
        // A quorum of positive answers does.
        assert_eq!(
            b.discovered_uninitialized(&[N1, N3]).unwrap().name(),
            "BootstrapElection"
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
        assert_eq!(b.state().name(), "Discovering");
        // Sensitivity control: same node, same state — a real quorum DOES open
        // the election, even alongside an outsider and a duplicate in the answer.
        assert_eq!(
            b.discovered_uninitialized(&[N1, N2, N2, OUTSIDER]).unwrap().name(),
            "BootstrapElection"
        );
    }

    /// Fencing rule (c): a data-dir that was ever part of an initialized cluster
    /// can never re-initialize — even if discovery (wrongly) attests a quorum.
    #[test]
    fn initialized_data_dir_refuses_reinit() {
        let mut b = Bootstrap::with_seeds(N1, vec![N1, N2, N3]);
        b.mark_data_dir_initialized();
        assert!(b.discovered_uninitialized(&[N1, N2, N3]).is_err());
        assert_eq!(b.state().name(), "Discovering");
        // The Joining path stays open.
        assert_eq!(b.on_event(found("a")).unwrap().name(), "Joining");
        assert_eq!(b.cluster_id(), Some(cid("a")));
    }

    /// The winner's `MetadataInitialized` sets the marker, so a crashed-and-restarted
    /// initializer that kept its data-dir cannot fork a second cluster.
    #[test]
    fn initialization_sets_the_marker() {
        let mut b = Bootstrap::new(N1);
        b.on_event(BootstrapEvent::FoundUninitialized).unwrap();
        b.on_event(BootstrapEvent::WonElection).unwrap();
        b.on_event(minted("b")).unwrap();
        assert!(b.is_serving());
        assert!(b.data_dir_initialized());
        assert_eq!(b.cluster_id(), Some(cid("b")));
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
        b.on_event(minted("c")).unwrap();
        write_init_marker(&dir).unwrap();

        // "Restart": a new FSM over the same dir must refuse re-init…
        let mut restarted = Bootstrap::with_seeds_at(N1, vec![N1, N2, N3], &dir);
        assert!(restarted.data_dir_initialized());
        assert!(restarted.discovered_uninitialized(&[N1, N2, N3]).is_err());
        // …but joins normally.
        assert_eq!(restarted.on_event(found("c")).unwrap().name(), "Joining");

        // Control (sensitivity): a wiped dir is a NEW node — bootstrap opens again.
        let _ = std::fs::remove_dir_all(&dir);
        let mut wiped = Bootstrap::with_seeds_at(N1, vec![N1, N2, N3], &dir);
        assert!(!wiped.data_dir_initialized());
        assert_eq!(
            wiped.discovered_uninitialized(&[N1, N2]).unwrap().name(),
            "BootstrapElection"
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
        // Catalog appears locally: fingerprint retires, register path begins.
        assert_eq!(b.on_event(minted("d")).unwrap().name(), "Joining");
        assert!(b.data_dir_initialized());
        assert_eq!(b.on_event(BootstrapEvent::Registered).unwrap().name(), "Serving");
        assert_eq!(b.cluster_id(), Some(cid("d")));
    }

    /// Structural fingerprint retirement: the fp is readable in every
    /// pre-initialization state and GONE (not stale — absent) afterward.
    #[test]
    fn fingerprint_retires_at_initialization() {
        let mut b = Bootstrap::with_seeds_fp(N1, vec![N1, N2, N3], 0xFEED);
        assert_eq!(b.bootstrap_fingerprint(), Some(0xFEED));
        assert_eq!(b.cluster_id(), None);
        b.discovered_uninitialized(&[N1, N2]).unwrap();
        assert_eq!(b.bootstrap_fingerprint(), Some(0xFEED));
        b.on_event(BootstrapEvent::WonElection).unwrap();
        assert_eq!(b.bootstrap_fingerprint(), Some(0xFEED));
        b.on_event(minted("e")).unwrap();
        // Post-init: fp is unreachable, identity is the ClusterId.
        assert_eq!(b.bootstrap_fingerprint(), None);
        assert_eq!(b.cluster_id(), Some(cid("e")));
    }

    /// Debug prints the BARE variant name — the stable wire form status files
    /// publish. A derived Debug would leak payloads into scripts' grep space.
    #[test]
    fn debug_form_is_the_bare_name() {
        let b = Bootstrap::with_seeds_fp(N1, vec![N1, N2, N3], 7);
        assert_eq!(format!("{:?}", b.state()), "Discovering");
        let mut b = Bootstrap::new(N1);
        b.on_event(BootstrapEvent::FoundUninitialized).unwrap();
        b.on_event(BootstrapEvent::WonElection).unwrap();
        b.on_event(minted("f")).unwrap();
        assert_eq!(format!("{:?}", b.state()), "Serving");
    }

    /// The P0 recovery mechanism: a durable catalog identity beats a missing
    /// marker. From Discovering, observing a local identity advances to
    /// Joining with THAT id and engages rule (c) — the node can never again
    /// attest an uninitialized quorum. Sensitivity: stubbing the transition
    /// out (return Ok(false) unconditionally) turns the first assert red.
    #[test]
    fn local_catalog_identity_beats_missing_marker() {
        let mut b = Bootstrap::with_seeds_fp(N1, vec![N1, N2, N3], 5);
        // Nothing local yet: no-op, still free to quorum later.
        assert!(!b.observe_local_identity(None).unwrap());
        assert_eq!(b.state().name(), "Discovering");
        // A durable local identity: straight to Joining, rule (c) engaged.
        assert!(b.observe_local_identity(Some(cid("a"))).unwrap());
        assert_eq!(b.state().name(), "Joining");
        assert_eq!(b.cluster_id(), Some(cid("a")));
        assert!(b.data_dir_initialized());
        // Outside Discovering it is a no-op (idempotent per tick).
        assert!(!b.observe_local_identity(Some(cid("a"))).unwrap());

        // And a node with a local identity can never attest uninitialized:
        // rule (c) blocks the quorum path on a FRESH fsm marked the same way.
        let mut fenced = Bootstrap::with_seeds_fp(N1, vec![N1, N2, N3], 5);
        fenced.observe_local_identity(Some(cid("a"))).unwrap();
        assert!(fenced.discovered_uninitialized(&[N1, N2, N3]).is_err());
    }

    /// Join-existing mode: the local identity must still match the expected
    /// one — a wrong-environment data-dir is a typed error, not a join.
    #[test]
    fn observe_local_identity_checks_join_expectation() {
        let dir = std::env::temp_dir().join(format!(
            "kv9-observe-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let mut b =
            Bootstrap::join_existing_at(NodeId(4), vec![N1, N2, N3], cid("a"), 1, &dir).unwrap();
        assert!(b.observe_local_identity(Some(cid("b"))).is_err());
        assert_eq!(b.state().name(), "Discovering");
        assert!(b.observe_local_identity(Some(cid("a"))).unwrap());
        assert_eq!(b.state().name(), "Joining");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Join-existing mode: fail closed everywhere except the one legitimate
    /// path (matching initialized answer → Joining → Registered → Serving).
    #[test]
    fn join_existing_mode_fails_closed() {
        let dir = std::env::temp_dir().join(format!(
            "kv9-join-mode-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);

        // Constructor validation: self in the voter set / empty set refused.
        assert!(Bootstrap::join_existing_at(N1, vec![N1, N2, N3], cid("a"), 1, &dir).is_err());
        assert!(Bootstrap::join_existing_at(NodeId(4), Vec::new(), cid("a"), 1, &dir).is_err());

        let mut b =
            Bootstrap::join_existing_at(NodeId(4), vec![N1, N2, N3], cid("a"), 9, &dir).unwrap();
        assert!(b.is_joining_mode());
        assert_eq!(b.bootstrap_fingerprint(), Some(9));

        // A joiner can NEVER attest bootstrap quorum — not even with a full
        // quorum of positive answers, not even via the bare event.
        assert!(b.discovered_uninitialized(&[N1, N2, N3]).is_err());
        assert!(b.on_event(BootstrapEvent::FoundUninitialized).is_err());
        assert_eq!(b.state().name(), "Discovering");

        // The WRONG cluster's initialized answer is refused (typed), state holds.
        assert!(b.on_event(found("b")).is_err());
        assert_eq!(b.state().name(), "Discovering");

        // Control: the EXPECTED cluster admits it down the join path.
        assert_eq!(b.on_event(found("a")).unwrap().name(), "Joining");
        assert_eq!(b.cluster_id(), Some(cid("a")));
        assert_eq!(b.on_event(BootstrapEvent::Registered).unwrap().name(), "Serving");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
