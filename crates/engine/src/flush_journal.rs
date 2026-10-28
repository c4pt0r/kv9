//! Crash-safe ownership of a prepared, possibly submitted flush. This file is
//! local scheduling state; it NEVER authorizes WAL reclamation. Only ordered
//! manifest apply can do that. A valid pending file is always reconciled first.
use std::fs::{self, File, OpenOptions};
use std::io::BufReader;
use std::io::Seek;
use std::path::{Path, PathBuf};

use kv9_common::{Error, Result};

use crate::checkpoint::{
    CheckpointManifest, DiskObjects, PlannedFlush, PreparedFlush, RemoteUploader, SpooledFlush,
    MAX_CHECKPOINT_BYTES, MAX_STREAMED_CHECKPOINT_BYTES,
};

mod codec;
mod spool;
mod transfer;
use crate::checkpoint_pending_budget::{PendingDiskBudget, TrackedFile};
use codec::PendingRef;
pub use transfer::{PendingTransfer, TransferJournal, TransferLock, TransferReader};

const MAGIC: &[u8] = b"KV9PENDING\x01";
const PLANNED_MAGIC: &[u8] = b"KV9PENDING\x02";
const HASH_LEN: usize = 32;
const MAX_FILE_BYTES: u64 = MAX_STREAMED_CHECKPOINT_BYTES + 1024 * 1024 + 128;
const MAX_RESIDENT_FILE_BYTES: u64 = MAX_CHECKPOINT_BYTES as u64 + 1024 * 1024 + 128;

/// One local prepared slot. The runtime gives it one owner per Raft group.
/// The file is atomically published and directory-synced before first propose.
#[derive(Debug)]
pub struct FlushJournal {
    path: PathBuf,
    image_budget: Option<std::sync::Arc<crate::checkpoint_memory::ImageBudget>>,
    streaming: bool,
    pending_budget: Option<std::sync::Arc<PendingDiskBudget>>,
}

/// Both outcomes retain the exact durable recovery identity. Keeping a plan
/// under disk pressure may repeat verified uploads after restart, never a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadedJournal {
    Prepared,
    PlannedRetained,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PublicationStep {
    Written,
    FileSynced,
    Renamed,
    DirectorySynced,
}

/// Only `FlushJournal::load` can recover this value from a checksummed file.
/// The caller checks its cluster/region and position against local Raft history
/// before asking it to recreate a remotely verified prepared capability.
#[derive(Debug)]
pub struct PendingFlush {
    generation: u64,
    manifest: CheckpointManifest,
    objects: Option<Vec<Vec<u8>>>,
    disk_objects: Option<DiskObjects>,
    reservation: Option<crate::checkpoint_memory::Reservation>,
}
impl PendingFlush {
    pub fn manifest(&self) -> &CheckpointManifest {
        &self.manifest
    }
    pub fn expected_generation(&self) -> u64 {
        self.generation
    }
    pub fn requires_upload(&self) -> bool {
        self.objects.is_some() || self.disk_objects.is_some()
    }
    pub fn recover(self, uploader: &RemoteUploader) -> Result<(PreparedFlush, u64)> {
        let prepared = match (self.objects, self.disk_objects) {
            (Some(objects), None) => uploader.upload_planned(PlannedFlush {
                manifest: self.manifest,
                objects,
                reservation: self.reservation,
            })?,
            (None, Some(objects)) => uploader.upload_disk(self.manifest, objects)?,
            (None, None) => uploader.recover_prepared(&self.manifest)?,
            (Some(_), Some(_)) => {
                return Err(Error::Engine("duplicate pending object source".into()))
            }
        };
        Ok((prepared, self.generation))
    }
}
impl FlushJournal {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            image_budget: None,
            streaming: false,
            pending_budget: None,
        }
    }

    pub fn with_image_budget(
        mut self,
        budget: std::sync::Arc<crate::checkpoint_memory::ImageBudget>,
    ) -> Self {
        self.image_budget = Some(budget);
        self
    }

    /// Validate complete journal/SST checksums while retaining only its opened
    /// inode and descriptor. Upload admits and verifies each SST again.
    pub fn with_streaming_objects(mut self) -> Self {
        self.streaming = true;
        self
    }

    pub fn with_pending_budget(mut self, budget: std::sync::Arc<PendingDiskBudget>) -> Self {
        self.pending_budget = Some(budget);
        self
    }

    fn tracked(&self, file: File) -> Result<TrackedFile> {
        match &self.pending_budget {
            Some(budget) => budget.read(&self.path, file),
            None => Ok(TrackedFile::untracked(file)),
        }
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
        // Allocate each bounded object directly; do not retain an additional
        // whole-file buffer alongside all decoded objects.
        if self.streaming {
            return self.load_streamed(file).map(Some);
        }
        let mut file = self.tracked(file)?;
        let len = file.metadata().map_err(io)?.len();
        let pending = PendingFlush::read_with_budget(
            BufReader::new(&mut file),
            len,
            self.image_budget.as_ref(),
        )?;
        file.sync_all().map_err(io)?;
        sync_parent(&self.path)?;
        Ok(Some(pending))
    }

    /// Save only a real prepared capability; there is no loose descriptor writer.
    /// Repeating the exact save is safe, but a different pending attempt cannot
    /// replace an unresolved one. No caller may propose after a save error.
    pub fn stage(&mut self, prepared: &PreparedFlush, expected_generation: u64) -> Result<()> {
        self.stage_prepared(prepared, expected_generation, false)
            .map(|_| ())
    }

    /// Allow settlement to free an existing plan even with zero publication
    /// space. Only a quota refusal before creating a temporary file may retain
    /// the already validated exact durable plan; all I/O errors still fail.
    pub fn record_uploaded(
        &mut self,
        prepared: &PreparedFlush,
        expected_generation: u64,
    ) -> Result<UploadedJournal> {
        self.stage_prepared(prepared, expected_generation, true)
    }

    fn stage_prepared(
        &mut self,
        prepared: &PreparedFlush,
        expected_generation: u64,
        allow_plan: bool,
    ) -> Result<UploadedJournal> {
        let manifest = prepared.descriptor();
        let pending = PendingRef {
            generation: expected_generation,
            manifest: &manifest,
            objects: None,
        };
        let encoded_manifest = pending.validate()?;
        let mut exact_plan = false;
        if let Some(old) = self.load()? {
            if pending.matches(&old) {
                return Ok(UploadedJournal::Prepared);
            }
            // A sealed remotely verified capability may replace the exact local
            // upload plan. No new identity or generation may cross this boundary.
            if !old.requires_upload()
                || old.generation != pending.generation
                || old.manifest != *pending.manifest
            {
                return Err(Error::Engine(
                    "pending flush must settle before staging another".into(),
                ));
            }
            exact_plan = true;
        }
        if self.publish(&pending, &encoded_manifest)? {
            Ok(UploadedJournal::Prepared)
        } else if allow_plan && exact_plan {
            if let Some(budget) = &self.pending_budget {
                budget.deferred_prepared();
            }
            Ok(UploadedJournal::PlannedRetained)
        } else {
            Err(disk_full())
        }
    }

    /// Persist the exact bounded upload bytes before acquiring the remote owner
    /// or issuing the first PUT. A crash resumes this identity, never a new cut.
    pub fn stage_planned(
        &mut self,
        planned: &PlannedFlush,
        expected_generation: u64,
    ) -> Result<()> {
        let pending = PendingRef {
            generation: expected_generation,
            manifest: &planned.manifest,
            objects: Some(&planned.objects),
        };
        let encoded_manifest = pending.validate()?;
        if let Some(old) = self.load()? {
            if pending.matches(&old) {
                return Ok(());
            }
            return Err(Error::Engine(
                "pending flush must settle before staging another".into(),
            ));
        }
        if self.publish(&pending, &encoded_manifest)? {
            Ok(())
        } else {
            Err(disk_full())
        }
    }

    fn publish(&self, pending: &PendingRef<'_>, manifest: &[u8]) -> Result<bool> {
        let bytes = journal_bytes(
            manifest.len(),
            pending
                .objects
                .map(|objects| objects.iter().map(|o| o.len() as u64).sum()),
        );
        self.publish_with(bytes, |file| pending.write_to(file, manifest))
    }

    fn clear_temporary(&self) -> Result<()> {
        let tmp = self.path.with_extension("pending.tmp");
        if let Some(budget) = &self.pending_budget {
            budget.check_no_writer(&tmp)?;
        }
        remove_if_present(&tmp)?;
        sync_parent(&tmp)?;
        if let Some(budget) = &self.pending_budget {
            budget.forgotten(&tmp);
        }
        Ok(())
    }

    /// Reserve old+new bytes before opening or writing a replacement. Failure
    /// leaves named debt charged until verified cleanup or startup inventory.
    fn publish_with(
        &self,
        bytes: u64,
        write: impl FnOnce(&mut TrackedFile) -> Result<()>,
    ) -> Result<bool> {
        self.publish_observed(bytes, write, &mut |_| Ok(()))
    }
    fn publish_observed(
        &self,
        bytes: u64,
        write: impl FnOnce(&mut TrackedFile) -> Result<()>,
        observe: &mut impl FnMut(PublicationStep) -> Result<()>,
    ) -> Result<bool> {
        self.clear_temporary()?;
        let tmp = self.path.with_extension("pending.tmp");
        let lease = match &self.pending_budget {
            Some(budget) => match budget.reserve(&tmp, bytes)? {
                Some(lease) => Some(lease),
                None => return Ok(false),
            },
            None => None,
        };
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(io)?;
        let mut file = match lease {
            Some(lease) => lease.opened(file)?,
            None => TrackedFile::untracked(file),
        };
        write(&mut file)?;
        if file.stream_position().map_err(io)? != bytes {
            return Err(Error::Engine(
                "pending journal length differs from reservation".into(),
            ));
        }
        observe(PublicationStep::Written)?;
        file.sync_all().map_err(io)?;
        observe(PublicationStep::FileSynced)?;
        fs::rename(&tmp, &self.path).map_err(io)?;
        observe(PublicationStep::Renamed)?;
        sync_parent(&self.path)?;
        observe(PublicationStep::DirectorySynced)?;
        if let Some(budget) = &self.pending_budget {
            budget.published(&tmp, &self.path)?;
        }
        Ok(true)
    }

    /// Call only after a typed settlement. If deletion's durability is unknown,
    /// retrying or recovering the old slot is safe: history is still retained.
    pub fn clear_settled(&mut self) -> Result<()> {
        self.clear_observed(&mut || Ok(()))
    }
    fn clear_observed(&mut self, after_remove: &mut impl FnMut() -> Result<()>) -> Result<()> {
        let tmp = self.path.with_extension("pending.tmp");
        if let Some(budget) = &self.pending_budget {
            budget.check_no_writer(&tmp)?;
        }
        remove_if_present(&self.path)?;
        // A wrong-type directory was never a publication file. Leave it for
        // diagnosis, while allowing an absent candidate's history pin to clear.
        let temporary_file = !fs::symlink_metadata(&tmp).is_ok_and(|m| m.is_dir());
        if temporary_file {
            remove_if_present(&tmp)?;
        }
        after_remove()?;
        sync_parent(&self.path)?;
        if let Some(budget) = &self.pending_budget {
            budget.forgotten(&self.path);
            if temporary_file {
                budget.forgotten(&tmp);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod disk_tests;
fn journal_bytes(manifest: usize, objects: Option<u64>) -> u64 {
    (MAGIC.len() + 8 + HASH_LEN + manifest) as u64 + objects.map_or(0, |bytes| bytes + 4)
}
fn remove_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io(e)),
    }
}
fn disk_full() -> Error {
    Error::Engine("pending journal disk budget exhausted".into())
}
pub(crate) fn sync_parent(path: &Path) -> Result<()> {
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
    use sha2::{Digest, Sha256};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    pub(super) fn fixture() -> (PathBuf, WalEngine, PlannedFlush) {
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
    pub(super) fn recovered_plan(pending: PendingFlush) -> PlannedFlush {
        PlannedFlush {
            manifest: pending.manifest,
            objects: pending.objects.unwrap(),
            reservation: pending.reservation,
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

#[cfg(test)]
mod codec_tests;
