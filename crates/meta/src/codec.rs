//! Row/index key + tag-length value encoding for the catalog (METADATA-CATALOG §3).
//!
//! The whole catalog lives under the **system keyspace** prefix ([`kv9_common::codec`],
//! mode `'s'`, keyspace [`KeyspaceId::SYSTEM`]). Every physical key is memcomparable so
//! range scans work:
//!
//! ```text
//! row       :  <sys-prefix> <table_id:u32> 'r' <pk-cols memcmp>            → value = <tagged column set>
//! sec-index :  <sys-prefix> <table_id:u32> 'i' <index_id:u8> <idx-cols>   → <pk memcmp>
//! ```
//!
//! Row values are **tag-length encoded** columns (protobuf-ish): adding a column is
//! forward-compatible — old readers skip unknown tags, new readers default missing ones
//! (METADATA-CATALOG §3, §7).

use kv9_common::codec::{encode_key, KeyMode};
use kv9_common::{Error, KeyspaceId, Result};

use crate::schema::{ColumnId, IndexId, TableId};

/// Tag byte separating the row-space from the index-space within a table (§3).
const ROW_MARKER: u8 = b'r';
/// Tag byte marking the secondary-index space within a table (§3).
const INDEX_MARKER: u8 = b'i';

/// A single primary-key component, already reduced to its memcomparable bytes.
///
/// Callers build these from typed columns via [`memcmp_uint`] / [`memcmp_bytes`].
pub type PkComponent = Vec<u8>;

/// Build the raw system-keyspace suffix that a catalog key sits under, then wrap it in
/// the physical [`kv9_common::codec`] prefix (mode `'s'`, keyspace 0).
fn wrap_system(suffix: Vec<u8>) -> Result<Vec<u8>> {
    encode_key(KeyMode::System, KeyspaceId::SYSTEM, &suffix)
}

/// Encode a **row key** for `(table_id, pk)` (METADATA-CATALOG §3):
/// `<sys-prefix> <table_id:u32-be> 'r' <pk-cols memcmp>`.
pub fn encode_row_key(table_id: TableId, pk: &[PkComponent]) -> Result<Vec<u8>> {
    let mut suffix = Vec::with_capacity(5 + pk.iter().map(|c| c.len()).sum::<usize>());
    suffix.extend_from_slice(&table_id.0.to_be_bytes());
    suffix.push(ROW_MARKER);
    for comp in pk {
        suffix.extend_from_slice(comp);
    }
    wrap_system(suffix)
}

/// The `[start, end)` bounds that cover **all rows** of a table (for a full table scan).
pub fn row_range(table_id: TableId) -> Result<(Vec<u8>, Vec<u8>)> {
    let start = {
        let mut s = table_id.0.to_be_bytes().to_vec();
        s.push(ROW_MARKER);
        wrap_system(s)?
    };
    let end = {
        let mut s = table_id.0.to_be_bytes().to_vec();
        s.push(ROW_MARKER + 1);
        wrap_system(s)?
    };
    Ok((start, end))
}

/// Encode a **secondary index key** (METADATA-CATALOG §3):
/// `<sys-prefix> <table_id:u32-be> 'i' <index_id:u8> <idx-cols memcmp> [<pk memcmp>]`.
///
/// For a non-unique index the primary key is appended to keep entries distinct; for a
/// unique index the pk is the *value*, not part of the key (see [`index_value`]).
pub fn encode_index_key(
    table_id: TableId,
    index_id: IndexId,
    cols: &[PkComponent],
    pk: &[PkComponent],
) -> Result<Vec<u8>> {
    let mut suffix = Vec::new();
    suffix.extend_from_slice(&table_id.0.to_be_bytes());
    suffix.push(INDEX_MARKER);
    suffix.push(index_id.0);
    for c in cols {
        suffix.extend_from_slice(c);
    }
    for c in pk {
        suffix.extend_from_slice(c);
    }
    wrap_system(suffix)
}

/// The `[start, end)` bounds covering an index-prefix lookup on `cols` (the driver for
/// `index_scan`, METADATA-CATALOG §4).
pub fn index_prefix_range(
    table_id: TableId,
    index_id: IndexId,
    cols: &[PkComponent],
) -> Result<(Vec<u8>, Vec<u8>)> {
    let start = encode_index_key(table_id, index_id, cols, &[])?;
    let mut end = start.clone();
    prefix_successor(&mut end);
    Ok((start, end))
}

/// For a **unique** index, the stored value is the encoded primary key (§3).
pub fn index_value(pk: &[PkComponent]) -> Vec<u8> {
    pk.concat()
}

/// Bump a byte prefix to its exclusive successor (`\xff...` rolls over by truncation).
fn prefix_successor(buf: &mut Vec<u8>) {
    while let Some(last) = buf.last().copied() {
        if last == 0xff {
            buf.pop();
        } else {
            *buf.last_mut().unwrap() = last + 1;
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// Memcomparable column helpers (METADATA-CATALOG §3). Stubs where a full
// order-preserving escape scheme is not needed for Phase-1.
// ---------------------------------------------------------------------------

/// Memcomparable encoding of an unsigned integer: big-endian preserves numeric order.
pub fn memcmp_uint(v: u64) -> PkComponent {
    v.to_be_bytes().to_vec()
}

/// Memcomparable encoding of a byte string.
///
/// Phase-1 stub: appends the raw bytes. A production encoder escapes `0x00` and adds a
/// terminator so that variable-length components stay order-preserving and
/// unambiguously delimited (TiDB-style). `// TODO(phase1): order-preserving escape`.
pub fn memcmp_bytes(v: &[u8]) -> PkComponent {
    v.to_vec()
}

/// Memcomparable encoding of text (delegates to [`memcmp_bytes`] over UTF-8).
pub fn memcmp_text(v: &str) -> PkComponent {
    memcmp_bytes(v.as_bytes())
}

// ---------------------------------------------------------------------------
// Tag-length row value encoding (METADATA-CATALOG §3, forward-compatible).
// ---------------------------------------------------------------------------

/// The wire representation of one column's value in a tag-length row (§3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColumnValue {
    Uint(u64),
    Text(String),
    Bytes(Vec<u8>),
}

impl ColumnValue {
    fn type_tag(&self) -> u8 {
        match self {
            ColumnValue::Uint(_) => 0,
            ColumnValue::Text(_) => 1,
            ColumnValue::Bytes(_) => 2,
        }
    }

    fn payload(&self) -> Vec<u8> {
        match self {
            ColumnValue::Uint(v) => v.to_be_bytes().to_vec(),
            ColumnValue::Text(s) => s.as_bytes().to_vec(),
            ColumnValue::Bytes(b) => b.clone(),
        }
    }
}

/// A tag-length row value: an ordered list of `(column_id, value)` pairs (§3).
///
/// Encoding per column: `column_id:u16-be | type_tag:u8 | len:u32-be | payload`.
/// Unknown column ids are skipped by old readers (forward-compatible); missing columns
/// are defaulted by new readers (METADATA-CATALOG §3, §7).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RowValue {
    columns: Vec<(ColumnId, ColumnValue)>,
}

impl RowValue {
    pub fn new() -> Self {
        RowValue::default()
    }

    /// Set (or append) a column's value.
    pub fn set(&mut self, col: ColumnId, value: ColumnValue) -> &mut Self {
        self.columns.push((col, value));
        self
    }

    /// Read a column's value by id, if present.
    pub fn get(&self, col: ColumnId) -> Option<&ColumnValue> {
        self.columns.iter().find(|(c, _)| *c == col).map(|(_, v)| v)
    }

    /// Serialize to tag-length bytes (METADATA-CATALOG §3).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (col, val) in &self.columns {
            out.extend_from_slice(&col.0.to_be_bytes());
            out.push(val.type_tag());
            let payload = val.payload();
            out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            out.extend_from_slice(&payload);
        }
        out
    }

    /// Deserialize from tag-length bytes, skipping unknown-tag columns gracefully
    /// (forward-compatible; METADATA-CATALOG §3, §7).
    pub fn decode(mut buf: &[u8]) -> Result<RowValue> {
        let mut row = RowValue::new();
        while !buf.is_empty() {
            if buf.len() < 7 {
                return Err(Error::MalformedKey("truncated row value header".into()));
            }
            let col = ColumnId(u16::from_be_bytes([buf[0], buf[1]]));
            let type_tag = buf[2];
            let len = u32::from_be_bytes([buf[3], buf[4], buf[5], buf[6]]) as usize;
            buf = &buf[7..];
            if buf.len() < len {
                return Err(Error::MalformedKey("truncated row value payload".into()));
            }
            let payload = &buf[..len];
            buf = &buf[len..];
            let value = match type_tag {
                0 => {
                    let mut b = [0u8; 8];
                    if payload.len() != 8 {
                        return Err(Error::MalformedKey("bad uint payload width".into()));
                    }
                    b.copy_from_slice(payload);
                    ColumnValue::Uint(u64::from_be_bytes(b))
                }
                1 => ColumnValue::Text(
                    String::from_utf8(payload.to_vec())
                        .map_err(|_| Error::MalformedKey("bad utf-8 text column".into()))?,
                ),
                2 => ColumnValue::Bytes(payload.to_vec()),
                // Unknown type tag from a newer writer: preserve as opaque bytes so we
                // neither panic nor lose data (principle: never panic on unknown).
                _ => ColumnValue::Bytes(payload.to_vec()),
            };
            row.set(col, value);
        }
        Ok(row)
    }
}
