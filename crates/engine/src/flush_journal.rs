//! Crash-safe ownership of a prepared, possibly submitted flush. This file is
//! local scheduling state; it NEVER authorizes WAL reclamation. Only ordered
//! manifest apply can do that. A valid pending file is always reconciled first.
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use kv9_common::{Error, Result};
use sha2::{Digest, Sha256};

use crate::checkpoint::{
    checked_sst_bytes, CheckpointManifest, PlannedFlush, PreparedFlush, RemoteUploader,
    MAX_CHECKPOINT_BYTES,
};

const MAGIC: &[u8] = b"KV9PENDING\x01";
const PLANNED_MAGIC: &[u8] = b"KV9PENDING\x02";
const HASH_LEN: usize = 32;
const MAX_FILE_BYTES: u64 = MAX_CHECKPOINT_BYTES as u64 + 1024 * 1024 + 128;

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
    objects: Option<Vec<Vec<u8>>>,
}
impl PendingFlush {
    pub fn manifest(&self) -> &CheckpointManifest {
        &self.manifest
    }
    pub fn expected_generation(&self) -> u64 {
        self.generation
    }
    pub fn requires_upload(&self) -> bool {
        self.objects.is_some()
    }
    pub fn recover(self, uploader: &RemoteUploader) -> Result<(PreparedFlush, u64)> {
        let prepared = match self.objects {
            Some(objects) => uploader.upload_planned(PlannedFlush {
                manifest: self.manifest,
                objects,
            })?,
            None => uploader.recover_prepared(&self.manifest)?,
        };
        Ok((prepared, self.generation))
    }

    fn encode(&self) -> Result<Vec<u8>> {
        let mut bytes = if self.objects.is_some() {
            PLANNED_MAGIC
        } else {
            MAGIC
        }
        .to_vec();
        bytes.extend(self.generation.to_le_bytes());
        let manifest = self.manifest.encode()?;
        if self.objects.is_some() {
            bytes.extend((manifest.len() as u32).to_le_bytes());
        }
        bytes.extend(manifest);
        if let Some(objects) = &self.objects {
            for object in objects {
                bytes.extend(object);
            }
        }
        let hash = Sha256::digest(&bytes);
        bytes.extend(hash);
        Ok(bytes)
    }
    fn decode(bytes: &[u8]) -> Result<Self> {
        let planned = bytes.starts_with(PLANNED_MAGIC);
        if bytes.len() <= MAGIC.len() + 8 + HASH_LEN
            || bytes.len() as u64 > MAX_FILE_BYTES
            || (!planned && !bytes.starts_with(MAGIC))
        {
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
        let start = MAGIC.len() + 8;
        let (manifest, objects) = if planned {
            let size_bytes = bytes
                .get(start..start + 4)
                .ok_or_else(|| Error::Engine("planned manifest length missing".into()))?;
            let size = u32::from_le_bytes(size_bytes.try_into().unwrap()) as usize;
            let body_start = start + 4;
            if size > 1024 * 1024 || body_start + size > end {
                return Err(Error::Engine("planned manifest length invalid".into()));
            }
            let manifest = CheckpointManifest::decode(&bytes[body_start..body_start + size])?;
            let mut offset = body_start + size;
            let mut objects = Vec::with_capacity(manifest.files.len());
            for file in &manifest.files {
                let size = usize::try_from(file.size)
                    .map_err(|_| Error::Engine("planned object size invalid".into()))?;
                if size > end - offset {
                    return Err(Error::Engine("planned object is truncated".into()));
                }
                let object = &bytes[offset..offset + size];
                checked_sst_bytes(file, object)?;
                objects.push(object.to_vec());
                offset += size;
            }
            if offset != end {
                return Err(Error::Engine("planned journal has trailing bytes".into()));
            }
            (manifest, Some(objects))
        } else {
            (CheckpointManifest::decode(&bytes[start..end])?, None)
        };
        Ok(Self {
            generation,
            manifest,
            objects,
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
            objects: None,
        };
        let bytes = pending.encode()?;
        // Validate what recovery will accept before publishing any file.
        PendingFlush::decode(&bytes)?;
        if let Some(old) = self.load()? {
            if old.encode()? == bytes {
                return Ok(());
            }
            // A sealed remotely verified capability may replace the exact local
            // upload plan. No new identity or generation may cross this boundary.
            if old.objects.is_some()
                && old.generation == pending.generation
                && old.manifest == pending.manifest
            {
                return self.publish(&bytes);
            }
            return Err(Error::Engine(
                "pending flush must settle before staging another".into(),
            ));
        }
        self.publish(&bytes)
    }

    /// Persist the exact bounded upload bytes before acquiring the remote owner
    /// or issuing the first PUT. A crash resumes this identity, never a new cut.
    pub fn stage_planned(
        &mut self,
        planned: &PlannedFlush,
        expected_generation: u64,
    ) -> Result<()> {
        let pending = PendingFlush {
            generation: expected_generation,
            manifest: planned.manifest.clone(),
            objects: Some(planned.objects.clone()),
        };
        let bytes = pending.encode()?;
        PendingFlush::decode(&bytes)?;
        if let Some(old) = self.load()? {
            if old.encode()? == bytes {
                return Ok(());
            }
            return Err(Error::Engine(
                "pending flush must settle before staging another".into(),
            ));
        }
        self.publish(&bytes)
    }

    fn publish(&self, bytes: &[u8]) -> Result<()> {
        let tmp = self.path.with_extension("pending.tmp");
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)
            .map_err(io)?;
        file.write_all(bytes).map_err(io)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkpoint::{upload_planned, FlushScope};
    use crate::{
        ColumnFamily, MemoryObjectStore, ObjectKey, ObjectStore, ReplicatedEngine, WalEngine,
        WriteBatch,
    };
    use kv9_common::AppliedPosition;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    fn fixture() -> (PathBuf, WalEngine, PlannedFlush) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kv9-planned-journal-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        let engine = WalEngine::open(dir.join("wal")).unwrap().0;
        let mut batch = WriteBatch::new();
        for cf in ColumnFamily::ALL {
            batch.put(cf, b"key".to_vec(), b"original".to_vec());
        }
        engine
            .write_applied(batch, AppliedPosition { term: 3, index: 7 })
            .unwrap();
        let plan = RemoteUploader::plan(
            engine
                .freeze(FlushScope {
                    cluster: "planned-test".into(),
                    region: 1,
                    conf_ver: 1,
                    version: 1,
                })
                .unwrap(),
        )
        .unwrap();
        (dir, engine, plan)
    }
    fn recovered_plan(pending: PendingFlush) -> PlannedFlush {
        PlannedFlush {
            manifest: pending.manifest,
            objects: pending.objects.unwrap(),
        }
    }
    #[test]
    fn durable_plan_keeps_exact_bytes_before_io_and_upgrades_only_after_verification() {
        let (dir, engine, plan) = fixture();
        let manifest = plan.manifest.clone();
        let mut journal = FlushJournal::new(dir.join("pending"));
        journal.stage_planned(&plan, 8).unwrap();
        let before = fs::read(dir.join("pending")).unwrap();
        journal.stage_planned(&plan, 8).unwrap();
        assert!(journal.stage_planned(&plan, 9).is_err());
        assert_eq!(fs::read(dir.join("pending")).unwrap(), before);
        drop(journal);
        let mut journal = FlushJournal::new(dir.join("pending"));
        let pending = journal.load().unwrap().unwrap();
        assert!(pending.requires_upload());
        assert_eq!(pending.manifest(), &manifest);
        assert_eq!(pending.objects.as_ref().unwrap(), &plan.objects);
        let store = MemoryObjectStore::new();
        assert!(
            store.list("").unwrap().is_empty(),
            "journal preparation must precede all object I/O"
        );
        let prepared = upload_planned(&store, recovered_plan(pending)).unwrap();
        journal.stage(&prepared, 8).unwrap();
        let verified = journal.load().unwrap().unwrap();
        assert!(!verified.requires_upload());
        assert_eq!(verified.manifest(), &manifest);
        assert_eq!(verified.expected_generation(), 8);
        assert!(fs::read(dir.join("pending")).unwrap().starts_with(MAGIC));
        assert!(
            journal.stage_planned(&plan, 8).is_err(),
            "a verified marker cannot regress to an upload plan"
        );
        drop(engine);
        fs::remove_dir_all(dir).unwrap();
    }

    #[derive(Debug)]
    struct AmbiguousStore {
        inner: MemoryObjectStore,
        fail: AtomicBool,
    }
    impl ObjectStore for AmbiguousStore {
        fn put(&self, key: &ObjectKey, value: &[u8]) -> Result<()> {
            self.inner.put(key, value)?;
            if self.fail.swap(false, Ordering::SeqCst) {
                return Err(Error::Engine(
                    "injected unknown PUT after object creation".into(),
                ));
            }
            Ok(())
        }
        fn get(&self, key: &ObjectKey) -> Result<Option<Vec<u8>>> {
            self.inner.get(key)
        }
        fn delete(&self, key: &ObjectKey) -> Result<()> {
            self.inner.delete(key)
        }
        fn list(&self, prefix: &str) -> Result<Vec<ObjectKey>> {
            self.inner.list(prefix)
        }
    }
    #[test]
    fn unknown_put_restarts_the_original_durable_plan_without_new_identity() {
        let (dir, engine, plan) = fixture();
        let mut journal = FlushJournal::new(dir.join("pending"));
        journal.stage_planned(&plan, 8).unwrap();
        let before = fs::read(dir.join("pending")).unwrap();
        let store = AmbiguousStore {
            inner: MemoryObjectStore::new(),
            fail: AtomicBool::new(true),
        };
        let pending = journal.load().unwrap().unwrap();
        assert!(upload_planned(&store, recovered_plan(pending)).is_err());
        assert_eq!(
            store.list("").unwrap().len(),
            1,
            "the uncertain PUT actually created an object"
        );
        assert_eq!(fs::read(dir.join("pending")).unwrap(), before);
        drop(journal);
        let mut journal = FlushJournal::new(dir.join("pending"));
        let pending = journal.load().unwrap().unwrap();
        let prepared = upload_planned(&store, recovered_plan(pending)).unwrap();
        assert_eq!(prepared.descriptor(), plan.manifest);
        assert_eq!(store.list("").unwrap().len(), plan.manifest.files.len());
        journal.stage(&prepared, 8).unwrap();
        drop(engine);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn planned_journal_refuses_all_truncations_and_valid_checksum_bad_payloads() {
        let (dir, engine, plan) = fixture();
        let mut journal = FlushJournal::new(dir.join("pending"));
        journal.stage_planned(&plan, 8).unwrap();
        let bytes = fs::read(dir.join("pending")).unwrap();
        for end in 0..bytes.len() {
            assert!(PendingFlush::decode(&bytes[..end]).is_err(), "cut {end}");
        }
        for variant in 0..4 {
            let mut changed = bytes[..bytes.len() - HASH_LEN].to_vec();
            match variant {
                0 => {
                    let last = changed.len() - 1;
                    changed[last] ^= 1;
                }
                1 => changed[MAGIC.len() + 8..MAGIC.len() + 12]
                    .copy_from_slice(&u32::MAX.to_le_bytes()),
                2 => changed.push(0),
                _ => changed[MAGIC.len()..MAGIC.len() + 8].copy_from_slice(&u64::MAX.to_le_bytes()),
            }
            let digest = Sha256::digest(&changed);
            changed.extend(digest);
            fs::write(dir.join("pending"), &changed).unwrap();
            assert!(
                journal.load().is_err(),
                "checksum-valid malformed plan {variant}"
            );
            assert_eq!(fs::read(dir.join("pending")).unwrap(), changed);
        }
        drop(engine);
        fs::remove_dir_all(dir).unwrap();
    }
}
