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

    /// Encode to opaque bytes for [`crate::RaftGroup::propose`].
    ///
    /// Layout (all integers big-endian):
    ///
    /// ```text
    /// version:u8 = ENTRY_VERSION | tag:u8 | payload
    /// Put        (tag 1): cf:u8 | key_len:u32 | key | value_len:u32 | value
    /// CatalogTxn (tag 2): op_count:u32 | ops…   op: op_tag:u8 (1 put, 2 delete) |
    ///                     cf:u8 | key_len:u32 | key [| value_len:u32 | value]
    /// ConfChange (tag 3): add:u8 | node:u64
    /// Noop       (tag 4): (empty)
    /// ```
    ///
    /// The version byte gates format evolution: decoders reject unknown versions and
    /// unknown tags with a typed error, never panic (ROADMAP cross-cutting; DESIGN
    /// principle "forward-compatible formats, never panic on the unknown").
    /// `// TODO(phase1): back by openraft` — once the consensus layer owns the on-wire
    /// entry format this codec becomes the app-payload layer inside it.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(ENTRY_VERSION);
        match self {
            Command::Put { cf, key, value } => {
                out.push(TAG_PUT);
                out.push(*cf);
                put_bytes(&mut out, key);
                put_bytes(&mut out, value);
            }
            Command::CatalogTxn { ops } => {
                out.push(TAG_CATALOG_TXN);
                out.extend_from_slice(&(ops.len() as u32).to_be_bytes());
                for op in ops {
                    match op {
                        KvOp::Put { cf, key, value } => {
                            out.push(OP_PUT);
                            out.push(*cf);
                            put_bytes(&mut out, key);
                            put_bytes(&mut out, value);
                        }
                        KvOp::Delete { cf, key } => {
                            out.push(OP_DELETE);
                            out.push(*cf);
                            put_bytes(&mut out, key);
                        }
                    }
                }
            }
            Command::ConfChange { add, node } => {
                out.push(TAG_CONF_CHANGE);
                out.push(u8::from(*add));
                out.extend_from_slice(&node.to_be_bytes());
            }
            Command::Noop => out.push(TAG_NOOP),
        }
        out
    }

    /// Decode from opaque committed-entry bytes (inverse of [`Command::encode`]).
    ///
    /// Unknown versions/tags and truncated payloads return a typed error — a mixed-version
    /// cluster must surface, not corrupt (DESIGN principle 12).
    pub fn decode(bytes: &[u8]) -> kv9_common::Result<Command> {
        let mut r = Reader { buf: bytes };
        let version = r.u8()?;
        if version != ENTRY_VERSION {
            return Err(kv9_common::Error::Raft(format!(
                "unknown command entry version {version} (this binary speaks {ENTRY_VERSION})"
            )));
        }
        let cmd = match r.u8()? {
            TAG_PUT => Command::Put {
                cf: r.u8()?,
                key: r.bytes()?,
                value: r.bytes()?,
            },
            TAG_CATALOG_TXN => {
                let count = r.u32()? as usize;
                // Cap preallocation by what the buffer could actually hold (defensive
                // against a corrupt count; each op is ≥ 7 bytes).
                let mut ops = Vec::with_capacity(count.min(r.buf.len() / 7 + 1));
                for _ in 0..count {
                    let op = match r.u8()? {
                        OP_PUT => KvOp::Put {
                            cf: r.u8()?,
                            key: r.bytes()?,
                            value: r.bytes()?,
                        },
                        OP_DELETE => KvOp::Delete {
                            cf: r.u8()?,
                            key: r.bytes()?,
                        },
                        other => {
                            return Err(kv9_common::Error::Raft(format!(
                                "unknown CatalogTxn op tag {other}"
                            )))
                        }
                    };
                    ops.push(op);
                }
                Command::CatalogTxn { ops }
            }
            TAG_CONF_CHANGE => Command::ConfChange {
                add: r.u8()? != 0,
                node: r.u64()?,
            },
            TAG_NOOP => Command::Noop,
            other => {
                return Err(kv9_common::Error::Raft(format!(
                    "unknown command tag {other}"
                )))
            }
        };
        if !r.buf.is_empty() {
            return Err(kv9_common::Error::Raft(format!(
                "{} trailing bytes after command",
                r.buf.len()
            )));
        }
        Ok(cmd)
    }
}

/// Entry-format version this binary writes (bumped on layout change; decoders reject
/// versions they don't know rather than guessing).
pub const ENTRY_VERSION: u8 = 1;

const TAG_PUT: u8 = 1;
const TAG_CATALOG_TXN: u8 = 2;
const TAG_CONF_CHANGE: u8 = 3;
const TAG_NOOP: u8 = 4;
const OP_PUT: u8 = 1;
const OP_DELETE: u8 = 2;

fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len() as u32).to_be_bytes());
    out.extend_from_slice(b);
}

/// A bounds-checked cursor over the entry bytes; every read is total (no panics on
/// truncated input).
struct Reader<'a> {
    buf: &'a [u8],
}

impl<'a> Reader<'a> {
    fn u8(&mut self) -> kv9_common::Result<u8> {
        let (&b, rest) = self
            .buf
            .split_first()
            .ok_or_else(|| kv9_common::Error::Raft("truncated command entry".into()))?;
        self.buf = rest;
        Ok(b)
    }

    fn take(&mut self, n: usize) -> kv9_common::Result<&'a [u8]> {
        if self.buf.len() < n {
            return Err(kv9_common::Error::Raft(
                "truncated command entry".into(),
            ));
        }
        let (head, rest) = self.buf.split_at(n);
        self.buf = rest;
        Ok(head)
    }

    fn u32(&mut self) -> kv9_common::Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> kv9_common::Result<u64> {
        let b = self.take(8)?;
        let mut arr = [0u8; 8];
        arr.copy_from_slice(b);
        Ok(u64::from_be_bytes(arr))
    }

    fn bytes(&mut self) -> kv9_common::Result<Vec<u8>> {
        let len = self.u32()? as usize;
        Ok(self.take(len)?.to_vec())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(cmd: &Command) {
        let bytes = cmd.encode();
        assert_eq!(&Command::decode(&bytes).unwrap(), cmd);
    }

    #[test]
    fn encode_decode_roundtrip_all_variants() {
        roundtrip(&Command::Put {
            cf: 0,
            key: b"k".to_vec(),
            value: b"v".to_vec(),
        });
        roundtrip(&Command::CatalogTxn { ops: Vec::new() });
        roundtrip(&Command::CatalogTxn {
            ops: vec![
                KvOp::Put {
                    cf: 1,
                    key: vec![0x00, 0xff, 0x00],
                    value: Vec::new(),
                },
                KvOp::Delete {
                    cf: 2,
                    key: b"gone".to_vec(),
                },
            ],
        });
        roundtrip(&Command::ConfChange {
            add: true,
            node: u64::MAX,
        });
        roundtrip(&Command::ConfChange {
            add: false,
            node: 0,
        });
        roundtrip(&Command::Noop);
    }

    #[test]
    fn decode_rejects_unknown_version() {
        let mut bytes = Command::Noop.encode();
        bytes[0] = ENTRY_VERSION + 1;
        assert!(Command::decode(&bytes).is_err());
    }

    #[test]
    fn decode_rejects_unknown_tag_and_op_tag() {
        assert!(Command::decode(&[ENTRY_VERSION, 0xEE]).is_err());
        // CatalogTxn claiming one op with an unknown op tag.
        let bytes = vec![ENTRY_VERSION, TAG_CATALOG_TXN, 0, 0, 0, 1, 0xEE];
        assert!(Command::decode(&bytes).is_err());
    }

    #[test]
    fn decode_rejects_truncation_and_trailing_bytes() {
        let full = Command::Put {
            cf: 0,
            key: b"key".to_vec(),
            value: b"value".to_vec(),
        }
        .encode();
        // Every strict prefix must fail cleanly (no panic, no partial success).
        for cut in 0..full.len() {
            assert!(Command::decode(&full[..cut]).is_err(), "prefix len {cut}");
        }
        // Trailing garbage must be rejected too.
        let mut padded = full.clone();
        padded.push(0);
        assert!(Command::decode(&padded).is_err());
        assert!(Command::decode(&[]).is_err());
    }

    #[test]
    fn decode_rejects_corrupt_op_count() {
        // Claims 1000 ops but carries none — must error, not hang or over-allocate.
        let bytes = vec![ENTRY_VERSION, TAG_CATALOG_TXN, 0, 0, 3, 0xE8];
        assert!(Command::decode(&bytes).is_err());
    }
}
