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
//! MAGIC  4      "KV9S"
//! VERSION 1     format version; an unknown value is refused, never guessed
//! CF      1     which column family these entries belong to
//! COUNT   4     number of entries
//! entries       COUNT × ( klen u32 | key | vlen u32 | value ), sorted by key, unique
//! FOOTER
//!   min_len u32 | min_key
//!   max_len u32 | max_key
//!   crc     u4   CRC-32 over every preceding byte
//! ```
//!
//! The key range is stored explicitly rather than derived on open, so a reader can learn
//! whether an object is worth fetching without parsing its entries — that is what makes a
//! manifest's range-filtered read cheap later. It is also *verified* against the entries
//! on open, because a range that merely claims to bound the data would be a second source
//! of truth about the same thing.

use kv9_common::{Error, Result, UserKey, Value};

use crate::cf::ColumnFamily;

const MAGIC: [u8; 4] = *b"KV9S";
const VERSION: u8 = 1;

/// Guards against a corrupt length field driving a huge allocation, exactly as the WAL
/// does. A single SST far larger than this is a bug, not a legitimate flush.
const MAX_SST_LEN: usize = 1 << 30;

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
    pub fn add(&mut self, key: UserKey, value: Value) -> Result<()> {
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

        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC);
        out.push(VERSION);
        out.push(cf_code(self.cf));
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());

        for (k, v) in &self.entries {
            out.extend_from_slice(&(k.len() as u32).to_le_bytes());
            out.extend_from_slice(k);
            out.extend_from_slice(&(v.len() as u32).to_le_bytes());
            out.extend_from_slice(v);
        }

        let smallest = &self.entries.first().expect("non-empty checked above").0;
        let largest = &self.entries.last().expect("non-empty checked above").0;
        out.extend_from_slice(&(smallest.len() as u32).to_le_bytes());
        out.extend_from_slice(smallest);
        out.extend_from_slice(&(largest.len() as u32).to_le_bytes());
        out.extend_from_slice(largest);

        let crc = crc32(&out);
        out.extend_from_slice(&crc.to_le_bytes());
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
        if n > MAX_SST_LEN {
            return Err(Error::Engine(format!(
                "sst: length field {n} exceeds the {MAX_SST_LEN} byte limit"
            )));
        }
        Ok(self.take(n)?.to_vec())
    }
}

impl Sst {
    /// Parse and verify. Every failure here is a refusal, never a silent empty result.
    pub fn parse(bytes: &[u8]) -> Result<Sst> {
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
        if count > MAX_SST_LEN {
            return Err(Error::Engine(format!(
                "sst: entry count {count} exceeds the {MAX_SST_LEN} limit"
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

    #[test]
    fn empty_values_survive() {
        let bytes = build(ColumnFamily::Default, &[(b"k", b"")]);
        let sst = Sst::parse(&bytes).unwrap();
        assert_eq!(sst.get(b"k").map(Vec::as_slice), Some(b"".as_slice()));
    }
}
