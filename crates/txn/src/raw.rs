//! Raw (direct-KV) executor for `raw` keyspaces (DESIGN §9.2).
//!
//! `RawPut/RawGet/RawDelete/RawScan/RawBatchGet`, optional TTL, optional causal
//! timestamps for ordering without full transactions. No locks, no 2PC.

use kv9_common::{Error, Result, UserKey, Value};

/// Optional per-key metadata for raw writes (DESIGN §9.2).
#[derive(Debug, Clone, Copy, Default)]
pub struct RawWriteOptions {
    /// TTL in seconds (`None` = no expiry).
    pub ttl_secs: Option<u64>,
    /// Optional causal timestamp (monotonic per key) for ordering (DESIGN §9.2).
    pub causal_ts: Option<u64>,
}

/// The raw executor (DESIGN §9.2). Skeleton: signatures are real; bodies return
/// `NotImplemented` until the engine write path lands in M1.
pub struct RawExecutor;

impl RawExecutor {
    pub fn new() -> Self {
        RawExecutor
    }

    pub fn put(&self, _key: UserKey, _value: Value, _opts: RawWriteOptions) -> Result<()> {
        Err(Error::NotImplemented("RawExecutor::put"))
    }

    pub fn get(&self, _key: &[u8]) -> Result<Option<Value>> {
        Err(Error::NotImplemented("RawExecutor::get"))
    }

    pub fn delete(&self, _key: &[u8]) -> Result<()> {
        Err(Error::NotImplemented("RawExecutor::delete"))
    }

    pub fn scan(&self, _start: &[u8], _end: &[u8], _limit: usize) -> Result<Vec<(UserKey, Value)>> {
        Err(Error::NotImplemented("RawExecutor::scan"))
    }

    pub fn delete_range(&self, _start: &[u8], _end: &[u8]) -> Result<()> {
        Err(Error::NotImplemented("RawExecutor::delete_range"))
    }
}

impl Default for RawExecutor {
    fn default() -> Self {
        Self::new()
    }
}
