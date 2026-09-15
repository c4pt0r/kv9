//! Automatic ownership for the current whole-engine checkpoint worker.
//! These records do not certify legacy backfill, receiver admission or GC.
use std::num::NonZeroU64;

use kv9_common::retention::{OwnerId, PinPhase, ResourceIdentity, ResourceKind};
use kv9_common::{Error, Result, RootDescriptor, RootDigest};
use kv9_engine::checkpoint::CheckpointManifest;
use kv9_meta::retention::{
    decode_owner_observation, encode_request, LedgerRequest, OwnerBinding, OwnerDescriptor,
    OwnerKind,
};

use crate::api::AdminApi;

pub(crate) struct CheckpointOwners {
    root: RootDigest,
    pending: OwnerBinding,
    version: OwnerBinding,
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
        let manifest = CheckpointManifest::decode(&manifest.encode()?)?;
        if manifest.scope.cluster != root.cluster_id.to_string()
            || manifest.scope.region != kv9_common::META_REGION_0.0
        {
            return Err(Error::Engine("checkpoint owner root/scope differs".into()));
        }
        let root = root.digest();
        let operation: [u8; 32] = manifest.change_id(generation)?.try_into().unwrap();
        let subject = *RootDigest::sha256(&manifest.encode()?).as_bytes();
        let mut resources = Vec::with_capacity(manifest.files.len());
        for file in &manifest.files {
            let mut content = [0; 32];
            for (n, pair) in file.sha256.as_bytes().chunks_exact(2).enumerate() {
                // Canonical lowercase hexadecimal was validated by the codec.
                let nibble = |c: u8| {
                    if c.is_ascii_digit() {
                        c - b'0'
                    } else {
                        c - b'a' + 10
                    }
                };
                content[n] = nibble(pair[0]) * 16 + nibble(pair[1]);
            }
            let instance = RootDigest::sha256(file.key.as_bytes()).as_bytes()[..16]
                .try_into()
                .unwrap();
            resources.push(
                ResourceIdentity::new(root, ResourceKind::Sst, instance, content)
                    .map_err(|e| Error::Engine(e.to_string()))?,
            );
        }
        resources.sort_by_key(|r| *r.instance());
        let binding = |kind: OwnerKind, label: &[u8]| -> Result<OwnerBinding> {
            let mut identity = label.to_vec();
            identity.extend(root.as_bytes());
            identity.extend(operation);
            let id = OwnerId::new(
                RootDigest::sha256(&identity).as_bytes()[..16]
                    .try_into()
                    .unwrap(),
            )
            .map_err(|e| Error::Engine(e.to_string()))?;
            let binding = OwnerBinding {
                descriptor: OwnerDescriptor {
                    root,
                    id,
                    kind,
                    region: manifest.scope.region,
                    conf_ver: manifest.scope.conf_ver,
                    version: manifest.scope.version,
                    operation,
                    subject,
                    subject_is_anchor: false,
                    local: None,
                },
                generation: NonZeroU64::new(1).unwrap(),
                resources: resources.clone(),
            };
            // Run the exact production semantic/closure bounds before any I/O.
            encode_request(root, &LedgerRequest::Acquire(binding.clone()))?;
            Ok(binding)
        };
        Ok(Self {
            root,
            pending: binding(OwnerKind::Pending, b"kv9-checkpoint-pending-v1")?,
            version: binding(OwnerKind::Version, b"kv9-checkpoint-version-v1")?,
        })
    }
    fn apply(&self, api: &dyn AdminApi, request: LedgerRequest) -> Result<()> {
        api.apply_retention("checkpoint-worker", encode_request(self.root, &request)?)?;
        Ok(())
    }
    fn phase(&self, api: &dyn AdminApi, binding: &OwnerBinding) -> Result<Option<PinPhase>> {
        let Some(bytes) =
            api.get_retention_owner("checkpoint-worker", self.root, binding.descriptor.id)?
        else {
            return Ok(None);
        };
        let owner = decode_owner_observation(&bytes, self.root, binding.descriptor.id)?;
        if owner.binding != *binding {
            return Err(Error::Engine("checkpoint retention binding differs".into()));
        }
        Ok(Some(owner.phase))
    }
    fn published_version(&self, api: &dyn AdminApi) -> Result<()> {
        if self.phase(api, &self.version)? != Some(PinPhase::Published) {
            return Err(Error::Engine(
                "checkpoint handoff lacks exact published successor".into(),
            ));
        }
        Ok(())
    }
    /// The local plan is already durable. Only exact applied ledger receipts
    /// permit external PUT/GET. A partially finished transfer never reacquires a
    /// released generation: its immutable published successor supplies coverage.
    pub(crate) fn before_io(&self, api: &dyn AdminApi) -> Result<()> {
        self.apply(api, LedgerRequest::Initialize)?;
        self.apply(api, LedgerRequest::Register(self.pending.resources.clone()))?;
        match self.phase(api, &self.pending)? {
            None => {
                self.apply(api, LedgerRequest::Acquire(self.pending.clone()))?;
                self.apply(api, LedgerRequest::Publish(self.pending.token()))
            }
            Some(PinPhase::Held) => self.apply(api, LedgerRequest::Publish(self.pending.token())),
            Some(PinPhase::Published) => Ok(()),
            Some(PinPhase::Quiesced | PinPhase::Released) => self.published_version(api),
        }
    }
    /// Called only after the existing seam supplies positive typed settlement.
    /// Errors retain the journal; restart repeats this exact transfer prefix.
    pub(crate) fn after_positive_settlement(&self, api: &dyn AdminApi) -> Result<()> {
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
