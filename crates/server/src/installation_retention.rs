//! Offline installation reads under explicit, exact image owners. The stopped
//! store lock covers outbox recovery, object-job drain and installer publication.
use crate::checkpoint_retention::{drain_outbox::DrainOutbox, CheckpointLedger, CheckpointRead};
use crate::retention::RetentionClient;
use kv9_common::retention::{OwnerId, PinPhase};
use kv9_common::store_lifecycle::StoreGuard;
use kv9_common::{Error, ProcessIncarnation, Result, RootDescriptor, RootDigest, StoreIdentity};
use kv9_engine::checkpoint::{CheckpointManifest, CheckpointRestore, RemoteUploader};
use kv9_engine::VersionedMemEngine;
use kv9_meta::retention::{
    decode_owner_observation, encode_owner_observation, LocalLifetime, OwnerBinding, OwnerKind,
};
use std::sync::{Arc, Mutex};

pub const MAX_INSTALL_READ_SOURCES: usize = 16;

fn invalid(message: &str) -> Error {
    Error::Engine(format!("retained installation: {message}"))
}

pub(crate) struct RetainedImages<'a, L: CheckpointLedger + ?Sized> {
    uploader: &'a RemoteUploader,
    ledger: &'a L,
    root: &'a RootDescriptor,
    sources: Vec<OwnerBinding>,
}
impl<'a, L: CheckpointLedger + ?Sized> RetainedImages<'a, L> {
    pub(crate) fn new(
        uploader: &'a RemoteUploader,
        ledger: &'a L,
        root: &'a RootDescriptor,
        owners: &[OwnerId],
    ) -> Result<Self> {
        if owners.is_empty() || owners.len() > MAX_INSTALL_READ_SOURCES {
            return Err(invalid("image owner count is outside the bound"));
        }
        let mut sources = Vec::with_capacity(owners.len());
        for id in owners {
            if sources
                .iter()
                .any(|s: &OwnerBinding| s.descriptor.id == *id)
            {
                return Err(invalid("duplicate image owner"));
            }
            let bytes = ledger
                .owner(root.digest(), *id)?
                .ok_or_else(|| invalid("image owner absent"))?;
            let owner = decode_owner_observation(&bytes, root.digest(), *id)?;
            // Validate the same source restrictions as the eventual Finish.
            kv9_meta::retention::image_reader::binding(&owner.binding, ledger.local_lifetime()?)?;
            if owner.phase != PinPhase::Published {
                return Err(invalid("image owner is not Published"));
            }
            sources.push(owner.binding);
        }
        Ok(Self {
            uploader,
            ledger,
            root,
            sources,
        })
    }
}
impl<L: CheckpointLedger + ?Sized> CheckpointRestore for RetainedImages<'_, L> {
    fn restore(&self, manifest: &CheckpointManifest) -> Result<VersionedMemEngine> {
        let resources =
            crate::checkpoint_retention::manifest_resources(self.root.digest(), manifest)?;
        let subject = *RootDigest::sha256(&manifest.encode()?).as_bytes();
        let source = self
            .sources
            .iter()
            .find(|source| {
                let d = &source.descriptor;
                manifest.scope.cluster == self.root.cluster_id.to_string()
                    && d.region == manifest.scope.region
                    && d.conf_ver == manifest.scope.conf_ver
                    && d.version == manifest.scope.version
                    && d.subject == subject
                    && source.resources == resources
                    && matches!(d.kind, OwnerKind::Snapshot | OwnerKind::Migration)
            })
            .ok_or_else(|| invalid("no exact owner covers the requested image"))?;
        let mut reader = CheckpointRead::for_image(source.clone(), manifest, self.ledger)?;
        let (result, report) = self.uploader.with_local_read_report(|scoped| {
            reader.before_io(self.ledger, scoped)?;
            scoped.restore(manifest)
        })?;
        reader.observe_drain(report)?;
        reader.finish(self.ledger)?;
        result
    }
}

struct OfflineLedger {
    client: Mutex<RetentionClient>,
    root: RootDigest,
    lifetime: LocalLifetime,
    outbox: Arc<DrainOutbox>,
    data_read_fences: Arc<kv9_common::data_read_fence::DataReadFences>,
}
impl CheckpointLedger for OfflineLedger {
    fn data_read_fences(&self) -> Result<Option<Arc<kv9_common::data_read_fence::DataReadFences>>> {
        Ok(Some(self.data_read_fences.clone()))
    }

    fn apply(&self, request: Vec<u8>) -> Result<()> {
        self.client
            .lock()
            .expect("offline ledger")
            .apply_encoded(self.root, request)
            .map(|_| ())
            .map_err(|e| Error::MetaNotReady(e.to_string()))
    }
    fn owner(&self, root: RootDigest, owner: OwnerId) -> Result<Option<Vec<u8>>> {
        if root != self.root {
            return Err(invalid("ledger root differs"));
        }
        self.client
            .lock()
            .expect("offline ledger")
            .get(root, owner)
            .map_err(|e| Error::MetaNotReady(e.to_string()))?
            .as_ref()
            .map(encode_owner_observation)
            .transpose()
    }
    fn local_lifetime(&self) -> Result<LocalLifetime> {
        Ok(self.lifetime)
    }
    fn drain_outbox(&self) -> Result<Option<Arc<DrainOutbox>>> {
        Ok(Some(self.outbox.clone()))
    }
}

/// The CLI's complete retained install path. An uncertain report is kept for
/// exact replay by another offline invocation or the node's normal sweep.
/// Owners cover both the old selected image and the new image on replacement.
pub fn install_retained_migration_image(
    guard: &StoreGuard,
    root: &RootDescriptor,
    identity: StoreIdentity,
    record: &[u8],
    uploader: &RemoteUploader,
    client: RetentionClient,
    owners: &[OwnerId],
) -> Result<kv9_raft::snapshot_install::InstalledImage> {
    identity.verify(root, identity.node_id)?;
    guard.verify(&identity)?;
    let outbox = DrainOutbox::open(guard.directory(), identity)?;
    let ledger = OfflineLedger {
        client: Mutex::new(client),
        root: root.digest(),
        outbox: outbox.clone(),
        data_read_fences: guard.data_read_fences(&identity)?,
        lifetime: LocalLifetime {
            node: identity.node_id.0,
            store: *identity.store_incarnation.as_bytes(),
            process: *ProcessIncarnation::mint()?.as_bytes(),
        },
    };
    // Store ownership excludes a concurrent runtime. Inventory is capped at
    // 1024 reports; failures stop before any new Reader or installation I/O.
    for _ in 0..1024 {
        if !outbox.replay_one(&ledger)? {
            break;
        }
    }
    if outbox.has_pending() {
        return Err(invalid("prior reports remain unresolved"));
    }
    let reader = RetainedImages::new(uploader, &ledger, root, owners)?;
    kv9_raft::snapshot_install::install_captured_record_with_reader(
        guard, identity, record, &reader,
    )
}
