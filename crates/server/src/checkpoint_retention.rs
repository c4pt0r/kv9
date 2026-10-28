//! Automatic ownership for metadata and data-group whole-engine checkpoints.
//! These records do not certify legacy backfill, receiver admission or GC.
use std::num::NonZeroU64;

use kv9_common::retention::{OwnerId, PinPhase, ResourceIdentity};
use kv9_common::{Error, Result, RootDescriptor, RootDigest};
use kv9_engine::checkpoint::CheckpointManifest;
use kv9_meta::retention::{
    decode_owner_observation, encode_request, LedgerRequest, OwnerBinding, OwnerKind,
};

use crate::api::AdminApi;

use kv9_meta::retention::checkpoint_intent::CheckpointIntent;

pub(crate) mod drain_outbox;
mod execution;
mod reader;
mod retirement;
pub(crate) use execution::CheckpointExecution;
pub(crate) use reader::CheckpointRead;

/// Narrow checkpoint dependency, implemented by local admin fixtures and the
/// production metadata-leader router. Each call requires an exact receipt.
pub(crate) trait CheckpointLedger: Send + Sync {
    /// Real runtime and offline readers supply the exclusively owned store's
    /// shared provider. Only unit-test ledger harnesses may omit one.
    fn data_read_fences(
        &self,
    ) -> Result<Option<std::sync::Arc<kv9_common::data_read_fence::DataReadFences>>> {
        #[cfg(test)]
        {
            Ok(None)
        }
        #[cfg(not(test))]
        {
            Err(Error::Engine("DATA read floor provider unavailable".into()))
        }
    }
    fn check_data_read(
        &self,
        root: RootDigest,
        image: Option<&kv9_common::data_read_fence::ReadImage>,
    ) -> Result<()> {
        let Some(image) = image else { return Ok(()) };
        if let Some(fences) = self.data_read_fences()? {
            if fences.identity().root_digest != root {
                return Err(Error::Config("DATA read floor root differs".into()));
            }
            fences.check_read(image)?;
        }
        Ok(())
    }
    fn check_data_recovery(
        &self,
        identity: kv9_raft::state_machine::checkpoint_recovery::data_group::DataCheckpointIdentity,
        manifest: Option<&CheckpointManifest>,
    ) -> Result<()> {
        if let Some(fences) = self.data_read_fences()? {
            kv9_raft::state_machine::checkpoint_recovery::data_group::check_local_read_floor(
                &fences, identity, manifest,
            )?;
        }
        Ok(())
    }
    fn apply(&self, request: Vec<u8>) -> Result<()>;
    fn owner(&self, root: RootDigest, owner: OwnerId) -> Result<Option<Vec<u8>>>;
    fn drain_outbox(&self) -> Result<Option<std::sync::Arc<drain_outbox::DrainOutbox>>> {
        Ok(None)
    }
    fn executors(
        &self,
        _root: RootDigest,
        _pending: OwnerId,
        _after: Option<OwnerId>,
        _limit: usize,
    ) -> Result<kv9_meta::retention::checkpoint_admission::ExecutorPage> {
        Err(Error::Engine(
            "checkpoint executor query unavailable".into(),
        ))
    }
    fn local_lifetime(&self) -> Result<kv9_meta::retention::LocalLifetime> {
        Err(Error::Engine(
            "checkpoint executor requires an exact runtime lifetime".into(),
        ))
    }
}
impl<T: AdminApi + Send + Sync + ?Sized> CheckpointLedger for T {
    fn apply(&self, request: Vec<u8>) -> Result<()> {
        self.apply_retention("checkpoint-worker", request)
            .map(|_| ())
    }
    fn owner(&self, root: RootDigest, owner: OwnerId) -> Result<Option<Vec<u8>>> {
        self.get_retention_owner("checkpoint-worker", root, owner)
    }
}

/// Small identity carried by a Reader; never retain a second manifest copy.
pub(crate) fn data_read_image(
    manifest: &CheckpointManifest,
) -> Result<Option<kv9_common::data_read_fence::ReadImage>> {
    if manifest.scope.region < 100 {
        return Ok(None);
    }
    Ok(Some(kv9_common::data_read_fence::ReadImage {
        region: kv9_common::RegionId(manifest.scope.region),
        digest: RootDigest::sha256(&manifest.encode()?),
        cut: manifest.position(),
        conf_ver: manifest.scope.conf_ver,
        version: manifest.scope.version,
    }))
}

/// The exact sorted SST closure of one canonical manifest, as ledger
/// resources. Derivation is deterministic; it grants nothing by itself.
pub(crate) fn manifest_resources(
    root: RootDigest,
    manifest: &CheckpointManifest,
) -> Result<Vec<ResourceIdentity>> {
    kv9_meta::retention::checkpoint_intent::manifest_resources(root, manifest)
}

pub(crate) struct CheckpointOwners {
    root: RootDigest,
    pending: OwnerBinding,
    version: OwnerBinding,
    intent: CheckpointIntent,
}
impl CheckpointOwners {
    pub(crate) fn pending_id(&self) -> OwnerId {
        self.pending.descriptor.id
    }
    pub(crate) fn version_id(&self) -> OwnerId {
        self.version.descriptor.id
    }
    pub(crate) fn new(
        root: &RootDescriptor,
        manifest: &CheckpointManifest,
        generation: u64,
    ) -> Result<Self> {
        let intent = CheckpointIntent::new(root, manifest, generation)?;
        Ok(Self {
            root: root.digest(),
            pending: intent.binding(OwnerKind::Pending)?,
            version: intent.binding(OwnerKind::Version)?,
            intent,
        })
    }

    fn apply(&self, api: &(impl CheckpointLedger + ?Sized), request: LedgerRequest) -> Result<()> {
        api.apply(encode_request(self.root, &request)?)?;
        Ok(())
    }
    fn phase(
        &self,
        api: &(impl CheckpointLedger + ?Sized),
        binding: &OwnerBinding,
    ) -> Result<Option<PinPhase>> {
        let Some(bytes) = api.owner(self.root, binding.descriptor.id)? else {
            return Ok(None);
        };
        let owner = decode_owner_observation(&bytes, self.root, binding.descriptor.id)?;
        if owner.binding != *binding {
            return Err(Error::Engine("checkpoint retention binding differs".into()));
        }
        Ok(Some(owner.phase))
    }
    fn published_version(&self, api: &(impl CheckpointLedger + ?Sized)) -> Result<()> {
        if self.phase(api, &self.version)? != Some(PinPhase::Published) {
            return Err(Error::Engine(
                "checkpoint handoff lacks exact published successor".into(),
            ));
        }
        Ok(())
    }
    /// Called after authoritative settlement, before discarding the journal.
    /// The gate stops new entry; admitted work and global coverage remain.
    pub(crate) fn close_admission(&self, api: &(impl CheckpointLedger + ?Sized)) -> Result<()> {
        self.apply(
            api,
            LedgerRequest::CloseCheckpointAdmission(self.pending.token()),
        )
    }
    /// The local plan is already durable. Only exact applied ledger receipts
    /// permit external PUT/GET. A partially finished transfer never reacquires a
    /// released generation: its immutable published successor supplies coverage.
    pub(crate) fn before_io(&self, api: &(impl CheckpointLedger + ?Sized)) -> Result<()> {
        self.apply(api, LedgerRequest::BeginCheckpoint(self.intent.clone()))
    }

    /// Called only after the existing seam supplies positive typed settlement.
    /// Errors retain the journal; restart repeats this exact transfer prefix.
    pub(crate) fn after_positive_settlement(
        &self,
        api: &(impl CheckpointLedger + ?Sized),
    ) -> Result<()> {
        self.apply(api, LedgerRequest::BeginCheckpoint(self.intent.clone()))?;
        // Successful publication also ends this attempt's upload admission.
        // Close before handoff or any Released fast path so retrying a partial
        // transfer cannot leave the old attempt open indefinitely.
        self.close_admission(api)?;
        match self.phase(api, &self.pending)? {
            Some(PinPhase::Released) => return self.published_version(api),
            Some(PinPhase::Quiesced) => self.published_version(api)?,
            Some(PinPhase::Published) => {
                self.apply(
                    api,
                    LedgerRequest::Share {
                        from: self.pending.token(),
                        to: self.version.clone(),
                    },
                )?;
                self.apply(api, LedgerRequest::Publish(self.version.token()))?;
                self.apply(
                    api,
                    LedgerRequest::QuiesceAfterTransfer {
                        from: self.pending.token(),
                        to: self.version.token(),
                    },
                )?;
            }
            _ => {
                return Err(Error::Engine(
                    "checkpoint settlement lacks acquired pending owner".into(),
                ))
            }
        }
        self.apply(api, LedgerRequest::Release(self.pending.token()))
    }
}
