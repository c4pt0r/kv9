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
//! requires validating the whole object, including every entry. Pre-fetch filtering
//! is the manifest's job (`docs/OBJECT-STORAGE.md` §5: `PreparedSst` carries `key_range`,
//! and the manifest is consulted before any object is retrieved). Giving *this* layout that
//! capability would need a fixed-offset trailer plus an API that reads it alone, and round
//! one does not require it. Recorded because an earlier revision of this comment claimed
//! the capability, which the code has never had.

use kv9_common::{Error, Result, UserKey, Value};

use crate::cf::ColumnFamily;

mod view;
pub(crate) use view::{SharedSst, SstView};
mod encode;
pub(crate) use encode::encode_borrowed;

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

/// The three limits, carried as a value.
///
/// **Why a value and not three constants read directly:** a limit whose only boundary lies
/// at a gigabyte cannot be driven by a test, so nothing can stand at the production call
/// site. Extracting the *predicate* into a free function was not enough — that let a unit
/// test prove the predicate while a mutation deleting the **call** still survived, because
/// proving a helper is correct is a different claim from proving production invokes it.
/// Passing limits in lets a test drive the real code path at a small boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Limits {
    field_len: usize,
    entries: usize,
    total_bytes: usize,
}

impl Limits {
    const PRODUCTION: Limits = Limits {
        field_len: MAX_FIELD_LEN,
        entries: MAX_ENTRIES,
        total_bytes: MAX_SST_BYTES,
    };
}

/// Convert a *byte length* to its on-disk `u32`, refusing rather than truncating.
///
/// `n as u32` is a *silent* truncating cast: a length of 4_294_967_300 is written as 4,
/// producing a well-formed file with a wrong length prefix. This module's whole stance is
/// "refuse, never guess", and the cast broke that on the write side, where the damage only
/// becomes visible at read time.
fn on_disk_len(n: usize, what: &str, limit: usize) -> Result<u32> {
    if n > limit {
        return Err(Error::Engine(format!(
            "sst: {what} of {n} bytes exceeds the {limit} byte limit"
        )));
    }
    u32::try_from(n)
        .map_err(|_| Error::Engine(format!("sst: {what} length {n} does not fit in u32")))
}

/// Convert an *entry count* to its on-disk `u32`.
///
/// Separate from [`on_disk_len`] because a count and a byte length are different
/// dimensions. Routing the count through the byte checker — which an earlier revision did —
/// re-merges exactly the two quantities this module split apart, and leaves the count with
/// no boundary of its own.
fn on_disk_count(n: usize, limit: usize) -> Result<u32> {
    if n > limit {
        return Err(Error::Engine(format!(
            "sst: entry count {n} exceeds the {limit} entry limit"
        )));
    }
    u32::try_from(n).map_err(|_| Error::Engine(format!("sst: entry count {n} does not fit in u32")))
}

/// Refuse a whole-table size over the limit.
fn check_total_size(encoded: usize, limit: usize) -> Result<()> {
    if encoded > limit {
        return Err(Error::Engine(format!(
            "sst: table would encode to {encoded} bytes, over the {limit} byte limit"
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
    entry_bytes: usize,
    limits: Limits,
}

impl SstWriter {
    pub fn new(cf: ColumnFamily) -> Self {
        SstWriter {
            cf,
            entries: Vec::new(),
            entry_bytes: 0,
            limits: Limits::PRODUCTION,
        }
    }

    /// A writer with small limits, so the production gates can be *driven* at a boundary
    /// instead of merely unit-tested in isolation. Same code path as [`SstWriter::new`];
    /// only the numbers differ.
    #[cfg(test)]
    fn with_limits(cf: ColumnFamily, limits: Limits) -> Self {
        SstWriter {
            cf,
            entries: Vec::new(),
            entry_bytes: 0,
            limits,
        }
    }

    /// Append one entry. Keys must strictly ascend.
    ///
    /// Field lengths and the entry count are checked here, against the same limits the
    /// reader applies, so an over-large entry is refused at the point it is offered rather
    /// than discovered when the finished table fails to parse.
    pub fn add(&mut self, key: UserKey, value: Value) -> Result<()> {
        on_disk_len(key.len(), "key", self.limits.field_len)?;
        on_disk_len(value.len(), "value", self.limits.field_len)?;
        if self.entries.len() >= self.limits.entries {
            return Err(Error::Engine(format!(
                "sst: entry count would exceed the {} entry limit",
                self.limits.entries
            )));
        }
        if let Some((last, _)) = self.entries.last() {
            if key <= *last {
                return Err(Error::Engine(
                    "sst: entries must be added in strictly ascending key order".into(),
                ));
            }
        }
        self.entry_bytes = self.next_entry_bytes(key.len(), value.len())?;
        self.entries.push((key, value));
        Ok(())
    }

    /// Legacy checkpoint planner oracle for partition/acceptance regression tests.
    #[cfg(test)]
    pub(crate) fn add_bounded(&mut self, key: &[u8], value: &[u8], limit: usize) -> Result<()> {
        let entries = self.next_entry_bytes(key.len(), value.len())?;
        let smallest = self.entries.first().map_or(key.len(), |(k, _)| k.len());
        let encoded = Self::table_bytes(entries, smallest, key.len())?;
        check_total_size(encoded, limit.min(self.limits.total_bytes))?;
        self.add(key.to_vec(), value.to_vec())
    }

    fn next_entry_bytes(&self, key: usize, value: usize) -> Result<usize> {
        self.entry_bytes
            .checked_add(8)
            .and_then(|n| n.checked_add(key))
            .and_then(|n| n.checked_add(value))
            .ok_or_else(|| Error::Engine("sst: encoded length overflows usize".into()))
    }

    pub(crate) fn table_bytes(entries: usize, smallest: usize, largest: usize) -> Result<usize> {
        // Header (10), two footer length prefixes (8), checksum (4).
        entries
            .checked_add(22)
            .and_then(|n| n.checked_add(smallest))
            .and_then(|n| n.checked_add(largest))
            .ok_or_else(|| Error::Engine("sst: encoded length overflows usize".into()))
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
        let smallest = self.entries.first().map(|(k, _)| k.len()).unwrap_or(0);
        let largest = self.entries.last().map(|(k, _)| k.len()).unwrap_or(0);
        Self::table_bytes(self.entry_bytes, smallest, largest)
    }

    /// Serialise. Consumes the writer: an SST is written once.
    ///
    /// Refuses an empty table. An SST with no entries has no key range, so it could not be
    /// range-filtered, and a manifest referencing it would carry an entry that can never
    /// match a read — a silent waste rather than an error.
    pub fn finish(self) -> Result<Vec<u8>> {
        encode::encode_rows(
            self.cf,
            self.entries.len(),
            self.encoded_len()?,
            self.limits,
            self.entries
                .iter()
                .map(|(key, value)| (key.as_slice(), value.as_slice())),
        )
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

impl Sst {
    /// Parse and verify. Every failure here is a refusal, never a silent empty result.
    pub fn parse(bytes: &[u8]) -> Result<Sst> {
        Sst::parse_with_limits(bytes, Limits::PRODUCTION)
    }

    /// Parse under explicit limits. `parse` is this with the production numbers; tests use
    /// small ones so the real refusal path is driven at a reachable boundary instead of
    /// allocating an object the size of the production cap just to watch it be refused.
    fn parse_with_limits(bytes: &[u8], limits: Limits) -> Result<Sst> {
        let view = SstView::parse_with_limits(bytes, limits)?;
        Ok(Sst {
            cf: view.column_family(),
            entries: view.iter().map(|(k, v)| (k.to_vec(), v.to_vec())).collect(),
            smallest: view.smallest_key().to_vec(),
            largest: view.largest_key().to_vec(),
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
        for k in *b"ba" {
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
        assert!(on_disk_len(0, "k", MAX_FIELD_LEN).is_ok());
        assert!(
            on_disk_len(MAX_FIELD_LEN, "k", MAX_FIELD_LEN).is_ok(),
            "the limit itself is allowed"
        );

        let over = on_disk_len(MAX_FIELD_LEN + 1, "k", MAX_FIELD_LEN).unwrap_err();
        assert!(format!("{over}").contains("exceeds"), "{over}");

        let huge = on_disk_len((u32::MAX as usize) + 5, "k", MAX_FIELD_LEN).unwrap_err();
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
    fn boundary_shaped_writer_outputs_round_trip_through_the_parser() {
        // Symmetry as behaviour rather than as matching constants. NOTE the scope: five
        // boundary-shaped corpora, not a universal claim — the earlier name said "whatever
        // the writer produces", which is a for-all assertion resting on a finite sample.
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
            on_disk_len(MAX_FIELD_LEN, "k", MAX_FIELD_LEN).is_ok(),
            "exact max is allowed"
        );
        assert!(
            on_disk_len(MAX_FIELD_LEN + 1, "k", MAX_FIELD_LEN).is_err(),
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
        assert!(check_total_size(0, MAX_SST_BYTES).is_ok());
        assert!(
            check_total_size(MAX_SST_BYTES, MAX_SST_BYTES).is_ok(),
            "exactly the limit is allowed"
        );
        let err = check_total_size(MAX_SST_BYTES + 1, MAX_SST_BYTES).unwrap_err();
        assert!(format!("{err}").contains("byte limit"), "{err}");
    }

    #[test]
    fn an_oversized_buffer_is_refused_before_the_checksum_is_computed() {
        // Driven at a SMALL limit. An earlier version allocated MAX_SST_BYTES + 1 — about a
        // gigabyte — which contradicted the very purpose of the limit and would abort on a
        // constrained runner before `parse` was ever entered.
        let tiny = Limits {
            total_bytes: 16,
            ..Limits::PRODUCTION
        };
        let err = Sst::parse_with_limits(&[0u8; 17], tiny).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("exceeds"), "{msg}");
        assert!(
            !msg.contains("checksum"),
            "size must be refused before CRC is computed: {msg}"
        );
        // Exactly at the limit passes the SIZE gate and fails later, on structure.
        let at = Sst::parse_with_limits(&[0u8; 16], tiny).unwrap_err();
        assert!(
            !format!("{at}").contains("exceeds"),
            "exact limit must pass the size gate"
        );
    }

    #[test]
    fn the_writer_total_size_gate_is_reached_from_finish() {
        // Stands at the PRODUCTION CALL SITE, not on the helper. Deleting
        // `check_total_size(...)` from finish() must red this; a unit test of the predicate
        // alone cannot, which is how the deleted call previously survived a mutation.
        let tiny = Limits {
            total_bytes: 24,
            ..Limits::PRODUCTION
        };
        let mut w = SstWriter::with_limits(ColumnFamily::Default, tiny);
        w.add(b"key".to_vec(), vec![0u8; 64]).unwrap();
        let err = w.finish().unwrap_err();
        assert!(
            format!("{err}").contains("byte limit"),
            "finish() must consult the total-size gate: {err}"
        );
    }

    #[test]
    fn the_writer_entry_count_gate_is_reached_from_add() {
        // Stands at add()'s production gate. Disabling that condition must red this.
        let tiny = Limits {
            entries: 2,
            ..Limits::PRODUCTION
        };
        let mut w = SstWriter::with_limits(ColumnFamily::Default, tiny);
        w.add(b"a".to_vec(), b"1".to_vec()).unwrap();
        w.add(b"b".to_vec(), b"2".to_vec()).unwrap();
        let err = w.add(b"c".to_vec(), b"3".to_vec()).unwrap_err();
        assert!(format!("{err}").contains("entry limit"), "{err}");
    }

    #[test]
    fn the_writer_field_length_gate_is_reached_from_add() {
        let tiny = Limits {
            field_len: 4,
            ..Limits::PRODUCTION
        };
        // BOTH call sites, separately. An earlier version only offered an over-long VALUE,
        // so the key check had no witness and deleting it survived a mutation: "the
        // field-length gate is covered" was true of one of its two call sites.
        let mut w = SstWriter::with_limits(ColumnFamily::Default, tiny);
        assert!(
            w.add(b"ok".to_vec(), b"1234".to_vec()).is_ok(),
            "exact limit allowed on both fields"
        );

        let mut wk = SstWriter::with_limits(ColumnFamily::Default, tiny);
        let key_err = wk.add(b"12345".to_vec(), b"v".to_vec()).unwrap_err();
        assert!(
            format!("{key_err}").contains("key of 5 bytes"),
            "an over-long KEY must be refused, and named as the key: {key_err}"
        );

        let mut wv = SstWriter::with_limits(ColumnFamily::Default, tiny);
        let val_err = wv.add(b"k".to_vec(), b"12345".to_vec()).unwrap_err();
        assert!(
            format!("{val_err}").contains("value of 5 bytes"),
            "an over-long VALUE must be refused, and named as the value: {val_err}"
        );
    }

    #[test]
    fn on_disk_count_is_its_own_dimension() {
        // finish() previously routed the ENTRY COUNT through the BYTE checker, re-merging
        // the two quantities this module separated. Distinct helper, distinct limit.
        assert!(on_disk_count(4, 4).is_ok(), "exact limit allowed");
        let err = on_disk_count(5, 4).unwrap_err();
        assert!(format!("{err}").contains("entry limit"), "{err}");
        assert!(
            !format!("{err}").contains("byte"),
            "a count is not a byte length: {err}"
        );
    }

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
        let err = on_disk_len(u32::MAX as usize, "key", MAX_FIELD_LEN).unwrap_err();
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
