//! # kv9-raw
//!
//! The raw (direct-KV) executor (DESIGN §9): stateless planners that turn public
//! raw operations into engine [`kv9_engine::WriteBatch`] plans, plus the leader-only
//! read helpers. A plan is applied by the Raft driver as ONE fenced command, which is
//! what makes a batch atomic within its range (DESIGN §9.1).

pub mod raw;

pub use raw::{LeaderRead, RawExecutor, RawMutation, RawWriteOptions};
