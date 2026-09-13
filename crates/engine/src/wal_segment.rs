//! Immutable closed WAL segments and bounded, strict streaming recovery.
//!
//! The stream owner selects the active segment from durable topology metadata;
//! directory contents or an old numeric sequence do not supply that authority.
//! This module owns one file. Rotation, checkpoint authority, legacy migration
//! and whole-segment reclamation belong to the stream owner, not this codec.
//! See `docs/SEGMENTED-WAL.md` for the integration and publication contract.
//!
//! Unlike the legacy reader, a complete malformed record fails the entire
//! recovery. Only an incomplete final frame in the selected active segment can
//! be discarded. A separately checksummed frame header validates its length
//! before allocation, so a flipped length cannot turn corruption into a torn tail.

use std::fs::{File, OpenOptions};
use std::io::{BufReader, IoSlice, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kv9_common::metrics::WalIoMetrics;
use kv9_common::{AppliedPosition, Error, Result};

use crate::wal::{crc32, crc32_parts, decode_batch, encode_batch};
use crate::wal_v2::MAX_RECORD_LEN;
use crate::WriteBatch;

const MAGIC: &[u8; 8] = b"KV9SEG01";
const FRAME_MAGIC: &[u8; 4] = b"KV9R";
/// Eight magic/version bytes, stream identity, sequence, optional previous
/// position, reserved bytes and CRC-32 of all preceding header bytes.
pub const HEADER_BYTES: usize = 60;
/// Magic, payload length, position kind, reserved bytes, term/index and CRC-32.
pub const FRAME_HEADER_BYTES: usize = 32;

/// Identity and predecessor selected by the stream's durable topology.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SegmentHeader {
    pub stream_id: [u8; 16],
    pub sequence: u64,
    pub previous: Option<AppliedPosition>,
}

/// Summary of complete, synchronized records. An unpositioned record pins the
/// segment until a separate verified migration accounts for that data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SegmentSummary {
    pub bytes: u64,
    pub records: u64,
    pub first: Option<AppliedPosition>,
    pub last: Option<AppliedPosition>,
    pub has_unpositioned: bool,
}

impl Default for SegmentSummary {
    fn default() -> Self {
        Self {
            bytes: HEADER_BYTES as u64,
            records: 0,
            first: None,
            last: None,
            has_unpositioned: false,
        }
    }
}

impl SegmentSummary {
    fn add(&mut self, bytes: u64, position: Option<AppliedPosition>) -> Result<()> {
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or_else(|| bad("byte count overflow"))?;
        self.records = self
            .records
            .checked_add(1)
            .ok_or_else(|| bad("record count overflow"))?;
        if let Some(at) = position {
            self.first.get_or_insert(at);
            self.last = Some(at);
        } else {
            self.has_unpositioned = true;
        }
        Ok(())
    }
}

/// Sealing consumes the writer. A stream must durably publish this descriptor
/// before using it to select a successor or reclaim this file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClosedSegment {
    header: SegmentHeader,
    summary: SegmentSummary,
}

impl ClosedSegment {
    pub(crate) fn from_published(header: SegmentHeader, summary: SegmentSummary) -> Result<Self> {
        header.encode()?;
        let minimum = summary
            .records
            .checked_mul((FRAME_HEADER_BYTES + 8) as u64)
            .and_then(|bytes| bytes.checked_add(HEADER_BYTES as u64))
            .ok_or_else(|| bad("published summary length overflow"))?;
        if summary.bytes < minimum
            || summary.first.is_some() != summary.last.is_some()
            || (summary.records == 0 && summary != SegmentSummary::default())
            || (summary.records > 0 && summary.first.is_none() && !summary.has_unpositioned)
        {
            return Err(bad("invalid published closed summary"));
        }
        check_position(header.previous, summary.first)?;
        if let (Some(first), Some(last)) = (summary.first, summary.last) {
            if last.index < first.index || last.term < first.term {
                return Err(bad("published summary positions regress"));
            }
        }
        Ok(Self { header, summary })
    }
    pub fn header(&self) -> SegmentHeader {
        self.header
    }
    pub fn summary(&self) -> SegmentSummary {
        self.summary
    }

    /// Replay a retained closed segment without modifying it. The caller must
    /// discard all visitor effects if any later record or summary check fails.
    pub fn replay(
        &self,
        path: impl AsRef<Path>,
        visitor: impl FnMut(WriteBatch, Option<AppliedPosition>) -> Result<()>,
    ) -> Result<()> {
        let mut file = File::open(path).map_err(io)?;
        if file.metadata().map_err(io)?.len() != self.summary.bytes {
            return Err(bad(
                "closed segment length differs from its published summary",
            ));
        }
        let report = scan(&mut file, self.header, false, visitor)?;
        if report.summary != self.summary {
            return Err(bad(
                "closed segment contents differ from its published summary",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryReport {
    pub summary: SegmentSummary,
    pub discarded_tail_bytes: u64,
}

/// An exclusively owned active file. The enclosing store guard must outlive
/// this writer; opening the same path twice does not establish another owner.
#[derive(Debug)]
pub struct WalSegment {
    path: PathBuf,
    file: File,
    header: SegmentHeader,
    summary: SegmentSummary,
    poisoned: bool,
    metrics: Arc<WalIoMetrics>,
}

impl WalSegment {
    /// Create a new file in an existing directory. Existing paths are never
    /// replaced. Successful creation synchronizes both file and namespace.
    pub fn create(
        path: impl AsRef<Path>,
        header: SegmentHeader,
        metrics: Arc<WalIoMetrics>,
    ) -> Result<Self> {
        let encoded = header.encode()?;
        let path = path.as_ref().to_path_buf();
        let mut file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(io)?;
        metrics
            .write
            .measure(|| file.write_all(&encoded))
            .map_err(io)?;
        metrics.sync.measure(|| file.sync_all()).map_err(io)?;
        metrics
            .namespace_publish
            .measure(|| sync_namespace(&path))
            .map_err(io)?;
        Ok(Self {
            path,
            file,
            header,
            summary: SegmentSummary::default(),
            poisoned: false,
            metrics,
        })
    }

    /// Recover an existing file selected as active by durable topology. Memory
    /// use is bounded by one record and the visitor's own retained state.
    /// Validation/visitor failures preserve the file. A failed repair sync may
    /// have removed an incomplete tail, but never returns an authorized writer.
    pub fn recover_active(
        path: impl AsRef<Path>,
        header: SegmentHeader,
        metrics: Arc<WalIoMetrics>,
        visitor: impl FnMut(WriteBatch, Option<AppliedPosition>) -> Result<()>,
    ) -> Result<(Self, RecoveryReport)> {
        let path = path.as_ref().to_path_buf();
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(io)?;
        let report = scan(&mut file, header, true, visitor)?;
        if report.discarded_tail_bytes != 0 {
            file.set_len(report.summary.bytes).map_err(io)?;
        }
        file.seek(SeekFrom::Start(report.summary.bytes))
            .map_err(io)?;
        metrics
            .recovery_sync
            .measure(|| file.sync_all())
            .map_err(io)?;
        metrics
            .namespace_publish
            .measure(|| sync_namespace(&path))
            .map_err(io)?;
        Ok((
            Self {
                path,
                file,
                header,
                summary: report.summary,
                poisoned: false,
                metrics,
            },
            report,
        ))
    }

    /// Acknowledges only after the complete data/position frame is synchronized.
    /// Any write or sync error fences the handle until it is dropped and recovered.
    pub fn append(&mut self, batch: &WriteBatch, position: Option<AppliedPosition>) -> Result<()> {
        if self.poisoned {
            return Err(bad("failed writer requires recovery"));
        }
        check_position(self.summary.last.or(self.header.previous), position)?;
        encoded_size(batch)?;
        let payload = encode_batch(batch);
        let length = u32::try_from(payload.len()).map_err(|_| bad("record length overflow"))?;
        if length > MAX_RECORD_LEN {
            return Err(bad("record exceeds the size limit"));
        }
        let header = frame_header(length, position);
        let crc = frame_crc(&self.header.encode()?, &header, &payload).to_le_bytes();
        let mut next = self.summary;
        next.add(
            FRAME_HEADER_BYTES as u64 + payload.len() as u64 + 4,
            position,
        )?;
        let result = self
            .metrics
            .write
            .measure(|| write_frame(&mut self.file, &header, &payload, &crc))
            .and_then(|_| self.metrics.sync.measure(|| self.file.sync_all()));
        if let Err(error) = result {
            self.poisoned = true;
            return Err(io(error));
        }
        self.summary = next;
        Ok(())
    }

    pub fn summary(&self) -> SegmentSummary {
        self.summary
    }
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns a closed descriptor only after durability succeeds. No writer
    /// handle remains available through this API after sealing.
    pub fn seal(self) -> Result<ClosedSegment> {
        if self.poisoned {
            return Err(bad("failed writer cannot seal"));
        }
        self.metrics
            .sync
            .measure(|| self.file.sync_all())
            .map_err(io)?;
        Ok(ClosedSegment {
            header: self.header,
            summary: self.summary,
        })
    }
}

impl SegmentHeader {
    pub(crate) fn encode(self) -> Result<[u8; HEADER_BYTES]> {
        if self.stream_id == [0; 16] || self.sequence == 0 {
            return Err(bad("zero stream identity or sequence"));
        }
        let mut bytes = [0u8; HEADER_BYTES];
        bytes[..8].copy_from_slice(MAGIC);
        bytes[8..24].copy_from_slice(&self.stream_id);
        bytes[24..32].copy_from_slice(&self.sequence.to_le_bytes());
        if let Some(at) = self.previous {
            bytes[32] = 1;
            bytes[40..48].copy_from_slice(&at.term.to_le_bytes());
            bytes[48..56].copy_from_slice(&at.index.to_le_bytes());
        }
        let checksum = crc32(&bytes[..56]);
        bytes[56..].copy_from_slice(&checksum.to_le_bytes());
        Ok(bytes)
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != HEADER_BYTES {
            return Err(bad("invalid published header size"));
        }
        let previous = match bytes[32] {
            0 => None,
            1 => Some(AppliedPosition {
                term: u64::from_le_bytes(bytes[40..48].try_into().unwrap()),
                index: u64::from_le_bytes(bytes[48..56].try_into().unwrap()),
            }),
            _ => return Err(bad("invalid published predecessor kind")),
        };
        let header = Self {
            stream_id: bytes[8..24].try_into().unwrap(),
            sequence: u64::from_le_bytes(bytes[24..32].try_into().unwrap()),
            previous,
        };
        if bytes != header.encode()? {
            return Err(bad("invalid published segment header"));
        }
        Ok(header)
    }
}

pub(crate) fn encoded_size(batch: &WriteBatch) -> Result<u64> {
    let mut payload = 4u64;
    for mutation in batch.mutations() {
        let extra = match mutation {
            crate::Mutation::Put { key, value, .. } => 10u64
                .checked_add(key.len() as u64)
                .and_then(|n| n.checked_add(value.len() as u64)),
            crate::Mutation::Delete { key, .. } => 6u64.checked_add(key.len() as u64),
        }
        .ok_or_else(|| bad("record length overflow"))?;
        payload = payload
            .checked_add(extra)
            .ok_or_else(|| bad("record length overflow"))?;
        if payload > u64::from(MAX_RECORD_LEN) {
            return Err(bad("record exceeds the size limit"));
        }
    }
    Ok(payload + FRAME_HEADER_BYTES as u64 + 4)
}

fn frame_header(length: u32, position: Option<AppliedPosition>) -> [u8; FRAME_HEADER_BYTES] {
    let mut bytes = [0u8; FRAME_HEADER_BYTES];
    bytes[..4].copy_from_slice(FRAME_MAGIC);
    bytes[4..8].copy_from_slice(&length.to_le_bytes());
    if let Some(at) = position {
        bytes[8] = 1;
        bytes[12..20].copy_from_slice(&at.term.to_le_bytes());
        bytes[20..28].copy_from_slice(&at.index.to_le_bytes());
    }
    let checksum = crc32(&bytes[..28]);
    bytes[28..].copy_from_slice(&checksum.to_le_bytes());
    bytes
}

fn frame_crc(
    segment: &[u8; HEADER_BYTES],
    header: &[u8; FRAME_HEADER_BYTES],
    payload: &[u8],
) -> u32 {
    // Exclude both inner CRC fields: CRC(message || CRC(message)) has a fixed
    // residue and would erase the very identity/position we need to bind.
    crc32_parts(&[
        &segment[..HEADER_BYTES - 4],
        &header[..FRAME_HEADER_BYTES - 4],
        payload,
    ])
}

fn check_position(
    previous: Option<AppliedPosition>,
    current: Option<AppliedPosition>,
) -> Result<()> {
    if let (Some(before), Some(after)) = (previous, current) {
        if after.index <= before.index || after.term < before.term {
            return Err(bad(
                "position must advance its index without regressing its term",
            ));
        }
    }
    Ok(())
}

fn scan(
    file: &mut File,
    expected: SegmentHeader,
    active: bool,
    mut visitor: impl FnMut(WriteBatch, Option<AppliedPosition>) -> Result<()>,
) -> Result<RecoveryReport> {
    let length = file.metadata().map_err(io)?.len();
    file.seek(SeekFrom::Start(0)).map_err(io)?;
    let mut reader = BufReader::new(file);
    let mut segment_header = [0u8; HEADER_BYTES];
    reader.read_exact(&mut segment_header).map_err(io)?;
    // Exact expected encoding checks version, CRC, identity, sequence, prior
    // position and all reserved bytes before visiting the first batch.
    if segment_header != expected.encode()? {
        return Err(bad("segment header does not match its authority"));
    }
    let mut summary = SegmentSummary::default();
    while summary.bytes < length {
        let remaining = length - summary.bytes;
        if remaining < FRAME_HEADER_BYTES as u64 {
            break;
        }
        let mut header = [0u8; FRAME_HEADER_BYTES];
        reader.read_exact(&mut header).map_err(io)?;
        if &header[..4] != FRAME_MAGIC
            || header[9..12] != [0; 3]
            || crc32(&header[..28]) != u32::from_le_bytes(header[28..].try_into().unwrap())
        {
            return Err(bad("invalid complete frame header"));
        }
        let size = u32::from_le_bytes(header[4..8].try_into().unwrap());
        if size > MAX_RECORD_LEN {
            return Err(bad("frame exceeds the size limit"));
        }
        let position = match header[8] {
            0 if header[12..28] == [0; 16] => None,
            1 => Some(AppliedPosition {
                term: u64::from_le_bytes(header[12..20].try_into().unwrap()),
                index: u64::from_le_bytes(header[20..28].try_into().unwrap()),
            }),
            _ => return Err(bad("invalid frame position kind or absent-position bytes")),
        };
        let frame_bytes = FRAME_HEADER_BYTES as u64 + u64::from(size) + 4;
        if remaining < frame_bytes {
            break;
        }
        let mut payload = vec![0u8; size as usize];
        reader.read_exact(&mut payload).map_err(io)?;
        let mut crc = [0u8; 4];
        reader.read_exact(&mut crc).map_err(io)?;
        if frame_crc(&segment_header, &header, &payload) != u32::from_le_bytes(crc) {
            return Err(bad("invalid complete data/position frame checksum"));
        }
        check_position(summary.last.or(expected.previous), position)?;
        let batch = decode_batch(&payload)?;
        visitor(batch, position)?;
        summary.add(frame_bytes, position)?;
    }
    let discarded_tail_bytes = length - summary.bytes;
    if !active && discarded_tail_bytes != 0 {
        return Err(bad("incomplete frame in a closed segment"));
    }
    Ok(RecoveryReport {
        summary,
        discarded_tail_bytes,
    })
}

/// Write the same header/payload/checksum sequence without joining its buffers.
/// A successful short write advances only the accepted prefix. Interrupted
/// writes retry the same suffix; zero progress and other errors reach the
/// caller's existing poison fence. Synchronization stays with the caller.
fn write_frame<W: Write>(
    writer: &mut W,
    header: &[u8],
    payload: &[u8],
    checksum: &[u8],
) -> std::io::Result<()> {
    let mut slices = [
        IoSlice::new(header),
        IoSlice::new(payload),
        IoSlice::new(checksum),
    ];
    let mut remaining = &mut slices[..];
    IoSlice::advance_slices(&mut remaining, 0);
    while !remaining.is_empty() {
        match writer.write_vectored(remaining) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "failed to write complete WAL frame",
                ));
            }
            Ok(written) => IoSlice::advance_slices(&mut remaining, written),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod vectored_tests;

fn sync_namespace(path: &Path) -> std::io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let canonical = parent.canonicalize()?;
    for ancestor in canonical.ancestors() {
        File::open(ancestor)?.sync_all()?;
    }
    Ok(())
}

fn bad(message: &str) -> Error {
    Error::Engine(format!("wal segment: {message}"))
}
fn io(error: std::io::Error) -> Error {
    bad(&format!("I/O: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ColumnFamily, Mutation};

    fn path(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("kv9-segment-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory.join("0000000000000001.wal")
    }
    fn header() -> SegmentHeader {
        SegmentHeader {
            stream_id: [19; 16],
            sequence: 1,
            previous: Some(at(2, 3)),
        }
    }
    fn at(term: u64, index: u64) -> AppliedPosition {
        AppliedPosition { term, index }
    }
    fn batch(key: &[u8], value: &[u8]) -> WriteBatch {
        let mut batch = WriteBatch::new();
        batch.put(ColumnFamily::Default, key.to_vec(), value.to_vec());
        batch
    }
    fn create(path: &Path) -> WalSegment {
        WalSegment::create(path, header(), WalIoMetrics::shared()).unwrap()
    }
    fn recover(path: &Path) -> Result<(WalSegment, RecoveryReport)> {
        WalSegment::recover_active(path, header(), WalIoMetrics::shared(), |_, _| Ok(()))
    }

    #[test]
    fn sealed_replay_preserves_batches_positions_gaps_and_unpositioned_pin() {
        let path = path("roundtrip");
        let mut segment = create(&path);
        segment
            .append(&batch(b"a", b"one"), Some(at(2, 4)))
            .unwrap();
        segment.append(&batch(b"b", b"local"), None).unwrap();
        let mut deletion = WriteBatch::new();
        deletion.delete(ColumnFamily::Default, b"a".to_vec());
        segment.append(&deletion, Some(at(4, 19))).unwrap();
        assert!(segment.summary().has_unpositioned);
        let sealed = segment.seal().unwrap();
        assert_eq!(sealed.summary().first, Some(at(2, 4)));
        assert_eq!(sealed.summary().last, Some(at(4, 19)));
        let mut positions = Vec::new();
        let mut writes = Vec::new();
        sealed
            .replay(&path, |batch, at| {
                positions.push(at);
                writes.extend_from_slice(batch.mutations());
                Ok(())
            })
            .unwrap();
        assert_eq!(positions, [Some(at(2, 4)), None, Some(at(4, 19))]);
        assert!(matches!(&writes[2], Mutation::Delete { key, .. } if key == b"a"));
        let (_, replay) = recover(&path).unwrap();
        assert_eq!(replay.summary, sealed.summary());
        assert_eq!(replay.discarded_tail_bytes, 0);
    }

    #[test]
    fn every_torn_active_frame_recovers_prior_ack_and_accepts_a_new_append() {
        let path = path("torn");
        let mut segment = create(&path);
        segment
            .append(&batch(b"first", b"acknowledged"), Some(at(2, 5)))
            .unwrap();
        let first_end = segment.summary().bytes as usize;
        segment
            .append(
                &batch(b"second", b"not acknowledged at a partial cut"),
                Some(at(2, 11)),
            )
            .unwrap();
        let sealed = segment.seal().unwrap();
        let complete = std::fs::read(&path).unwrap();
        let victim = path.with_extension("cut");
        for cut in first_end..complete.len() {
            std::fs::write(&victim, &complete[..cut]).unwrap();
            assert!(sealed.replay(&victim, |_, _| Ok(())).is_err());
            assert_eq!(std::fs::read(&victim).unwrap(), complete[..cut]);
            let mut observed = Vec::new();
            let (mut writer, report) =
                WalSegment::recover_active(&victim, header(), WalIoMetrics::shared(), |_, at| {
                    observed.push(at);
                    Ok(())
                })
                .unwrap();
            assert_eq!(observed, [Some(at(2, 5))], "cut {cut}");
            assert_eq!(report.discarded_tail_bytes, (cut - first_end) as u64);
            writer
                .append(&batch(b"third", b"after recovery"), Some(at(3, 21)))
                .unwrap();
            drop(writer);
            let (_, report) = recover(&victim).unwrap();
            assert_eq!(report.summary.records, 2);
            assert_eq!(report.summary.last, Some(at(3, 21)));
            assert_eq!(report.discarded_tail_bytes, 0);
        }
    }

    #[test]
    fn complete_corruption_and_nonmonotonic_records_fail_without_editing_the_file() {
        let path = path("corruption");
        let mut writer = create(&path);
        writer
            .append(&batch(b"before", b"one"), Some(at(2, 4)))
            .unwrap();
        let second = writer.summary().bytes as usize;
        writer
            .append(&batch(b"after", b"two"), Some(at(3, 17)))
            .unwrap();
        drop(writer);
        let original = std::fs::read(&path).unwrap();
        // These are all inside complete replayable records, including the
        // length field that a tail-tolerant reader must not trust before CRC.
        let mut corruptions = Vec::new();
        for offset in [
            0,
            8,
            24,
            HEADER_BYTES,
            HEADER_BYTES + 4,
            second + 8,
            second + 12,
            second + FRAME_HEADER_BYTES + 6,
        ] {
            let mut bytes = original.clone();
            bytes[offset] ^= 0x80;
            corruptions.push(bytes);
        }
        for position in [at(2, 4), at(3, 2), at(1, 20)] {
            let mut bytes = original.clone();
            let payload_len = u32::from_le_bytes(bytes[second + 4..second + 8].try_into().unwrap());
            bytes[second..second + FRAME_HEADER_BYTES]
                .copy_from_slice(&frame_header(payload_len, Some(position)));
            let end = bytes.len() - 4;
            let checksum = crc32_parts(&[
                &bytes[..HEADER_BYTES - 4],
                &bytes[second..second + FRAME_HEADER_BYTES - 4],
                &bytes[second + FRAME_HEADER_BYTES..end],
            ]);
            bytes[end..].copy_from_slice(&checksum.to_le_bytes());
            std::fs::write(&path, &bytes).unwrap();
            assert!(recover(&path)
                .unwrap_err()
                .to_string()
                .contains("position must advance"));
            corruptions.push(bytes);
        }
        let mut oversized = original.clone();
        oversized[second..second + FRAME_HEADER_BYTES]
            .copy_from_slice(&frame_header(u32::MAX, Some(at(3, 17))));
        corruptions.push(oversized);
        for (case, bytes) in corruptions.iter().enumerate() {
            std::fs::write(&path, bytes).unwrap();
            assert!(
                recover(&path).is_err(),
                "complete corruption {case} was accepted"
            );
            assert_eq!(
                &std::fs::read(&path).unwrap(),
                bytes,
                "failed open edited case {case}"
            );
        }
    }

    #[test]
    fn complete_frames_bind_data_to_position_and_stream_identity() {
        let path = path("frame-binding");
        let mut writer = create(&path);
        writer.append(&batch(b"a", b"one"), Some(at(2, 4))).unwrap();
        let second = writer.summary().bytes as usize;
        writer
            .append(&batch(b"b", b"two"), Some(at(3, 17)))
            .unwrap();
        let sealed = writer.seal().unwrap();
        let original = std::fs::read(&path).unwrap();
        let first_payload = HEADER_BYTES + FRAME_HEADER_BYTES;
        let second_payload = second + FRAME_HEADER_BYTES;
        assert_eq!(second - first_payload, original.len() - second_payload);
        let mut swapped = original.clone();
        swapped[first_payload..second].copy_from_slice(&original[second_payload..]);
        swapped[second_payload..].copy_from_slice(&original[first_payload..second]);
        std::fs::write(&path, &swapped).unwrap();
        assert!(sealed.replay(&path, |_, _| Ok(())).is_err());
        assert!(recover(&path)
            .unwrap_err()
            .to_string()
            .contains("data/position frame checksum"));
        assert_eq!(std::fs::read(&path).unwrap(), swapped);

        // Moving whole valid frames to another selected stream is also refused.
        let other = SegmentHeader {
            stream_id: [20; 16],
            ..header()
        };
        let mut transplanted = original;
        transplanted[..HEADER_BYTES].copy_from_slice(&other.encode().unwrap());
        std::fs::write(&path, &transplanted).unwrap();
        assert!(
            WalSegment::recover_active(&path, other, WalIoMetrics::shared(), |_, _| Ok(()))
                .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), transplanted);
    }

    #[test]
    fn valid_checksum_does_not_authorize_invalid_batch_or_tail_repair() {
        let path = path("batch-codec");
        let mut writer = create(&path);
        writer.append(&batch(b"a", b"one"), Some(at(2, 4))).unwrap();
        drop(writer);
        let mut bytes = std::fs::read(&path).unwrap();
        // The mutation tag follows the batch count. Preserve both checksums
        // so the malformed payload must reach the batch decoder itself.
        bytes[HEADER_BYTES + FRAME_HEADER_BYTES + 4] = 255;
        let end = bytes.len() - 4;
        let checksum = crc32_parts(&[
            &bytes[..HEADER_BYTES - 4],
            &bytes[HEADER_BYTES..HEADER_BYTES + FRAME_HEADER_BYTES - 4],
            &bytes[HEADER_BYTES + FRAME_HEADER_BYTES..end],
        ]);
        bytes[end..].copy_from_slice(&checksum.to_le_bytes());
        bytes.extend_from_slice(b"KV9R");
        std::fs::write(&path, &bytes).unwrap();
        assert!(recover(&path)
            .unwrap_err()
            .to_string()
            .contains("unknown mutation tag"));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn identity_and_predecessor_checks_precede_replay_and_missing_files_stay_missing() {
        let path = path("identity");
        assert!(recover(&path).is_err());
        assert!(!path.exists());
        let mut writer = create(&path);
        writer
            .append(&batch(b"key", b"value"), Some(at(3, 10)))
            .unwrap();
        drop(writer);
        let original = std::fs::read(&path).unwrap();
        assert!(WalSegment::create(&path, header(), WalIoMetrics::shared()).is_err());
        for wrong in [
            SegmentHeader {
                stream_id: [20; 16],
                ..header()
            },
            SegmentHeader {
                sequence: 2,
                ..header()
            },
            SegmentHeader {
                previous: Some(at(2, 4)),
                ..header()
            },
        ] {
            let mut visited = false;
            assert!(
                WalSegment::recover_active(&path, wrong, WalIoMetrics::shared(), |_, _| {
                    visited = true;
                    Ok(())
                })
                .is_err()
            );
            assert!(!visited);
        }
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[test]
    fn a_rejected_append_and_failed_visitor_preserve_existing_authority() {
        let path = path("refusals");
        let mut writer = create(&path);
        writer
            .append(&batch(b"key", b"value"), Some(at(3, 12)))
            .unwrap();
        let original = std::fs::read(&path).unwrap();
        let summary = writer.summary();
        for at in [at(3, 12), at(4, 8), at(2, 13)] {
            assert!(writer.append(&batch(b"bad", b"bad"), Some(at)).is_err());
        }
        assert_eq!(writer.summary(), summary);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        drop(writer);
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"KV9R")
            .unwrap();
        let torn = std::fs::read(&path).unwrap();
        assert!(
            WalSegment::recover_active(&path, header(), WalIoMetrics::shared(), |_, _| {
                Err(bad("visitor refused the recovered batch"))
            })
            .is_err()
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            torn,
            "failed visitor allowed tail repair"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn actual_write_failure_fences_append_and_seal() {
        let path = path("enospc");
        let mut writer = create(&path);
        writer.file = OpenOptions::new().write(true).open("/dev/full").unwrap();
        assert!(writer
            .append(&batch(b"key", b"value"), Some(at(2, 5)))
            .is_err());
        assert!(writer.poisoned);
        assert_eq!(writer.summary().records, 0);
        // Even after replacing the failing descriptor, the handle cannot
        // publish a later acknowledged record behind the uncertain append.
        writer.file = OpenOptions::new().append(true).open(&path).unwrap();
        assert!(writer
            .append(&batch(b"later", b"value"), Some(at(2, 6)))
            .is_err());
        assert!(writer.seal().is_err());
        let (_, report) = recover(&path).unwrap();
        assert_eq!(report.summary.records, 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn actual_sync_failure_fences_an_unknown_durability_result() {
        let path = path("sync-failure");
        let mut writer = create(&path);
        let before = writer.summary();
        // This Linux device accepts writes and rejects fsync. A successful
        // write syscall alone must never advance the acknowledged summary.
        writer.file = OpenOptions::new().write(true).open("/dev/null").unwrap();
        assert!(writer
            .append(&batch(b"key", b"value"), Some(at(2, 5)))
            .is_err());
        assert!(writer.poisoned);
        assert_eq!(writer.summary(), before);
        writer.file = OpenOptions::new().append(true).open(&path).unwrap();
        assert!(writer
            .append(&batch(b"later", b"value"), Some(at(2, 6)))
            .is_err());
        assert!(writer.seal().is_err());
        assert_eq!(recover(&path).unwrap().1.summary.records, 0);
    }
}
