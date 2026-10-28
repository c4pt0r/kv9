//! Local winning-publication inspection before a selected image can be read.
//! This does not restore data, certify TTL/range state or authorize retention.
use super::*;
use std::cell::RefCell;
use std::path::Path;

/// Immutable inputs observed from the selected local WAL and committed Raft.
/// A runtime may use the predecessor to locate the exact retention source;
/// this plan itself grants no object pin, read permission or recovery receipt.
#[derive(Debug)]
pub struct CheckpointReadPlan {
    manifest: Option<CheckpointManifest>,
    publication: Option<(AppliedPosition, u64)>,
}
impl CheckpointReadPlan {
    pub fn manifest(&self) -> Option<&CheckpointManifest> {
        self.manifest.as_ref()
    }
    pub fn publication(&self) -> Option<AppliedPosition> {
        self.publication.map(|(at, _)| at)
    }
    pub fn predecessor(&self) -> Option<u64> {
        self.publication.map(|(_, generation)| generation - 1)
    }
    pub(super) fn check_selection(&self, manifest: Option<&CheckpointManifest>) -> Result<()> {
        if self.manifest.as_ref() != manifest {
            return Err(bad("checkpoint selection changed after read inspection"));
        }
        Ok(())
    }
    pub(super) fn check_publication(
        &self,
        recovered: Option<&RecoveredCheckpointPublication>,
    ) -> Result<()> {
        if recovered.map(|r| (r.publication(), r.generation())) != self.publication
            || recovered.is_some_and(|r| Some(r.manifest()) != self.manifest.as_ref())
        {
            return Err(bad("checkpoint publication changed after read inspection"));
        }
        Ok(())
    }
}

/// Stream the selected checkpoint's WAL tail using the same exact committed
/// publication checks as full recovery, with no SST GET or local file repair.
/// The caller holds exclusive store/group ownership until full recovery ends.
pub fn inspect_checkpoint_read(
    storage: &mut DiskRaftStorage,
    path: impl AsRef<Path>,
    cluster: String,
    region: u64,
) -> Result<CheckpointReadPlan> {
    inspect_for_read(storage, path.as_ref(), cluster, region, None)
}

pub(super) fn inspect_for_read(
    storage: &mut DiskRaftStorage,
    path: &Path,
    cluster: String,
    region: u64,
    // Only the immutable original adoption anchor may lack local publication.
    installed: Option<&CheckpointManifest>,
) -> Result<CheckpointReadPlan> {
    let recovery = RefCell::new(CheckpointRecovery::new(storage, cluster, region));
    let mut manifest = None;
    let mut witness = TailWitness::default();
    WalEngine::inspect_checkpoint_tail(
        path,
        |base| {
            if installed.is_some_and(|anchor| {
                base != anchor && (base.index <= anchor.index || base.term < anchor.term)
            }) {
                return Err(bad("selected checkpoint contradicts adopted image"));
            }
            manifest = Some(base.clone());
            recovery
                .borrow_mut()
                .checkpoint((Some(base) != installed).then_some(base))
        },
        |batch, at| {
            let mut recovery = recovery.borrow_mut();
            recovery.batch(batch, at)?;
            witness.batch(&recovery, batch)
        },
    )?;
    let recovery = recovery.into_inner();
    witness.finish(&recovery)?;
    Ok(CheckpointReadPlan {
        manifest,
        publication: recovery.publication,
    })
}

// Only the first successful generation's history row and the latest atomic
// pair matter here. Do not construct a second engine or retain the whole tail.
#[derive(Default)]
struct TailWitness {
    history_present: bool,
    pair: Option<ManifestPair>,
}
impl TailWitness {
    fn batch(&mut self, recovery: &CheckpointRecovery<'_>, batch: &WriteBatch) -> Result<()> {
        let (Some(selected), Some((_, generation))) = (&recovery.selected, recovery.publication)
        else {
            return Ok(());
        };
        let history_key = manifest_gen_key(recovery.region, generation);
        for mutation in batch.mutations() {
            let (cf, key, value) = match mutation {
                Mutation::Put { cf, key, value } => (cf, key, Some(value)),
                Mutation::Delete { cf, key } => (cf, key, None),
            };
            if *cf != ColumnFamily::Default {
                continue;
            }
            if key == &history_key {
                self.history_present = value == Some(&selected.installed);
            }
            if key == &selected.pair_key {
                self.pair = value.map(|v| ManifestPair::decode(v)).transpose()?;
            }
        }
        Ok(())
    }
    fn finish(self, recovery: &CheckpointRecovery<'_>) -> Result<()> {
        let Some(selected) = &recovery.selected else {
            return Ok(());
        };
        let (_, generation) = recovery
            .publication
            .ok_or_else(|| bad("checkpoint lacks a retained successful publication batch"))?;
        let pair = self
            .pair
            .ok_or_else(|| bad("inspected manifest pair is missing"))?;
        if !self.history_present
            || pair.generation < generation
            || (pair.generation == generation
                && pair.last_change_id != selected.manifest.change_id(generation - 1)?)
        {
            return Err(bad(
                "inspected WAL does not retain its checkpoint publication",
            ));
        }
        Ok(())
    }
}
