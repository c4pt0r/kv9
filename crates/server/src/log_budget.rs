//! Inventory observes log debt, including quarantined and orphan layouts. It
//! grants neither recovery nor deletion authority. One budget serves the node.
use kv9_common::log_budget::headroom::{OsDiskSpaceProbe, DEFAULT_MIN_FREE_BYTES};
use kv9_common::log_budget::rewrite::DEFAULT_REWRITE_BYTES;
use kv9_common::log_budget::{tree_bytes, LogBudget, DEFAULT_LOG_BYTES};
use kv9_common::{Error, Result};
use kv9_engine::WalEngine;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

pub(crate) fn limit_from_env() -> Result<u64> {
    parse(std::env::var("KV9_WAL_RAFT_BYTES"))
}
pub(crate) fn headroom_from_env() -> Result<u64> {
    parse_field(
        std::env::var("KV9_MIN_FREE_DISK_BYTES"),
        "KV9_MIN_FREE_DISK_BYTES",
        DEFAULT_MIN_FREE_BYTES,
    )
}
pub(crate) fn rewrite_from_env() -> Result<u64> {
    parse_field(
        std::env::var("KV9_RAFT_REWRITE_BYTES"),
        "KV9_RAFT_REWRITE_BYTES",
        DEFAULT_REWRITE_BYTES,
    )
}
fn parse(value: std::result::Result<String, std::env::VarError>) -> Result<u64> {
    parse_field(value, "KV9_WAL_RAFT_BYTES", DEFAULT_LOG_BYTES)
}
fn parse_field(
    value: std::result::Result<String, std::env::VarError>,
    name: &str,
    default: u64,
) -> Result<u64> {
    let invalid = || Error::Config(format!("{name} must be a decimal u64 of at least 16 MiB"));
    match value {
        Err(std::env::VarError::NotPresent) => Ok(default),
        Ok(v) if !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()) => {
            let bytes: u64 = v.parse().map_err(|_| invalid())?;
            if bytes < 16 * 1024 * 1024 {
                return Err(invalid());
            }
            Ok(bytes)
        }
        _ => Err(invalid()),
    }
}

pub(crate) fn recover(
    directory: &Path,
    limit: u64,
    minimum_free_bytes: u64,
    rewrite_bytes: u64,
) -> Result<Arc<LogBudget>> {
    let probe = Arc::new(OsDiskSpaceProbe::open(directory).map_err(io)?);
    let budget = Arc::new(
        LogBudget::with_disk_headroom(limit, minimum_free_bytes, probe)?
            .with_rewrite_limit(rewrite_bytes)?,
    );
    inventory(directory, budget)
}
fn inventory(directory: &Path, budget: Arc<LogBudget>) -> Result<Arc<LogBudget>> {
    let mut todo = vec![directory.to_path_buf()];
    let mut engines = BTreeSet::from([directory.join("catalog.wal")]);
    while let Some(directory) = todo.pop() {
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(io(e)),
        };
        for entry in entries {
            let entry = entry.map_err(io)?;
            let path = entry.path();
            let kind = entry.file_type().map_err(io)?;
            budget.check_path(&path)?;
            if entry.file_name() == "raft" {
                budget.track(&path, budget.tree_bytes(&path)?)?;
            } else if let Some(wal) = wal_family(&path) {
                engines.insert(wal);
            } else if kind.is_dir() {
                todo.push(path);
            }
        }
    }
    for path in engines {
        budget.track(&path, WalEngine::log_disk_bytes_on_budget(&path, &budget)?)?;
    }
    Ok(budget)
}

// Recover the owner path even when only a failed migration/rotation temporary
// remains. These are the same layouts measured by WalEngine::log_disk_bytes.
fn wal_family(path: &Path) -> Option<std::path::PathBuf> {
    let extension = path.extension()?;
    if ["wal", "segments", "checkpoint", "migration"]
        .iter()
        .any(|ext| extension == *ext)
    {
        return Some(path.with_extension("wal"));
    }
    if extension == "tmp" {
        let inner = path.with_extension("");
        if inner.extension().is_some_and(|extension| {
            ["checkpoint", "tail", "upgrade", "topology"]
                .iter()
                .any(|ext| extension == *ext)
        }) {
            return Some(inner.with_extension("wal"));
        }
    }
    None
}

pub(crate) fn reclaimed(directory: &Path, budget: &Arc<LogBudget>) -> Result<()> {
    for debt in budget.sources_under(directory)? {
        let bytes = if debt.path().extension().is_some_and(|e| e == "wal") {
            WalEngine::log_disk_bytes(debt.path())?
        } else {
            tree_bytes(debt.path())?
        };
        debt.reconcile(bytes)?;
        if bytes == 0 {
            debt.applied_through(u64::MAX);
            debt.forget_empty()?;
        }
    }
    Ok(())
}
fn io(e: std::io::Error) -> Error {
    Error::Engine(format!("log inventory: {e}"))
}

#[cfg(test)]
mod tests;
