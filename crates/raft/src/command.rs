//! Replicated metadata / data commands (Phase-1 spine; ROADMAP Phase 1).
//!
//! A [`Command`] is the logical payload of a raft log entry. The leader `propose`s a
//! command; once committed it is handed to the region/meta apply loop as a
//! [`crate::CommittedEntry`] and applied into the state machine ([`crate::StateMachine`]).
//!
//! Phase-1 keeps the command shape small and pure-Rust: a versioned,
//! self-describing framing (no serde; raft-rs's protobuf is used only for the
//! raft protocol layer, not for command payloads).

use kv9_engine::{ColumnFamily, Mutation, WriteBatch};

/// A single put/delete carried by a [`Command`] (mirrors [`kv9_engine::Mutation`] in a
/// serialization-friendly, owned form).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KvOp {
    Put {
        cf: u8,
        key: Vec<u8>,
        value: Vec<u8>,
    },
    Delete {
        cf: u8,
        key: Vec<u8>,
    },
}

/// The region epoch a proposer expects, carried into ordered apply so every replica
/// re-checks it against the epoch established by the same log (task #48 layer 2;
/// DESIGN §6.1). Plain fields, not `kv9_region::RegionEpoch`: the dependency points
/// the other way (kv9-region depends on this crate), so the wire carries the pair and
/// the typed comparison — `RegionEpoch::is_fresh_as`, the SAME predicate the router's
/// `check_epoch` uses, so propose-side and apply-side verdicts cannot drift — happens
/// at the apply boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionFence {
    pub region_id: u64,
    pub conf_ver: u64,
    pub version: u64,
}

/// The commands a [`RegionFence`] may wrap. A separate enum — not `Box<Command>` — so
/// the two invalid envelopes are unrepresentable rather than checked: a fence cannot
/// nest inside a fence, and commands with no region semantics (`Noop`, `ConfChange`,
/// `CatalogTxn` — the catalog lives in the system keyspace, which regions never
/// split) cannot be wrapped. Split/merge admin commands join this enum in M2; each
/// addition is a deliberate whitelist entry, mirrored in `decode`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FencedInner {
    /// The fenced twin of [`Command::Write`]: a user-data batch that applies only if
    /// the region epoch at its ordered-apply position still matches the fence.
    Write { ops: Vec<KvOp> },
}

impl FencedInner {
    /// Lower the inner ops into a [`WriteBatch`] — the ACCEPTED-path twin of
    /// [`Command::to_write_batch`], reachable only after the apply loop has checked
    /// the fence (the envelope itself deliberately lowers to nothing; see
    /// [`Command::to_write_batch`]).
    pub fn to_write_batch(&self) -> WriteBatch {
        match self {
            FencedInner::Write { ops } => Command::Write { ops: ops.clone() }.to_write_batch(),
        }
    }
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
    /// A replicated **user-data write batch** (raw `put`/`delete`/`batch`; DESIGN §9.2).
    ///
    /// Same KV effect as [`Command::CatalogTxn`] at the state-machine layer (one atomic
    /// engine write), but deliberately a distinct command: user writes carry no catalog
    /// semantics and must NOT serialize behind the caller-side catalog-txn lock. A
    /// `delete_range` arrives as explicitly expanded deletes, chunked by the proposer —
    /// each chunk is atomic; the range as a whole is not (the proposer documents that).
    Write { ops: Vec<KvOp> },
    /// A membership / configuration change (add/remove peer). Phase-1 records the
    /// intent only — NOT wired to raft-rs `propose_conf_change`/`apply_conf_change`.
    /// Dynamic membership (learner → voter) ships together with raft snapshots in a
    /// later phase; Phase 1-final runs a fixed declared seed set.
    ConfChange { add: bool, node: u64 },
    /// A no-op (leader-establish barrier / heartbeat filler).
    Noop,
    /// A command wrapped with the proposer's expected region epoch (task #48 layer 2).
    /// Every replica re-checks the fence against the region epoch at this entry's
    /// ordered-apply position; on mismatch the entry is LOGICALLY rejected — it still
    /// advances the applied watermark (through the same batch-tail publication as an
    /// accepted entry), writes nothing, and the rejection is surfaced to the proposer
    /// through the proposal receipt path. The check is deterministic because the
    /// epoch itself is established by the same log every replica applies in order.
    Fenced {
        fence: RegionFence,
        inner: FencedInner,
    },
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

    /// Build a [`Command::Write`] from an engine [`WriteBatch`] — the USER-DATA
    /// twin of [`Command::from_batch`]. The two constructors exist so the
    /// choice of wire tag is made HERE, next to the semantic difference,
    /// rather than at call sites: `from_batch` = catalog transaction
    /// (serialized behind the caller's catalog-txn lock), `write_from_batch`
    /// = raw user data (no catalog semantics, no catalog lock).
    pub fn write_from_batch(batch: &WriteBatch) -> Command {
        match Command::from_batch(batch) {
            Command::CatalogTxn { ops } => Command::Write { ops },
            _ => unreachable!("from_batch always yields CatalogTxn"),
        }
    }

    /// Build a [`Command::Fenced`] user-data write from an engine [`WriteBatch`] —
    /// the fenced twin of [`Command::write_from_batch`], and the only proposer entry
    /// point for epoch-checked user writes. The fence rides the wire into ordered
    /// apply; see [`Command::Fenced`] for the apply-side contract.
    pub fn fenced_write_from_batch(fence: RegionFence, batch: &WriteBatch) -> Command {
        match Command::from_batch(batch) {
            Command::CatalogTxn { ops } => Command::Fenced {
                fence,
                inner: FencedInner::Write { ops },
            },
            _ => unreachable!("from_batch always yields CatalogTxn"),
        }
    }

    /// Lower this command's KV effect into a [`WriteBatch`] for the state machine to
    /// apply (Phase-1). `ConfChange`/`Noop` produce an empty batch.
    ///
    /// `Fenced` also produces an EMPTY batch: the apply loop must destructure the
    /// envelope, check the fence in ordered apply, and lower the inner ops itself.
    /// Lowering the envelope through this method would mean the fence was never
    /// checked, so this path yields no effect — an unchecked fence fails closed
    /// (nothing written, loud in any test that expects the write) rather than open.
    pub fn to_write_batch(&self) -> WriteBatch {
        let mut wb = WriteBatch::new();
        match self {
            Command::Put { cf, key, value } => {
                wb.put(cf_from_code(*cf), key.clone(), value.clone());
            }
            Command::CatalogTxn { ops } | Command::Write { ops } => {
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
            Command::ConfChange { .. } | Command::Noop | Command::Fenced { .. } => {}
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
    /// Write      (tag 5): op_count:u32 | ops…   (same op layout as CatalogTxn)
    /// Fenced     (tag 6): region_id:u64 | conf_ver:u64 | version:u64 |
    ///                     inner_tag:u8 | inner payload
    /// ```
    ///
    /// The version byte gates format evolution: decoders reject unknown versions and
    /// unknown tags with a typed error, never panic (ROADMAP cross-cutting; DESIGN
    /// principle "forward-compatible formats, never panic on the unknown"). This codec
    /// is the app-payload layer inside raft-rs entries (`Entry.data`).
    ///
    /// The `Fenced` inner command reuses the outer tag space but does NOT repeat the
    /// version byte — one version governs the whole frame. Its decoder accepts only
    /// the [`FencedInner`] whitelist (today `Write`); nesting and non-fenceable tags
    /// are typed errors. Future fence-shape changes take a NEW envelope tag rather
    /// than sub-versioning the fence — and note this codec is fail-closed (trailing
    /// bytes reject), so ANY new tag or layout is a decode-before-propose two-phase
    /// rollout: every replica must decode the shape before any proposer emits it.
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
            Command::CatalogTxn { ops } | Command::Write { ops } => {
                out.push(match self {
                    Command::Write { .. } => TAG_WRITE,
                    _ => TAG_CATALOG_TXN,
                });
                put_ops(&mut out, ops);
            }
            Command::ConfChange { add, node } => {
                out.push(TAG_CONF_CHANGE);
                out.push(u8::from(*add));
                out.extend_from_slice(&node.to_be_bytes());
            }
            Command::Noop => out.push(TAG_NOOP),
            Command::Fenced { fence, inner } => {
                out.push(TAG_FENCED);
                out.extend_from_slice(&fence.region_id.to_be_bytes());
                out.extend_from_slice(&fence.conf_ver.to_be_bytes());
                out.extend_from_slice(&fence.version.to_be_bytes());
                match inner {
                    FencedInner::Write { ops } => {
                        out.push(TAG_WRITE);
                        put_ops(&mut out, ops);
                    }
                }
            }
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
            TAG_CATALOG_TXN => Command::CatalogTxn {
                ops: read_ops(&mut r, "CatalogTxn")?,
            },
            TAG_WRITE => Command::Write {
                ops: read_ops(&mut r, "Write")?,
            },
            TAG_CONF_CHANGE => Command::ConfChange {
                add: r.u8()? != 0,
                node: r.u64()?,
            },
            TAG_NOOP => Command::Noop,
            TAG_FENCED => {
                let fence = RegionFence {
                    region_id: r.u64()?,
                    conf_ver: r.u64()?,
                    version: r.u64()?,
                };
                // Whitelist of fenceable inner tags, mirroring [`FencedInner`]. Every
                // other tag — nested fence, known-but-unfenceable, unknown — is a
                // typed error; a widened whitelist is a deliberate edit HERE plus a
                // `FencedInner` variant, never a fall-through.
                let inner = match r.u8()? {
                    TAG_WRITE => FencedInner::Write {
                        ops: read_ops(&mut r, "Fenced(Write)")?,
                    },
                    TAG_FENCED => {
                        return Err(kv9_common::Error::Raft(
                            "fenced command cannot nest a fence".into(),
                        ))
                    }
                    other => {
                        return Err(kv9_common::Error::Raft(format!(
                            "command tag {other} is not fenceable"
                        )))
                    }
                };
                Command::Fenced { fence, inner }
            }
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
const TAG_WRITE: u8 = 5;
const TAG_FENCED: u8 = 6;
const OP_PUT: u8 = 1;
const OP_DELETE: u8 = 2;

/// Decode a length-prefixed op list (shared by `CatalogTxn` and `Write`).
fn read_ops(r: &mut Reader<'_>, ctx: &str) -> kv9_common::Result<Vec<KvOp>> {
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
                    "unknown {ctx} op tag {other}"
                )))
            }
        };
        ops.push(op);
    }
    Ok(ops)
}

/// Encode a length-prefixed op list (inverse of [`read_ops`]; shared by
/// `CatalogTxn`, `Write`, and the `Fenced` envelope's inner `Write`).
fn put_ops(out: &mut Vec<u8>, ops: &[KvOp]) {
    out.extend_from_slice(&(ops.len() as u32).to_be_bytes());
    for op in ops {
        match op {
            KvOp::Put { cf, key, value } => {
                out.push(OP_PUT);
                out.push(*cf);
                put_bytes(out, key);
                put_bytes(out, value);
            }
            KvOp::Delete { cf, key } => {
                out.push(OP_DELETE);
                out.push(*cf);
                put_bytes(out, key);
            }
        }
    }
}

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
            return Err(kv9_common::Error::Raft("truncated command entry".into()));
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
        roundtrip(&Command::Write { ops: Vec::new() });
        roundtrip(&Command::Write {
            ops: vec![
                KvOp::Put {
                    cf: 0,
                    key: b"user-key".to_vec(),
                    value: b"user-value".to_vec(),
                },
                KvOp::Delete {
                    cf: 0,
                    key: b"user-gone".to_vec(),
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
        // Same negative through the Write tag (shared op codec, both entrances).
        let bytes = vec![ENTRY_VERSION, TAG_WRITE, 0, 0, 0, 1, 0xEE];
        assert!(Command::decode(&bytes).is_err());
    }

    /// `Write` and `CatalogTxn` must stay distinct on the wire even with identical
    /// ops — collapsing them would let user data re-enter through the catalog path
    /// (and its lock) on replay.
    #[test]
    fn write_and_catalog_txn_are_distinct_on_the_wire() {
        let ops = vec![KvOp::Put {
            cf: 0,
            key: b"k".to_vec(),
            value: b"v".to_vec(),
        }];
        let w = Command::Write { ops: ops.clone() }.encode();
        let c = Command::CatalogTxn { ops }.encode();
        assert_ne!(w, c);
        assert!(matches!(
            Command::decode(&w).unwrap(),
            Command::Write { .. }
        ));
        assert!(matches!(
            Command::decode(&c).unwrap(),
            Command::CatalogTxn { .. }
        ));
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

    fn sample_fence() -> RegionFence {
        RegionFence {
            region_id: 7,
            conf_ver: 3,
            version: 11,
        }
    }

    fn sample_fenced() -> Command {
        Command::Fenced {
            fence: sample_fence(),
            inner: FencedInner::Write {
                ops: vec![
                    KvOp::Put {
                        cf: 0,
                        key: b"user-key".to_vec(),
                        value: b"user-value".to_vec(),
                    },
                    KvOp::Delete {
                        cf: 0,
                        key: b"user-gone".to_vec(),
                    },
                ],
            },
        }
    }

    #[test]
    fn fenced_write_roundtrips_with_the_exact_fence() {
        roundtrip(&sample_fenced());
        roundtrip(&Command::Fenced {
            fence: RegionFence {
                region_id: u64::MAX,
                conf_ver: 0,
                version: u64::MAX,
            },
            inner: FencedInner::Write { ops: Vec::new() },
        });
        // The fence fields must survive individually — a swapped or dropped field
        // decodes to a DIFFERENT fence, so equality on the whole command covers it,
        // but assert the fields by name so a red points at the fence, not the ops.
        let decoded = Command::decode(&sample_fenced().encode()).unwrap();
        let Command::Fenced { fence, .. } = decoded else {
            panic!("fenced write must decode back to the envelope");
        };
        assert_eq!(
            (fence.region_id, fence.conf_ver, fence.version),
            (7, 3, 11),
            "the decoded fence must carry the proposer's exact expected epoch"
        );
    }

    /// A fenced and an unfenced write with identical ops must stay distinct on the
    /// wire — collapsing them would let an epoch-checked write replay as unchecked
    /// (the same shape as the Write/CatalogTxn separation above).
    #[test]
    fn fenced_and_unfenced_write_are_distinct_on_the_wire() {
        let ops = vec![KvOp::Put {
            cf: 0,
            key: b"k".to_vec(),
            value: b"v".to_vec(),
        }];
        let fenced = Command::Fenced {
            fence: sample_fence(),
            inner: FencedInner::Write { ops: ops.clone() },
        }
        .encode();
        let bare = Command::Write { ops }.encode();
        assert_ne!(fenced, bare);
        assert!(matches!(
            Command::decode(&fenced).unwrap(),
            Command::Fenced { .. }
        ));
    }

    #[test]
    fn decode_rejects_a_nested_fence() {
        // Hand-built: a Fenced envelope whose inner tag is again TAG_FENCED. The
        // type system cannot express this; the decoder must refuse it rather than
        // recurse or fall through to "unknown tag".
        let mut bytes = vec![ENTRY_VERSION, TAG_FENCED];
        bytes.extend_from_slice(&[0u8; 24]); // fence fields
        bytes.push(TAG_FENCED);
        let err = Command::decode(&bytes).expect_err("nested fence must be refused");
        assert!(
            err.to_string().contains("cannot nest"),
            "nesting must be refused as nesting, not as an unknown tag: {err}"
        );
    }

    #[test]
    fn decode_rejects_unfenceable_inner_tags() {
        // Every known non-Write tag must be refused inside the envelope: the
        // whitelist is the boundary, not "whatever happens to decode".
        for inner_tag in [TAG_PUT, TAG_CATALOG_TXN, TAG_CONF_CHANGE, TAG_NOOP] {
            let mut bytes = vec![ENTRY_VERSION, TAG_FENCED];
            bytes.extend_from_slice(&[0u8; 24]);
            bytes.push(inner_tag);
            let err =
                Command::decode(&bytes).expect_err("only whitelisted inner tags may ride a fence");
            assert!(
                err.to_string().contains("not fenceable"),
                "tag {inner_tag} must be refused as unfenceable: {err}"
            );
        }
    }

    #[test]
    fn fenced_decode_rejects_truncation_and_trailing_bytes() {
        let full = sample_fenced().encode();
        for cut in 0..full.len() {
            assert!(Command::decode(&full[..cut]).is_err(), "prefix len {cut}");
        }
        let mut padded = full;
        padded.push(0);
        assert!(Command::decode(&padded).is_err());
    }

    /// Lowering the envelope directly must produce NOTHING: the apply loop checks
    /// the fence and lowers the inner ops itself, so a path that forgets the fence
    /// fails closed (no write) instead of open (unchecked write).
    #[test]
    fn fenced_lowered_without_a_fence_check_writes_nothing() {
        assert!(
            sample_fenced().to_write_batch().mutations().is_empty(),
            "an unchecked fenced write must not leak its ops through to_write_batch"
        );
    }
}
