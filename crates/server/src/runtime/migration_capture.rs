//! Applied metadata selects the operation; the local durable slot fixes its
//! image. Retain the actual group owner until upload and object-job drain end.
use super::*;
use kv9_engine::checkpoint::{CheckpointManifest, RemoteUploader};
use kv9_meta::data_groups::CaptureSubject;
use kv9_raft::snapshot_install::capture::{
    plan_capture_durable, CaptureDescriptor, CaptureRequest, CapturedSourceImage, PlannedCapture,
};
mod catalog;
mod payload;
mod publisher;

pub(super) enum CaptureWork {
    Local(Box<PlannedCapture>),
    Remote(Box<CaptureDescriptor>),
}
impl CaptureWork {
    pub(super) fn descriptor(&self) -> Result<CaptureDescriptor> {
        match self {
            Self::Local(p) => p.descriptor(),
            Self::Remote(d) => Ok(d.as_ref().clone()),
        }
    }
    pub(super) fn manifest(&self) -> &CheckpointManifest {
        match self {
            Self::Local(p) => p.manifest(),
            Self::Remote(d) => d.manifest(),
        }
    }
    pub(super) fn range(&self) -> &kv9_common::data_range::DataRange {
        match self {
            Self::Local(p) => p.range(),
            Self::Remote(d) => &d.request().range,
        }
    }
    pub(super) fn cut(&self) -> AppliedPosition {
        match self {
            Self::Local(p) => p.cut(),
            Self::Remote(d) => d.image().cut,
        }
    }
    pub(super) fn upload(
        self,
        backend: &RuntimeBackend,
        uploader: &RemoteUploader,
        payload_lock: &mut Option<kv9_engine::checkpoint::TransferLock>,
    ) -> Result<CapturedSourceImage> {
        match self {
            Self::Local(p) => {
                *payload_lock = p.hold_transfer_lock();
                p.upload(uploader)
            }
            Self::Remote(d) => {
                if let Ok(image) = d.verify_uploaded(uploader) {
                    return Ok(image);
                }
                let journal = backend.ensure_local_capture(&d)?;
                let planned = d.resume_replica(journal)?;
                *payload_lock = planned.hold_transfer_lock();
                planned.upload(uploader)
            }
        }
    }
}

impl RuntimeBackend {
    pub(super) fn with_migration_capture<T>(
        &self,
        root: RootDigest,
        operation: [u8; 16],
        work: impl FnOnce(
            &RootDescriptor,
            &CaptureSubject,
            CaptureWork,
            &RemoteUploader,
        ) -> Result<T>,
    ) -> Result<T> {
        self.ensure_serving()?;
        let uploader = self
            .uploader
            .as_deref()
            .ok_or_else(|| Error::Config("capture requires configured object storage".into()))?;
        let store = &self.node.meta_raft.store;
        let certified = kv9_meta::root::certified_root(&store.begin()?)?
            .ok_or_else(|| Error::MetaNotReady("capture requires a certified root".into()))?;
        if certified.digest() != root {
            return Err(Error::Config("capture request root differs".into()));
        }
        let migration = kv9_meta::data_groups::capture_subject(store, operation)?
            .ok_or_else(|| Error::Config("migration operation is not committed".into()))?;
        let aborts = kv9_meta::data_groups::abort::committed_aborts(store)?;
        let backups = kv9_meta::data_groups::backup::committed_backups(store)?;
        {
            let txn = store.begin()?;
            if kv9_meta::data_groups::capture_subject_aborted(&txn, root, &migration)? {
                return Err(Error::Config(
                    "the operation is aborted; capture refuses permanently".into(),
                ));
            }
        }
        let installed = kv9_meta::data_groups::evidence::committed_install_evidence(store)?;
        // An operation whose image already proved itself (install evidence for a
        // migration, an anchor for a backup range) gets no second capture.
        let allow_create = !installed.iter().any(|e| e.operation() == operation)
            && !backups.anchors.iter().any(|a| a.range_operation() == operation);
        let committed_range = kv9_meta::data_groups::ranges::committed_ranges(store)?
            .into_iter()
            .find(|r| r.creation().intent().region() == migration.region())
            .ok_or_else(|| Error::Config("migration group has no committed range".into()))?;
        let group = self
            .raw_directory
            .by_region(migration.region())
            .ok_or_else(|| Error::MetaNotReady("migration group is not active locally".into()))?;
        let (driver, engine, binding) = group.capture_parts();
        if binding != committed_range.range() {
            return Err(Error::Config(
                "live group binding differs from the committed range".into(),
            ));
        }
        let source = self.ttl_context()?.local.identity();
        if source.root_digest != root
            || source.cluster_id != certified.cluster_id
            || source.node_id != self.node.id
        {
            return Err(Error::Config(
                "capture source store identity differs".into(),
            ));
        }
        let destination = migration.destination();
        let request = CaptureRequest {
            source,
            destination: StoreIdentity {
                cluster_id: source.cluster_id,
                root_digest: root,
                node_id: destination.node,
                store_incarnation: destination.incarnation,
            },
            operation,
            range: binding.clone(),
        };
        let status = driver.status();
        if status.fatal.is_some() || driver.async_apply_snapshot().stopped {
            return Err(Error::Raft("capture group driver unavailable".into()));
        }
        if status.role != Role::Leader {
            return Err(Error::NotLeader {
                leader: status.leader_id,
            });
        }
        let fixed = kv9_meta::data_groups::capture::committed_capture(
            store.begin()?.into_view().as_ref(),
            &certified,
            operation,
        )?;
        if let Some(fixed) = fixed {
            let descriptor = fixed.descriptor();
            let original = descriptor.request();
            if original.destination != request.destination || original.range != request.range {
                return Err(Error::Config(
                    "capture catalog differs from current migration binding".into(),
                ));
            }
            let cut = descriptor.image().cut;
            match engine.applied_position()? {
                kv9_engine::DurableAppliedPosition::AppliedThrough(at)
                    if at.index >= cut.index
                        && at.term >= cut.term
                        && (at.index != cut.index || at.term == cut.term) => {}
                _ => {
                    return Err(Error::MetaNotReady(
                        "capture successor has not applied through cut".into(),
                    ))
                }
            }
            let local_slot = engine
                .path()
                .parent()
                .ok_or_else(|| Error::Config("capture directory absent".into()))?
                .join("capture.plan");
            let planned = if original.source == source
                && local_slot
                    .try_exists()
                    .map_err(|e| Error::Engine(format!("capture slot lookup: {e}")))?
            {
                let local =
                    plan_capture_durable(driver, engine, original, uploader, false, |_, _, _| {
                        Ok(false)
                    })?;
                if local.descriptor()?.encode() != descriptor.encode() {
                    return Err(Error::Config(
                        "durable slot differs from committed capture".into(),
                    ));
                }
                CaptureWork::Local(Box::new(local))
            } else {
                CaptureWork::Remote(Box::new(descriptor.clone()))
            };
            return work(&certified, &migration, planned, uploader);
        }
        let planned = plan_capture_durable(
            driver,
            engine,
            &request,
            uploader,
            allow_create,
            |old, manifest, image_digest| {
                let subject = *RootDigest::sha256(&manifest.encode()?).as_bytes();
                let matches = |operation,
                               root,
                               region,
                               destination: kv9_meta::data_groups::InitialReplica,
                               selected| {
                    operation == old.operation
                        && root == old.range.root
                        && region == old.range.region
                        && destination.node == old.destination.node_id
                        && destination.incarnation == old.destination.store_incarnation
                        && selected == subject
                };
                // A slot held by an earlier BACKUP range is settled once that
                // range is anchored with this exact image, or its backup is
                // aborted; nothing else may reuse it.
                let old_backup = backups.range_by_capture_operation(old.operation);
                let backup_settled = old_backup.is_some_and(|(intent, range)| {
                    old.range.region == range.region()
                        && (backups.aborted.contains(&intent.operation())
                            || backups
                                .anchor_of_range(range)
                                .is_some_and(|a| a.image_digest() == image_digest && a.cut() == manifest.position()))
                });
                Ok(backup_settled
                    || aborts.iter().any(|a| {
                        matches(
                            a.operation(),
                            a.root(),
                            a.region(),
                            a.destination(),
                            a.subject(),
                        )
                    })
                    || installed.iter().any(|e| {
                        matches(
                            e.operation(),
                            e.root(),
                            e.region(),
                            e.destination(),
                            e.subject(),
                        ) && e.image_digest() == image_digest
                            && e.cut() == manifest.position()
                    }))
            },
        )?;
        let descriptor = planned.descriptor()?;
        // Independent observation must acquire the actual source file lock;
        // never hold it while waiting on metadata's callback to this store.
        drop(planned);
        checkpoint_retention::RoutedLedger::record_capture(self, descriptor.encode().to_vec())?;
        let planned =
            plan_capture_durable(driver, engine, &request, uploader, false, |_, _, _| {
                Ok(false)
            })?;
        if planned.descriptor()?.encode() != descriptor.encode() {
            return Err(Error::Config(
                "capture changed after catalog confirmation".into(),
            ));
        }
        // The group Arc and transfer file lock remain owned during `work`.
        // Retirement cannot delete payload until these actual owners drain.
        work(
            &certified,
            &migration,
            CaptureWork::Local(Box::new(planned)),
            uploader,
        )
    }
}
