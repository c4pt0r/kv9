//! Append-only write-ahead log: record framing, encoding, and crash-tolerant replay.
//!
//! One record per [`crate::WriteBatch`]. The frame is deliberately boring:
//!
//! ```text
//! magic(4) version(1)=2 kind(1) [term(8) index(8)] len(4) payload(len) crc(4)
//! ```
//!
//! ## What the framing has to survive
//!
//! A process can die *during* an append, so the last record may be half-written. That is
//! not corruption, it is the normal shape of a crash, and replay must tolerate it: we
//! stop at the first record that does not verify and keep everything before it. The
//! alternative — refusing every partial tail — would prevent automatic restart.
//!
//! We cannot distinguish "torn tail" from "bit-rot in the middle" by inspection alone, so
//! we do not try: both stop replay at the same place. What that costs is stated plainly in
//! [`Replay::discarded_tail_bytes`], which reports how many bytes were discarded so a
//! caller can log or refuse rather than silently accept truncation.
//!
//! Versioned so an unknown version is rejected rather than misparsed (DESIGN §13
//! principle 12, "forward-compatible formats, never panic on the unknown").

use std::fs::{File, OpenOptions};
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::wal_v2::{kind, CRC_LEN, MAGIC, MAX_RECORD_LEN, VERSION_V1, VERSION_V2};
use kv9_common::{AppliedPosition, Error, Result};

use crate::cf::ColumnFamily;
use crate::write_batch::{Mutation, WriteBatch};

#[cfg(test)]
const HEADER_LEN: usize = crate::wal_v2::UNPOSITIONED_HEADER_LEN;

fn io(e: std::io::Error) -> Error {
    Error::Engine(format!("wal io: {e}"))
}

/// CRC-32 (IEEE), computed without pulling in a dependency.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
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
        other => Err(Error::Engine(format!("wal: unknown column family {other}"))),
    }
}

/// Serialize a batch's mutations: `count(4)` then `tag(1) cf(1) klen(4) k vlen(4) v`.
fn encode_batch(batch: &WriteBatch) -> Vec<u8> {
    let mut out = Vec::new();
    put_u32(&mut out, batch.mutations().len() as u32);
    for m in batch.mutations() {
        match m {
            Mutation::Put { cf, key, value } => {
                out.push(0);
                out.push(cf_code(*cf));
                put_u32(&mut out, key.len() as u32);
                out.extend_from_slice(key);
                put_u32(&mut out, value.len() as u32);
                out.extend_from_slice(value);
            }
            Mutation::Delete { cf, key } => {
                out.push(1);
                out.push(cf_code(*cf));
                put_u32(&mut out, key.len() as u32);
                out.extend_from_slice(key);
            }
        }
    }
    out
}

/// Cursor-based reader over a record payload; every read is bounds-checked so malformed
/// input yields a typed error rather than a panic.
struct Cursor<'a> {
    buf: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Cursor { buf, at: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .at
            .checked_add(n)
            .ok_or_else(|| Error::Engine("wal: length overflow".into()))?;
        if end > self.buf.len() {
            return Err(Error::Engine(format!(
                "wal: record truncated (want {n} at {}, have {})",
                self.at,
                self.buf.len() - self.at.min(self.buf.len())
            )));
        }
        let out = &self.buf[self.at..end];
        self.at = end;
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn bytes(&mut self) -> Result<Vec<u8>> {
        let n = self.u32()? as usize;
        Ok(self.take(n)?.to_vec())
    }
}

fn decode_batch(payload: &[u8]) -> Result<WriteBatch> {
    let mut c = Cursor::new(payload);
    let count = c.u32()?;
    let mut batch = WriteBatch::new();
    for _ in 0..count {
        let tag = c.u8()?;
        let cf = cf_from_code(c.u8()?)?;
        match tag {
            0 => {
                let key = c.bytes()?;
                let value = c.bytes()?;
                batch.put(cf, key, value);
            }
            1 => {
                let key = c.bytes()?;
                batch.delete(cf, key);
            }
            other => return Err(Error::Engine(format!("wal: unknown mutation tag {other}"))),
        }
    }
    if c.at != payload.len() {
        return Err(Error::Engine(format!(
            "wal: {} trailing bytes after {count} mutations",
            payload.len() - c.at
        )));
    }
    Ok(batch)
}

fn read_complete(reader: &mut impl Read, bytes: &mut [u8]) -> Result<bool> {
    match reader.read_exact(bytes) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(io(e)),
    }
}

fn check_position(previous: Option<AppliedPosition>, at: AppliedPosition) -> Result<()> {
    if previous.is_some_and(|p| at.index <= p.index) {
        return Err(Error::Engine(format!(
            "wal: applied position must advance; previous {previous:?}, refused {at:?}"
        )));
    }
    Ok(())
}

/// Outcome of replaying a log at open time.
#[derive(Debug, Clone)]
pub struct Replay {
    /// Batches recovered, oldest first.
    pub batches: Vec<WriteBatch>,
    /// Position attached to each batch, in the same order. v1 and unpositioned v2
    /// records carry None and never authorize reclaim.
    pub positions: Vec<Option<AppliedPosition>>,
    /// Bytes discarded from the tail because they did not form a complete, verified
    /// record — normally a partial append interrupted by a crash. Non-zero is expected
    /// after an unclean shutdown; it is surfaced rather than hidden so a caller can log
    /// it instead of silently accepting truncation.
    pub discarded_tail_bytes: u64,
}

/// An append-only write-ahead log file.
#[derive(Debug)]
pub struct Wal {
    path: PathBuf,
    file: File,
    applied: Option<AppliedPosition>,
    poisoned: bool,
}

impl Wal {
    /// Open (creating if absent) the log at `path` and replay it.
    ///
    /// Returns the recovered batches alongside the handle. The file is left positioned at
    /// the end of the last *valid* record, so a torn tail is overwritten by the next
    /// append rather than being read again on the following open.
    pub fn open(path: impl AsRef<Path>) -> Result<(Self, Replay)> {
        let path = path.as_ref().to_path_buf();
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir).map_err(io)?;
            }
        }
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .map_err(io)?;

        let replay = Self::replay(&mut file)?;

        // Drop a torn tail so the next append starts from a clean boundary.
        let valid_len = file.stream_position().map_err(io)?;
        if replay.discarded_tail_bytes > 0 {
            file.set_len(valid_len).map_err(io)?;
        }
        file.seek(SeekFrom::Start(valid_len)).map_err(io)?;

        // Persist creation/truncation before advertising a durable empty engine.
        file.sync_all().map_err(io)?;
        if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            File::open(dir).and_then(|f| f.sync_all()).map_err(io)?;
        }
        let applied = replay.positions.iter().rev().flatten().next().copied();
        Ok((
            Wal {
                path,
                file,
                applied,
                poisoned: false,
            },
            replay,
        ))
    }

    /// Read every complete, checksum-verified record, stopping at the first that is not.
    ///
    /// Leaves `file` positioned immediately after the last good record.
    fn replay(file: &mut File) -> Result<Replay> {
        let total = file.metadata().map_err(io)?.len();
        file.seek(SeekFrom::Start(0)).map_err(io)?;
        let mut reader = BufReader::new(&mut *file);

        let mut batches = Vec::new();
        let mut positions = Vec::new();
        let mut previous: Option<AppliedPosition> = None;
        let mut good_end: u64 = 0;

        loop {
            let mut preamble = [0u8; 5];
            if !read_complete(&mut reader, &mut preamble)? {
                break;
            }
            if preamble[..4] != MAGIC {
                break;
            }
            let mut check = vec![preamble[4]];
            let position = match preamble[4] {
                VERSION_V1 => None,
                VERSION_V2 => {
                    let mut tag = [0u8; 1];
                    if !read_complete(&mut reader, &mut tag)? {
                        break;
                    }
                    check.extend_from_slice(&tag);
                    match tag[0] {
                        kind::UNPOSITIONED => None,
                        kind::POSITIONED => {
                            let mut bytes = [0u8; 16];
                            if !read_complete(&mut reader, &mut bytes)? {
                                break;
                            }
                            check.extend_from_slice(&bytes);
                            Some(AppliedPosition {
                                term: u64::from_le_bytes(bytes[..8].try_into().unwrap()),
                                index: u64::from_le_bytes(bytes[8..].try_into().unwrap()),
                            })
                        }
                        other => {
                            return Err(Error::Engine(format!("wal: unknown record kind {other}")))
                        }
                    }
                }
                other => {
                    return Err(Error::Engine(format!(
                        "wal: unsupported record version {other}"
                    )))
                }
            };
            let mut len_bytes = [0u8; 4];
            if !read_complete(&mut reader, &mut len_bytes)? {
                break;
            }
            check.extend_from_slice(&len_bytes);
            let len = u32::from_le_bytes(len_bytes);
            if len > MAX_RECORD_LEN {
                break;
            }
            let mut body = vec![0u8; len as usize + CRC_LEN];
            if !read_complete(&mut reader, &mut body)? {
                break;
            }
            let (payload, crc_bytes) = body.split_at(len as usize);
            check.extend_from_slice(payload);
            let want = u32::from_le_bytes(crc_bytes.try_into().unwrap());
            if crc32(&check) != want {
                break;
            }
            // Only a complete, CRC-valid record can assert a position. An ordering
            // violation fails the entire open; it must never truncate to a usable prefix.
            if let Some(at) = position {
                check_position(previous, at)?;
                previous = Some(at);
            }
            batches.push(decode_batch(payload)?);
            positions.push(position);
            good_end += (4 + check.len() + CRC_LEN) as u64;
        }

        file.seek(SeekFrom::Start(good_end)).map_err(io)?;
        Ok(Replay {
            batches,
            positions,
            discarded_tail_bytes: total - good_end,
        })
    }

    /// Append an unpositioned v2 batch. It conveys no replicated progress.
    pub fn append(&mut self, batch: &WriteBatch) -> Result<()> {
        self.append_record(batch, None)
    }

    /// Persist the batch and its exact apply position under a single CRC and fsync.
    pub fn append_applied(&mut self, batch: &WriteBatch, at: AppliedPosition) -> Result<()> {
        check_position(self.applied, at)?;
        self.append_record(batch, Some(at))
    }

    fn append_record(&mut self, batch: &WriteBatch, at: Option<AppliedPosition>) -> Result<()> {
        if self.poisoned {
            return Err(Error::Engine(
                "wal: previous append failed; reopen required".into(),
            ));
        }
        let payload = encode_batch(batch);
        let len = u32::try_from(payload.len())
            .map_err(|_| Error::Engine("wal: batch exceeds u32 length".into()))?;
        if len > MAX_RECORD_LEN {
            return Err(Error::Engine(format!(
                "wal: batch exceeds the {MAX_RECORD_LEN} byte record limit"
            )));
        }
        let mut check = vec![
            VERSION_V2,
            if at.is_some() {
                kind::POSITIONED
            } else {
                kind::UNPOSITIONED
            },
        ];
        if let Some(at) = at {
            check.extend_from_slice(&at.term.to_le_bytes());
            check.extend_from_slice(&at.index.to_le_bytes());
        }
        check.extend_from_slice(&len.to_le_bytes());
        check.extend_from_slice(&payload);
        let mut record = MAGIC.to_vec();
        record.extend_from_slice(&check);
        record.extend_from_slice(&crc32(&check).to_le_bytes());
        // Never append a successful record behind an incomplete/uncertain append.
        // Otherwise replay would discard a subsequently acknowledged write.
        if let Err(e) = self
            .file
            .write_all(&record)
            .and_then(|_| self.file.sync_all())
        {
            self.poisoned = true;
            return Err(io(e));
        }
        if at.is_some() {
            self.applied = at;
        }
        Ok(())
    }

    pub(crate) fn relocated(&mut self, path: PathBuf) {
        self.path = path;
    }

    /// The log's path, for diagnostics.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch(entries: &[(&[u8], &[u8])]) -> WriteBatch {
        let mut b = WriteBatch::new();
        for (k, v) in entries {
            b.put(ColumnFamily::Default, k.to_vec(), v.to_vec());
        }
        b
    }

    fn tmpdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kv9-wal-test-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn roundtrip_across_reopen() {
        let path = tmpdir("roundtrip").join("wal");
        {
            let (mut wal, replay) = Wal::open(&path).unwrap();
            assert!(replay.batches.is_empty(), "a fresh log replays as empty");
            wal.append(&batch(&[(b"a", b"1")])).unwrap();
            wal.append(&batch(&[(b"b", b"2")])).unwrap();
        }
        let (_wal, replay) = Wal::open(&path).unwrap();
        assert_eq!(replay.batches.len(), 2);
        assert_eq!(replay.discarded_tail_bytes, 0);
        assert_eq!(replay.batches[0].mutations().len(), 1);
    }

    #[test]
    fn deletes_and_multiple_column_families_survive() {
        let path = tmpdir("cfs").join("wal");
        {
            let (mut wal, _) = Wal::open(&path).unwrap();
            let mut b = WriteBatch::new();
            b.put(ColumnFamily::Default, b"d".to_vec(), b"1".to_vec());
            b.put(ColumnFamily::Lock, b"l".to_vec(), b"2".to_vec());
            b.delete(ColumnFamily::Write, b"w".to_vec());
            wal.append(&b).unwrap();
        }
        let (_w, replay) = Wal::open(&path).unwrap();
        let ms = replay.batches[0].mutations();
        assert_eq!(ms.len(), 3);
        assert!(matches!(
            ms[1],
            Mutation::Put {
                cf: ColumnFamily::Lock,
                ..
            }
        ));
        assert!(matches!(
            ms[2],
            Mutation::Delete {
                cf: ColumnFamily::Write,
                ..
            }
        ));
    }

    /// A crash mid-append leaves a partial record. Everything committed before it must
    /// still come back, and opening must not fail.
    #[test]
    fn torn_tail_keeps_the_committed_prefix() {
        let path = tmpdir("torn").join("wal");
        {
            let (mut wal, _) = Wal::open(&path).unwrap();
            wal.append(&batch(&[(b"a", b"1")])).unwrap();
            wal.append(&batch(&[(b"b", b"2")])).unwrap();
        }
        let full = std::fs::metadata(&path).unwrap().len();

        // Chop the file at every byte inside the second record; the first must always
        // survive and the open must always succeed.
        let (first_len, _) = {
            let (_w, r) = Wal::open(&path).unwrap();
            (r.batches.len(), r)
        };
        assert_eq!(first_len, 2);

        for cut in (full / 2)..full {
            std::fs::copy(&path, path.with_extension("bak")).unwrap();
            let f = OpenOptions::new().write(true).open(&path).unwrap();
            f.set_len(cut).unwrap();
            drop(f);

            let (_w, replay) = Wal::open(&path).expect("a torn tail must not fail the open");
            assert!(
                replay.batches.len() <= 2,
                "replay must never invent records"
            );
            if cut >= full / 2 {
                assert!(
                    !replay.batches.is_empty(),
                    "the first committed record must survive a tear at byte {cut}"
                );
            }
            std::fs::copy(path.with_extension("bak"), &path).unwrap();
        }
    }

    /// Control for the test above: if the tear detection were broken (e.g. it accepted
    /// anything), this would pass too. So check the converse — a flipped byte in a
    /// *complete* record must be caught by the CRC and stop replay.
    #[test]
    fn a_corrupted_record_is_detected_not_returned() {
        let path = tmpdir("corrupt").join("wal");
        {
            let (mut wal, _) = Wal::open(&path).unwrap();
            wal.append(&batch(&[(b"a", b"1")])).unwrap();
            wal.append(&batch(&[(b"b", b"2")])).unwrap();
        }
        // Flip a byte inside the first record's payload.
        let mut bytes = std::fs::read(&path).unwrap();
        let victim = HEADER_LEN + 6;
        bytes[victim] ^= 0xFF;
        std::fs::write(&path, &bytes).unwrap();

        let (_w, replay) = Wal::open(&path).unwrap();
        assert_eq!(
            replay.batches.len(),
            0,
            "a corrupt first record must stop replay, not be handed back"
        );
        assert!(replay.discarded_tail_bytes > 0);
    }

    #[test]
    fn unknown_version_is_rejected_rather_than_misparsed() {
        let path = tmpdir("version").join("wal");
        {
            let (mut wal, _) = Wal::open(&path).unwrap();
            wal.append(&batch(&[(b"a", b"1")])).unwrap();
        }
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[4] = 99; // version byte
        std::fs::write(&path, &bytes).unwrap();

        let err = Wal::open(&path);
        assert!(
            err.is_err(),
            "an unknown format version must be an error, not a guess"
        );
    }

    #[test]
    fn garbage_file_does_not_panic() {
        let path = tmpdir("garbage").join("wal");
        std::fs::write(&path, b"this is not a kv9 write-ahead log at all").unwrap();
        let (_w, replay) = Wal::open(&path).unwrap();
        assert!(replay.batches.is_empty());
        assert!(replay.discarded_tail_bytes > 0);
    }

    /// A corrupt length field must not drive a huge allocation.
    #[test]
    fn absurd_length_is_refused() {
        let path = tmpdir("len").join("wal");
        {
            let (mut wal, _) = Wal::open(&path).unwrap();
            wal.append(&batch(&[(b"a", b"1")])).unwrap();
        }
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[6..10].copy_from_slice(&u32::MAX.to_le_bytes());
        std::fs::write(&path, &bytes).unwrap();

        let (_w, replay) = Wal::open(&path).unwrap();
        assert!(replay.batches.is_empty());
    }

    /// After reopening a torn log, the next append must land at a clean boundary and be
    /// readable — otherwise recovery works once and corrupts the log thereafter.
    #[test]
    fn append_after_recovery_is_readable() {
        let path = tmpdir("reappend").join("wal");
        {
            let (mut wal, _) = Wal::open(&path).unwrap();
            wal.append(&batch(&[(b"a", b"1")])).unwrap();
        }
        // Simulate a crash mid-second-append by appending junk.
        {
            let mut f = OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(b"KV9W\x01\x05\x00").unwrap();
        }
        {
            let (mut wal, replay) = Wal::open(&path).unwrap();
            assert_eq!(replay.batches.len(), 1);
            assert!(replay.discarded_tail_bytes > 0);
            wal.append(&batch(&[(b"c", b"3")])).unwrap();
        }
        let (_w, replay) = Wal::open(&path).unwrap();
        assert_eq!(
            replay.batches.len(),
            2,
            "the record written after recovery must be readable"
        );
        assert_eq!(replay.discarded_tail_bytes, 0);
    }

    #[test]
    fn crc32_matches_known_vectors() {
        // Guards against an arithmetic slip in the hand-rolled CRC.
        assert_eq!(crc32(b""), 0x0000_0000);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b"a"), 0xE8B7_BE43);
    }
}

#[cfg(test)]
mod positioned_tests {
    use super::*;
    fn path(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kv9-positioned-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("wal")
    }
    fn batch(value: &[u8]) -> WriteBatch {
        let mut batch = WriteBatch::new();
        batch.put(ColumnFamily::Default, b"key".to_vec(), value.to_vec());
        batch
    }
    fn at(term: u64, index: u64) -> AppliedPosition {
        AppliedPosition { term, index }
    }

    #[test]
    fn positioned_data_and_exact_pair_survive_every_torn_tail_cut() {
        let source = path("tears");
        let (mut wal, _) = Wal::open(&source).unwrap();
        wal.append_applied(&batch(b"first"), at(7, 4)).unwrap();
        let first_len = std::fs::metadata(&source).unwrap().len() as usize;
        wal.append_applied(&batch(b"second"), at(9, 15)).unwrap();
        drop(wal);
        let bytes = std::fs::read(&source).unwrap();
        let torn = source.with_extension("torn");
        for cut in first_len..bytes.len() {
            std::fs::write(&torn, &bytes[..cut]).unwrap();
            let (_, replay) = Wal::open(&torn).unwrap();
            assert_eq!(
                replay.positions,
                vec![Some(at(7, 4))],
                "torn position must not become authoritative at byte {cut}"
            );
            assert_eq!(
                replay.batches.len(),
                1,
                "data and position must recover together"
            );
        }
        let (_, replay) = Wal::open(&source).unwrap();
        assert_eq!(replay.positions, vec![Some(at(7, 4)), Some(at(9, 15))]);
    }

    #[test]
    fn crc_covers_term_index_kind_and_payload() {
        let source = path("crc-position");
        let (mut wal, _) = Wal::open(&source).unwrap();
        wal.append_applied(&batch(b"data"), at(1, 12)).unwrap();
        drop(wal);
        let original = std::fs::read(&source).unwrap();
        for offset in [5, 6, 14, 27] {
            let mut bytes = original.clone();
            bytes[offset] ^= 1;
            let damaged = source.with_extension("damaged");
            std::fs::write(&damaged, bytes).unwrap();
            if let Ok((_, replay)) = Wal::open(&damaged) {
                assert!(
                    replay.positions.is_empty(),
                    "CRC must cover field at {offset}"
                );
                assert!(replay.batches.is_empty());
            }
        }
    }

    #[test]
    fn valid_crc_out_of_order_records_refuse_the_whole_open_without_truncation() {
        for next in [7, 3] {
            let source = path(&format!("ordering-{next}"));
            let second = source.with_extension("second");
            let (mut first, _) = Wal::open(&source).unwrap();
            first.append_applied(&batch(b"first"), at(2, 7)).unwrap();
            let (mut later, _) = Wal::open(&second).unwrap();
            later
                .append_applied(&batch(b"invalid"), at(3, next))
                .unwrap();
            drop((first, later));
            let mut bytes = std::fs::read(&source).unwrap();
            bytes.extend(std::fs::read(second).unwrap());
            std::fs::write(&source, &bytes).unwrap();
            let error = Wal::open(&source).unwrap_err();
            assert!(
                error.to_string().contains("applied position must advance"),
                "{error}"
            );
            assert_eq!(
                std::fs::read(&source).unwrap(),
                bytes,
                "ordering refusal must not expose/truncate to a prefix"
            );
        }
    }

    #[test]
    fn v1_and_unpositioned_v2_do_not_invent_an_applied_position() {
        let source = path("mixed");
        let payload = encode_batch(&batch(b"legacy"));
        let mut check = vec![VERSION_V1];
        check.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        check.extend_from_slice(&payload);
        let mut record = MAGIC.to_vec();
        record.extend_from_slice(&check);
        record.extend_from_slice(&crc32(&check).to_le_bytes());
        std::fs::write(&source, record).unwrap();
        let (mut wal, _) = Wal::open(&source).unwrap();
        wal.append(&batch(b"local")).unwrap();
        wal.append_applied(&batch(b"replicated"), at(4, 30))
            .unwrap();
        drop(wal);
        let (_, replay) = Wal::open(&source).unwrap();
        assert_eq!(replay.positions, vec![None, None, Some(at(4, 30))]);
    }

    #[test]
    fn failed_append_poison_prevents_acknowledgements_behind_a_torn_record() {
        let source = path("poison");
        let (mut wal, _) = Wal::open(&source).unwrap();
        wal.file = File::open(&source).unwrap(); // force a real EBADF on write
        assert!(wal.append(&batch(b"fails")).is_err());
        wal.file = OpenOptions::new().append(true).open(&source).unwrap();
        let error = wal.append(&batch(b"must-not-ack")).unwrap_err();
        assert!(error.to_string().contains("reopen required"));
        assert_eq!(std::fs::metadata(source).unwrap().len(), 0);
    }
}
