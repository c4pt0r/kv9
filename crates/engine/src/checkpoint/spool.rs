//! A local anonymous file holds planned SST bytes. Only one SST is serialized
//! or read at a time; neither a spool nor a journal is remote durability proof.
use super::*;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

/// An exclusive local spool, not a remotely verified publication capability.
/// ```compile_fail
/// fn duplicate(p: kv9_engine::checkpoint::SpooledFlush) { let _ = p.clone(); }
/// ```
/// ```compile_fail
/// fn prepared(p: kv9_engine::checkpoint::SpooledFlush) -> kv9_engine::checkpoint::PreparedFlush { p }
/// ```
#[derive(Debug)]
pub struct SpooledFlush {
    pub(crate) manifest: CheckpointManifest,
    pub(crate) objects: DiskObjects,
    // Drop the actual spool FD before returning disk admission.
    _disk: crate::checkpoint_spool_budget::Reservation,
}
impl SpooledFlush {
    pub fn manifest(&self) -> &CheckpointManifest {
        &self.manifest
    }
}

/// Own the opened inode, never reopen a pathname after its validation. A
/// prepared-journal replacement cannot redirect an already loaded upload.
#[derive(Debug)]
pub(crate) struct DiskObjects {
    pub file: crate::checkpoint_pending_budget::TrackedFile,
    pub start: u64,
    pub budget: Option<Arc<crate::checkpoint_memory::ImageBudget>>,
}
impl DiskObjects {
    pub(crate) fn visit(
        &mut self,
        manifest: &CheckpointManifest,
        fallback: Option<&Arc<crate::checkpoint_memory::ImageBudget>>,
        mut use_object: impl FnMut(&SstReference, &[u8]) -> Result<()>,
    ) -> Result<()> {
        self.file.seek(SeekFrom::Start(self.start)).map_err(io)?;
        for reference in &manifest.files {
            let _reservation = self
                .budget
                .as_ref()
                .or(fallback)
                .map(|budget| budget.reserve(reference.size))
                .transpose()?;
            let size = usize::try_from(reference.size)
                .map_err(|_| Error::Engine("checkpoint object size overflow".into()))?;
            let mut bytes = vec![0; size];
            self.file.read_exact(&mut bytes).map_err(io)?;
            // Recheck the actual bytes on every use, even after load verified
            // the whole journal. In-place disk changes cannot mint PreparedSst.
            checked_sst_bytes(reference, &bytes)?;
            use_object(reference, &bytes)?;
        }
        Ok(())
    }
}

impl RemoteUploader {
    /// Borrowed layout preflight, then one admitted SST at a time into a local
    /// anonymous spool. Checkpoint workers durably stage it before remote I/O.
    pub fn plan_spooled(
        &self,
        frozen: FrozenFlush,
        journal: &mut FlushJournal,
    ) -> Result<SpooledFlush> {
        self.plan_spooled_with(frozen, || journal.scratch())
    }

    /// Independently owned anonymous spool for concurrent transfer captures.
    /// Unlike the exclusive checkpoint journal scratch slot, these files can
    /// never truncate or unlink another live capture/worker's spool. Admission
    /// precedes file creation and any SST allocation; no remote I/O occurs.
    pub fn plan_spooled_in(
        &self,
        frozen: FrozenFlush,
        directory: &std::path::Path,
    ) -> Result<SpooledFlush> {
        self.plan_spooled_with(frozen, || tempfile::tempfile_in(directory).map_err(io))
    }

    fn plan_spooled_with(
        &self,
        frozen: FrozenFlush,
        create: impl FnOnce() -> Result<File>,
    ) -> Result<SpooledFlush> {
        let limit = usize::try_from(MAX_STREAMED_CHECKPOINT_BYTES)
            .map_err(|_| Error::Engine("streamed checkpoint limit is not addressable".into()))?;
        let layout = image_layout(&frozen, SST_TARGET, limit)?;
        if layout.total > MAX_CHECKPOINT_BYTES
            && layout.tables.iter().map(Vec::len).sum::<usize>() > MAX_STREAMED_SSTS
        {
            return Err(Error::Engine(
                "large checkpoint exceeds retention object limit".into(),
            ));
        }
        let disk = self.spool_budget.reserve(layout.total as u64)?;
        let mut file = create()?;
        let mut files = Vec::new();
        for ((cf_id, cf), tables) in ColumnFamily::ALL.into_iter().enumerate().zip(layout.tables) {
            let mut rows = frozen.view.iter_all(cf);
            for table in tables {
                let _reservation = self.image_budget.reserve(table.bytes as u64)?;
                let owned: Vec<(Vec<u8>, Vec<u8>)> = rows.by_ref().take(table.count).collect();
                let bytes = crate::sst::encode_borrowed(
                    cf,
                    table.count,
                    table.bytes,
                    owned.iter().map(|(k, v)| (k.as_slice(), v.as_slice())),
                )?;
                let (reference, bytes) = plan_sst(&frozen.scope, cf_id as u8, bytes)?;
                file.write_all(&bytes).map_err(io)?;
                files.push(reference);
            }
            if rows.next().is_some() {
                return Err(Error::Engine("checkpoint layout omitted rows".into()));
            }
        }
        if file.stream_position().map_err(io)? != layout.total as u64 {
            return Err(Error::Engine("checkpoint layout size mismatch".into()));
        }
        let manifest = CheckpointManifest {
            scope: frozen.scope,
            term: frozen.position.term,
            index: frozen.position.index,
            files,
        };
        CheckpointManifest::decode(&manifest.encode()?)?;
        Ok(SpooledFlush {
            manifest,
            _disk: disk,
            objects: DiskObjects {
                file: crate::checkpoint_pending_budget::TrackedFile::untracked(file),
                start: 0,
                budget: Some(self.image_budget.clone()),
            },
        })
    }

    /// Upload a transfer's exact spool after its durable retention owners have
    /// been published. A spool alone is neither recovery nor retention authority.
    /// Checkpoint workers still require their durable pending journal; transfers
    /// keep their Published owners on failure, including uncertain remote PUT.
    pub fn upload_spooled(&self, planned: SpooledFlush) -> Result<PreparedFlush> {
        let SpooledFlush {
            manifest,
            objects,
            _disk,
        } = planned;
        let result = self.upload_disk(manifest, objects);
        // The file closes inside upload_disk before its disk admission returns.
        drop(_disk);
        result
    }

    pub(crate) fn upload_disk(
        &self,
        manifest: CheckpointManifest,
        mut objects: DiskObjects,
    ) -> Result<PreparedFlush> {
        let manifest = CheckpointManifest::decode(&manifest.encode()?)?;
        let store = self.store();
        let mut files = Vec::with_capacity(manifest.files.len());
        objects.visit(&manifest, Some(&self.image_budget), |reference, bytes| {
            let key = ObjectKey::new(reference.key.clone())?;
            store.put(&key, bytes)?;
            if store.get_shared_bounded(&key, bytes.len())?.as_deref() != Some(bytes) {
                return Err(Error::Engine("uploaded SST was not durably visible".into()));
            }
            files.push(PreparedSst {
                reference: reference.clone(),
            });
            Ok(())
        })?;
        Ok(PreparedFlush {
            scope: manifest.scope,
            position: AppliedPosition {
                term: manifest.term,
                index: manifest.index,
            },
            files,
        })
    }
}

fn io(error: std::io::Error) -> Error {
    Error::Engine(format!("checkpoint spool io: {error}"))
}

#[cfg(test)]
mod tests;
