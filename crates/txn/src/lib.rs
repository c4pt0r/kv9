//! # kv9-txn
//!
//! Transaction executors (DESIGN §9): the Percolator 2PC executor for `txn` keyspaces
//! (with the txn-group confinement check) and the raw executor for `raw` keyspaces.

pub mod identity;
pub mod percolator;
pub mod raw;

pub use identity::{
    CommitAuthority, QualifiedKey, TimelineGeneration, TxnAuthorityProvider, TxnDescriptor, TxnId,
    TxnStatus,
};
pub use percolator::{
    check_txn_group_confinement, resolve_confined_group, PercolatorExecutor, TxnContext,
    TxnMutation,
};
pub use raw::{LeaderRead, RawExecutor, RawWriteOptions};
