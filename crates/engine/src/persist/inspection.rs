//! Read-only checkpoint selection and tail inspection before remote I/O.
use super::*;
use std::fs::File;
use std::io::Read;

impl WalEngine {
    /// Inspect a selected checkpoint's recoverable WAL tail without object I/O,
    /// file creation, synchronization or torn-tail repair. Memory is bounded by
    /// the selector and one WAL record, plus the observers' own retained state.
    /// With no selected checkpoint, return None without scanning the WAL.
    ///
    /// The caller must hold the enclosing store/group lock across inspection
    /// and the eventual full open. Callbacks are provisional: successful
    /// inspection grants neither an engine nor checkpoint/retention authority.
    pub fn inspect_checkpoint_tail(
        path: impl AsRef<Path>,
        checkpoint_observer: impl FnOnce(&CheckpointManifest) -> Result<()>,
        mut batch_observer: impl FnMut(&WriteBatch, Option<AppliedPosition>) -> Result<()>,
    ) -> Result<Option<EngineReplay>> {
        let path = path.as_ref();
        validate_layout(path)?;
        if RecoveryPlan::is_segmented(path)? {
            let plan = RecoveryPlan::read(path)?;
            let Some(manifest) = plan.checkpoint() else {
                return Ok(None);
            };
            checkpoint_observer(manifest)?;
            let report = plan.inspect_tail(|batch, at| batch_observer(&batch, at))?;
            return Ok(Some(EngineReplay {
                replayed_records: report.replayed_records,
                covered_records: report.covered_records,
                discarded_tail_bytes: report.discarded_tail_bytes,
            }));
        }
        let Some(manifest) = legacy_checkpoint(path)? else {
            return Ok(None);
        };
        checkpoint_observer(&manifest)?;
        let mut report = EngineReplay::default();
        report.discarded_tail_bytes = Wal::inspect(path, |batch, at| {
            if checkpoint_covers(manifest.position(), at)? {
                report.covered_records += 1;
            } else {
                batch_observer(&batch, at)?;
                report.replayed_records += 1;
            }
            Ok(())
        })?;
        if RecoveryPlan::is_segmented(path)? || legacy_checkpoint(path)?.as_ref() != Some(&manifest)
        {
            return Err(kv9_common::Error::Engine(
                "checkpoint selection changed during inspection".into(),
            ));
        }
        Ok(Some(report))
    }
}

pub(super) fn legacy_checkpoint(path: &Path) -> Result<Option<CheckpointManifest>> {
    let file = match File::open(path.with_extension("checkpoint")) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(checkpoint_io(e)),
    };
    let mut bytes = Vec::new();
    file.take(crate::checkpoint::MAX_MANIFEST_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(checkpoint_io)?;
    Ok(Some(CheckpointManifest::decode(&bytes)?))
}

pub(super) fn validate_layout(path: &Path) -> Result<()> {
    if path
        .with_extension("segments")
        .try_exists()
        .map_err(checkpoint_io)?
    {
        match std::fs::metadata(path) {
            Ok(metadata) if metadata.len() >= 5 => {}
            Ok(_) => {
                return Err(kv9_common::Error::Engine(
                    "segment directory exists with a truncated WAL topology".into(),
                ))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(kv9_common::Error::Engine(
                    "segment directory exists without its WAL topology".into(),
                ))
            }
            Err(e) => return Err(checkpoint_io(e)),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
