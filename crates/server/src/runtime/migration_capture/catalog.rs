//! Independently observe the source before taking the catalog lock; drain
//! prior proposals and recheck exact endpoint/bindings before one term-fenced
//! append. An already committed identical plan survives loss of its source.
use super::*;
use kv9_raft::grpc::checkpoint_retention::LedgerReply;
use kv9_raft::grpc::migration_capture::{inspect_on_channel, CaptureTicket};
use kv9_raft::snapshot_install::capture::{observe_durable_capture, CaptureDescriptor};

fn remaining(deadline: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| Error::MetaNotReady("capture catalog deadline expired".into()))
}
impl RuntimeBackend {
    /// Generic ledger requests cannot race the high-level bind path and attach
    /// a different image to a source/destination ID already fixed by the catalog.
    /// This runs inside the catalog lock after its earlier proposals drain.
    pub(in crate::runtime) fn check_capture_owner_binding(
        &self,
        view: &dyn kv9_engine::ReadView,
        root: &RootDescriptor,
        request: &kv9_meta::retention::LedgerRequest,
    ) -> Result<()> {
        use kv9_meta::retention::LedgerRequest;
        let binding = match request {
            LedgerRequest::Acquire(binding) | LedgerRequest::Share { to: binding, .. } => binding,
            _ => return Ok(()),
        };
        for migration in
            kv9_meta::data_groups::migration::committed_migrations(&self.node.meta_raft.store)?
        {
            let intent = migration.intent();
            let (source, destination) = crate::migration_retention::migration_owner_ids(
                root.digest(),
                intent.operation(),
                &intent.destination().incarnation,
            )?;
            if binding.descriptor.id != source && binding.descriptor.id != destination {
                continue;
            }
            if let Some(fixed) =
                kv9_meta::data_groups::capture::committed_capture(view, root, intent.operation())?
            {
                let descriptor = fixed.descriptor();
                crate::migration_retention::MigrationOwners::new(
                    root,
                    &migration,
                    &descriptor.request().range,
                    descriptor.manifest(),
                )?
                .verify_binding(binding)?;
            }
        }
        Ok(())
    }

    pub(in crate::runtime) fn observe_migration_capture(
        &self,
        root: RootDigest,
        region: RegionId,
        operation: [u8; 16],
        deadline: Instant,
    ) -> Result<Option<[u8; 32]>> {
        self.ensure_serving()?;
        remaining(deadline)?;
        let source = self.ttl_context()?.local.identity();
        if source.root_digest != root {
            return Err(Error::Config("capture observation root differs".into()));
        }
        let group = self
            .raw_directory
            .by_region(region)
            .ok_or_else(|| Error::MetaNotReady("capture source group unavailable".into()))?;
        let (driver, engine, binding) = group.capture_parts();
        let uploader = self
            .uploader
            .as_deref()
            .ok_or_else(|| Error::MetaNotReady("capture source uploader absent".into()))?;
        let observed = observe_durable_capture(driver, engine, source, operation, uploader)?;
        remaining(deadline)?;
        match observed {
            Some(d) if &d.request().range == binding => Ok(Some(*d.digest().as_bytes())),
            None => Ok(None),
            _ => Err(Error::Config(
                "capture source directory binding differs".into(),
            )),
        }
    }

    pub(in crate::runtime) fn record_capture_plan(
        &self,
        root: RootDigest,
        bytes: &[u8],
        deadline: Instant,
    ) -> Result<LedgerReply> {
        let descriptor = CaptureDescriptor::decode(bytes)?;
        if descriptor.request().source.root_digest != root {
            return Err(Error::Config("capture catalog root differs".into()));
        }
        self.driver.read_barrier(remaining(deadline)?)?;
        let txn = self.node.meta_raft.store.begin()?;
        let certified = kv9_meta::root::certified_root(&txn)?
            .ok_or_else(|| Error::MetaNotReady("capture root unavailable".into()))?;
        if certified.digest() != root {
            return Err(Error::Config("capture catalog root changed".into()));
        }
        let source = descriptor.request().source;
        let route = kv9_meta::endpoint::node_endpoint(&txn, source.node_id)?;
        let existing = kv9_meta::data_groups::capture::committed_capture(
            txn.into_view().as_ref(),
            &certified,
            descriptor.request().operation,
        )?;
        let observed = if let Some(old) = existing {
            if old.descriptor().encode() != bytes {
                return Err(Error::Config(
                    "migration already binds a different capture".into(),
                ));
            }
            None
        } else {
            let route = route
                .filter(|e| e.active && e.incarnation == source.store_incarnation)
                .ok_or_else(|| Error::MetaNotReady("capture source endpoint unavailable".into()))?;
            let context = self.ttl_context()?;
            let ticket = CaptureTicket::new(&descriptor)?;
            let proof = tokio::runtime::Handle::current().block_on(async {
                let channel = context.channel(route.address, deadline).await?;
                inspect_on_channel(
                    channel,
                    &ticket,
                    self.node.id,
                    Some(context.token.clone()),
                    remaining(deadline)?,
                )
                .await
            })?;
            Some((proof, route))
        };
        remaining(deadline)?;
        let _guard = self
            .node
            .meta_raft
            .wait_catalog_txn(
                deadline.min(Instant::now() + super::super::checkpoint_retention::CATALOG_WAIT),
            )
            .ok_or_else(|| Error::MetaNotReady("capture catalog busy".into()))?;
        remaining(deadline)?;
        let term = self.prepare_catalog()?;
        remaining(deadline)?;
        // Existing pins from a prior binary/attempt must not be contradicted by
        // registering another protocol image under the same operation.
        let migration =
            kv9_meta::data_groups::migration::committed_migrations(&self.node.meta_raft.store)?
                .into_iter()
                .find(|m| m.intent().operation() == descriptor.request().operation)
                .ok_or_else(|| Error::Config("capture migration missing".into()))?;
        let owners = crate::migration_retention::MigrationOwners::new(
            &certified,
            &migration,
            &descriptor.request().range,
            descriptor.manifest(),
        )?;
        owners.verify_compatible_locally(
            self.node.meta_raft.store.begin()?.into_view().as_ref(),
            &certified,
        )?;
        let (batch, changed) = kv9_meta::data_groups::capture::plan_record(
            self.node.meta_raft.store.begin()?,
            &descriptor,
            observed.as_ref().map(|(proof, route)| (proof, *route)),
        )?;
        let command = if changed {
            Command::from_batch(&batch)
        } else {
            Command::Noop
        };
        let applied = self.commit_catalog(&command, term)?;
        remaining(deadline)?;
        Ok(LedgerReply::Applied {
            revision: 0,
            changed,
            applied,
        })
    }
}
