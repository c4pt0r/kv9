//! MVCC column-family layout for `txn` keyspaces (DESIGN §8.1, §6.2).
//!
//! Borrowed from TiKV/Percolator: three logical column families. `raw` keyspaces use
//! only `Default`.

/// The three column families of the Percolator MVCC layout (DESIGN §8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColumnFamily {
    /// User data, keyed `user_key + inverted(commit_ts)` for `txn` keyspaces
    /// (DESIGN §3.4). Also the sole CF for `raw` keyspaces.
    Default,
    /// In-flight transaction intents / locks written during prewrite (DESIGN §8.1).
    Lock,
    /// Committed-version index; `lock` → `write` on commit (DESIGN §8.1).
    Write,
}

impl ColumnFamily {
    /// Stable string name of the column family.
    pub fn name(self) -> &'static str {
        match self {
            ColumnFamily::Default => "default",
            ColumnFamily::Lock => "lock",
            ColumnFamily::Write => "write",
        }
    }

    /// All column families, in a stable order.
    pub const ALL: [ColumnFamily; 3] = [
        ColumnFamily::Default,
        ColumnFamily::Lock,
        ColumnFamily::Write,
    ];
}
