//! Replicated metadata / data commands (Phase-1 spine; ROADMAP Phase 1).
//!
//! A [`Command`] is the logical payload of a raft log entry. The leader `propose`s a
//! command; once committed it is handed to the region/meta apply loop as a
//! [`crate::CommittedEntry`] and applied into the state machine ([`crate::StateMachine`]).
//!
//! Phase-1 keeps the command shape small and pure-Rust. The serialization here is a
//! deliberately trivial, self-describing framing (no `serde`/protobuf dependency yet) so
//! the workspace stays native-build-free; `// TODO(phase1): back by openraft` marks
//! where a real codec + consensus plug in.

use kv9_engine::{ColumnFamily, Mutation, WriteBatch};

/// A single put/delete carried by a [`Command`] (mirrors [`kv9_engine::Mutation`] in a
/// serialization-friendly, owned form).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KvOp {
    Put { cf: u8, key: Vec<u8>, value: Vec<u8> },
    Delete { cf: u8, key: Vec<u8> },
}

/// The set of commands the metadata-plane raft group replicates (ROADMAP Phase 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// A single raw put into the state machine's KV (the `propose(put)→apply→get`
    /// round-trip of the first Phase-1 task).
    Put {
        cf: u8,
        key: Vec<u8>,
        value: Vec<u8>,
    },
    /// A committed **catalog transaction**: the atomic multi-table write batch produced
    /// by a `meta::MetaTxn` (e.g. `CreateKeyspace` inserts `keyspaces` + `txn_groups` +
    /// their index rows). Applied as one atomic engine write (METADATA-CATALOG §5).
    CatalogTxn { ops: Vec<KvOp> },
    /// A membership / configuration change (add/remove peer). Phase-1 records the intent;
    /// real conf-change replication arrives with openraft.
    ConfChange { add: bool, node: u64 },
    /// A no-op (leader-establish barrier / heartbeat filler).
    Noop,
}

impl Command {
    /// Build a [`Command::CatalogTxn`] from an engine [`WriteBatch`] (METADATA-CATALOG §5).
    pub fn from_batch(batch: &WriteBatch) -> Command {
        let ops = batch
            .mutations()
            .iter()
            .map(|m| match m {
                Mutation::Put { cf, key, value } => KvOp::Put {
                    cf: cf_code(*cf),
                    key: key.clone(),
                    value: value.clone(),
                },
                Mutation::Delete { cf, key } => KvOp::Delete {
                    cf: cf_code(*cf),
                    key: key.clone(),
                },
            })
            .collect();
        Command::CatalogTxn { ops }
    }

    /// Lower this command's KV effect into a [`WriteBatch`] for the state machine to
    /// apply (Phase-1). `ConfChange`/`Noop` produce an empty batch.
    pub fn to_write_batch(&self) -> WriteBatch {
        let mut wb = WriteBatch::new();
        match self {
            Command::Put { cf, key, value } => {
                wb.put(cf_from_code(*cf), key.clone(), value.clone());
            }
            Command::CatalogTxn { ops } => {
                for op in ops {
                    match op {
                        KvOp::Put { cf, key, value } => {
                            wb.put(cf_from_code(*cf), key.clone(), value.clone());
                        }
                        KvOp::Delete { cf, key } => {
                            wb.delete(cf_from_code(*cf), key.clone());
                        }
                    }
                }
            }
            Command::ConfChange { .. } | Command::Noop => {}
        }
        wb
    }

    /// Encode to opaque bytes for [`crate::RaftGroup::propose`] (Phase-1 framing).
    ///
    /// Phase-1 stub: a real impl uses a versioned codec (never panics on unknown —
    /// ROADMAP cross-cutting). `// TODO(phase1): back by openraft` — the consensus layer
    /// owns the on-wire entry format.
    pub fn encode(&self) -> Vec<u8> {
        unimplemented!("Command::encode — versioned entry codec (ROADMAP Phase 1)")
    }

    /// Decode from opaque committed-entry bytes (inverse of [`Command::encode`]).
    pub fn decode(_bytes: &[u8]) -> kv9_common::Result<Command> {
        unimplemented!("Command::decode — versioned entry codec (ROADMAP Phase 1)")
    }
}

/// Stable numeric code for a column family (keeps `Command` free of a direct
/// dependency on the CF enum's representation in serialized form).
pub fn cf_code(cf: ColumnFamily) -> u8 {
    match cf {
        ColumnFamily::Default => 0,
        ColumnFamily::Lock => 1,
        ColumnFamily::Write => 2,
    }
}

/// Inverse of [`cf_code`]; unknown codes default to `Default` (never panic on unknown).
pub fn cf_from_code(code: u8) -> ColumnFamily {
    match code {
        1 => ColumnFamily::Lock,
        2 => ColumnFamily::Write,
        _ => ColumnFamily::Default,
    }
}
