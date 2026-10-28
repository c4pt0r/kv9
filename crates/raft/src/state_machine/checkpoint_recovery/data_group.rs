//! Per-data-group recovery from an actual selected checkpoint and WAL tail.
//! A decoded manifest is not sufficient: the common recovery seam verifies its
//! exact successful publication against retained committed protocol history.
//! The enclosing runtime supplies its committed creation/store identity and
//! keeps the exclusive store lock until this complete open succeeds.
use super::{CheckpointRecovery, RecoveredCheckpointPublication};
use crate::state_machine::{manifest_gen_key, manifest_pair_key};
use crate::storage::DiskRaftStorage;
use crate::ManifestPair;
use kv9_common::codec::{decode_key, KeyMode};
use kv9_common::data_range::{DataRange, COMPACTION_CONFIRMED_KEY, RANGE_KEY};
use kv9_common::raw_ttl::{ClockWatermark, ExpiryRecord, CLOCK_KEY};
use kv9_common::{AppliedPosition, ClusterId, Error, RegionId, Result, RootDigest};
use kv9_engine::checkpoint::{CheckpointManifest, CheckpointRestore, RemoteUploader};
use kv9_engine::{ColumnFamily, Engine, EngineReplay, ReadView, WalEngine};
use std::cell::{Cell, RefCell};
use std::path::Path;

mod dependencies;
mod image;
mod tail;
pub use dependencies::{
    open_with_dependencies, open_with_retained_reader, DataRecoveryDependencies,
    InstalledRecoveryDependency, RecoveryImage, RecoveryImageState, RecoveryPublication,
};
pub use image::{
    checkpoint_scope, checkpoint_state, inspect_base, inspect_checkpoint, DataCheckpointState,
    InspectedCheckpoint,
};
pub(crate) use image::{inspect_image, InspectedDataImage};

/// Recovery reads distinguish an installed original image from an exact local
/// publication. The predecessor is supplied by verified WAL/Raft inspection,
/// never guessed from the latest manifest pair or supplied by an RPC caller.
pub trait DataCheckpointReader: CheckpointRestore {
    /// Runtime readers check the durable store floor even when WAL inspection
    /// selects no image, before recovery can repair a tail or issue object I/O.
    fn check_selection(
        &self,
        _identity: DataCheckpointIdentity,
        _manifest: Option<&CheckpointManifest>,
    ) -> Result<()> {
        Ok(())
    }

    fn restore_published(
        &self,
        manifest: &CheckpointManifest,
        predecessor: u64,
    ) -> Result<kv9_engine::MemEngine>;
}
impl DataCheckpointReader for RemoteUploader {
    fn restore_published(
        &self,
        manifest: &CheckpointManifest,
        _: u64,
    ) -> Result<kv9_engine::MemEngine> {
        self.restore(manifest)
    }
}

struct SelectedReader<'a> {
    reader: &'a dyn DataCheckpointReader,
    plan: &'a super::CheckpointReadPlan,
}
impl CheckpointRestore for SelectedReader<'_> {
    fn restore(&self, manifest: &CheckpointManifest) -> Result<kv9_engine::MemEngine> {
        self.plan.check_selection(Some(manifest))?;
        match self.plan.predecessor() {
            Some(predecessor) => self.reader.restore_published(manifest, predecessor),
            None => self.reader.restore(manifest),
        }
    }
}

/// Supplied only by the validated durable adoption record. This exact image
/// may precede local publication; subsequent checkpoints may not use that exception.
pub(crate) struct InstalledBase<'a> {
    pub manifest: &'a CheckpointManifest,
    pub range: &'a DataRange,
    pub configuration: &'a raft::prelude::ConfState,
    /// From the durable adoption receipt, minted from its verified original image.
    pub watermark: Option<ClockWatermark>,
}

/// Expected immutable identity, supplied from committed creation and the
/// runtime's verified store. Constructing this data grants no authority.
#[derive(Debug, Clone, Copy)]
pub struct DataCheckpointIdentity {
    pub cluster: ClusterId,
    pub root: RootDigest,
    pub region: RegionId,
    pub creation: RootDigest,
}
impl DataCheckpointIdentity {
    fn validate(self) -> Result<()> {
        if self.cluster.as_bytes() == &[0; 16]
            || self.root.as_bytes() == &[0; 32]
            || self.region.0 < 100
            || self.creation.as_bytes() == &[0; 32]
        {
            return Err(invalid("invalid expected immutable identity"));
        }
        Ok(())
    }
    fn check_range(self, range: &DataRange) -> Result<()> {
        range.validate()?;
        if (range.root, range.region, range.creation) != (self.root, self.region, self.creation) {
            return Err(invalid("image belongs to another root, group or creation"));
        }
        Ok(())
    }
}

/// Enforce the exclusively owned store's local floor before any recovery
/// mutation or object read, including a pure-WAL selection with no uploader.
pub fn check_local_read_floor(
    floors: &kv9_common::data_read_fence::DataReadFences,
    identity: DataCheckpointIdentity,
    manifest: Option<&CheckpointManifest>,
) -> Result<()> {
    identity.validate()?;
    let store = floors.identity();
    if store.cluster_id != identity.cluster || store.root_digest != identity.root {
        return Err(invalid("local read floor store identity differs"));
    }
    let image = manifest
        .map(|m| {
            if m.scope.region != identity.region.0
                || m.scope.cluster != identity.cluster.to_string()
            {
                return Err(invalid("local read floor image identity differs"));
            }
            Ok(kv9_common::data_read_fence::ReadImage {
                region: identity.region,
                digest: RootDigest::sha256(&m.encode()?),
                cut: m.position(),
                conf_ver: m.scope.conf_ver,
                version: m.scope.version,
            })
        })
        .transpose()?;
    floors.check_recovery(identity.region, identity.creation, image.as_ref())
}

/// Local ownership/activation consistency of one image or engine state: a bound
/// range must carry this group's identity, and TTL activation needs live (unsealed)
/// bound ownership.
pub(crate) fn validate_local(
    identity: (
        kv9_common::RootDigest,
        kv9_common::RegionId,
        kv9_common::RootDigest,
    ),
    range: Option<&DataRange>,
    clock: Option<ClockWatermark>,
) -> Result<()> {
    if range.is_some_and(|r| (r.root, r.region, r.creation) != identity) {
        return Err(invalid("stored range has a foreign group identity"));
    }
    if clock.is_some() && range.is_none_or(|r| r.sealed) {
        return Err(invalid("TTL activation without live bound ownership"));
    }
    Ok(())
}

/// A local completed-open observation, never a transferable install/retention
/// capability. The bound state and watermark belong to the restored IMAGE, before
/// later WAL entries; publication comes from that same successful recovery.

#[derive(Debug)]
pub struct RecoveredDataCheckpoint {
    state: DataCheckpointState,
    watermark: Option<ClockWatermark>,
    publication: RecoveredCheckpointPublication,
}
impl RecoveredDataCheckpoint {
    pub fn state(&self) -> &DataCheckpointState {
        &self.state
    }
    pub fn range(&self) -> Option<&DataRange> {
        self.state.bound_range()
    }
    pub fn watermark(&self) -> Option<ClockWatermark> {
        self.watermark
    }
    pub fn publication(&self) -> &RecoveredCheckpointPublication {
        &self.publication
    }
}

pub fn open(
    storage: &mut DiskRaftStorage,
    path: impl AsRef<Path>,
    uploader: Option<&RemoteUploader>,
    identity: DataCheckpointIdentity,
) -> Result<(WalEngine, EngineReplay, Option<RecoveredDataCheckpoint>)> {
    open_inner(
        storage,
        path.as_ref(),
        uploader,
        uploader.map(|u| u as &dyn DataCheckpointReader),
        identity,
        None,
    )
}

pub(crate) fn open_installed(
    storage: &mut DiskRaftStorage,
    path: &Path,
    uploader: &RemoteUploader,
    reader: &dyn DataCheckpointReader,
    identity: DataCheckpointIdentity,
    installed: InstalledBase<'_>,
) -> Result<(WalEngine, EngineReplay, Option<RecoveredDataCheckpoint>)> {
    let base = storage
        .compacted_base()?
        .ok_or_else(|| invalid("adopted protocol base missing"))?;
    let cut = installed.manifest.position();
    if base.index < cut.index || base.term < cut.term || (base.index == cut.index && base != cut) {
        return Err(invalid("protocol base contradicts adopted image"));
    }
    open_inner(
        storage,
        path,
        Some(uploader),
        Some(reader),
        identity,
        Some(installed),
    )
}

fn open_inner(
    storage: &mut DiskRaftStorage,
    path: &Path,
    uploader: Option<&RemoteUploader>,
    reader: Option<&dyn DataCheckpointReader>,
    identity: DataCheckpointIdentity,
    installed: Option<InstalledBase<'_>>,
) -> Result<(WalEngine, EngineReplay, Option<RecoveredDataCheckpoint>)> {
    identity.validate()?;
    let read_plan = super::preflight::inspect_for_read(
        storage,
        path,
        identity.cluster.to_string(),
        identity.region.0,
        installed.as_ref().map(|base| base.manifest),
    )?;
    if let Some(reader) = reader {
        reader.check_selection(identity, read_plan.manifest())?;
    }
    let recovery = RefCell::new(CheckpointRecovery::new(
        storage,
        identity.cluster.to_string(),
        identity.region.0,
    ));
    let mut observed = None;
    let installed_selected = Cell::new(false);
    let tail = RefCell::new(tail::TailState::default());
    let reader = reader.map(|reader| SelectedReader {
        reader,
        plan: &read_plan,
    });
    let (engine, replay) = WalEngine::open_with_checkpoint_reader(
        path,
        reader.as_ref().map(|r| r as &dyn CheckpointRestore),
        |base| {
            read_plan.check_selection(base)?;
            let mut recovery = recovery.borrow_mut();
            if let Some(anchor) = &installed {
                let manifest = base.ok_or_else(|| invalid("adopted engine checkpoint missing"))?;
                if manifest == anchor.manifest {
                    let crate::storage::ConfigurationLookup::Found(configuration) = recovery
                        .storage
                        .configuration_at_committed(manifest.position())?
                    else {
                        return Err(invalid("adopted image configuration history unavailable"));
                    };
                    if configuration.state() != anchor.configuration {
                        return Err(invalid(
                            "adopted image configuration differs from protocol history",
                        ));
                    }
                    installed_selected.set(true);
                    return recovery.checkpoint(None);
                }
                if manifest.index <= anchor.manifest.index || manifest.term < anchor.manifest.term {
                    return Err(invalid("selected checkpoint contradicts adopted image"));
                }
            }
            recovery.checkpoint(base)
        },
        |manifest, view| {
            if observed.is_some() {
                return Err(invalid("base image observed twice"));
            }
            let image = inspect_checkpoint(view, manifest, identity)?;
            let watermark = image.watermark();
            if let Some(anchor) = &installed {
                let range = image.state().bound_range().ok_or_else(|| {
                    invalid("staging image cannot follow an installed bound range")
                })?;
                if installed_selected.get()
                    && (range != anchor.range || watermark != anchor.watermark)
                {
                    return Err(invalid(
                        "selected original image differs from verified adoption state",
                    ));
                }
                if range != anchor.range && (watermark.is_some() || !range.may_follow(anchor.range))
                {
                    return Err(invalid("selected image violates adopted range transition"));
                }
                if anchor
                    .watermark
                    .is_some_and(|prior| watermark.is_none_or(|now| now < prior))
                {
                    return Err(invalid("selected image regresses adopted TTL state"));
                }
            }
            *tail.borrow_mut() = tail::TailState::from_image(
                image.state().bound_range().cloned(),
                watermark,
                identity,
            )?;
            observed = Some((image.into_state(), watermark));
            Ok(())
        },
        |batch, at| {
            let mut recovery = recovery.borrow_mut();
            // Preserve the ordinary group's validation even without a selected
            // checkpoint. A remote image never licenses an unrelated WAL tail.
            match at {
                Some(at) if position_in_history(recovery.storage, at)? => {}
                None if batch.is_empty() => {}
                _ => return Err(invalid("engine WAL lacks matching committed group history")),
            }
            tail.borrow_mut().batch(batch, identity)?;
            recovery.batch(batch, at)
        },
    )?;
    let publication = recovery.into_inner().finish(&engine)?;
    read_plan.check_publication(publication.as_ref())?;
    tail.into_inner().finish(&engine)?;
    let checkpoint = match (observed, publication) {
        (None, None) => None,
        (Some(_), None) if installed_selected.get() => None,
        (Some((state, watermark)), Some(publication)) => Some(RecoveredDataCheckpoint {
            state,
            watermark,
            publication,
        }),
        _ => return Err(invalid("base observation and publication do not match")),
    };
    let journal = uploader.map_or_else(
        || kv9_engine::checkpoint::FlushJournal::new(path.with_extension("pending")),
        |u| u.journal(path.with_extension("pending")),
    );
    if let Some(pending) = journal.load()? {
        if uploader.is_none() {
            return Err(invalid(
                "pending checkpoint requires configured object access",
            ));
        }
        let manifest = pending.manifest();
        if manifest.scope.cluster != identity.cluster.to_string()
            || manifest.scope.region != identity.region.0
        {
            return Err(invalid("pending checkpoint belongs to another group"));
        }
        if !matches!(
            storage.configuration_at_committed(manifest.position())?,
            crate::storage::ConfigurationLookup::Found(_)
        ) {
            return Err(invalid(
                "pending checkpoint cut lacks retained committed configuration",
            ));
        }
        // Rebuilt before binding a driver, resuming compaction or voting. This
        // reserves history only; ownership and remote bytes are checked later.
        engine.restore_checkpoint_candidate(&pending)?;
    }
    Ok((engine, replay, checkpoint))
}

fn position_in_history(storage: &DiskRaftStorage, at: AppliedPosition) -> Result<bool> {
    if at.term == 0 || at.index == 0 {
        return Ok(false);
    }
    if let Some(base) = storage.compacted_base()? {
        if at.index < base.index {
            return Ok(at.term <= base.term);
        }
        if at.index == base.index {
            return Ok(at.term == base.term);
        }
    }
    Ok(storage.committed_term(at.index)? == at.term)
}

fn invalid(message: &str) -> Error {
    Error::Raft(format!("data checkpoint recovery: {message}"))
}

#[cfg(test)]
mod tests;
