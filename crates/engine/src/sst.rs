//! Immutable sorted string tables — the objects that live in object storage.
//!
//! An SST is written once, never updated, and identified by the hash of its own bytes
//! (`docs/OBJECT-STORAGE.md` §2, §5). Everything here follows from those two facts:
//!
//! - There is no `append`, no `update`, no builder that can be reopened. [`SstWriter`]
//!   consumes itself to produce bytes, and [`Sst`] only reads.
//! - The reader **fails closed**. A truncated file, a bad magic, an unknown version or a
//!   checksum mismatch returns a typed error and never returns data. A corrupt SST that
//!   answered `Ok(None)` would be indistinguishable from a key that was never written,
//!   which is the difference between "not found" and "your data is gone".
//!
//! # Layout
//!
//! ```text
//! MAGIC   4     "KV9S"
//! VERSION 1     format version; an unknown value is refused, never guessed
//! CF      1     which column family these entries belong to
//! COUNT   4     number of entries
//! entries       COUNT × ( klen u32 | key | vlen u32 | value ), sorted by key, unique
//! FOOTER
//!   min_len u32 | min_key
//!   max_len u32 | max_key
//!   crc     4 bytes   CRC-32 over every preceding byte
//! ```
//!
//! The key range is stored in the footer and **verified against the entries on open**,
//! because a declared range that could disagree with the data it bounds would be a second
//! source of truth about the same question.
//!
//! **What that range is *not*, in this version:** a way to decide whether an object is
//! worth fetching before fetching it. There is no locatable footer — the only entry point
//! is [`Sst::parse`] over the whole object, which parses every entry. Pre-fetch filtering
//! is the manifest's job (`docs/OBJECT-STORAGE.md` §5: `PreparedSst` carries `key_range`,
//! and the manifest is consulted before any object is retrieved). Giving *this* layout that
//! capability would need a fixed-offset trailer plus an API that reads it alone, and round
//! one does not require it. Recorded because an earlier revision of this comment claimed
//! the capability, which the code has never had.

use kv9_common::{Error, Result, UserKey, Value};

use crate::cf::ColumnFamily;

const MAGIC: [u8; 4] = *b"KV9S";
const VERSION: u8 = 1;

// Resource limits. Three separate quantities, deliberately not one constant reused:
// a byte bound and an entry count are different dimensions, and a single `MAX` serving
// both bounds neither (1<<30 *entries* at 8 bytes each is 8 GiB, so a byte constant used
// as a count limit does not constrain memory at all).
//
// **The writer enforces exactly the same limits as the reader.** An asymmetry here would
// let the writer emit a table its own reader refuses — and that failure surfaces at read
// time, long after the cause, quite possibly after a manifest already references the
// object.

/// Largest single key or value, on both the write and the read side.
const MAX_FIELD_LEN: usize = 64 << 20;

/// Largest number of entries in one table.
const MAX_ENTRIES: usize = 16 << 20;

/// Largest serialized table.
const MAX_SST_BYTES: usize = 1 << 30;

// The three limits are three dimensions, and collapsing any two of them is the defect this
// module already had once: a byte constant used as an entry count bounded nothing, because
// 1<<30 entries is gigabytes. Asserted at compile time rather than in a test — a runtime
// assertion over constants cannot fail in a way that reflects behaviour, so it would be
// theatre; this one fails the build.
const _: () = assert!(
    MAX_FIELD_LEN != MAX_ENTRIES,
    "field-byte limit and entry-count limit must stay distinct quantities"
);
const _: () = assert!(
    MAX_FIELD_LEN <= MAX_SST_BYTES,
    "a single field may not be allowed to exceed a whole table"
);

/// Convert a length to its on-disk `u32`, refusing rather than truncating.
///
/// `n as u32` is a *silent* truncating cast: a length of 4_294_967_300 is written as 4,
/// producing a well-formed file with a wrong length prefix. This module's whole stance is
/// "refuse, never guess", and the cast broke that on the write side, where the damage only
/// becomes visible at read time.
fn on_disk_len(n: usize, what: &str) -> Result<u32> {
    if n > MAX_FIELD_LEN {
        return Err(Error::Engine(format!(
            "sst: {what} of {n} bytes exceeds the {MAX_FIELD_LEN} byte limit"
        )));
    }
    u32::try_from(n)
        .map_err(|_| Error::Engine(format!("sst: {what} length {n} does not fit in u32")))
}

/// Refuse a whole-table size over the limit.
///
/// A free function purely so the boundary is *reachable by a test*. Inline in `finish()`
/// the predicate could only be exercised by actually building a gigabyte, so no test stood
/// on it — a mutation deleting the check survived, which is how this was found. A limit
/// nothing can red is a limit nobody is holding.
fn check_total_size(encoded: usize) -> Result<()> {
    if encoded > MAX_SST_BYTES {
        return Err(Error::Engine(format!(
            "sst: table would encode to {encoded} bytes, over the {MAX_SST_BYTES} byte limit"
        )));
    }
    Ok(())
}

fn cf_code(cf: ColumnFamily) -> u8 {
    match cf {
        ColumnFamily::Default => 0,
        ColumnFamily::Lock => 1,
        ColumnFamily::Write => 2,
    }
}

fn cf_from_code(code: u8) -> Result<ColumnFamily> {
    match code {
        0 => Ok(ColumnFamily::Default),
        1 => Ok(ColumnFamily::Lock),
        2 => Ok(ColumnFamily::Write),
        other => Err(Error::Engine(format!(
            "sst: unknown column-family code {other} — refusing to guess"
        ))),
    }
}

/// CRC-32 (IEEE), computed without pulling in a dependency. Same routine as the WAL's, so
/// the two agree on what "checksum" means.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Builds one immutable SST.
///
/// Entries must be added in ascending key order. That is enforced rather than assumed:
/// silently sorting for the caller would hide a bug in the flush path, and accepting
/// unsorted input would produce a file whose reader cannot binary-search it.
#[derive(Debug)]
pub struct SstWriter {
    cf: ColumnFamily,
    entries: Vec<(UserKey, Value)>,
}

impl SstWriter {
    pub fn new(cf: ColumnFamily) -> Self {
        SstWriter {
            cf,
            entries: Vec::new(),
        }
    }

    /// Append one entry. Keys must strictly ascend.
    ///
    /// Field lengths and the entry count are checked here, against the same limits the
    /// reader applies, so an over-large entry is refused at the point it is offered rather
    /// than discovered when the finished table fails to parse.
    pub fn add(&mut self, key: UserKey, value: Value) -> Result<()> {
        on_disk_len(key.len(), "key")?;
        on_disk_len(value.len(), "value")?;
        if self.entries.len() >= MAX_ENTRIES {
            return Err(Error::Engine(format!(
                "sst: entry count would exceed the {MAX_ENTRIES} entry limit"
            )));
        }
        if let Some((last, _)) = self.entries.last() {
            if key <= *last {
                return Err(Error::Engine(
                    "sst: entries must be added in strictly ascending key order".into(),
                ));
            }
        }
        self.entries.push((key, value));
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Exact serialised size, computed with checked arithmetic so an overflow is a refusal
    /// rather than a wrapped total that would pass a limit check it should have failed.
    fn encoded_len(&self) -> Result<usize> {
        let overflow = || Error::Engine("sst: encoded length overflows usize".into());
        // magic + version + cf + count
        let mut n: usize = 4 + 1 + 1 + 4;
        for (k, v) in &self.entries {
            n = n
                .checked_add(4)
                .and_then(|n| n.checked_add(k.len()))
                .and_then(|n| n.checked_add(4))
                .and_then(|n| n.checked_add(v.len()))
                .ok_or_else(overflow)?;
        }
        let smallest = self.entries.first().map(|(k, _)| k.len()).unwrap_or(0);
        let largest = self.entries.last().map(|(k, _)| k.len()).unwrap_or(0);
        n = n
            .checked_add(4)
            .and_then(|n| n.checked_add(smallest))
            .and_then(|n| n.checked_add(4))
            .and_then(|n| n.checked_add(largest))
            .and_then(|n| n.checked_add(4)) // crc
            .ok_or_else(overflow)?;
        Ok(n)
    }

    /// Serialise. Consumes the writer: an SST is written once.
    ///
    /// Refuses an empty table. An SST with no entries has no key range, so it could not be
    /// range-filtered, and a manifest referencing it would carry an entry that can never
    /// match a read — a silent waste rather than an error.
    pub fn finish(self) -> Result<Vec<u8>> {
        if self.entries.is_empty() {
            return Err(Error::Engine(
                "sst: refusing to write an empty table — it would have no key range".into(),
            ));
        }

        let count = on_disk_len(self.entries.len(), "entry count")?;

        // Total size is computed and refused BEFORE serialising. Checking afterwards would
        // mean building the whole buffer — up to a gigabyte — only to throw it away, and
        // the point of a resource limit is not to pay the cost first.
        let encoded = self.encoded_len()?;
        check_total_size(encoded)?;

        let mut out = Vec::with_capacity(encoded);
        out.extend_from_slice(&MAGIC);
        out.push(VERSION);
        out.push(cf_code(self.cf));
        out.extend_from_slice(&count.to_le_bytes());

        for (k, v) in &self.entries {
            out.extend_from_slice(&on_disk_len(k.len(), "key")?.to_le_bytes());
            out.extend_from_slice(k);
            out.extend_from_slice(&on_disk_len(v.len(), "value")?.to_le_bytes());
            out.extend_from_slice(v);
        }

        let smallest = &self.entries.first().expect("non-empty checked above").0;
        let largest = &self.entries.last().expect("non-empty checked above").0;
        out.extend_from_slice(&on_disk_len(smallest.len(), "smallest key")?.to_le_bytes());
        out.extend_from_slice(smallest);
        out.extend_from_slice(&on_disk_len(largest.len(), "largest key")?.to_le_bytes());
        out.extend_from_slice(largest);

        let crc = crc32(&out);
        out.extend_from_slice(&crc.to_le_bytes());
        debug_assert_eq!(
            out.len(),
            encoded,
            "encoded_len must predict the serialised size exactly, or the pre-check bounds \
             a different quantity than the one produced"
        );
        Ok(out)
    }
}

/// A parsed, verified SST.
///
/// Construction is the verification: if you hold one of these, its checksum matched, its
/// version was known, and its declared key range agreed with its entries. There is no
/// way to obtain an unverified `Sst`, which is why the read path cannot forget to check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sst {
    cf: ColumnFamily,
    entries: Vec<(UserKey, Value)>,
    smallest: UserKey,
    largest: UserKey,
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| Error::Engine("sst: length overflow".into()))?;
        if end > self.bytes.len() {
            return Err(Error::Engine(format!(
                "sst: truncated — wanted {n} bytes at offset {}, {} remain",
                self.pos,
                self.bytes.len() - self.pos
            )));
        }
        let out = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn len_prefixed(&mut self) -> Result<Vec<u8>> {
        let n = self.u32()? as usize;
        if n > MAX_FIELD_LEN {
            return Err(Error::Engine(format!(
                "sst: length field {n} exceeds the {MAX_FIELD_LEN} byte limit"
            )));
        }
        Ok(self.take(n)?.to_vec())
    }
}

impl Sst {
    /// Parse and verify. Every failure here is a refusal, never a silent empty result.
    pub fn parse(bytes: &[u8]) -> Result<Sst> {
        if bytes.len() > MAX_SST_BYTES {
            return Err(Error::Engine(format!(
                "sst: buffer of {} bytes exceeds the {MAX_SST_BYTES} byte limit",
                bytes.len()
            )));
        }
        // Checksum first: nothing else in the buffer may be trusted until it matches.
        if bytes.len() < 4 {
            return Err(Error::Engine("sst: too short to contain a checksum".into()));
        }
        let split = bytes.len() - 4;
        let (body, stored) = bytes.split_at(split);
        let stored = u32::from_le_bytes([stored[0], stored[1], stored[2], stored[3]]);
        let actual = crc32(body);
        if stored != actual {
            return Err(Error::Engine(format!(
                "sst: checksum mismatch — stored {stored:#010x}, computed {actual:#010x}"
            )));
        }

        let mut c = Cursor {
            bytes: body,
            pos: 0,
        };
        if c.take(4)? != MAGIC {
            return Err(Error::Engine("sst: bad magic — not an SST".into()));
        }
        let version = c.take(1)?[0];
        if version != VERSION {
            return Err(Error::Engine(format!(
                "sst: unknown format version {version} (this build writes {VERSION}) — \
                 refusing to guess"
            )));
        }
        let cf = cf_from_code(c.take(1)?[0])?;
        let count = c.u32()? as usize;
        if count > MAX_ENTRIES {
            return Err(Error::Engine(format!(
                "sst: entry count {count} exceeds the {MAX_ENTRIES} entry limit"
            )));
        }

        let mut entries = Vec::with_capacity(count.min(1024));
        let mut prev: Option<UserKey> = None;
        for i in 0..count {
            let key = c.len_prefixed()?;
            let value = c.len_prefixed()?;
            if let Some(p) = &prev {
                if key <= *p {
                    return Err(Error::Engine(format!(
                        "sst: entries out of order at index {i} — keys must strictly ascend"
                    )));
                }
            }
            prev = Some(key.clone());
            entries.push((key, value));
        }

        let smallest = c.len_prefixed()?;
        let largest = c.len_prefixed()?;

        if c.pos != body.len() {
            return Err(Error::Engine(format!(
                "sst: {} trailing bytes after the footer",
                body.len() - c.pos
            )));
        }

        if entries.is_empty() {
            return Err(Error::Engine("sst: no entries — refusing".into()));
        }

        // The declared range is checked against the data rather than believed. A file
        // whose footer disagrees with its entries has two answers to one question.
        if smallest != entries[0].0 || largest != entries[entries.len() - 1].0 {
            return Err(Error::Engine(
                "sst: declared key range disagrees with the entries it bounds".into(),
            ));
        }

        Ok(Sst {
            cf,
            entries,
            smallest,
            largest,
        })
    }

    pub fn column_family(&self) -> ColumnFamily {
        self.cf
    }

    /// Inclusive smallest key.
    pub fn smallest_key(&self) -> &[u8] {
        &self.smallest
    }

    /// Inclusive largest key.
    pub fn largest_key(&self) -> &[u8] {
        &self.largest
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether `key` falls inside this table's declared range.
    ///
    /// A cheap pre-filter only: inside the range does **not** mean present.
    pub fn may_contain(&self, key: &[u8]) -> bool {
        key >= self.smallest.as_slice() && key <= self.largest.as_slice()
    }

    /// Point lookup. `Ok(None)` means "verified absent from this table" — reachable only
    /// because [`Sst::parse`] already established the bytes are sound.
    pub fn get(&self, key: &[u8]) -> Option<&Value> {
        self.entries
            .binary_search_by(|(k, _)| k.as_slice().cmp(key))
            .ok()
            .map(|i| &self.entries[i].1)
    }

    /// Entries in `[start, end)`, ascending.
    pub fn range<'a>(
        &'a self,
        start: &'a [u8],
        end: &'a [u8],
    ) -> impl Iterator<Item = (&'a [u8], &'a [u8])> {
        self.entries
            .iter()
            .filter(move |(k, _)| k.as_slice() >= start && k.as_slice() < end)
            .map(|(k, v)| (k.as_slice(), v.as_slice()))
    }

    pub fn iter(&self) -> impl Iterator<Item = (&[u8], &[u8])> {
        self.entries
            .iter()
            .map(|(k, v)| (k.as_slice(), v.as_slice()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(cf: ColumnFamily, pairs: &[(&[u8], &[u8])]) -> Vec<u8> {
        let mut w = SstWriter::new(cf);
        for (k, v) in pairs {
            w.add(k.to_vec(), v.to_vec()).unwrap();
        }
        w.finish().unwrap()
    }

    #[test]
    fn round_trip_preserves_entries_and_range() {
        let bytes = build(
            ColumnFamily::Default,
            &[(b"a", b"1"), (b"b", b"2"), (b"c", b"3")],
        );
        let sst = Sst::parse(&bytes).unwrap();

        assert_eq!(sst.column_family(), ColumnFamily::Default);
        assert_eq!(sst.len(), 3);
        assert_eq!(sst.smallest_key(), b"a");
        assert_eq!(sst.largest_key(), b"c");
        assert_eq!(sst.get(b"b").map(Vec::as_slice), Some(b"2".as_slice()));
        assert_eq!(sst.get(b"zz"), None);
    }

    #[test]
    fn column_family_survives_the_round_trip() {
        // The CF is part of the file, not context the caller must remember: an SST read
        // back in isolation still knows which family it belongs to.
        for cf in ColumnFamily::ALL {
            let bytes = build(cf, &[(b"k", b"v")]);
            assert_eq!(Sst::parse(&bytes).unwrap().column_family(), cf);
        }
    }

    #[test]
    fn writer_refuses_unsorted_input() {
        let mut w = SstWriter::new(ColumnFamily::Default);
        w.add(b"b".to_vec(), b"1".to_vec()).unwrap();
        let err = w.add(b"a".to_vec(), b"2".to_vec()).unwrap_err();
        assert!(
            format!("{err}").contains("ascending"),
            "unsorted input must be refused, not silently sorted: {err}"
        );
    }

    #[test]
    fn writer_refuses_duplicate_keys() {
        let mut w = SstWriter::new(ColumnFamily::Default);
        w.add(b"a".to_vec(), b"1".to_vec()).unwrap();
        assert!(w.add(b"a".to_vec(), b"2".to_vec()).is_err());
    }

    #[test]
    fn writer_refuses_an_empty_table() {
        let err = SstWriter::new(ColumnFamily::Default).finish().unwrap_err();
        assert!(format!("{err}").contains("empty"));
    }

    // ---- fail-closed: every corruption is a typed error, never data and never `None` ----

    #[test]
    fn a_flipped_byte_anywhere_is_refused() {
        let bytes = build(
            ColumnFamily::Default,
            &[(b"a", b"1"), (b"b", b"2"), (b"c", b"3")],
        );
        // Every single-byte corruption, at every offset, must be caught. This is the
        // sensitivity control for the checksum: a test that flips one chosen byte proves
        // only that that byte is covered.
        for i in 0..bytes.len() {
            let mut corrupt = bytes.clone();
            corrupt[i] ^= 0xFF;
            assert!(
                Sst::parse(&corrupt).is_err(),
                "corruption at offset {i} was accepted"
            );
        }
    }

    #[test]
    fn truncation_at_every_length_is_refused() {
        let bytes = build(ColumnFamily::Default, &[(b"a", b"1"), (b"b", b"2")]);
        for n in 0..bytes.len() {
            assert!(
                Sst::parse(&bytes[..n]).is_err(),
                "a {n}-byte prefix was accepted as a whole SST"
            );
        }
    }

    #[test]
    fn trailing_bytes_are_refused() {
        // Appending data and fixing the checksum must still fail: the footer's end is the
        // file's end, and anything after it is a second file's worth of ambiguity.
        let bytes = build(ColumnFamily::Default, &[(b"a", b"1")]);
        let mut extended = bytes[..bytes.len() - 4].to_vec();
        extended.extend_from_slice(b"junk");
        let crc = crc32(&extended);
        extended.extend_from_slice(&crc.to_le_bytes());
        let err = Sst::parse(&extended).unwrap_err();
        assert!(format!("{err}").contains("trailing"), "{err}");
    }

    #[test]
    fn an_unknown_version_is_refused_rather_than_guessed() {
        let bytes = build(ColumnFamily::Default, &[(b"a", b"1")]);
        let mut future = bytes[..bytes.len() - 4].to_vec();
        future[4] = VERSION + 1;
        let crc = crc32(&future);
        future.extend_from_slice(&crc.to_le_bytes());
        let err = Sst::parse(&future).unwrap_err();
        assert!(format!("{err}").contains("unknown format version"), "{err}");
    }

    #[test]
    fn bad_magic_is_refused() {
        let mut bytes = build(ColumnFamily::Default, &[(b"a", b"1")]);
        let n = bytes.len();
        bytes[0] = b'X';
        let crc = crc32(&bytes[..n - 4]);
        bytes[n - 4..].copy_from_slice(&crc.to_le_bytes());
        let err = Sst::parse(&bytes).unwrap_err();
        assert!(format!("{err}").contains("magic"), "{err}");
    }

    #[test]
    fn an_unknown_column_family_is_refused() {
        let bytes = build(ColumnFamily::Default, &[(b"a", b"1")]);
        let mut bad = bytes[..bytes.len() - 4].to_vec();
        bad[5] = 9;
        let crc = crc32(&bad);
        bad.extend_from_slice(&crc.to_le_bytes());
        let err = Sst::parse(&bad).unwrap_err();
        assert!(format!("{err}").contains("column-family"), "{err}");
    }

    #[test]
    fn a_footer_range_that_disagrees_with_the_entries_is_refused() {
        // The declared range is verified, not believed. Hand-build a file whose footer
        // claims a wider range than its entries cover.
        let mut body = Vec::new();
        body.extend_from_slice(&MAGIC);
        body.push(VERSION);
        body.push(cf_code(ColumnFamily::Default));
        body.extend_from_slice(&1u32.to_le_bytes());
        body.extend_from_slice(&1u32.to_le_bytes());
        body.push(b'b');
        body.extend_from_slice(&1u32.to_le_bytes());
        body.push(b'v');
        // footer claims min="a", max="z" while the only entry is "b"
        body.extend_from_slice(&1u32.to_le_bytes());
        body.push(b'a');
        body.extend_from_slice(&1u32.to_le_bytes());
        body.push(b'z');
        let crc = crc32(&body);
        body.extend_from_slice(&crc.to_le_bytes());

        let err = Sst::parse(&body).unwrap_err();
        assert!(format!("{err}").contains("declared key range"), "{err}");
    }

    #[test]
    fn out_of_order_entries_in_a_well_formed_file_are_refused() {
        // The writer cannot produce this; a corrupted or hostile file can.
        let mut body = Vec::new();
        body.extend_from_slice(&MAGIC);
        body.push(VERSION);
        body.push(cf_code(ColumnFamily::Default));
        body.extend_from_slice(&2u32.to_le_bytes());
        for k in [b'b', b'a'] {
            body.extend_from_slice(&1u32.to_le_bytes());
            body.push(k);
            body.extend_from_slice(&1u32.to_le_bytes());
            body.push(b'v');
        }
        body.extend_from_slice(&1u32.to_le_bytes());
        body.push(b'b');
        body.extend_from_slice(&1u32.to_le_bytes());
        body.push(b'a');
        let crc = crc32(&body);
        body.extend_from_slice(&crc.to_le_bytes());

        let err = Sst::parse(&body).unwrap_err();
        assert!(format!("{err}").contains("out of order"), "{err}");
    }

    #[test]
    fn a_corrupt_sst_never_answers_not_found() {
        // The distinction this whole module exists for: "absent" and "unreadable" must not
        // look alike. A reader that returned Ok(None) here would report data loss as a
        // missing key.
        let bytes = build(ColumnFamily::Default, &[(b"a", b"1")]);
        let mut corrupt = bytes.clone();
        let n = corrupt.len();
        corrupt[n - 8] ^= 0xFF;
        match Sst::parse(&corrupt) {
            Err(_) => {}
            Ok(sst) => panic!(
                "corrupt SST parsed; get(\"a\") would have answered {:?}",
                sst.get(b"a")
            ),
        }
    }

    // ---- range behaviour ----

    #[test]
    fn may_contain_is_a_filter_not_an_answer() {
        let bytes = build(ColumnFamily::Default, &[(b"a", b"1"), (b"z", b"2")]);
        let sst = Sst::parse(&bytes).unwrap();
        assert!(sst.may_contain(b"m"), "m is inside [a, z]");
        assert_eq!(sst.get(b"m"), None, "but it is not present");
        assert!(!sst.may_contain(b"zz"));
    }

    #[test]
    fn range_is_half_open() {
        let bytes = build(
            ColumnFamily::Default,
            &[(b"a", b"1"), (b"b", b"2"), (b"c", b"3")],
        );
        let sst = Sst::parse(&bytes).unwrap();
        let got: Vec<_> = sst.range(b"a", b"c").map(|(k, _)| k.to_vec()).collect();
        assert_eq!(got, vec![b"a".to_vec(), b"b".to_vec()], "end is exclusive");
    }

    // ---- integer boundaries and resource limits ----

    #[test]
    fn on_disk_len_refuses_rather_than_truncating() {
        // The defect this replaced: `n as u32` is a silent truncating cast, so a length of
        // u32::MAX + 5 was written as 4 — a well-formed file with a wrong length prefix.
        // Tested through the predicate so the boundary is reachable without allocating it.
        assert!(on_disk_len(0, "k").is_ok());
        assert!(
            on_disk_len(MAX_FIELD_LEN, "k").is_ok(),
            "the limit itself is allowed"
        );

        let over = on_disk_len(MAX_FIELD_LEN + 1, "k").unwrap_err();
        assert!(format!("{over}").contains("exceeds"), "{over}");

        let huge = on_disk_len((u32::MAX as usize) + 5, "k").unwrap_err();
        assert!(format!("{huge}").contains("exceeds"), "{huge}");

        // And the cast it replaced would have produced this instead of an error:
        assert_eq!(
            ((u32::MAX as usize) + 5) as u32,
            4,
            "the truncation being prevented"
        );
    }

    #[test]
    fn writer_and_reader_enforce_the_same_field_limit() {
        // Asymmetry here would let the writer emit a table its own reader refuses, with the
        // failure surfacing at read time — possibly after a manifest already references it.
        // Both sides are checked against MAX_FIELD_LEN; assert they are the same number.
        let mut w = SstWriter::new(ColumnFamily::Default);
        let ok = w.add(b"k".to_vec(), vec![0u8; 1024]);
        assert!(ok.is_ok());

        // Reader's bound, exercised through a hand-built length prefix just over the limit.
        let mut body = Vec::new();
        body.extend_from_slice(&MAGIC);
        body.push(VERSION);
        body.push(cf_code(ColumnFamily::Default));
        body.extend_from_slice(&1u32.to_le_bytes());
        body.extend_from_slice(&((MAX_FIELD_LEN + 1) as u32).to_le_bytes());
        let crc = crc32(&body);
        body.extend_from_slice(&crc.to_le_bytes());
        let err = Sst::parse(&body).unwrap_err();
        assert!(
            format!("{err}").contains(&format!("{MAX_FIELD_LEN}")),
            "reader must cite the same limit the writer does: {err}"
        );
    }

    #[test]
    fn an_entry_count_beyond_the_limit_is_refused_before_allocating() {
        // A count field is untrusted input. The limit is on entries, not bytes: reusing a
        // byte constant here bounded nothing, since 1<<30 entries is gigabytes.
        let mut body = Vec::new();
        body.extend_from_slice(&MAGIC);
        body.push(VERSION);
        body.push(cf_code(ColumnFamily::Default));
        body.extend_from_slice(&((MAX_ENTRIES + 1) as u32).to_le_bytes());
        let crc = crc32(&body);
        body.extend_from_slice(&crc.to_le_bytes());
        let err = Sst::parse(&body).unwrap_err();
        assert!(format!("{err}").contains("entry limit"), "{err}");
    }

    #[test]
    fn whatever_the_writer_produces_the_same_version_parser_accepts() {
        // The symmetry stated as a property rather than as matching constants: it is not
        // enough that both sides *cite* MAX_FIELD_LEN, the writer's actual output must be
        // acceptable. Shapes chosen to sit on the edges the format cares about.
        let cases: Vec<Vec<(Vec<u8>, Vec<u8>)>> = vec![
            vec![(b"k".to_vec(), b"".to_vec())],
            vec![(b"".to_vec(), b"v".to_vec())],
            vec![(vec![0u8], vec![0u8; 1024])],
            vec![(vec![0xFF; 300], vec![0xFF; 300])],
            (0u8..64).map(|i| (vec![i], vec![i; i as usize])).collect(),
        ];
        for (i, pairs) in cases.iter().enumerate() {
            let mut w = SstWriter::new(ColumnFamily::Default);
            for (k, v) in pairs {
                w.add(k.clone(), v.clone()).unwrap();
            }
            let bytes = w.finish().unwrap_or_else(|e| panic!("case {i} write: {e}"));
            let sst = Sst::parse(&bytes).unwrap_or_else(|e| {
                panic!(
                    "case {i}: writer emitted {} bytes its own parser refused: {e}",
                    bytes.len()
                )
            });
            assert_eq!(sst.len(), pairs.len(), "case {i}");
            for (k, v) in pairs {
                assert_eq!(
                    sst.get(k).map(Vec::as_slice),
                    Some(v.as_slice()),
                    "case {i}"
                );
            }
        }
    }

    #[test]
    fn encoded_len_predicts_the_serialised_size_exactly() {
        // The pre-serialisation size check is only a real bound if it measures the same
        // quantity the writer goes on to produce.
        for pairs in [
            vec![(b"a".to_vec(), b"1".to_vec())],
            vec![(b"".to_vec(), b"".to_vec())],
            (0u8..32).map(|i| (vec![i], vec![i; 7])).collect::<Vec<_>>(),
        ] {
            let mut w = SstWriter::new(ColumnFamily::Default);
            for (k, v) in &pairs {
                w.add(k.clone(), v.clone()).unwrap();
            }
            let predicted = w.encoded_len().unwrap();
            let actual = w.finish().unwrap().len();
            assert_eq!(predicted, actual, "predicted size must equal produced size");
        }
    }

    #[test]
    fn every_limit_is_tested_at_exactly_max_and_max_plus_one() {
        // MAX_FIELD_LEN: predicate form, so the boundary is reachable without allocating it.
        assert!(
            on_disk_len(MAX_FIELD_LEN, "k").is_ok(),
            "exact max is allowed"
        );
        assert!(
            on_disk_len(MAX_FIELD_LEN + 1, "k").is_err(),
            "max+1 refused"
        );

        // MAX_ENTRIES: reader side, via a count field on either side of the limit.
        let count_frame = |n: usize| {
            let mut b = Vec::new();
            b.extend_from_slice(&MAGIC);
            b.push(VERSION);
            b.push(cf_code(ColumnFamily::Default));
            b.extend_from_slice(&(n as u32).to_le_bytes());
            let crc = crc32(&b);
            b.extend_from_slice(&crc.to_le_bytes());
            b
        };
        let at_max = Sst::parse(&count_frame(MAX_ENTRIES)).unwrap_err();
        assert!(
            !format!("{at_max}").contains("entry limit"),
            "exact max must pass the COUNT limit and fail later on truncation, not on the \
             limit itself: {at_max}"
        );
        let over = Sst::parse(&count_frame(MAX_ENTRIES + 1)).unwrap_err();
        assert!(format!("{over}").contains("entry limit"), "{over}");

        // MAX_SST_BYTES has its own cell below; the relationships between the three
        // constants are compile-time assertions near their definitions, because an
        // assertion over constants cannot fail at runtime for a behavioural reason.
    }

    #[test]
    fn check_total_size_refuses_at_the_boundary() {
        // Exists because the inline form of this check could not be exercised without
        // building a gigabyte, so a mutation removing it SURVIVED. Found by
        // scripts/mutation-guard.sh, not by reading the code.
        assert!(check_total_size(0).is_ok());
        assert!(
            check_total_size(MAX_SST_BYTES).is_ok(),
            "exactly the limit is allowed"
        );
        let err = check_total_size(MAX_SST_BYTES + 1).unwrap_err();
        assert!(format!("{err}").contains("byte limit"), "{err}");
    }

    #[test]
    fn an_oversized_buffer_is_refused_before_the_checksum_is_computed() {
        // Ordering matters: CRC over an untrusted gigabyte is exactly the cost the limit
        // exists to avoid, so the size check must come first. Asserted by the error text —
        // a buffer this size cannot produce a checksum complaint if it never got there.
        let huge = vec![0u8; MAX_SST_BYTES + 1];
        let err = Sst::parse(&huge).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("exceeds"), "{msg}");
        assert!(
            !msg.contains("checksum"),
            "size must be refused before CRC: {msg}"
        );
    }

    // ---- input-space coverage, which mutation evidence does not reach ----
    //
    // Mutation testing perturbs *code*: "if this check is removed or loosened, does a test
    // go red?". It says nothing about *which inputs reach a line*. Both are needed, and
    // neither implies the other — a function can have every check load-bearing and still
    // panic on a value no test ever supplies. (Cindy, on task #9: three mutations all fired
    // while a `u64::MAX` overflow sat in the same function.)

    #[test]
    fn no_buffer_of_any_short_length_panics() {
        // Every length from 0 through past the header, in several byte patterns. The
        // assertion is not a value — it is that parse RETURNS. A panic fails the test.
        for pattern in [0x00u8, 0xFF, b'K'] {
            for n in 0..80usize {
                let buf = vec![pattern; n];
                let _ = Sst::parse(&buf);
            }
        }
        // And a well-formed prefix followed by nothing, at each cut point.
        let good = build(ColumnFamily::Default, &[(b"a", b"1"), (b"bb", b"22")]);
        for n in 0..good.len() {
            let _ = Sst::parse(&good[..n]);
        }
    }

    #[test]
    fn extreme_header_values_are_refused_without_panicking() {
        // The count and length fields are attacker-controlled u32s. u32::MAX is the value a
        // limit check must survive, and it is not the same input as `limit + 1`: the tests
        // above use MAX_ENTRIES + 1, which is nowhere near the type's edge.
        let frame = |count: u32, first_len: Option<u32>| {
            let mut b = Vec::new();
            b.extend_from_slice(&MAGIC);
            b.push(VERSION);
            b.push(cf_code(ColumnFamily::Default));
            b.extend_from_slice(&count.to_le_bytes());
            if let Some(l) = first_len {
                b.extend_from_slice(&l.to_le_bytes());
            }
            let crc = crc32(&b);
            b.extend_from_slice(&crc.to_le_bytes());
            b
        };

        for count in [0u32, 1, u32::MAX - 1, u32::MAX] {
            for len in [
                None,
                Some(0u32),
                Some(1),
                Some(u32::MAX - 1),
                Some(u32::MAX),
            ] {
                let buf = frame(count, len);
                // Must be a typed refusal, never a panic and never a successful parse of a
                // frame that carries no entries.
                assert!(
                    Sst::parse(&buf).is_err(),
                    "count={count} len={len:?} was accepted"
                );
            }
        }
    }

    #[test]
    fn a_length_field_at_the_type_edge_is_refused_by_the_limit_not_by_arithmetic() {
        // u32::MAX as a field length must be rejected by the MAX_FIELD_LEN check, so the
        // refusal is a policy decision rather than an allocation failure or a wrap.
        let err = on_disk_len(u32::MAX as usize, "key").unwrap_err();
        assert!(format!("{err}").contains("exceeds"), "{err}");
        assert!(
            !format!("{err}").contains("does not fit"),
            "u32::MAX fits in u32; the limit must reject it first: {err}"
        );
    }

    #[test]
    fn empty_values_survive() {
        let bytes = build(ColumnFamily::Default, &[(b"k", b"")]);
        let sst = Sst::parse(&bytes).unwrap();
        assert_eq!(sst.get(b"k").map(Vec::as_slice), Some(b"".as_slice()));
    }
}
