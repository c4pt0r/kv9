//! Crate-wide error type (DESIGN §11). `thiserror`-based, shared across all kv9 crates.

use crate::ids::{KeyspaceId, RegionId, TxnGroupId};

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
