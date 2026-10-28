//! Runtime recovery reads admitted against exact committed image/checkpoint
//! owners. Local publication is verified by Raft before this adapter sees it.
use crate::checkpoint_retention::{CheckpointLedger, CheckpointRead};
use crate::installation_retention::RetainedImages;
use kv9_common::retention::{OwnerId, PinPhase};
use kv9_common::{Error, Result, RootDescriptor, StoreIdentity};
use kv9_engine::checkpoint::{CheckpointManifest, CheckpointRestore, RemoteUploader};
use kv9_engine::MemEngine;
use kv9_meta::retention::{
    checkpoint_intent::CheckpointIntent, decode_owner_observation, OwnerKind,
};
use kv9_raft::state_machine::checkpoint_recovery::data_group::DataCheckpointReader;
use std::sync::Arc;

pub(crate) struct RecoveryRetention {
    root: RootDescriptor,
    identity: StoreIdentity,
    ledger: Arc<dyn CheckpointLedger>,
}
impl RecoveryRetention {
    pub(crate) fn new(
        root: RootDescriptor,
        identity: StoreIdentity,
        ledger: Arc<dyn CheckpointLedger>,
    ) -> Self {
        Self {
            root,
            identity,
            ledger,
        }
    }
    pub(crate) fn reader<'a>(
        &'a self,
        uploader: &'a RemoteUploader,
        image: Option<OwnerId>,
    ) -> RecoveryReader<'a> {
        RecoveryReader {
            retention: self,
            uploader,
            image,
        }
    }
    fn ready(&self) -> Result<()> {
        self.identity.verify(&self.root, self.identity.node_id)?;
        let lifetime = self.ledger.local_lifetime()?;
        if lifetime.node != self.identity.node_id.0
            || lifetime.store != *self.identity.store_incarnation.as_bytes()
        {
            return Err(Error::Config(
                "recovery Reader belongs to another store".into(),
            ));
        }
        if self.ledger.drain_outbox()?.is_none() {
            return Err(Error::Config(
                "runtime recovery requires a durable Reader outbox".into(),
            ));
        }
        Ok(())
    }
}

pub(crate) struct RecoveryReader<'a> {
    retention: &'a RecoveryRetention,
    uploader: &'a RemoteUploader,
    image: Option<OwnerId>,
}
fn retry(error: Error) -> Error {
    // Admission/receipt and object availability failures leave the group
    // unopened and eligible for another bounded maintenance turn. Corrupt
    // local WAL/Raft or restored semantic state is still rejected by the
    // enclosing full recovery, outside this transient dependency boundary.
    Error::MetaNotReady(format!("recovery Reader unavailable: {error}"))
}
impl CheckpointRestore for RecoveryReader<'_> {
    fn restore(&self, manifest: &CheckpointManifest) -> Result<MemEngine> {
        (|| {
            self.retention.ready()?;
            let image = self
                .image
                .ok_or_else(|| Error::Engine("recovery lacks an original image owner".into()))?;
            RetainedImages::new(
                self.uploader,
                self.retention.ledger.as_ref(),
                &self.retention.root,
                &[image],
            )?
            .restore(manifest)
        })()
        .map_err(retry)
    }
}
impl DataCheckpointReader for RecoveryReader<'_> {
    fn check_selection(
        &self,
        identity: kv9_raft::state_machine::checkpoint_recovery::data_group::DataCheckpointIdentity,
        manifest: Option<&CheckpointManifest>,
    ) -> Result<()> {
        self.retention.ready()?;
        self.retention
            .ledger
            .check_data_recovery(identity, manifest)
    }

    fn restore_published(
        &self,
        manifest: &CheckpointManifest,
        predecessor: u64,
    ) -> Result<MemEngine> {
        (|| {
            self.retention.ready()?;
            let root = &self.retention.root;
            let ledger = self.retention.ledger.as_ref();
            let intent = CheckpointIntent::new(root, manifest, predecessor)?;
            // Point queries only. Prefer the durable Version; an in-progress
            // positive handoff may still have its exact Published Pending.
            let mut source = None;
            for kind in [OwnerKind::Version, OwnerKind::Pending] {
                let expected = intent.binding(kind)?;
                if let Some(bytes) = ledger.owner(root.digest(), expected.descriptor.id)? {
                    let owner =
                        decode_owner_observation(&bytes, root.digest(), expected.descriptor.id)?;
                    if owner.binding != expected {
                        return Err(Error::Engine(
                            "recovery checkpoint owner binding differs".into(),
                        ));
                    }
                    if owner.phase == PinPhase::Published {
                        source = Some(kind);
                        break;
                    }
                }
            }
            let kind = source.ok_or_else(|| {
                Error::MetaNotReady("exact recovery checkpoint source is not Published".into())
            })?;
            let mut reader = CheckpointRead::new(root, manifest, predecessor, kind, ledger)?;
            let (result, report) = self.uploader.with_local_read_report(|scoped| {
                reader.before_io(ledger, scoped)?;
                scoped.restore(manifest)
            })?;
            reader.observe_drain(report)?;
            reader.finish(ledger)?;
            result
        })()
        .map_err(retry)
    }
}
