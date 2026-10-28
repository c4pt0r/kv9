//! Bounded observations of the dependencies used by one successful local open.
//! These hashes identify descriptors; they neither contain a portable recovery
//! anchor nor authorize releasing objects. In particular, later live selection
//! does not update the completed-open observation.
use super::*;
use kv9_common::{StoreIdentity, StoreIncarnation};
use kv9_engine::{DurableAppliedPosition, ReplicatedEngine};
use protobuf::Message;

/// Identity of an actual restored image. No SST list or range/import payload is
/// retained here, so keeping one observation per runtime group has fixed cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryImage {
    manifest: RootDigest,
    cut: AppliedPosition,
    state: RecoveryImageState,
    watermark: Option<ClockWatermark>,
    configuration: RootDigest,
    configuration_at: Option<AppliedPosition>,
    publication: Option<RecoveryPublication>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryImageState {
    /// Hash of the exact DataRange encoding restored from the image.
    Bound(RootDigest),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryPublication {
    at: AppliedPosition,
    generation: u64,
    change_id: [u8; 32],
}
impl RecoveryPublication {
    pub fn at(&self) -> AppliedPosition {
        self.at
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn change_id(&self) -> &[u8; 32] {
        &self.change_id
    }
}
impl RecoveryImage {
    pub fn manifest(&self) -> RootDigest {
        self.manifest
    }
    pub fn cut(&self) -> AppliedPosition {
        self.cut
    }
    pub fn state(&self) -> RecoveryImageState {
        self.state
    }
    pub fn watermark(&self) -> Option<ClockWatermark> {
        self.watermark
    }
    /// Hash of the recovered configuration's protobuf encoding, including all
    /// membership sets. This is descriptor identity, not configuration authority.
    pub fn configuration(&self) -> RootDigest {
        self.configuration
    }
    pub fn configuration_at(&self) -> Option<AppliedPosition> {
        self.configuration_at
    }
    pub fn publication(&self) -> Option<RecoveryPublication> {
        self.publication
    }
}

/// The original installation's local records remain necessary even when its
/// remote SSTs were not read by this open. Shared objects and other readers may
/// still depend on those SSTs; absence here is never global deletion authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstalledRecoveryDependency {
    store: StoreIdentity,
    generation: StoreIncarnation,
    image_record: RootDigest,
    image: RecoveryImage,
    verified_original_during_adoption: bool,
}
impl InstalledRecoveryDependency {
    pub fn store(&self) -> StoreIdentity {
        self.store
    }
    pub fn generation(&self) -> StoreIncarnation {
        self.generation
    }
    pub fn image_record(&self) -> RootDigest {
        self.image_record
    }
    pub fn image(&self) -> &RecoveryImage {
        &self.image
    }
    /// First adoption and V1-to-V2 upgrade read the original image BEFORE
    /// engine open. A later selected checkpoint does not erase that reader.
    pub fn verified_original_during_adoption(&self) -> bool {
        self.verified_original_during_adoption
    }

    pub(crate) fn verified(
        store: StoreIdentity,
        original: (StoreIncarnation, RootDigest, bool),
        manifest: &CheckpointManifest,
        range: &DataRange,
        configuration: &raft::prelude::ConfState,
        watermark: Option<ClockWatermark>,
    ) -> Result<Self> {
        Ok(Self {
            store,
            generation: original.0,
            image_record: original.1,
            verified_original_during_adoption: original.2,
            image: RecoveryImage {
                manifest: RootDigest::sha256(&manifest.encode()?),
                cut: manifest.position(),
                state: RecoveryImageState::Bound(RootDigest::sha256(&range.encode())),
                watermark,
                configuration: configuration_digest(configuration)?,
                configuration_at: None, // installed base, not a fabricated change position
                publication: None,      // installation has no local ManifestChange publication
            },
        })
    }
}

/// Minted after the complete engine, committed publication, WAL tail and
/// pending-history restoration succeed. Not constructible from decoded data.
///
/// ```compile_fail
/// use kv9_raft::state_machine::checkpoint_recovery::data_group::DataRecoveryDependencies;
/// let forged = DataRecoveryDependencies { selected: None };
/// ```
#[derive(Debug, Clone, Copy)]
pub struct DataRecoveryDependencies {
    identity: DataCheckpointIdentity,
    through: DurableAppliedPosition,
    protocol_base: Option<AppliedPosition>,
    selected: Option<RecoveryImage>,
    installed: Option<InstalledRecoveryDependency>,
}
impl DataRecoveryDependencies {
    pub fn identity(&self) -> DataCheckpointIdentity {
        self.identity
    }
    pub fn recovered_through(&self) -> DurableAppliedPosition {
        self.through
    }
    pub fn protocol_base(&self) -> Option<AppliedPosition> {
        self.protocol_base
    }
    /// None means this successful open recovered from local WAL without a
    /// remote base. It does not mean an unavailable/failed open has no dependencies.
    pub fn selected(&self) -> Option<&RecoveryImage> {
        self.selected.as_ref()
    }
    pub fn installed(&self) -> Option<&InstalledRecoveryDependency> {
        self.installed.as_ref()
    }
    /// Only describes which image engine open fetched. Adoption itself may have
    /// read the original image too; see verified_original_during_adoption.
    /// Original local adoption
    /// records remain required, and no claim is made about other owners.
    pub fn used_original_install_image(&self) -> bool {
        self.installed
            .as_ref()
            .is_some_and(|i| self.selected == Some(i.image))
    }

    pub(crate) fn completed(
        storage: &DiskRaftStorage,
        engine: &WalEngine,
        identity: DataCheckpointIdentity,
        checkpoint: Option<&RecoveredDataCheckpoint>,
        installed: Option<InstalledRecoveryDependency>,
    ) -> Result<Self> {
        let selected = if let Some(checkpoint) = checkpoint {
            let p = checkpoint.publication();
            let generation = p.generation();
            let predecessor = generation
                .checked_sub(1)
                .ok_or_else(|| invalid("recovered publication generation is zero"))?;
            Some(RecoveryImage {
                manifest: RootDigest::sha256(&p.manifest().encode()?),
                cut: p.manifest().position(),
                state: match checkpoint.state() {
                    DataCheckpointState::Bound(range) => {
                        RecoveryImageState::Bound(RootDigest::sha256(&range.encode()))
                    }
                },
                watermark: checkpoint.watermark(),
                configuration: configuration_digest(p.configuration().state())?,
                configuration_at: p.configuration().applied_at(),
                publication: Some(RecoveryPublication {
                    at: p.publication(),
                    generation,
                    change_id: p
                        .manifest()
                        .change_id(predecessor)?
                        .try_into()
                        .map_err(|_| invalid("invalid recovered change ID"))?,
                }),
            })
        } else {
            installed.as_ref().map(|i| i.image)
        };
        // An observation belongs to this same completed open, including the
        // installed-original exception. Never infer selection from absence of
        // an ordinary publication alone.
        let actual = engine.selected_checkpoint()?;
        if actual
            .as_ref()
            .map(|m| m.encode().map(|b| RootDigest::sha256(&b)))
            .transpose()?
            != selected.map(|s| s.manifest)
        {
            return Err(invalid("completed-open selection differs"));
        }
        let through = engine.applied_position()?;
        if through == DurableAppliedPosition::Volatile {
            return Err(invalid("completed open has a volatile applied position"));
        }
        Ok(Self {
            identity,
            through,
            protocol_base: storage.compacted_base()?,
            selected,
            installed,
        })
    }
}

fn configuration_digest(configuration: &raft::prelude::ConfState) -> Result<RootDigest> {
    Ok(RootDigest::sha256(
        &configuration
            .write_to_bytes()
            .map_err(|e| invalid(&format!("configuration encoding: {e}")))?,
    ))
}

/// Like `open`, retaining a fixed-size completed-open dependency observation.
/// The caller must keep the actual engine and exclusive store/group locks;
/// copying this observation does not transfer that ownership.
pub fn open_with_dependencies(
    storage: &mut DiskRaftStorage,
    path: impl AsRef<Path>,
    uploader: Option<&RemoteUploader>,
    identity: DataCheckpointIdentity,
) -> Result<(WalEngine, EngineReplay, DataRecoveryDependencies)> {
    let (engine, replay, checkpoint) = open(storage, path, uploader, identity)?;
    let dependencies =
        DataRecoveryDependencies::completed(storage, &engine, identity, checkpoint.as_ref(), None)?;
    Ok((engine, replay, dependencies))
}

/// Same complete recovery with an explicitly retained GET dependency. The
/// uploader is used only for the local pending-journal budget in this path.
pub fn open_with_retained_reader(
    storage: &mut DiskRaftStorage,
    path: impl AsRef<Path>,
    uploader: Option<&RemoteUploader>,
    reader: Option<&dyn DataCheckpointReader>,
    identity: DataCheckpointIdentity,
) -> Result<(WalEngine, EngineReplay, DataRecoveryDependencies)> {
    let (engine, replay, checkpoint) =
        open_inner(storage, path.as_ref(), uploader, reader, identity, None)?;
    let dependencies =
        DataRecoveryDependencies::completed(storage, &engine, identity, checkpoint.as_ref(), None)?;
    Ok((engine, replay, dependencies))
}
