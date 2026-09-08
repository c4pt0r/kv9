//! Crash-safe ownership of a prepared, possibly submitted flush. This file is
//! local scheduling state; it NEVER authorizes WAL reclamation. Only ordered
//! manifest apply can do that. A valid pending file is always reconciled first.
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use kv9_common::{Error, Result};
use sha2::{Digest, Sha256};

use crate::checkpoint::{CheckpointManifest, PreparedFlush, RemoteUploader};

const MAGIC: &[u8] = b"KV9PENDING\x01";
const HASH_LEN: usize = 32;
const MAX_FILE_BYTES: u64 = 1024 * 1024 + 128;

/// One local prepared slot. The runtime gives it one owner per Raft group.
/// The file is atomically published and directory-synced before first propose.
#[derive(Debug)]
pub struct FlushJournal {
    path: PathBuf,
}

/// Only `FlushJournal::load` can recover this value from a checksummed file.
/// The caller checks its cluster/region and position against local Raft history
/// before asking it to recreate a remotely verified prepared capability.
#[derive(Debug)]
pub struct PendingFlush {
    generation: u64,
    manifest: CheckpointManifest,
}
impl PendingFlush {
    pub fn manifest(&self) -> &CheckpointManifest {
        &self.manifest
    }
    pub fn expected_generation(&self) -> u64 {
        self.generation
    }
    pub fn recover(self, uploader: &RemoteUploader) -> Result<(PreparedFlush, u64)> {
        Ok((uploader.recover_prepared(&self.manifest)?, self.generation))
    }

    fn encode(&self) -> Result<Vec<u8>> {
        let mut bytes = MAGIC.to_vec();
        bytes.extend(self.generation.to_le_bytes());
        bytes.extend(self.manifest.encode()?);
        let hash = Sha256::digest(&bytes);
        bytes.extend(hash);
        Ok(bytes)
    }
    fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() <= MAGIC.len() + 8 + HASH_LEN || !bytes.starts_with(MAGIC) {
            return Err(Error::Engine(
                "invalid pending flush version or length".into(),
            ));
        }
        let end = bytes.len() - HASH_LEN;
        if Sha256::digest(&bytes[..end]).as_slice() != &bytes[end..] {
            return Err(Error::Engine("pending flush checksum mismatch".into()));
        }
        let generation = u64::from_le_bytes(
            bytes[MAGIC.len()..MAGIC.len() + 8]
                .try_into()
                .expect("length checked"),
        );
        if generation == u64::MAX {
            return Err(Error::Engine("pending flush generation exhausted".into()));
        }
        let manifest = CheckpointManifest::decode(&bytes[MAGIC.len() + 8..end])?;
        Ok(Self {
            generation,
            manifest,
        })
    }
}
impl FlushJournal {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Recover a durable slot. Partial temporary files are never authoritative.
    /// Sync again before returning: this also seals a prior ambiguous rename
    /// whose directory sync failed while the original process was still alive.
    pub fn load(&self) -> Result<Option<PendingFlush>> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io(e)),
        };
        let mut bytes = Vec::new();
        (&file)
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(io)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(Error::Engine("pending flush file too large".into()));
        }
        let pending = PendingFlush::decode(&bytes)?;
        file.sync_all().map_err(io)?;
        sync_parent(&self.path)?;
        Ok(Some(pending))
    }

    /// Save only a real prepared capability; there is no loose descriptor writer.
    /// Repeating the exact save is safe, but a different pending attempt cannot
    /// replace an unresolved one. No caller may propose after a save error.
    pub fn stage(&mut self, prepared: &PreparedFlush, expected_generation: u64) -> Result<()> {
        let pending = PendingFlush {
            generation: expected_generation,
            manifest: prepared.descriptor(),
        };
        let bytes = pending.encode()?;
        // Validate what recovery will accept before publishing any file.
        PendingFlush::decode(&bytes)?;
        if let Some(old) = self.load()? {
            if old.encode()? == bytes {
                return Ok(());
            }
            return Err(Error::Engine(
                "pending flush must settle before staging another".into(),
            ));
        }
        let tmp = self.path.with_extension("pending.tmp");
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)
            .map_err(io)?;
        file.write_all(&bytes).map_err(io)?;
        file.sync_all().map_err(io)?;
        fs::rename(&tmp, &self.path).map_err(io)?;
        sync_parent(&self.path)
    }

    /// Call only after a typed settlement. If deletion's durability is unknown,
    /// retrying or recovering the old slot is safe: history is still retained.
    pub fn clear_settled(&mut self) -> Result<()> {
        match fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io(e)),
        }
        sync_parent(&self.path)
    }
}
fn sync_parent(path: &Path) -> Result<()> {
    File::open(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )
    .and_then(|f| f.sync_all())
    .map_err(io)
}
fn io(error: std::io::Error) -> Error {
    Error::Engine(format!("pending flush io: {error}"))
}
