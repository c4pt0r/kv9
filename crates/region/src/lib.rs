//! # kv9-region
//!
//! The region layer (DESIGN §3.3, §6, §9): `Region` + epoch, `RegionRouter`,
//! throughput-aware split/merge hooks, and the sharded WAL pool (DESIGN §6.4).

pub mod manifest;
pub mod region;
pub mod router;
pub mod wal;

pub use manifest::{ManifestAttempt, ManifestProposalState, ManifestSeam, SettledManifest};
pub use region::{Peer, Region, RegionEpoch};
pub use router::RegionRouter;
pub use wal::{AssignmentStrategy, WalAppend, WalPool, WalStream, WalStreamId};
