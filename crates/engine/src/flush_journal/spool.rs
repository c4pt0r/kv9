//! Stream the unchanged planned-journal format from an opened local spool.
use super::*;
use sha2::{Digest, Sha256};
use std::io::Write;

impl FlushJournal {
    pub(super) fn load_streamed(&self, file: File) -> Result<PendingFlush> {
        let pending = PendingFlush::read_disk(self.tracked(file)?, self.image_budget.as_ref())?;
        sync_parent(&self.path)?;
        Ok(pending)
    }

    /// The journal has one exclusive group owner, as for its publication tmp.
    /// Unlink immediately: failed/cancelled planning releases blocks on FD drop;
    /// a crash in the open/unlink window leaves one fixed, reusable scratch slot.
    pub(crate) fn scratch(&mut self) -> Result<File> {
        let path = self.path.with_extension("checkpoint-image.tmp");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .map_err(io)?;
        fs::remove_file(path).map_err(io)?;
        Ok(file)
    }

    pub fn stage_spooled(&mut self, planned: &mut SpooledFlush, generation: u64) -> Result<()> {
        let descriptor = PendingRef {
            generation,
            manifest: &planned.manifest,
            objects: None,
        };
        let manifest = descriptor.validate()?;
        // Validate old pending with a per-object bound even when this journal
        // was constructed by a compatibility caller without streaming enabled.
        let old = match File::open(&self.path) {
            Ok(file) => Some(self.load_streamed(file)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(io(e)),
        };
        if let Some(old) = old {
            if old.requires_upload()
                && old.generation == generation
                && old.manifest == planned.manifest
            {
                return Ok(());
            }
            return Err(Error::Engine(
                "pending flush must settle before staging another".into(),
            ));
        }
        let bytes = journal_bytes(
            manifest.len(),
            Some(planned.manifest.files.iter().map(|file| file.size).sum()),
        );
        let published = self.publish_with(bytes, |output| {
            let mut hash = Sha256::new();
            let mut part = |bytes: &[u8]| -> Result<()> {
                output.write_all(bytes).map_err(io)?;
                hash.update(bytes);
                Ok(())
            };
            part(PLANNED_MAGIC)?;
            part(&generation.to_le_bytes())?;
            part(&(manifest.len() as u32).to_le_bytes())?;
            part(&manifest)?;
            planned
                .objects
                .visit(&planned.manifest, self.image_budget.as_ref(), |_, bytes| {
                    part(bytes)
                })?;
            output.write_all(&hash.finalize()).map_err(io)?;
            Ok(())
        })?;
        if published {
            Ok(())
        } else {
            Err(disk_full())
        }
    }
}
