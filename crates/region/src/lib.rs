//! # kv9-region
//!
//! The region layer (DESIGN §3.3, §6, §9): `Region` + epoch, `RegionRouter`,
//! throughput-aware split/merge hooks, and the sharded WAL pool (DESIGN §6.4).

pub mod region;
pub mod router;
pub mod split_merge;
pub mod wal;

pub use region::{Peer, Region, RegionEpoch};
pub use router::RegionRouter;
pub use split_merge::{can_merge, evaluate_split, validate_split_key, RegionLoad, SplitDecision};
pub use wal::{AssignmentStrategy, WalAppend, WalPool, WalStream, WalStreamId};
