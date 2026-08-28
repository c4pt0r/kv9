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
// Memcomparable column helpers (METADATA-CATALOG §3).
// ---------------------------------------------------------------------------

/// Memcomparable encoding of an unsigned integer: big-endian preserves numeric order.
/// Fixed 8-byte width, so no terminator is needed.
pub fn memcmp_uint(v: u64) -> PkComponent {
    v.to_be_bytes().to_vec()
}

/// Decode the value of a [`memcmp_uint`] component.
pub fn decode_uint_component(comp: &[u8]) -> Result<u64> {
    if comp.len() != 8 {
        return Err(Error::MalformedKey(format!(
            "uint component width {} != 8",
            comp.len()
        )));
    }
    let mut b = [0u8; 8];
    b.copy_from_slice(comp);
    Ok(u64::from_be_bytes(b))
}

/// Escape lead byte inside a bytes component.
const ESCAPE: u8 = 0x00;
/// Follows [`ESCAPE`] for a literal `0x00` payload byte. Orders *above* the terminator,
/// so `"a\0"` sorts after `"a"`.
const ESCAPED_ZERO: u8 = 0xFF;
/// Follows [`ESCAPE`] to end the component. Orders *below* any payload byte's
/// continuation, so a prefix sorts before its extensions.
const TERMINATOR: u8 = 0x01;

/// Memcomparable encoding of a variable-length byte string: escape `0x00` as
/// `0x00 0xFF`, terminate with `0x00 0x01` (METADATA-CATALOG §3).
///
/// Order-preserving (`encode(a) < encode(b) ⇔ a < b` bytewise) and self-delimiting, so
/// concatenated components — index cols followed by pk cols — stay unambiguous and the
/// concatenation preserves the tuple order.
pub fn memcmp_bytes(v: &[u8]) -> PkComponent {
    let mut out = Vec::with_capacity(v.len() + 2);
    for &b in v {
        if b == ESCAPE {
            out.push(ESCAPE);
            out.push(ESCAPED_ZERO);
        } else {
            out.push(b);
        }
    }
    out.push(ESCAPE);
    out.push(TERMINATOR);
    out
}

/// Decode the payload of a single [`memcmp_bytes`] component.
pub fn decode_bytes_component(comp: &[u8]) -> Result<Vec<u8>> {
    let (payload, rest) = split_bytes_component(comp)?;
    if !rest.is_empty() {
        return Err(Error::MalformedKey(
            "trailing bytes after bytes component".into(),
        ));
    }
    Ok(payload)
}

/// Split one escaped bytes component off the front of `buf`, returning
/// `(decoded payload, remainder)`.
fn split_bytes_component(buf: &[u8]) -> Result<(Vec<u8>, &[u8])> {
    let mut payload = Vec::new();
    let mut i = 0;
    while i < buf.len() {
        let b = buf[i];
        if b != ESCAPE {
            payload.push(b);
            i += 1;
            continue;
        }
        match buf.get(i + 1) {
            Some(&ESCAPED_ZERO) => {
                payload.push(0x00);
                i += 2;
            }
            Some(&TERMINATOR) => return Ok((payload, &buf[i + 2..])),
            _ => {
                return Err(Error::MalformedKey(
                    "bad escape sequence in bytes component".into(),
                ))
            }
        }
    }
    Err(Error::MalformedKey("unterminated bytes component".into()))
}

/// Memcomparable encoding of text (delegates to [`memcmp_bytes`] over UTF-8).
pub fn memcmp_text(v: &str) -> PkComponent {
    memcmp_bytes(v.as_bytes())
}

/// Split a concatenation of encoded components back into per-column encoded components,
/// driven by the columns' declared types (fixed-width uints carry no terminator;
/// bytes/text self-delimit via the escape scheme).
///
/// Components are returned **still encoded** — symmetric with what callers pass to
/// `get`/`insert`; use [`decode_uint_component`] / [`decode_bytes_component`] for typed
/// values. Errors on truncation, bad escapes, or (with `expect_end`) trailing bytes.
pub fn split_components(
    types: &[crate::schema::ColumnType],
    mut buf: &[u8],
    expect_end: bool,
) -> Result<Vec<PkComponent>> {
    use crate::schema::ColumnType;
    let mut out = Vec::with_capacity(types.len());
    for ty in types {
        match ty {
            ColumnType::Uint => {
                if buf.len() < 8 {
                    return Err(Error::MalformedKey("truncated uint component".into()));
                }
                out.push(buf[..8].to_vec());
                buf = &buf[8..];
            }
            ColumnType::Text | ColumnType::Bytes => {
                let before = buf.len();
                let (_, rest) = split_bytes_component(buf)?;
                let consumed = before - rest.len();
                out.push(buf[..consumed].to_vec());
                buf = rest;
            }
        }
    }
    if expect_end && !buf.is_empty() {
        return Err(Error::MalformedKey(format!(
            "{} trailing bytes after components",
            buf.len()
        )));
    }
    Ok(out)
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
    /// A value written by a newer binary with a type tag this one doesn't know.
    /// Preserved verbatim — type tag and payload — so a read-modify-write by an old
    /// binary re-encodes it unchanged instead of silently rewriting the type
    /// (DESIGN principle 12: forward-compatible formats).
    Unknown { type_tag: u8, payload: Vec<u8> },
}

impl ColumnValue {
    fn type_tag(&self) -> u8 {
        match self {
            ColumnValue::Uint(_) => 0,
            ColumnValue::Text(_) => 1,
            ColumnValue::Bytes(_) => 2,
            ColumnValue::Unknown { type_tag, .. } => *type_tag,
        }
    }

    fn payload(&self) -> Vec<u8> {
        match self {
            ColumnValue::Uint(v) => v.to_be_bytes().to_vec(),
            ColumnValue::Text(s) => s.as_bytes().to_vec(),
            ColumnValue::Bytes(b) => b.clone(),
            ColumnValue::Unknown { payload, .. } => payload.clone(),
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

    /// Set a column's value, replacing any existing value for the same column id
    /// (last-write-wins within the row; the encoded form carries one entry per column).
    pub fn set(&mut self, col: ColumnId, value: ColumnValue) -> &mut Self {
        if let Some(slot) = self.columns.iter_mut().find(|(c, _)| *c == col) {
            slot.1 = value;
        } else {
            self.columns.push((col, value));
        }
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
                // Unknown type tag from a newer writer: preserve tag + payload verbatim
                // so re-encoding is lossless (never panic on unknown, never rewrite).
                tag => ColumnValue::Unknown {
                    type_tag: tag,
                    payload: payload.to_vec(),
                },
            };
            row.set(col, value);
        }
        Ok(row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::ColumnType;

    /// A corpus that hits the escape scheme's edges: empties, embedded 0x00/0xFF,
    /// prefix pairs, and escape-byte runs.
    fn corpus() -> Vec<Vec<u8>> {
        vec![
            vec![],
            vec![0x00],
            vec![0x00, 0x00],
            vec![0x00, 0x01],
            vec![0x00, 0xff],
            vec![0x01],
            vec![0xff],
            vec![0xff, 0xff],
            vec![0xff, 0x00],
            b"a".to_vec(),
            b"ab".to_vec(),
            b"a\x00".to_vec(),
            b"a\x00b".to_vec(),
            b"a\x01".to_vec(),
            b"b".to_vec(),
        ]
    }

    /// Contract item 11 (meta half): encoded order == raw byte order, for every pair
    /// in the corpus. If this breaks, `seek_le`-style routing silently returns the
    /// wrong region.
    #[test]
    fn memcmp_bytes_preserves_order() {
        let corpus = corpus();
        for a in &corpus {
            for b in &corpus {
                let (ea, eb) = (memcmp_bytes(a), memcmp_bytes(b));
                assert_eq!(
                    a.cmp(b),
                    ea.cmp(&eb),
                    "order broken for {a:x?} vs {b:x?} (encoded {ea:x?} vs {eb:x?})"
                );
            }
        }
    }

    #[test]
    fn memcmp_bytes_roundtrip() {
        for v in corpus() {
            assert_eq!(decode_bytes_component(&memcmp_bytes(&v)).unwrap(), v);
        }
    }

    #[test]
    fn memcmp_uint_roundtrip_and_order() {
        for v in [0u64, 1, 99, 100, u64::MAX - 1, u64::MAX] {
            assert_eq!(decode_uint_component(&memcmp_uint(v)).unwrap(), v);
        }
        assert!(memcmp_uint(1) < memcmp_uint(2));
        assert!(memcmp_uint(255) < memcmp_uint(256));
    }

    /// Concatenated components (index cols + pk) split back exactly; components with
    /// embedded zeros don't bleed into their neighbors.
    #[test]
    fn split_components_mixed() {
        let types = [ColumnType::Uint, ColumnType::Bytes, ColumnType::Uint];
        let comps = [
            memcmp_uint(7),
            memcmp_bytes(b"a\x00b"),
            memcmp_uint(u64::MAX),
        ];
        let joined: Vec<u8> = comps.concat();
        let split = split_components(&types, &joined, true).unwrap();
        assert_eq!(split.as_slice(), comps.as_slice());
        // Trailing bytes must be rejected when the caller expects the end.
        let mut extra = joined.clone();
        extra.push(0xAA);
        assert!(split_components(&types, &extra, true).is_err());
        // Truncation must be rejected.
        assert!(split_components(&types, &joined[..joined.len() - 1], true).is_err());
    }

    #[test]
    fn split_rejects_bad_escape_and_unterminated() {
        // 0x00 followed by neither 0xFF nor 0x01.
        assert!(decode_bytes_component(&[0x00, 0x02]).is_err());
        // No terminator at all.
        assert!(decode_bytes_component(b"abc").is_err());
    }

    #[test]
    fn row_value_set_replaces() {
        let mut row = RowValue::new();
        row.set(crate::schema::ColumnId(1), ColumnValue::Uint(1));
        row.set(crate::schema::ColumnId(1), ColumnValue::Uint(2));
        assert_eq!(
            row.get(crate::schema::ColumnId(1)),
            Some(&ColumnValue::Uint(2))
        );
        // Exactly one encoded entry for the column.
        let encoded = row.encode();
        let decoded = RowValue::decode(&encoded).unwrap();
        assert_eq!(decoded, row);
        assert_eq!(encoded.len(), 2 + 1 + 4 + 8);
    }

    /// Contract item 11: row value roundtrip, including lossless preservation of a
    /// newer writer's unknown type tag (principle 12: a read-modify-write by an old
    /// binary must not rewrite what it doesn't understand).
    #[test]
    fn row_value_roundtrip_preserves_unknown_type_tag() {
        let mut row = RowValue::new();
        row.set(crate::schema::ColumnId(1), ColumnValue::Uint(42));
        row.set(
            crate::schema::ColumnId(9),
            ColumnValue::Text("café".into()),
        );
        row.set(
            crate::schema::ColumnId(3),
            ColumnValue::Bytes(vec![0x00, 0xff]),
        );
        let mut encoded = row.encode();
        // Append a column with a type tag from the future (tag 7).
        encoded.extend_from_slice(&5u16.to_be_bytes());
        encoded.push(7);
        encoded.extend_from_slice(&3u32.to_be_bytes());
        encoded.extend_from_slice(&[1, 2, 3]);

        let decoded = RowValue::decode(&encoded).unwrap();
        assert_eq!(
            decoded.get(crate::schema::ColumnId(5)),
            Some(&ColumnValue::Unknown {
                type_tag: 7,
                payload: vec![1, 2, 3]
            })
        );
        // Re-encoding reproduces the original bytes exactly — tag included.
        assert_eq!(decoded.encode(), encoded);
    }

    #[test]
    fn row_value_decode_rejects_truncation() {
        let mut row = RowValue::new();
        row.set(crate::schema::ColumnId(1), ColumnValue::Uint(1));
        let encoded = row.encode();
        for cut in 1..encoded.len() {
            assert!(RowValue::decode(&encoded[..cut]).is_err(), "cut {cut}");
        }
    }

    /// Index-prefix ranges bound exactly the entries under the prefix: a text prefix
    /// must not match its extensions (the terminator guarantees "foo" ranges exclude
    /// "foobar" — the superstring-collision bug the escape scheme exists to prevent).
    #[test]
    fn index_prefix_range_excludes_superstrings() {
        use crate::schema::{IndexId, TableId};
        let t = TableId(2);
        let i = IndexId(2);
        let key_foo = encode_index_key(t, i, &[memcmp_text("foo")], &[]).unwrap();
        let key_foobar = encode_index_key(t, i, &[memcmp_text("foobar")], &[]).unwrap();
        let (start, end) = index_prefix_range(t, i, &[memcmp_text("foo")]).unwrap();
        assert!(start <= key_foo && key_foo < end, "exact match inside range");
        assert!(!(start <= key_foobar && key_foobar < end), "superstring outside");
    }
}
