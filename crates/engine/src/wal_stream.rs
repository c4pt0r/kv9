//! Durable selection, rotation and checkpoint retention of engine WAL segments.
//!
//! The enclosing store guard supplies exclusive ownership. The topology file
//! selects exactly one active segment; directory enumeration never elects a
//! successor. Checkpoint adoption publishes the recovery anchor before removing
//! covered closed files. A failed topology publication fences every later write.
//! Callers must validate committed checkpoint authority and restore its full
//! state before replaying the remaining tail. Raft log truncation is unrelated.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kv9_common::metrics::WalIoMetrics;
use kv9_common::{AppliedPosition, Error, Result};
use sha2::{Digest, Sha256};

use crate::checkpoint::CheckpointManifest;
use crate::wal_segment::{
    encoded_size, ClosedSegment, SegmentHeader, SegmentSummary, WalSegment, HEADER_BYTES,
};
use crate::WriteBatch;

// The prefix causes old Wal::open readers to reject VERSION=3 before editing.
const MAGIC: &[u8; 9] = b"KV9W\x03SEG1";
const MAX_TOPOLOGY_BYTES: usize = 4 * 1024 * 1024;
const MAX_CLOSED_SEGMENTS: usize = 4096;
pub const DEFAULT_SEGMENT_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
struct Topology {
    generation: u64,
    active: SegmentHeader,
    closed: Vec<ClosedSegment>,
    checkpoint: Option<CheckpointManifest>,
}

/// Immutable selected recovery metadata. Loading it performs no filesystem
/// mutation and does not itself certify the checkpoint's Raft authority.
#[derive(Debug)]
pub struct RecoveryPlan {
    path: PathBuf,
    topology: Topology,
    encoded: Vec<u8>,
}

impl RecoveryPlan {
    pub(crate) fn is_segmented(path: &Path) -> Result<bool> {
        let mut bytes = [0; 5];
        match File::open(path) {
            Ok(mut file) => match file.read_exact(&mut bytes) {
                Ok(()) => Ok(bytes == MAGIC[..5]),
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
                Err(e) => Err(io(e)),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(io(e)),
        }
    }
    pub fn read(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let bytes = read_bounded(&path)?;
        let topology = Topology::decode(&bytes)?;
        Ok(Self {
            path,
            topology,
            encoded: bytes,
        })
    }

    pub fn checkpoint(&self) -> Option<&CheckpointManifest> {
        self.topology.checkpoint.as_ref()
    }

    /// `restore` must check the selected checkpoint against committed authority
    /// and restore its complete state. It runs before file repair or tail replay.
    /// On any later error the caller discards all unpublished visitor state.
    pub fn recover(
        self,
        target_bytes: u64,
        metrics: Arc<WalIoMetrics>,
        restore: impl FnOnce(Option<&CheckpointManifest>) -> Result<()>,
        mut visitor: impl FnMut(WriteBatch, Option<AppliedPosition>) -> Result<()>,
    ) -> Result<(SegmentedWal, StreamRecovery)> {
        validate_target(target_bytes)?;
        restore(self.checkpoint())?;
        if read_bounded(&self.path)? != self.encoded {
            return Err(bad("recovery topology changed while validating its anchor"));
        }
        // Stabilize a complete visible topology before repair or orphan cleanup.
        File::open(&self.path)
            .and_then(|f| f.sync_all())
            .map_err(io)?;
        sync_parent(&self.path)?;
        let mut report = StreamRecovery::default();
        let checkpoint = self.checkpoint().map(CheckpointManifest::position);
        let mut replay = |batch, at: Option<AppliedPosition>| {
            if checkpoint.is_some() && at.is_none() {
                return Err(bad("unpositioned record beside a published checkpoint"));
            }
            if let (Some(cut), Some(at)) = (checkpoint, at) {
                if at.index <= cut.index {
                    if !covers(cut, Some(at)) {
                        return Err(bad("covered record disagrees with checkpoint term"));
                    }
                    report.covered_records += 1;
                    return Ok(());
                }
                if at.term < cut.term {
                    return Err(bad("tail term regresses behind checkpoint"));
                }
            }
            visitor(batch, at)?;
            report.replayed_records += 1;
            Ok(())
        };
        for closed in &self.topology.closed {
            closed.replay(segment_path(&self.path, closed.header()), &mut replay)?;
        }
        let (active, recovered) = WalSegment::recover_active(
            segment_path(&self.path, self.topology.active),
            self.topology.active,
            metrics.clone(),
            &mut replay,
        )?;
        report.discarded_tail_bytes = recovered.discarded_tail_bytes;
        let stream = SegmentedWal {
            path: self.path,
            topology: self.topology,
            active: Some(active),
            target_bytes,
            metrics,
            poisoned: false,
        };
        // Orphans from a crash are not selected by this plan and never replay.
        // Their reclamation can be retried separately after opening the stream.
        Ok((stream, report))
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StreamRecovery {
    pub replayed_records: u64,
    pub covered_records: u64,
    pub discarded_tail_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Reclaimed {
    pub segments: usize,
    pub bytes: u64,
}

#[derive(Debug)]
pub struct SegmentedWal {
    path: PathBuf,
    topology: Topology,
    active: Option<WalSegment>,
    target_bytes: u64,
    metrics: Arc<WalIoMetrics>,
    poisoned: bool,
}

impl SegmentedWal {
    /// Create an explicitly new stream. A caller chooses a fresh random identity
    /// independently of filenames. An existing topology is never overwritten.
    pub fn create_new(
        path: impl AsRef<Path>,
        stream_id: [u8; 16],
        target_bytes: u64,
        metrics: Arc<WalIoMetrics>,
    ) -> Result<Self> {
        Self::create_with_checkpoint(path, stream_id, target_bytes, metrics, None)
    }

    pub(crate) fn create_with_checkpoint(
        path: impl AsRef<Path>,
        stream_id: [u8; 16],
        target_bytes: u64,
        metrics: Arc<WalIoMetrics>,
        checkpoint: Option<CheckpointManifest>,
    ) -> Result<Self> {
        validate_target(target_bytes)?;
        let path = path.as_ref().to_path_buf();
        if path.try_exists().map_err(io)? {
            return Err(bad("topology already exists"));
        }
        let header = SegmentHeader {
            stream_id,
            sequence: 1,
            previous: checkpoint.as_ref().map(CheckpointManifest::position),
        };
        header.encode()?;
        std::fs::create_dir_all(segment_directory(&path, stream_id)).map_err(io)?;
        let active = WalSegment::create(segment_path(&path, header), header, metrics.clone())?;
        let topology = Topology {
            generation: 1,
            active: header,
            closed: Vec::new(),
            checkpoint,
        };
        let bytes = topology.encode()?;
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(io)?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(io)?;
        sync_parent(&path)?;
        Ok(Self {
            path,
            topology,
            active: Some(active),
            target_bytes,
            metrics,
            poisoned: false,
        })
    }

    pub fn applied_position(&self) -> Option<AppliedPosition> {
        self.active
            .as_ref()
            .and_then(|active| active.summary().last)
            .or(self.topology.active.previous)
    }
    pub fn closed_segments(&self) -> &[ClosedSegment] {
        &self.topology.closed
    }
    pub fn active_header(&self) -> SegmentHeader {
        self.topology.active
    }
    pub fn checkpoint(&self) -> Option<&CheckpointManifest> {
        self.topology.checkpoint.as_ref()
    }
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Install a completed offline migration at the original WAL path. Both
    /// names must resolve to the same stream directory. The caller has already
    /// fenced the legacy writer before this irreversible publication step.
    pub(crate) fn install_as(&mut self, target: &Path) -> Result<()> {
        self.ensure_healthy()?;
        if segment_directory(target, self.topology.active.stream_id)
            != segment_directory(&self.path, self.topology.active.stream_id)
        {
            return Err(bad("migration would change the selected segment directory"));
        }
        self.poisoned = true;
        std::fs::rename(&self.path, target).map_err(io)?;
        self.path = target.to_path_buf();
        self.metrics
            .namespace_publish
            .measure(|| sync_parent(target))?;
        self.poisoned = false;
        Ok(())
    }

    pub fn append(&mut self, batch: &WriteBatch, at: Option<AppliedPosition>) -> Result<()> {
        self.ensure_healthy()?;
        if self.checkpoint().is_some() && at.is_none() {
            return Err(bad("unpositioned append cannot follow checkpoint adoption"));
        }
        if let (Some(previous), Some(next)) = (self.applied_position(), at) {
            if next.index <= previous.index || next.term < previous.term {
                return Err(bad("append position must advance without term regression"));
            }
        }
        let size = encoded_size(batch)?;
        let active = self
            .active
            .as_ref()
            .ok_or_else(|| bad("active writer unavailable"))?;
        if active.summary().records != 0
            && active.summary().bytes.saturating_add(size) > self.target_bytes
        {
            self.rotate()?;
        }
        let result = self
            .active
            .as_mut()
            .ok_or_else(|| bad("active writer unavailable"))?
            .append(batch, at);
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }

    /// Rotation publishes a synchronized successor before accepting its first
    /// write. Failure after retiring the current writer requires recovery.
    pub fn rotate(&mut self) -> Result<()> {
        self.ensure_healthy()?;
        if self.topology.closed.len() >= MAX_CLOSED_SEGMENTS {
            return Err(bad("closed-segment capacity reached; checkpoint required"));
        }
        let mut next = self.topology.clone();
        next.generation = next
            .generation
            .checked_add(1)
            .ok_or_else(|| bad("topology generation exhausted"))?;
        next.active.sequence = next
            .active
            .sequence
            .checked_add(1)
            .ok_or_else(|| bad("segment sequence exhausted"))?;
        next.active.previous = self.applied_position();
        // Fence before consuming the old owner. No branch below may restore
        // authority unless the full topology publication succeeds.
        self.poisoned = true;
        let closed = self
            .active
            .take()
            .ok_or_else(|| bad("active writer unavailable"))?
            .seal()?;
        next.closed.push(closed);
        let path = segment_path(&self.path, next.active);
        // A successor left before topology publication has no acknowledged
        // writes. The current selected topology authorizes retiring that exact
        // orphan; never choose it because it has the greatest sequence.
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io(e)),
        }
        let active = WalSegment::create(&path, next.active, self.metrics.clone())?;
        self.publish(&next)?;
        self.topology = next;
        self.active = Some(active);
        self.poisoned = false;
        Ok(())
    }

    /// The caller supplies a checkpoint already confirmed by ordered apply and
    /// backed by durable objects. Its exact position must be covered locally.
    /// Returns false when unpositioned history prevents reclaim authority or an
    /// older checkpoint would move the anchor backwards.
    pub fn checkpoint_applied(&mut self, manifest: &CheckpointManifest) -> Result<bool> {
        self.ensure_healthy()?;
        CheckpointManifest::decode(&manifest.encode()?)?;
        let Some(applied) = self.applied_position() else {
            return Err(bad("checkpoint before positioned apply"));
        };
        if !covers(applied, Some(manifest.position())) {
            return Err(bad(
                "checkpoint is ahead of applied position or has a conflicting term",
            ));
        }
        if let Some(old) = self.checkpoint() {
            if old.scope.cluster != manifest.scope.cluster
                || old.scope.region != manifest.scope.region
            {
                return Err(bad("checkpoint scope changed within one stream"));
            }
            if old.index > manifest.index {
                return Ok(false);
            }
            if old.index == manifest.index && old != manifest {
                return Err(bad("conflicting checkpoint at one applied index"));
            }
            if old.term > manifest.term {
                return Err(bad("checkpoint term regresses"));
            }
            if old.scope.conf_ver > manifest.scope.conf_ver
                || old.scope.version > manifest.scope.version
            {
                return Err(bad("checkpoint epoch regresses"));
            }
        }
        // Refuse contradictions already visible in segment summaries before
        // publishing an anchor. Exact committed-position certification remains
        // the caller's obligation; this is not an O(tail) replay to re-prove it.
        let cut = manifest.position();
        for segment in &self.topology.closed {
            check_cut(cut, segment.header().previous)?;
            check_cut(cut, segment.summary().first)?;
            check_cut(cut, segment.summary().last)?;
        }
        check_cut(cut, self.topology.active.previous)?;
        if let Some(active) = &self.active {
            check_cut(cut, active.summary().first)?;
            check_cut(cut, active.summary().last)?;
        }
        if self
            .topology
            .closed
            .iter()
            .any(|segment| segment.summary().has_unpositioned)
            || self
                .active
                .as_ref()
                .is_some_and(|active| active.summary().has_unpositioned)
        {
            return Ok(false);
        }
        let mut next = self.topology.clone();
        next.generation = next
            .generation
            .checked_add(1)
            .ok_or_else(|| bad("topology generation exhausted"))?;
        next.checkpoint = Some(manifest.clone());
        next.closed.retain(|segment| {
            !covers(
                manifest.position(),
                segment.summary().last.or(segment.header().previous),
            )
        });
        self.poisoned = true;
        self.publish(&next)?;
        self.topology = next;
        self.poisoned = false;
        // Unlink is post-publication housekeeping: failure cannot erase the
        // durable checkpoint authority or force tail copying under the lock.
        self.reclaim_obsolete()?;
        Ok(true)
    }

    /// Delete only this selected stream's files older than its retained suffix.
    /// A missing file after unlink failure is harmless because topology no
    /// longer references it. Foreign streams and future orphan files stay intact.
    pub fn reclaim_obsolete(&self) -> Result<Reclaimed> {
        self.ensure_healthy()?;
        let Some(_) = self.checkpoint() else {
            return Ok(Reclaimed::default());
        };
        let first = self
            .topology
            .closed
            .first()
            .map(|s| s.header().sequence)
            .unwrap_or(self.topology.active.sequence);
        let mut reclaimed = Reclaimed::default();
        let directory = segment_directory(&self.path, self.topology.active.stream_id);
        for entry in std::fs::read_dir(&directory).map_err(io)? {
            let entry = entry.map_err(io)?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some(number) = name.strip_suffix(".wal") else {
                continue;
            };
            if number.len() != 20 || !number.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            let Ok(sequence) = number.parse::<u64>() else {
                continue;
            };
            if sequence == 0 || sequence >= first {
                continue;
            }
            let bytes = entry.metadata().map_err(io)?.len();
            std::fs::remove_file(entry.path()).map_err(io)?;
            reclaimed.segments += 1;
            reclaimed.bytes += bytes;
        }
        File::open(directory)
            .and_then(|file| file.sync_all())
            .map_err(io)?;
        Ok(reclaimed)
    }

    fn ensure_healthy(&self) -> Result<()> {
        if self.poisoned || self.active.is_none() {
            Err(bad("failed stream requires recovery"))
        } else {
            Ok(())
        }
    }
    fn publish(&self, next: &Topology) -> Result<()> {
        let bytes = next.encode()?;
        let temporary = self.path.with_extension("topology.tmp");
        let mut file = File::create(&temporary).map_err(io)?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(io)?;
        std::fs::rename(&temporary, &self.path).map_err(io)?;
        self.metrics
            .namespace_publish
            .measure(|| sync_parent(&self.path))?;
        Ok(())
    }
}

impl Topology {
    fn validate(&self) -> Result<()> {
        self.active.encode()?;
        if self.generation == 0 || self.closed.len() > MAX_CLOSED_SEGMENTS {
            return Err(bad("invalid topology generation or capacity"));
        }
        if let Some(checkpoint) = &self.checkpoint {
            CheckpointManifest::decode(&checkpoint.encode()?)?;
        }
        let first = self
            .closed
            .first()
            .map(ClosedSegment::header)
            .unwrap_or(self.active);
        if first.sequence == 1 {
            if first.previous.is_some()
                && !self
                    .checkpoint
                    .as_ref()
                    .is_some_and(|c| covers(c.position(), first.previous))
            {
                return Err(bad("first segment has a predecessor"));
            }
        } else if !self
            .checkpoint
            .as_ref()
            .is_some_and(|c| covers(c.position(), first.previous))
        {
            return Err(bad("missing segments lack checkpoint coverage"));
        }
        let mut expected = first;
        for segment in &self.closed {
            let header = segment.header();
            if header != expected {
                return Err(bad("closed segment chain is discontinuous"));
            }
            ClosedSegment::from_published(header, segment.summary())?;
            if self.checkpoint.is_some() && segment.summary().has_unpositioned {
                return Err(bad("checkpoint covers unpositioned history"));
            }
            expected.sequence = expected
                .sequence
                .checked_add(1)
                .ok_or_else(|| bad("sequence overflow"))?;
            expected.previous = segment.summary().last.or(header.previous);
        }
        if expected != self.active {
            return Err(bad("active segment does not extend the closed chain"));
        }
        Ok(())
    }
    fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&self.generation.to_le_bytes());
        bytes.extend_from_slice(&self.active.encode()?);
        let checkpoint = self
            .checkpoint
            .as_ref()
            .map(CheckpointManifest::encode)
            .transpose()?
            .unwrap_or_default();
        bytes.extend_from_slice(&(checkpoint.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&checkpoint);
        bytes.extend_from_slice(&(self.closed.len() as u32).to_le_bytes());
        for segment in &self.closed {
            bytes.extend_from_slice(&segment.header().encode()?);
            let summary = segment.summary();
            bytes.extend_from_slice(&summary.bytes.to_le_bytes());
            bytes.extend_from_slice(&summary.records.to_le_bytes());
            encode_position(&mut bytes, summary.first);
            encode_position(&mut bytes, summary.last);
            bytes.push(u8::from(summary.has_unpositioned));
        }
        let digest = Sha256::digest(&bytes);
        bytes.extend_from_slice(&digest);
        if bytes.len() > MAX_TOPOLOGY_BYTES {
            return Err(bad("topology exceeds size bound"));
        }
        Ok(bytes)
    }
    fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < MAGIC.len() + 8 + HEADER_BYTES + 4 + 4 + 32
            || bytes.len() > MAX_TOPOLOGY_BYTES
            || !bytes.starts_with(MAGIC)
        {
            return Err(bad("invalid topology format or size"));
        }
        let end = bytes.len() - 32;
        if Sha256::digest(&bytes[..end]).as_slice() != &bytes[end..] {
            return Err(bad("topology checksum mismatch"));
        }
        let mut input = Input(&bytes[MAGIC.len()..end]);
        let generation = input.u64()?;
        let active = SegmentHeader::decode(input.take(HEADER_BYTES)?)?;
        let count = input.u32()? as usize;
        let checkpoint = if count == 0 {
            None
        } else {
            Some(CheckpointManifest::decode(input.take(count)?)?)
        };
        let count = input.u32()? as usize;
        if count > MAX_CLOSED_SEGMENTS {
            return Err(bad("closed segment count exceeds bound"));
        }
        let mut closed = Vec::with_capacity(count);
        for _ in 0..count {
            let header = SegmentHeader::decode(input.take(HEADER_BYTES)?)?;
            let summary = SegmentSummary {
                bytes: input.u64()?,
                records: input.u64()?,
                first: input.position()?,
                last: input.position()?,
                has_unpositioned: match input.take(1)?[0] {
                    0 => false,
                    1 => true,
                    _ => return Err(bad("invalid unpositioned flag")),
                },
            };
            closed.push(ClosedSegment::from_published(header, summary)?);
        }
        if !input.0.is_empty() {
            return Err(bad("trailing topology bytes"));
        }
        let topology = Self {
            generation,
            active,
            closed,
            checkpoint,
        };
        topology.validate()?;
        Ok(topology)
    }
}

struct Input<'a>(&'a [u8]);
impl<'a> Input<'a> {
    fn take(&mut self, size: usize) -> Result<&'a [u8]> {
        if size > self.0.len() {
            return Err(bad("truncated topology"));
        }
        let (value, rest) = self.0.split_at(size);
        self.0 = rest;
        Ok(value)
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn position(&mut self) -> Result<Option<AppliedPosition>> {
        let kind = self.take(1)?[0];
        let at = AppliedPosition {
            term: self.u64()?,
            index: self.u64()?,
        };
        match kind {
            0 if at.term == 0 && at.index == 0 => Ok(None),
            1 => Ok(Some(at)),
            _ => Err(bad("invalid topology position")),
        }
    }
}
fn encode_position(bytes: &mut Vec<u8>, position: Option<AppliedPosition>) {
    bytes.push(u8::from(position.is_some()));
    let at = position.unwrap_or(AppliedPosition { term: 0, index: 0 });
    bytes.extend_from_slice(&at.term.to_le_bytes());
    bytes.extend_from_slice(&at.index.to_le_bytes());
}
fn covers(cut: AppliedPosition, at: Option<AppliedPosition>) -> bool {
    at.is_none_or(|at| (at.index < cut.index && at.term <= cut.term) || at == cut)
}
fn check_cut(cut: AppliedPosition, position: Option<AppliedPosition>) -> Result<()> {
    if let Some(at) = position {
        if (at.index <= cut.index && !covers(cut, Some(at)))
            || (at.index > cut.index && at.term < cut.term)
        {
            return Err(bad("checkpoint contradicts a known local position"));
        }
    }
    Ok(())
}
fn segment_directory(path: &Path, stream: [u8; 16]) -> PathBuf {
    let name: String = stream.iter().map(|byte| format!("{byte:02x}")).collect();
    path.with_extension("segments").join(name)
}
fn segment_path(path: &Path, header: SegmentHeader) -> PathBuf {
    segment_directory(path, header.stream_id).join(format!("{:020}.wal", header.sequence))
}
fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(io)?
        .take(MAX_TOPOLOGY_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() > MAX_TOPOLOGY_BYTES {
        return Err(bad("topology exceeds size bound"));
    }
    Ok(bytes)
}
fn validate_target(bytes: u64) -> Result<()> {
    if !(128..=256 * 1024 * 1024).contains(&bytes) {
        return Err(bad("segment target must be between 128 bytes and 256 MiB"));
    }
    Ok(())
}
fn sync_parent(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(io)
}
fn bad(message: &str) -> Error {
    Error::Engine(format!("wal stream: {message}"))
}
fn io(error: std::io::Error) -> Error {
    bad(&format!("I/O: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkpoint::{FlushScope, SstReference};
    use crate::{ColumnFamily, Mutation};

    fn path(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("kv9-stream-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory.join("catalog.wal")
    }
    fn at(index: u64) -> AppliedPosition {
        AppliedPosition { term: 2, index }
    }
    fn batch(index: u64) -> WriteBatch {
        let mut batch = WriteBatch::new();
        batch.put(
            ColumnFamily::Default,
            index.to_be_bytes().to_vec(),
            format!("value-{index}").into_bytes(),
        );
        batch
    }
    fn create(path: &Path, target: u64) -> SegmentedWal {
        SegmentedWal::create_new(path, [31; 16], target, WalIoMetrics::shared()).unwrap()
    }
    fn reopen(path: &Path) -> Result<(SegmentedWal, StreamRecovery)> {
        RecoveryPlan::read(path)?.recover(4096, WalIoMetrics::shared(), |_| Ok(()), |_, _| Ok(()))
    }
    // This unit fixture supplies only a structurally valid checkpoint. The
    // caller's Raft/object certification is an explicit simulated premise;
    // actual MinIO and whole-runtime integration remain separate gates.
    fn checkpoint(index: u64) -> CheckpointManifest {
        let sha256 = "a".repeat(64);
        CheckpointManifest {
            scope: FlushScope {
                cluster: "stream-test".into(),
                region: 1,
                conf_ver: 1,
                version: 1,
            },
            term: 2,
            index,
            files: vec![SstReference {
                key: format!("clusters/stream-test/regions/1/sst/{sha256}"),
                sha256,
                cf: 0,
                smallest: vec![0],
                largest: vec![255],
                size: 100,
                count: 1,
            }],
        }
    }

    #[test]
    fn rotation_and_recovery_preserve_all_batches_and_position_gaps() {
        let path = path("rotation");
        let mut stream = create(&path, 128);
        for index in [1, 7, 20, 33, 50] {
            stream.append(&batch(index), Some(at(index))).unwrap();
        }
        assert_eq!(stream.closed_segments().len(), 4);
        assert_eq!(stream.active_header().sequence, 5);
        drop(stream);
        let mut observed = Vec::new();
        let (mut stream, report) = RecoveryPlan::read(&path)
            .unwrap()
            .recover(
                128,
                WalIoMetrics::shared(),
                |anchor| {
                    assert!(anchor.is_none());
                    Ok(())
                },
                |batch, position| {
                    let Mutation::Put { key, value, .. } = &batch.mutations()[0] else {
                        panic!("expected put");
                    };
                    let index = u64::from_be_bytes(key.as_slice().try_into().unwrap());
                    assert_eq!(value, format!("value-{index}").as_bytes());
                    assert_eq!(position, Some(at(index)));
                    observed.push(index);
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(observed, [1, 7, 20, 33, 50]);
        assert_eq!(report.replayed_records, 5);
        assert_eq!(stream.applied_position(), Some(at(50)));
        stream.append(&batch(81), Some(at(81))).unwrap();
        drop(stream);
        assert_eq!(reopen(&path).unwrap().0.applied_position(), Some(at(81)));
    }

    #[test]
    fn checkpoint_reclaims_only_closed_covered_segments_and_skips_their_corruption() {
        let path = path("coverage");
        let mut stream = create(&path, 4096);
        for index in [1, 2] {
            stream.append(&batch(index), Some(at(index))).unwrap();
        }
        stream.rotate().unwrap();
        let covered = stream.closed_segments()[0];
        let covered_path = segment_path(&path, covered.header());
        let mut corrupted_covered = std::fs::read(&covered_path).unwrap();
        let offset = HEADER_BYTES + crate::wal_segment::FRAME_HEADER_BYTES + 6;
        assert!(offset + 4 < corrupted_covered.len());
        corrupted_covered[offset] ^= 0x80;
        for index in [3, 5] {
            stream.append(&batch(index), Some(at(index))).unwrap();
        }
        stream.rotate().unwrap();
        let straddling = stream.closed_segments()[1];
        let straddling_path = segment_path(&path, straddling.header());
        let original_straddling = std::fs::read(&straddling_path).unwrap();
        stream.append(&batch(8), Some(at(8))).unwrap();
        let active_path = segment_path(&path, stream.active_header());
        let original_active = std::fs::read(&active_path).unwrap();
        let manifest = checkpoint(3);
        assert!(stream.checkpoint_applied(&manifest).unwrap());
        assert!(!covered_path.exists());
        assert_eq!(stream.closed_segments(), &[straddling]);
        assert_eq!(
            std::fs::read(&straddling_path).unwrap(),
            original_straddling
        );
        assert_eq!(std::fs::read(&active_path).unwrap(), original_active);
        drop(stream);
        // Simulate a closed file whose unlink was not durable. It is corrupt
        // inside a real frame and would refuse without the checkpoint anchor.
        std::fs::write(&covered_path, &corrupted_covered).unwrap();
        assert!(covered
            .replay(&covered_path, |_, _| Ok(()))
            .unwrap_err()
            .to_string()
            .contains("frame checksum"));
        let mut observed = Vec::new();
        let (stream, report) = RecoveryPlan::read(&path)
            .unwrap()
            .recover(
                4096,
                WalIoMetrics::shared(),
                |anchor| {
                    assert_eq!(anchor, Some(&manifest));
                    Ok(())
                },
                |_, at| {
                    observed.push(at.unwrap().index);
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(observed, [5, 8]);
        assert_eq!(report.covered_records, 1);
        drop(stream);
        // The same within-frame offset in the retained straddling segment
        // must refuse the whole open, even though its first record is covered.
        let mut corrupted_tail = original_straddling.clone();
        assert!(offset + 4 < corrupted_tail.len());
        corrupted_tail[offset] ^= 0x80;
        std::fs::write(&straddling_path, &corrupted_tail).unwrap();
        assert!(reopen(&path)
            .unwrap_err()
            .to_string()
            .contains("frame checksum"));
        assert_eq!(std::fs::read(&straddling_path).unwrap(), corrupted_tail);
        std::fs::write(&straddling_path, original_straddling).unwrap();
        let (mut stream, _) = reopen(&path).unwrap();
        assert!(stream.checkpoint_applied(&checkpoint(5)).unwrap());
        assert!(!straddling_path.exists());
        assert_eq!(std::fs::read(&active_path).unwrap(), original_active);
    }

    #[test]
    fn failed_rotation_fences_writer_and_recovery_ignores_unpublished_successor() {
        let path = path("rotation-failure");
        let mut stream = create(&path, 4096);
        stream.append(&batch(7), Some(at(7))).unwrap();
        let before = std::fs::read(&path).unwrap();
        let blocker = path.with_extension("topology.tmp");
        std::fs::create_dir(&blocker).unwrap();
        assert!(stream.rotate().is_err());
        assert!(stream.append(&batch(8), Some(at(8))).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let successor = SegmentHeader {
            sequence: 2,
            previous: Some(at(7)),
            ..stream.active_header()
        };
        let orphan = segment_path(&path, successor);
        assert!(orphan.exists(), "rotation never reached successor creation");
        drop(stream);
        // Arbitrary orphan bytes cannot become the selected log or affect replay.
        std::fs::write(&orphan, b"unpublished successor").unwrap();
        std::fs::remove_dir(&blocker).unwrap();
        let (mut recovered, report) = reopen(&path).unwrap();
        assert_eq!(report.replayed_records, 1);
        assert_eq!(recovered.applied_position(), Some(at(7)));
        recovered.rotate().unwrap();
        recovered.append(&batch(11), Some(at(11))).unwrap();
        drop(recovered);
        assert_eq!(reopen(&path).unwrap().0.applied_position(), Some(at(11)));
    }

    #[test]
    fn failed_checkpoint_publication_preserves_tail_and_refuses_later_writes() {
        let path = path("checkpoint-failure");
        let mut stream = create(&path, 128);
        for index in [1, 2, 3] {
            stream.append(&batch(index), Some(at(index))).unwrap();
        }
        let files: Vec<_> = stream
            .closed_segments()
            .iter()
            .map(|s| {
                let p = segment_path(&path, s.header());
                let bytes = std::fs::read(&p).unwrap();
                (p, bytes)
            })
            .collect();
        let before = std::fs::read(&path).unwrap();
        let blocker = path.with_extension("topology.tmp");
        std::fs::create_dir(&blocker).unwrap();
        assert!(stream.checkpoint_applied(&checkpoint(2)).is_err());
        assert!(stream.append(&batch(4), Some(at(4))).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        for (path, bytes) in files {
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }
        drop(stream);
        std::fs::remove_dir(&blocker).unwrap();
        let (recovered, report) = reopen(&path).unwrap();
        assert!(recovered.checkpoint().is_none());
        assert_eq!(report.replayed_records, 3);
    }

    #[test]
    fn unpositioned_and_empty_segments_keep_the_predecessor_and_reclaim_ban() {
        let path = path("unpositioned");
        let mut stream = create(&path, 4096);
        stream.append(&batch(10), Some(at(10))).unwrap();
        stream.rotate().unwrap();
        stream.rotate().unwrap();
        stream.append(&batch(99), None).unwrap();
        stream.rotate().unwrap();
        assert_eq!(stream.active_header().previous, Some(at(10)));
        assert!(stream.append(&batch(9), Some(at(9))).is_err());
        assert!(!stream.checkpoint_applied(&checkpoint(10)).unwrap());
        assert!(stream.checkpoint().is_none());
        drop(stream);
        let (mut stream, report) = reopen(&path).unwrap();
        assert_eq!(stream.applied_position(), Some(at(10)));
        assert_eq!(report.replayed_records, 2);
        stream.append(&batch(20), Some(at(20))).unwrap();
        assert!(!stream.checkpoint_applied(&checkpoint(20)).unwrap());
    }

    #[test]
    fn anchor_refusal_and_changed_plan_precede_active_tail_repair() {
        let path = path("anchor-refusal");
        let mut stream = create(&path, 4096);
        stream.append(&batch(3), Some(at(3))).unwrap();
        stream.checkpoint_applied(&checkpoint(3)).unwrap();
        let active_path = segment_path(&path, stream.active_header());
        drop(stream);
        OpenOptions::new()
            .append(true)
            .open(&active_path)
            .unwrap()
            .write_all(b"KV9R")
            .unwrap();
        let torn = std::fs::read(&active_path).unwrap();
        assert!(RecoveryPlan::read(&path)
            .unwrap()
            .recover(
                4096,
                WalIoMetrics::shared(),
                |_| Err(bad("Raft/object anchor refused")),
                |_, _| panic!("tail visited before anchor validation")
            )
            .is_err());
        assert_eq!(std::fs::read(&active_path).unwrap(), torn);
        let plan = RecoveryPlan::read(&path).unwrap();
        let mut changed = plan.topology.clone();
        changed.generation += 1;
        std::fs::write(&path, changed.encode().unwrap()).unwrap();
        assert!(plan
            .recover(4096, WalIoMetrics::shared(), |_| Ok(()), |_, _| Ok(()))
            .is_err());
        assert_eq!(std::fs::read(&active_path).unwrap(), torn);
    }

    #[test]
    fn published_anchor_survives_failed_unlink_and_housekeeping_can_retry() {
        let path = path("unlink-failure");
        let mut stream = create(&path, 128);
        for index in [1, 2] {
            stream.append(&batch(index), Some(at(index))).unwrap();
        }
        let covered = segment_path(&path, stream.closed_segments()[0].header());
        let backup = covered.with_extension("backup");
        std::fs::rename(&covered, &backup).unwrap();
        std::fs::create_dir(&covered).unwrap();
        assert!(stream.checkpoint_applied(&checkpoint(1)).is_err());
        assert_eq!(
            RecoveryPlan::read(&path)
                .unwrap()
                .checkpoint()
                .unwrap()
                .index,
            1
        );
        // The topology is already durable. Cleanup failure does not invalidate
        // it or allow another write into the now-unreferenced covered segment.
        stream.append(&batch(3), Some(at(3))).unwrap();
        drop(stream);
        let (stream, report) = reopen(&path).unwrap();
        assert_eq!(report.replayed_records, 2);
        std::fs::remove_dir(&covered).unwrap();
        std::fs::rename(&backup, &covered).unwrap();
        assert_eq!(stream.reclaim_obsolete().unwrap().segments, 1);
        assert!(!covered.exists());
    }

    #[test]
    fn malformed_topologies_and_old_writer_refuse_without_editing_files() {
        let path = path("format");
        let mut stream = create(&path, 128);
        for index in [1, 7, 11] {
            stream.append(&batch(index), Some(at(index))).unwrap();
        }
        let original = std::fs::read(&path).unwrap();
        assert!(crate::Wal::open(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        let mut invalid = stream.topology.clone();
        invalid.closed.remove(0);
        assert!(
            invalid.encode().is_err(),
            "missing predecessor lacked checkpoint authority"
        );
        let mut invalid = stream.topology.clone();
        invalid.active.previous = Some(at(6));
        assert!(invalid.encode().is_err());
        drop(stream);
        for offset in [0, 4, 9, 17, original.len() - 1] {
            let mut corrupted = original.clone();
            corrupted[offset] ^= 0x80;
            std::fs::write(&path, &corrupted).unwrap();
            assert!(RecoveryPlan::read(&path).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), corrupted);
        }
    }

    #[test]
    fn checkpoint_epochs_may_advance_without_changing_stream_identity() {
        let path = path("checkpoint-epoch");
        let mut stream = create(&path, 128);
        for index in [1, 2, 3] {
            stream.append(&batch(index), Some(at(index))).unwrap();
        }
        assert!(stream.checkpoint_applied(&checkpoint(1)).unwrap());
        let mut next = checkpoint(2);
        next.scope.conf_ver = 2;
        next.scope.version = 3;
        assert!(stream.checkpoint_applied(&next).unwrap());
        assert!(!stream.checkpoint_applied(&checkpoint(1)).unwrap());
        let before = std::fs::read(&path).unwrap();
        assert!(stream
            .checkpoint_applied(&checkpoint(3))
            .unwrap_err()
            .to_string()
            .contains("epoch regresses"));
        let mut foreign = checkpoint(3);
        foreign.scope.region = 2;
        foreign.files[0].key = foreign.files[0].key.replace("regions/1", "regions/2");
        assert!(stream.checkpoint_applied(&foreign).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        drop(stream);
        let (stream, report) = reopen(&path).unwrap();
        assert_eq!(stream.checkpoint(), Some(&next));
        assert_eq!(report.replayed_records, 1);
    }

    #[test]
    fn contradictory_checkpoint_terms_refuse_before_publication() {
        let path = path("checkpoint-term");
        let mut stream = create(&path, 4096);
        stream
            .append(&batch(10), Some(AppliedPosition { term: 3, index: 10 }))
            .unwrap();
        stream.rotate().unwrap();
        stream
            .append(&batch(20), Some(AppliedPosition { term: 5, index: 20 }))
            .unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(stream
            .checkpoint_applied(&checkpoint(15))
            .unwrap_err()
            .to_string()
            .contains("contradicts a known local position"));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(stream.checkpoint().is_none());
        stream
            .append(&batch(21), Some(AppliedPosition { term: 5, index: 21 }))
            .unwrap();
    }

    #[test]
    fn empty_selected_successor_retains_floor_and_foreign_retained_file_refuses() {
        let path = path("empty-successor");
        let mut stream = create(&path, 4096);
        stream.append(&batch(10), Some(at(10))).unwrap();
        stream.rotate().unwrap();
        let closed = stream.closed_segments()[0];
        drop(stream);
        let (mut stream, report) = reopen(&path).unwrap();
        assert_eq!(report.replayed_records, 1);
        assert_eq!(stream.applied_position(), Some(at(10)));
        assert!(stream.append(&batch(9), Some(at(9))).is_err());
        let closed_path = segment_path(&path, closed.header());
        drop(stream);
        let mut bytes = std::fs::read(&closed_path).unwrap();
        let foreign = SegmentHeader {
            stream_id: [32; 16],
            ..closed.header()
        };
        bytes[..HEADER_BYTES].copy_from_slice(&foreign.encode().unwrap());
        std::fs::write(&closed_path, &bytes).unwrap();
        assert!(reopen(&path)
            .unwrap_err()
            .to_string()
            .contains("header does not match its authority"));
        assert_eq!(std::fs::read(&closed_path).unwrap(), bytes);
    }
}
