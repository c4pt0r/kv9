//! Committed retention owners for one described migration image.
//!
//! The source owner pins one exact manifest closure for transfer; the
//! destination owner joins the identical subject and closure through the
//! ledger's Share rule, keyed to the committed destination incarnation.
//! Nothing here uploads, reads or deletes an object, admits a receiver,
//! starts a peer or releases a pin: QuiesceAfterTransfer and Release have no
//! path in this increment, and physical deletion stays disabled ledger-wide.

use std::num::NonZeroU64;

use kv9_common::retention::{OwnerId, PinPhase};
use kv9_common::{Error, Result, RootDescriptor, RootDigest};
use kv9_engine::checkpoint::CheckpointManifest;
use kv9_meta::data_groups::migration::CommittedMigration;
use kv9_meta::retention::{
    decode_owner_observation, encode_request, LedgerRequest, OwnerBinding, OwnerDescriptor,
    OwnerKind,
};

use crate::api::AdminApi;
use crate::checkpoint_retention::manifest_resources;

fn invalid(message: &str) -> Error {
    Error::Config(format!("migration retention: {message}"))
}

pub(crate) struct MigrationOwners {
    root: RootDigest,
    source: OwnerBinding,
    destination: OwnerBinding,
}

impl MigrationOwners {
    /// Verify, from one LOCAL applied ledger view, that both owners are
    /// committed and Published for exactly this image. Local visibility
    /// implies commitment; absence only means "not yet locally applied",
    /// which refuses in the safe direction. No retention mutation happens.
    pub(crate) fn verify_published_locally(
        &self,
        view: &dyn kv9_engine::ReadView,
        root: &RootDescriptor,
    ) -> Result<()> {
        for binding in [&self.source, &self.destination] {
            let owner = kv9_meta::retention::retention_owner(view, root, binding.descriptor.id)?
                .ok_or_else(|| {
                    invalid("image owners are not committed; bind the planned manifest first")
                })?;
            if owner.binding != *binding {
                return Err(invalid(
                    "committed owner binds a different image than this cut",
                ));
            }
            if owner.phase != PinPhase::Published {
                return Err(invalid("image owner is not published"));
            }
        }
        Ok(())
    }

    pub(crate) fn source_id(&self) -> OwnerId {
        self.source.descriptor.id
    }
    pub(crate) fn destination_id(&self) -> OwnerId {
        self.destination.descriptor.id
    }

    /// Derive both owners from the committed migration and one canonical
    /// manifest description. The manifest is data: every binding to committed
    /// authority is re-checked here, and the ledger's immutable-binding rule
    /// prevents the same operation from ever naming a second image.
    pub(crate) fn new(
        root: &RootDescriptor,
        migration: &CommittedMigration,
        range: &kv9_common::data_range::DataRange,
        manifest: &CheckpointManifest,
    ) -> Result<Self> {
        let manifest = CheckpointManifest::decode(&manifest.encode()?)?;
        let intent = migration.intent();
        if intent.root() != root.digest() {
            return Err(invalid("migration root differs from the certified root"));
        }
        range.validate()?;
        if range.region != intent.region() || range.root != intent.root() {
            return Err(invalid("published range differs from the migration group"));
        }
        if manifest.scope.cluster != root.cluster_id.to_string()
            || manifest.scope.region != intent.region().0
            || manifest.scope.conf_ver != range.conf_ver
            || manifest.scope.version != range.version
        {
            return Err(invalid("manifest scope differs from the committed range"));
        }
        let root = root.digest();
        let mut operation = b"kv9-migration-operation-v1".to_vec();
        operation.extend(root.as_bytes());
        operation.extend(intent.operation());
        let operation = *RootDigest::sha256(&operation).as_bytes();
        let subject = *RootDigest::sha256(&manifest.encode()?).as_bytes();
        let resources = manifest_resources(root, &manifest)?;
        let binding = |kind: OwnerKind, label: &[u8], salt: &[u8]| -> Result<OwnerBinding> {
            let mut identity = label.to_vec();
            identity.extend(root.as_bytes());
            identity.extend(operation);
            identity.extend(salt);
            let id = OwnerId::new(
                RootDigest::sha256(&identity).as_bytes()[..16]
                    .try_into()
                    .unwrap(),
            )
            .map_err(|e| Error::Engine(e.to_string()))?;
            Ok(OwnerBinding {
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
            })
        };
        let source = binding(OwnerKind::Snapshot, b"kv9-migration-source-v1", &[])?;
        let destination = binding(
            OwnerKind::Migration,
            b"kv9-migration-destination-v1",
            intent.destination().incarnation.as_bytes(),
        )?;
        // Run the exact production semantic/closure bounds before any I/O.
        encode_request(root, &LedgerRequest::Acquire(source.clone()))?;
        encode_request(
            root,
            &LedgerRequest::Share {
                from: source.token(),
                to: destination.clone(),
            },
        )?;
        Ok(Self {
            root,
            source,
            destination,
        })
    }

    fn apply(&self, api: &dyn AdminApi, request: LedgerRequest) -> Result<()> {
        api.apply_retention("migration-authority", encode_request(self.root, &request)?)?;
        Ok(())
    }

    fn phase(&self, api: &dyn AdminApi, binding: &OwnerBinding) -> Result<Option<PinPhase>> {
        let Some(bytes) =
            api.get_retention_owner("migration-authority", self.root, binding.descriptor.id)?
        else {
            return Ok(None);
        };
        let owner = decode_owner_observation(&bytes, self.root, binding.descriptor.id)?;
        if owner.binding != *binding {
            return Err(invalid("committed owner binding differs from this image"));
        }
        Ok(Some(owner.phase))
    }

    /// Idempotently drive both owners to Published through committed ledger
    /// requests only. Every step re-reads applied state; an interrupted bind
    /// resumes its exact prefix. A quiesced or released owner is refused
    /// outright: no transfer settlement exists in this increment, so those
    /// phases can only mean foreign interference with the operation.
    pub(crate) fn bind(&self, api: &dyn AdminApi) -> Result<()> {
        self.apply(api, LedgerRequest::Initialize)?;
        self.apply(api, LedgerRequest::Register(self.source.resources.clone()))?;
        match self.phase(api, &self.source)? {
            None => {
                self.apply(api, LedgerRequest::Acquire(self.source.clone()))?;
                self.apply(api, LedgerRequest::Publish(self.source.token()))?;
            }
            Some(PinPhase::Held) => self.apply(api, LedgerRequest::Publish(self.source.token()))?,
            Some(PinPhase::Published) => {}
            Some(PinPhase::Quiesced | PinPhase::Released) => {
                return Err(invalid("source image pin was quiesced or released"));
            }
        }
        match self.phase(api, &self.destination)? {
            None => {
                self.apply(
                    api,
                    LedgerRequest::Share {
                        from: self.source.token(),
                        to: self.destination.clone(),
                    },
                )?;
                self.apply(api, LedgerRequest::Publish(self.destination.token()))?;
            }
            Some(PinPhase::Held) => {
                self.apply(api, LedgerRequest::Publish(self.destination.token()))?
            }
            Some(PinPhase::Published) => {}
            Some(PinPhase::Quiesced | PinPhase::Released) => {
                return Err(invalid("destination image pin was quiesced or released"));
            }
        }
        Ok(())
    }
}
