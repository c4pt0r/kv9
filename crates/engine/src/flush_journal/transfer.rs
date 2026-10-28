//! A transfer's metadata, manifest and SSTs publish as ONE durable file.
//! This is local retry state, not remote retention or installation authority.
use super::*;
use crate::checkpoint::checked_sst_bytes;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::sync::Arc;

const MAGIC: &[u8; 8] = b"KV9XFR01";
const MAX_METADATA: usize = 2 * 1024 * 1024 + 512;
const MAX_MANIFEST: usize = 1024 * 1024;
const OVERHEAD: usize = MAGIC.len() + 4 + 4 + HASH_LEN;

/// The stable lock inode must never be unlinked while this group can run.
/// Keep this guard through upload and actual object-job drain. Contention
/// refuses immediately; no second caller can replace the selected plan.
#[derive(Debug)]
pub struct TransferJournal {
    journal: FlushJournal,
    _lock: Arc<File>,
}

/// Keep the same advisory lock alive through an enclosing object-job scope.
#[derive(Debug)]
pub struct TransferLock {
    _lock: Arc<File>,
}

#[derive(Debug)]
pub struct PendingTransfer {
    metadata: Vec<u8>,
    manifest: CheckpointManifest,
    objects: DiskObjects,
}
impl PendingTransfer {
    pub fn metadata(&self) -> &[u8] {
        &self.metadata
    }
    pub fn manifest(&self) -> &CheckpointManifest {
        &self.manifest
    }
    /// The caller must validate application bindings and committed pins first.
    pub fn upload(self, uploader: &RemoteUploader) -> Result<PreparedFlush> {
        uploader.upload_disk(self.manifest, self.objects)
    }
}

impl TransferJournal {
    pub(crate) fn lock(journal: FlushJournal) -> Result<Self> {
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(journal.path.with_extension("lock"))
            .map_err(io)?;
        lock.try_lock()
            .map_err(|e| Error::Engine(format!("transfer slot is locked: {e}")))?;
        Ok(Self {
            journal,
            _lock: Arc::new(lock),
        })
    }

    pub fn hold_lock(&self) -> TransferLock {
        TransferLock {
            _lock: self._lock.clone(),
        }
    }

    /// Verify the full checksum, canonical manifest and each SST before
    /// returning a handle to the SAME opened inode. Partial tmp files do not
    /// select a plan; a corrupt selected file must never fall back to replanning.
    pub fn load(&self) -> Result<Option<PendingTransfer>> {
        let file = match File::open(&self.journal.path) {
            Ok(file) => self.journal.tracked(file)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io(e)),
        };
        let len = file.metadata().map_err(io)?.len();
        let (metadata, manifest, start, input) = decode_stream(
            BufReader::new(file),
            len,
            self.journal.image_budget.as_ref(),
        )?;
        let file = input.into_inner();
        file.sync_all().map_err(io)?;
        sync_parent(&self.journal.path)?;
        Ok(Some(PendingTransfer {
            metadata,
            manifest,
            objects: DiskObjects {
                file,
                start,
                budget: self.journal.image_budget.clone(),
            },
        }))
    }

    /// Export exactly the verified inode, holding its reader charge and stable
    /// slot lock until the actual consumer drops the reader. No path reopen.
    pub fn into_reader(self) -> Result<Option<TransferReader>> {
        let Some(pending) = self.load()? else {
            return Ok(None);
        };
        let mut file = pending.objects.file;
        let length = file.metadata().map_err(io)?.len();
        file.rewind().map_err(io)?;
        Ok(Some(TransferReader {
            file,
            remaining: length,
            length,
            _journal: self,
        }))
    }

    /// Validate incoming metadata, canonical manifest, every SST, full checksum
    /// and exact EOF BEFORE publishing. A failed receive retains only temporary
    /// charged debt; it cannot select a partial plan or replace another binding.
    pub fn stage_received(
        &mut self,
        metadata: &[u8],
        manifest: &CheckpointManifest,
        input: impl Read,
    ) -> Result<()> {
        self.receive_observed(metadata, manifest, input, &mut |_| Ok(()))
    }

    fn receive_observed(
        &mut self,
        metadata: &[u8],
        manifest: &CheckpointManifest,
        input: impl Read,
        observe: &mut impl FnMut(PublicationStep) -> Result<()>,
    ) -> Result<()> {
        let bytes = Self::encoded_len(metadata, manifest)?;
        if let Some(old) = self.load()? {
            if old.metadata == metadata && old.manifest == *manifest {
                return Ok(());
            }
            return Err(invalid("unsettled slot cannot name a different plan"));
        }
        let published = self.journal.publish_observed(
            bytes,
            |output| {
                let tee = Receiving {
                    input,
                    output,
                    remaining: bytes,
                };
                let (stored_metadata, stored_manifest, _, _) =
                    decode_stream(tee, bytes, self.journal.image_budget.as_ref())?;
                if stored_metadata != metadata || stored_manifest != *manifest {
                    return Err(invalid("received transfer binding differs"));
                }
                Ok(())
            },
            observe,
        )?;
        if published {
            Ok(())
        } else {
            Err(disk_full())
        }
    }

    pub fn encoded_len(metadata: &[u8], manifest: &CheckpointManifest) -> Result<u64> {
        if metadata.is_empty() || metadata.len() > MAX_METADATA {
            return Err(invalid("metadata length exceeds bound"));
        }
        let encoded = manifest.encode()?;
        CheckpointManifest::decode(&encoded)?;
        let bytes = manifest.files.iter().try_fold(
            (OVERHEAD + metadata.len() + encoded.len()) as u64,
            |n, file| {
                n.checked_add(file.size)
                    .ok_or_else(|| invalid("transfer length overflow"))
            },
        )?;
        if bytes > MAX_STREAMED_CHECKPOINT_BYTES + (OVERHEAD + MAX_METADATA + MAX_MANIFEST) as u64 {
            return Err(invalid("file length exceeds bound"));
        }
        Ok(bytes)
    }

    pub fn stage_spooled(&mut self, metadata: &[u8], planned: &mut SpooledFlush) -> Result<()> {
        self.stage_observed(metadata, planned, &mut |_| Ok(()))
    }

    fn stage_observed(
        &mut self,
        metadata: &[u8],
        planned: &mut SpooledFlush,
        observe: &mut impl FnMut(PublicationStep) -> Result<()>,
    ) -> Result<()> {
        if metadata.is_empty() || metadata.len() > MAX_METADATA {
            return Err(invalid("metadata length exceeds bound"));
        }
        let manifest = planned.manifest.encode()?;
        CheckpointManifest::decode(&manifest)?;
        if let Some(old) = self.load()? {
            if old.metadata == metadata && old.manifest == planned.manifest {
                return Ok(());
            }
            return Err(invalid("unsettled slot cannot name a different plan"));
        }
        let bytes = (OVERHEAD + metadata.len() + manifest.len()) as u64
            + planned
                .manifest
                .files
                .iter()
                .map(|file| file.size)
                .sum::<u64>();
        let published = self.journal.publish_observed(
            bytes,
            |output| {
                let mut hash = Sha256::new();
                let mut part = |bytes: &[u8]| -> Result<()> {
                    output.write_all(bytes).map_err(io)?;
                    hash.update(bytes);
                    Ok(())
                };
                part(MAGIC)?;
                part(&(metadata.len() as u32).to_le_bytes())?;
                part(metadata)?;
                part(&(manifest.len() as u32).to_le_bytes())?;
                part(&manifest)?;
                planned.objects.visit(
                    &planned.manifest,
                    self.journal.image_budget.as_ref(),
                    |_, bytes| part(bytes),
                )?;
                output.write_all(&hash.finalize()).map_err(io)
            },
            observe,
        )?;
        if published {
            Ok(())
        } else {
            Err(disk_full())
        }
    }

    /// Local payload cleanup only, after application-verified terminal state
    /// or group reclamation. This neither releases pins nor proves PUT closure.
    pub fn clear_settled(&mut self) -> Result<()> {
        self.journal.clear_settled()
    }
}

/// An export retains the original slot lock and the same charged file inode.
#[derive(Debug)]
pub struct TransferReader {
    file: TrackedFile,
    remaining: u64,
    length: u64,
    _journal: TransferJournal,
}
impl TransferReader {
    pub fn file_len(&self) -> u64 {
        self.length
    }
}
impl Read for TransferReader {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let len = output
            .len()
            .min(usize::try_from(self.remaining).unwrap_or(usize::MAX));
        if len == 0 {
            return Ok(0);
        }
        let read = self.file.read(&mut output[..len])?;
        if read == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        self.remaining -= read as u64;
        Ok(read)
    }
}
struct Receiving<'a, R> {
    input: R,
    output: &'a mut TrackedFile,
    remaining: u64,
}
impl<R: Read> Read for Receiving<'_, R> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        let n = self.input.read(bytes)?;
        if n as u64 > self.remaining {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "transfer has trailing bytes",
            ));
        }
        self.output.write_all(&bytes[..n])?;
        self.remaining -= n as u64;
        Ok(n)
    }
}

fn decode_stream<R: Read>(
    input: R,
    len: u64,
    budget: Option<&std::sync::Arc<crate::checkpoint_memory::ImageBudget>>,
) -> Result<(Vec<u8>, CheckpointManifest, u64, R)> {
    if len < OVERHEAD as u64
        || len > MAX_STREAMED_CHECKPOINT_BYTES + (OVERHEAD + MAX_METADATA + MAX_MANIFEST) as u64
    {
        return Err(invalid("invalid file length"));
    }
    let mut body = codec::Body {
        input,
        remaining: usize::try_from(len).map_err(|_| invalid("file length overflow"))? - HASH_LEN,
        hash: Sha256::new(),
    };
    if body.array::<8>()? != *MAGIC {
        return Err(invalid("invalid version"));
    }
    let size = u32::from_le_bytes(body.array()?) as usize;
    if size == 0 || size > MAX_METADATA {
        return Err(invalid("metadata length exceeds bound"));
    }
    let metadata = body.bytes(size)?;
    let size = u32::from_le_bytes(body.array()?) as usize;
    if size > MAX_MANIFEST {
        return Err(invalid("manifest length exceeds bound"));
    }
    let bytes = body.bytes(size)?;
    let manifest = CheckpointManifest::decode(&bytes)?;
    if manifest.encode()? != bytes {
        return Err(invalid("noncanonical manifest"));
    }
    let start = len - HASH_LEN as u64 - body.remaining as u64;
    for reference in &manifest.files {
        let _reservation = budget.map(|b| b.reserve(reference.size)).transpose()?;
        let size = usize::try_from(reference.size).map_err(|_| invalid("SST length overflow"))?;
        checked_sst_bytes(reference, &body.bytes(size)?)?;
    }
    Ok((metadata, manifest, start, body.finish()?))
}

fn invalid(message: &str) -> Error {
    Error::Engine(format!("transfer journal: {message}"))
}

#[cfg(test)]
mod tests;
