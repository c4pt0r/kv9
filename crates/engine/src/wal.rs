//! Append-only write-ahead log: record framing, encoding, and crash-tolerant replay.
//!
//! One record per [`crate::WriteBatch`]. The frame is deliberately boring:
//!
//! ```text
//! magic(4) version(1) len(4, LE) payload(len) crc(4, LE over version..payload)
//! ```
//!
//! ## What the framing has to survive
//!
//! A process can die *during* an append, so the last record may be half-written. That is
//! not corruption, it is the normal shape of a crash, and replay must tolerate it: we
//! stop at the first record that does not verify and keep everything before it. The
//! alternative — refusing to open — would turn every crash into data loss.
//!
//! We cannot distinguish "torn tail" from "bit-rot in the middle" by inspection alone, so
//! we do not try: both stop replay at the same place. What that costs is stated plainly in
//! [`Wal::replay`]'s return value, which reports how many bytes were discarded so a caller
//! can log or refuse rather than silently accept truncation.
//!
//! Versioned so an unknown version is rejected rather than misparsed (DESIGN §13
//! principle 12, "forward-compatible formats, never panic on the unknown").

use std::fs::{File, OpenOptions};
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use kv9_common::{Error, Result};

use crate::cf::ColumnFamily;
use crate::write_batch::{Mutation, WriteBatch};

const MAGIC: [u8; 4] = *b"KV9W";
const VERSION: u8 = 1;
const HEADER_LEN: usize = 4 + 1 + 4; // magic + version + len
const CRC_LEN: usize = 4;

/// Guards against a corrupt length field causing a huge allocation. A single batch far
/// larger than this is a bug, not a legitimate write (DESIGN §13 principle 13 — no
/// unbounded in-memory path driven by untrusted input).
const MAX_RECORD_LEN: u32 = 64 * 1024 * 1024;

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

/// Outcome of replaying a log at open time.
#[derive(Debug, Clone)]
pub struct Replay {
    /// Batches recovered, oldest first.
    pub batches: Vec<WriteBatch>,
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

        Ok((Wal { path, file }, replay))
    }

    /// Read every complete, checksum-verified record, stopping at the first that is not.
    ///
    /// Leaves `file` positioned immediately after the last good record.
    fn replay(file: &mut File) -> Result<Replay> {
        let total = file.metadata().map_err(io)?.len();
        file.seek(SeekFrom::Start(0)).map_err(io)?;
        let mut reader = BufReader::new(&mut *file);

        let mut batches = Vec::new();
        let mut good_end: u64 = 0;

        loop {
            let mut header = [0u8; HEADER_LEN];
            match reader.read_exact(&mut header) {
                Ok(()) => {}
                // A short read here is the tail; everything before it stands.
                Err(_) => break,
            }
            if header[0..4] != MAGIC {
                break;
            }
            if header[4] != VERSION {
                // Unknown version: refuse rather than misparse. This is not a torn tail,
                // so it is an error, not a silent truncation (principle 12).
                return Err(Error::Engine(format!(
                    "wal: unsupported record version {} (this build writes {VERSION})",
                    header[4]
                )));
            }
            let len = u32::from_le_bytes([header[5], header[6], header[7], header[8]]);
            if len > MAX_RECORD_LEN {
                break;
            }

            let mut body = vec![0u8; len as usize + CRC_LEN];
            if reader.read_exact(&mut body).is_err() {
                break;
            }
            let (payload, crc_bytes) = body.split_at(len as usize);
            let want = u32::from_le_bytes([crc_bytes[0], crc_bytes[1], crc_bytes[2], crc_bytes[3]]);

            // Cover version + len + payload, so a flipped length is caught too.
            let mut check = Vec::with_capacity(1 + 4 + payload.len());
            check.push(header[4]);
            check.extend_from_slice(&header[5..9]);
            check.extend_from_slice(payload);
            if crc32(&check) != want {
                break;
            }

            // `?`, deliberately NOT the `break` used for a CRC mismatch above: a record whose
            // checksum verifies but whose payload will not decode is a real inconsistency, not
            // a torn tail. Breaking here would silently truncate the log at a point the data
            // says is intact, so this must propagate.
            batches.push(decode_batch(payload)?);
            good_end += (HEADER_LEN + len as usize + CRC_LEN) as u64;
        }

        file.seek(SeekFrom::Start(good_end)).map_err(io)?;
        Ok(Replay {
            batches,
            discarded_tail_bytes: total - good_end,
        })
    }

    /// Append one batch and flush it to the OS, then to the device.
    ///
    /// `sync_all` is what makes the write actually survive power loss; without it the
    /// record sits in the page cache and the durability claim would be false.
    pub fn append(&mut self, batch: &WriteBatch) -> Result<()> {
        let payload = encode_batch(batch);
        let len = u32::try_from(payload.len())
            .map_err(|_| Error::Engine("wal: batch exceeds u32 length".into()))?;
        if len > MAX_RECORD_LEN {
            return Err(Error::Engine(format!(
                "wal: batch of {len} bytes exceeds the {MAX_RECORD_LEN} byte record limit"
            )));
        }

        let mut check = Vec::with_capacity(1 + 4 + payload.len());
        check.push(VERSION);
        check.extend_from_slice(&len.to_le_bytes());
        check.extend_from_slice(&payload);
        let crc = crc32(&check);

        let mut rec = Vec::with_capacity(HEADER_LEN + payload.len() + CRC_LEN);
        rec.extend_from_slice(&MAGIC);
        rec.push(VERSION);
        rec.extend_from_slice(&len.to_le_bytes());
        rec.extend_from_slice(&payload);
        rec.extend_from_slice(&crc.to_le_bytes());

        self.file.write_all(&rec).map_err(io)?;
        self.file.sync_all().map_err(io)?;
        Ok(())
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
        bytes[5..9].copy_from_slice(&u32::MAX.to_le_bytes());
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
