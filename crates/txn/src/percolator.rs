//! Percolator 2PC executor for `txn` keyspaces (DESIGN §9.1).
//!
//! Standard TiKV/Percolator model over the `default/lock/write` MVCC layout:
//! - `start_ts` from the group's oracle; snapshot reads see versions ≤ start_ts.
//! - **Prewrite** locks the primary then secondaries (intents in `lock`, data in `default`).
//! - **Commit** takes `commit_ts`, commits the primary (atomic point) then secondaries
//!   lazily (`lock`→`write`).
//! - Cross-region transactions are supported **within one txn group**; a transaction
//!   whose keys resolve to two txn groups is **rejected at begin** (DESIGN §3.6).

use kv9_common::{Error, KeyspaceId, Result, TimeStamp, TxnGroupId, UserKey, Value};

/// A single mutation in a transaction's write set (DESIGN §9.1).
#[derive(Debug, Clone)]
pub enum TxnMutation {
    Put { key: UserKey, value: Value },
    Delete { key: UserKey },
}

/// The context resolved for one transaction (DESIGN §3.6, §9.1).
#[derive(Debug, Clone)]
pub struct TxnContext {
    pub start_ts: TimeStamp,
    /// The single txn group all keys must belong to (confinement — DESIGN §3.6).
    pub txn_group: TxnGroupId,
    /// The primary key that is the atomic commit point (DESIGN §9.1).
    pub primary: UserKey,
}

/// The **txn-group confinement check** (DESIGN §3.6, §9.1).
///
/// Given the txn group of every keyspace a transaction touches, verify they all resolve
/// to a single group. Returns [`Error::CrossTxnGroup`] on the first mismatch. This is
/// what lets each group use its own sharded TSO timeline (DESIGN §8.1) without any
/// cross-group timestamp comparison.
pub fn check_txn_group_confinement<I>(groups: I) -> Result<TxnGroupId>
where
    I: IntoIterator<Item = TxnGroupId>,
{
    let mut chosen: Option<TxnGroupId> = None;
    for g in groups {
        match chosen {
            None => chosen = Some(g),
            Some(c) if c != g => return Err(Error::CrossTxnGroup { a: c, b: g }),
            Some(_) => {}
        }
    }
    chosen.ok_or_else(|| Error::WriteConflict("empty transaction key set".into()))
}

/// Resolve, per keyspace touched, the txn group and confinement in one step
/// (DESIGN §3.6). `lookup` maps a keyspace to its declared txn group (from the catalog).
pub fn resolve_confined_group<F>(keyspaces: &[KeyspaceId], lookup: F) -> Result<TxnGroupId>
where
    F: Fn(KeyspaceId) -> Result<TxnGroupId>,
{
    let groups: Result<Vec<TxnGroupId>> = keyspaces.iter().map(|ks| lookup(*ks)).collect();
    check_txn_group_confinement(groups?)
}

/// The Percolator 2PC executor (DESIGN §9.1). Skeleton: signatures are real; bodies
/// return `NotImplemented` until the engine/raft write path lands in M1/M2.
pub struct PercolatorExecutor;

impl PercolatorExecutor {
    pub fn new() -> Self {
        PercolatorExecutor
    }

    /// Snapshot read of a key at `start_ts` (DESIGN §9.1).
    pub fn get(&self, _ctx: &TxnContext, _key: &[u8]) -> Result<Option<Value>> {
        Err(Error::NotImplemented("PercolatorExecutor::get"))
    }

    /// Prewrite: lock primary then secondaries, write intents (DESIGN §9.1).
    pub fn prewrite(&self, _ctx: &TxnContext, _mutations: &[TxnMutation]) -> Result<()> {
        Err(Error::NotImplemented("PercolatorExecutor::prewrite"))
    }

    /// Commit: commit the primary (atomic point) then secondaries lazily (DESIGN §9.1).
    pub fn commit(&self, _ctx: &TxnContext, _commit_ts: TimeStamp, _keys: &[UserKey]) -> Result<()> {
        Err(Error::NotImplemented("PercolatorExecutor::commit"))
    }

    /// ResolveLock: clean up locks after a coordinator failure (DESIGN §9.1).
    pub fn resolve_lock(&self, _start_ts: TimeStamp, _commit_ts: Option<TimeStamp>) -> Result<()> {
        Err(Error::NotImplemented("PercolatorExecutor::resolve_lock"))
    }
}

impl Default for PercolatorExecutor {
    fn default() -> Self {
        Self::new()
    }
}
