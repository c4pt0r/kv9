//! # kv9-txn
//!
//! Transaction executors (DESIGN §9): the Percolator 2PC executor for `txn` keyspaces
//! (with the txn-group confinement check) and the raw executor for `raw` keyspaces.

pub mod percolator;
pub mod raw;

pub use percolator::{
    check_txn_group_confinement, resolve_confined_group, PercolatorExecutor, TxnContext,
    TxnMutation,
};
pub use raw::{RawExecutor, RawWriteOptions};
