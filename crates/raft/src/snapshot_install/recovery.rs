//! Bind adopted engine recovery to the immutable image selected by adoption.
use super::*;
use crate::state_machine::checkpoint_recovery::data_group::{
    self, DataCheckpointReader, DataRecoveryDependencies, InstalledBase,
    InstalledRecoveryDependency, RecoveredDataCheckpoint,
};
use kv9_engine::EngineReplay;

// Keep the validated inputs private: mutating public diagnostic fields on the
// receipt must not mint a different engine path, range or checkpoint exception.
pub(super) struct Anchor {
    identity: DataCheckpointIdentity,
    range: DataRange,
    manifest: CheckpointManifest,
    configuration: raft::prelude::ConfState,
    path: PathBuf,
    watermark: Option<ClockWatermark>,
    dependency: InstalledRecoveryDependency,
}
impl Anchor {
    pub(super) fn new(
        identity: StoreIdentity,
        range: DataRange,
        manifest: CheckpointManifest,
        configuration: raft::prelude::ConfState,
        path: PathBuf,
        watermark: Option<ClockWatermark>,
        original: (StoreIncarnation, RootDigest, bool),
    ) -> Result<Self> {
        let dependency = InstalledRecoveryDependency::verified(
            identity,
            original,
            &manifest,
            &range,
            &configuration,
            watermark,
        )?;
        Ok(Self {
            identity: DataCheckpointIdentity {
                cluster: identity.cluster_id,
                root: range.root,
                region: range.region,
                creation: range.creation,
            },
            range,
            manifest,
            configuration,
            path,
            watermark,
            dependency,
        })
    }
}

impl AdoptedGeneration {
    /// Complete the same validated open and retain its actual recovery inputs.
    /// Uses private adoption facts, never the mutable diagnostic receipt fields.
    /// No partial or failed open returns a dependency observation.
    pub fn open_engine_with_dependencies(
        &self,
        storage: &mut DiskRaftStorage,
        uploader: &RemoteUploader,
    ) -> Result<(WalEngine, EngineReplay, DataRecoveryDependencies)> {
        self.open_engine_with_retained_reader(storage, uploader, uploader)
    }

    pub fn open_engine_with_retained_reader(
        &self,
        storage: &mut DiskRaftStorage,
        uploader: &RemoteUploader,
        reader: &dyn DataCheckpointReader,
    ) -> Result<(WalEngine, EngineReplay, DataRecoveryDependencies)> {
        let (engine, replay, checkpoint) = self.open_engine_read(storage, uploader, reader)?;
        let dependencies = DataRecoveryDependencies::completed(
            storage,
            &engine,
            self.recovery.identity,
            checkpoint.as_ref(),
            Some(self.recovery.dependency),
        )?;
        Ok((engine, replay, dependencies))
    }

    /// Recover the live engine under this adoption's original image authority.
    /// Validates committed WAL positions, range/TTL transitions and subsequent
    /// local checkpoint publication; rebuilds pending checkpoint protections.
    /// A None publication means the original installed image remains selected.
    /// The caller still owns committed migration/compaction authority and must
    /// keep this receipt's group lock alive throughout the runtime's lifetime.
    pub fn open_engine(
        &self,
        storage: &mut DiskRaftStorage,
        uploader: &RemoteUploader,
    ) -> Result<(WalEngine, EngineReplay, Option<RecoveredDataCheckpoint>)> {
        self.open_engine_read(storage, uploader, uploader)
    }

    fn open_engine_read(
        &self,
        storage: &mut DiskRaftStorage,
        uploader: &RemoteUploader,
        reader: &dyn DataCheckpointReader,
    ) -> Result<(WalEngine, EngineReplay, Option<RecoveredDataCheckpoint>)> {
        let anchor = &self.recovery;
        let selected = WalEngine::checkpoint_reference(&anchor.path)?;
        data_group::check_local_read_floor(
            &self.data_read_fences,
            anchor.identity,
            selected.as_ref(),
        )?;
        data_group::open_installed(
            storage,
            &anchor.path,
            uploader,
            reader,
            anchor.identity,
            InstalledBase {
                manifest: &anchor.manifest,
                range: &anchor.range,
                configuration: &anchor.configuration,
                watermark: anchor.watermark,
            },
        )
    }
}
