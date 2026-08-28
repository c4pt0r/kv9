//! Node configuration (DESIGN §11, §5.2, §6.4).
//!
//! Populated from CLI flags (`--join`, `--data-dir`, `--addr`) by the `kv9` binary and
//! passed into node assembly. Note: txn groups are **not** node configuration — a txn
//! group is a TSO shard *inside a keyspace* (DESIGN §3.6), declared per keyspace at
//! `CREATE KEYSPACE ... [, txn_group = <g>]` (§3.2) and stored in the `txn_groups`
//! catalog table keyed by `keyspace_id`.

use serde::{Deserialize, Serialize};

/// Top-level configuration for one `kv9` process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// gRPC/serving address this node binds (DESIGN §11 `--addr`).
    pub addr: String,
    /// Local data directory for engine + raft state (DESIGN §11 `--data-dir`).
    pub data_dir: String,
    /// Seed / join set: peers to contact on start to discover the cluster
    /// (DESIGN §5.2 `--join`).
    pub join: Vec<String>,
    /// Number of WAL streams in this node's WAL pool (DESIGN §6.4). More streams =
    /// more write parallelism, less fsync amortization per stream.
    pub wal_streams: usize,
    /// Replication factor for new regions (DESIGN §3.3, default 3).
    pub replication_factor: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            addr: "127.0.0.1:20160".to_string(),
            data_dir: "./kv9-data".to_string(),
            join: Vec::new(),
            wal_streams: 1,
            replication_factor: 3,
        }
    }
}

impl Config {
    /// Basic validation of a config (DESIGN §11).
    pub fn validate(&self) -> crate::error::Result<()> {
        if self.wal_streams == 0 {
            return Err(crate::error::Error::Config(
                "wal_streams must be >= 1".into(),
            ));
        }
        if self.replication_factor == 0 {
            return Err(crate::error::Error::Config(
                "replication_factor must be >= 1".into(),
            ));
        }
        Ok(())
    }
}
