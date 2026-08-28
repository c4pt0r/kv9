//! Crate-wide error type (DESIGN §11). `thiserror`-based, shared across all kv9 crates.

use crate::ids::{KeyspaceId, NodeId, RegionId, TxnGroupId};

/// The unified kv9 result alias.
pub type Result<T> = std::result::Result<T, Error>;

/// All error conditions kv9 surfaces. Variants map to the design's invariants so
/// callers can react (retry after routing refresh, reject cross-group txn, etc.).
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A keyspace id was outside the encodable 3-byte range (DESIGN §3.4, §13 principle 4).
    #[error("keyspace id {0} exceeds max {max} (3-byte width)", max = crate::ids::KeyspaceId::MAX)]
    KeyspaceIdOutOfRange(u32),

    /// The physical key prefix was malformed / too short to decode (DESIGN §3.4).
    #[error("malformed encoded key: {0}")]
    MalformedKey(String),

    /// Unknown mode byte in an encoded key (expected `t`/`r`/`s`) — DESIGN §3.4.
    #[error("invalid key mode byte: {0:#x}")]
    InvalidKeyMode(u8),

    /// The referenced keyspace does not exist in the catalog (DESIGN §5.1).
    #[error("keyspace {0:?} not found")]
    KeyspaceNotFound(KeyspaceId),

    /// The request's API type did not match the keyspace declaration (DESIGN §8, §10).
    #[error("api type mismatch for keyspace {keyspace:?}")]
    ApiTypeMismatch { keyspace: KeyspaceId },

    /// No region owns the requested key, or routing is stale (DESIGN §6.1).
    #[error("no region found for key")]
    RegionNotFound,

    /// The request carried a stale region epoch; retry after routing refresh
    /// (DESIGN §6.1, TiKV semantics).
    #[error("stale region epoch for region {region:?}")]
    StaleEpoch { region: RegionId },

    /// A split point would cross a keyspace boundary (DESIGN §3.3, §13 principle 3 invariant).
    #[error("split key crosses keyspace boundary")]
    SplitCrossesKeyspace,

    /// A transaction's keys resolved to more than one txn group; rejected at begin
    /// time by the confinement invariant (DESIGN §3.6, §9.1).
    #[error("transaction crosses txn groups {a:?} and {b:?}")]
    CrossTxnGroup { a: TxnGroupId, b: TxnGroupId },

    /// A Percolator write conflict / lock conflict (DESIGN §9.1).
    #[error("write conflict: {0}")]
    WriteConflict(String),

    /// A key was locked by another transaction (DESIGN §9.1).
    #[error("key is locked")]
    KeyIsLocked,

    /// The TSO refused to serve (lease not confirmed, clock regression, etc.) — DESIGN §8.1.
    #[error("timestamp oracle unavailable: {0}")]
    TsoUnavailable(String),

    /// The metadata plane is not yet ready to serve (bootstrap / freshness watermark)
    /// — DESIGN §5.2, §5.4.
    #[error("metadata not ready: {0}")]
    MetaNotReady(String),

    /// Raft-level error from the consensus layer (DESIGN §6.1).
    #[error("raft error: {0}")]
    Raft(String),

    /// A range or batch spans more than one region, so a single `RequestContext` (which
    /// authorises exactly one region at one epoch) cannot cover it.
    ///
    /// Deliberately distinct from [`Error::StaleEpoch`] and `SplitCrossesKeyspace`: the
    /// caller's epoch may be perfectly current, and nothing is crossing a keyspace. The
    /// correct client reaction is to split the request by region, not to refresh an epoch.
    #[error("range or batch crosses a region boundary")]
    RangeCrossesRegion,

    /// A chunked range delete committed some chunks and then failed.
    ///
    /// This must not surface as a plain error: a plain error reads as "nothing happened",
    /// and here something did. The fields say exactly how far it got.
    /// `cause` is for humans and diagnostics only. The machine-readable contract is the
    /// three numbers, carried as response metadata; a client that parsed this string would
    /// break the moment someone rewords it.
    #[error("range delete stopped after {committed_chunks} committed chunk(s): {cause}")]
    PartialDeleteRange {
        committed_chunks: u64,
        last_applied_term: u64,
        last_applied_index: u64,
        cause: String,
    },

    /// This node does not currently lead the region, so it may not serve the request.
    ///
    /// Deliberately a distinct variant rather than a `Raft(String)`: a client's correct
    /// reaction is to retry against `leader`, and deciding that by matching on message
    /// text is the kind of check that silently stops working when the wording changes.
    /// The hint is `None` when this node does not yet know who leads (e.g. mid-election).
    /// The hint is `Option<NodeId>`, matching `RaftPeer::leader_hint()` and
    /// `NodeStatus::leader_id`, so that promoting an internal hint into this error is a
    /// move rather than a translation. A second id representation would leave clients
    /// handling two shapes of "you reached the wrong node".
    #[error("not leader{}", match .leader {
        Some(id) => format!("; try node {}", id.0),
        None => String::new(),
    })]
    NotLeader { leader: Option<NodeId> },

    /// An object store was asked to write a second, different object under a key it already
    /// holds (DESIGN §6.5).
    ///
    /// Distinct from [`Error::Engine`] because it is not a storage malfunction: the store is
    /// healthy and is refusing on purpose. Objects are immutable and write-once, so this means
    /// one file-id was assigned to two distinct objects; overwriting would corrupt bytes that a
    /// committed manifest already references. Re-writing *identical* content is not this error —
    /// retransmission is expected and succeeds. `key` is a file-id, never user data.
    #[error("object {key} already exists with different content")]
    ObjectContentMismatch { key: String },

    /// Storage engine error (DESIGN §6.2).
    #[error("engine error: {0}")]
    Engine(String),

    /// Invalid configuration (DESIGN §11).
    #[error("config error: {0}")]
    Config(String),

    /// A code path that is defined in the design but not yet implemented in the skeleton.
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}
