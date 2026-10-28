//! Column-family layout (DESIGN §6.2).
//!
//! Three logical column families, a layout inherited from TiKV. Raw user data lives in
//! `Default` only; `Lock` and `Write` are retained because their codes are part of the
//! WAL/checkpoint formats (and the catalog uses atomic cross-CF batches), not because any
//! transaction path writes them — transactions were removed from the design (§1).

/// The column families. Codes are persisted; never renumber.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColumnFamily {
    /// User data; the sole CF raw keyspaces write (DESIGN §3.4).
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
