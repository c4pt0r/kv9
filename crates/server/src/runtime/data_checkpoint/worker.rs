use super::*;
use crate::checkpoint_retention::{
    CheckpointExecution, CheckpointLedger, CheckpointOwners, CheckpointRead,
};
use crate::runtime::checkpoint_retention::RoutedLedger;
use kv9_engine::checkpoint::{CheckpointManifest, FlushJournal, PendingFlush};
use kv9_engine::checkpoint_history::CheckpointCandidate;
use kv9_meta::retention::OwnerKind;
use kv9_raft::state_machine::checkpoint_recovery::data_group::{self, DataCheckpointIdentity};
use kv9_region::manifest::{ManifestAttempt, ManifestProposalState, ManifestSeam, SettledManifest};

pub(super) struct Worker {
    seam: ManifestSeam,
    journal: FlushJournal,
    candidate: Option<CheckpointCandidate>,
    attempt: Option<Attempt>,
    settled: Option<SettledManifest>,
    execution: Option<CheckpointExecution>,
    reader: Option<CheckpointRead>,
    retire_after: Option<kv9_common::retention::OwnerId>,
    last_revision: u64,
    adopted: u64,
    selection_report: Option<(u64, kv9_common::AppliedPosition, [Vec<u8>; 2])>,
}
struct Attempt {
    manifest: CheckpointManifest,
    predecessor: u64,
    revision: Option<u64>,
}
impl Worker {
    pub(super) fn new(group: &Group, uploader: &RemoteUploader) -> Result<Self> {
        let (driver, engine, _) = group.capture_parts();
        Ok(Self {
            seam: ManifestSeam::mint(driver.mint_seam_handle()?),
            journal: uploader.journal(engine.path().with_extension("pending")),
            candidate: None,
            attempt: None,
            settled: None,
            execution: None,
            reader: None,
            retire_after: None,
            last_revision: 0,
            adopted: 0,
            selection_report: None,
        })
    }
    pub(super) fn tick(&mut self, context: &Context<'_>, work: &mut Work) -> Result<()> {
        context.check()?;
        // A previous pass already drained locally. Close its exact durable
        // generation before allowing a new upload or discarding local state.
        self.close_execution(&context.ledger)?;
        work.adopted += u64::from(self.report_selection(context)?);
        let (driver, engine, range) = context.group.capture_parts();
        if self.settled.is_some() {
            return self.finish(context, work);
        }
        if self.seam.in_flight(range.region.0) {
            let state = self
                .seam
                .converge(range.region.0, context.check()?.min(Duration::from_secs(1)))
                .map_err(seam_error)?;
            return self.settle(state, context, work);
        }
        if let Some(pending) = self.journal.load()? {
            if self.candidate.is_none() {
                self.candidate = Some(engine.claim_checkpoint_candidate(&pending)?);
            }
            if self.attempt.is_none() {
                self.attempt = Some(Attempt {
                    manifest: pending.manifest().clone(),
                    predecessor: pending.expected_generation(),
                    revision: None,
                });
            }
            return self.resume(pending, context, work);
        }
        if self
            .candidate
            .as_ref()
            .is_some_and(CheckpointCandidate::is_durable)
            || self.attempt.is_some()
        {
            return Err(invalid("unsettled durable checkpoint journal disappeared"));
        }
        // A failed pre-publication stage may have durably cancelled its empty
        // slot. Drop that ephemeral capability before freezing another image.
        self.candidate = None;
        let pair = driver.manifest_pair(range.region.0)?;
        if pair.generation > self.adopted {
            let (_, _, bytes) = driver
                .manifest_at(range.region.0, pair.generation)?
                .ok_or_else(|| invalid("applied checkpoint history missing"))?;
            let manifest = CheckpointManifest::decode(&bytes)?;
            work.adopted += u64::from(self.adopt(&manifest, pair.generation, context)?);
        }
        if !context.publish_new
            || driver.status().role != Role::Leader
            || engine.data_revision() == self.last_revision
        {
            return Ok(());
        }
        driver.read_barrier(context.check()?.min(Duration::from_secs(2)))?;
        let mut predecessor = 0;
        let (frozen, candidate) = engine.freeze_checkpoint_candidate(|view| {
            let (actual, scope, generation) = data_group::checkpoint_state(
                view,
                DataCheckpointIdentity {
                    cluster: context.root.cluster_id,
                    root: range.root,
                    region: range.region,
                    creation: range.creation,
                },
            )?;
            if actual.target() != range {
                return Err(Error::StaleEpoch {
                    region: range.region,
                });
            }
            predecessor = generation;
            Ok(scope)
        })?;
        let revision = frozen.data_revision();
        let mut plan = context.uploader.plan_spooled(frozen, &mut self.journal)?;
        CheckpointOwners::new(context.root, plan.manifest(), predecessor)?;
        context.check()?;
        self.candidate = Some(candidate);
        self.candidate.as_mut().unwrap().stage_spooled(
            &mut self.journal,
            &mut plan,
            predecessor,
        )?;
        self.attempt = Some(Attempt {
            manifest: plan.manifest().clone(),
            predecessor,
            revision: Some(revision),
        });
        // The fsynced journal now owns recovery. Release the anonymous spool
        // before loading the pinned journal inode and entering upload/readback.
        drop(plan);
        work.planned += 1;
        let pending = self
            .journal
            .load()?
            .ok_or_else(|| invalid("staged checkpoint journal disappeared"))?;
        self.resume(pending, context, work)
    }
    fn resume(
        &mut self,
        pending: PendingFlush,
        context: &Context<'_>,
        work: &mut Work,
    ) -> Result<()> {
        let attempt = self
            .attempt
            .as_ref()
            .ok_or_else(|| invalid("checkpoint attempt missing"))?;
        if pending.manifest() != &attempt.manifest
            || pending.expected_generation() != attempt.predecessor
        {
            return Err(invalid("checkpoint journal identity changed"));
        }
        context.check_history(&attempt.manifest)?;
        let owners = CheckpointOwners::new(context.root, &attempt.manifest, attempt.predecessor)?;
        owners.before_io(&context.ledger)?;
        context.check()?;
        if let Some(settled) = self
            .seam
            .inspect_checkpoint(&attempt.manifest, attempt.predecessor)
            .map_err(seam_error)?
        {
            return self.settle(ManifestProposalState::Settled(settled), context, work);
        }
        self.execution = Some(owners.execution(&context.ledger)?);
        self.execution
            .as_mut()
            .unwrap()
            .before_io_scoped(&context.ledger, context.uploader)?;
        let (prepared, predecessor) = pending.recover(context.uploader)?;
        // Preserve successful upload/readback even if the pass expired during I/O.
        // No consensus append is allowed without the next deadline/owner check.
        self.journal.record_uploaded(&prepared, predecessor)?;
        context.check()?;
        let manifest = attempt.manifest.clone();
        self.verify_image(&manifest, predecessor, OwnerKind::Pending, context)?;
        context.check()?;
        let state = self
            .seam
            .resume_in_flight(
                ManifestAttempt::from_prepared(prepared, predecessor)?,
                context.check()?.min(Duration::from_secs(1)),
            )
            .map_err(seam_error)?;
        self.settle(state, context, work)
    }
    /// The outer dispatcher calls this only after the scoped uploader drained.
    /// Failed ledger closure leaves the handle in this worker for exact retry.
    pub(super) fn close_execution(
        &mut self,
        ledger: &(impl CheckpointLedger + ?Sized),
    ) -> Result<()> {
        self.close_reader(ledger)?;
        if let Some(execution) = &self.execution {
            execution.after_local_drain(ledger)?;
            self.execution = None;
        }
        Ok(())
    }
    pub(super) fn observe_drain(
        &mut self,
        report: kv9_engine::checkpoint::DrainedExecution,
    ) -> Result<()> {
        if let Some(execution) = &mut self.execution {
            execution.observe_drain(report)
        } else if report.identity().is_some() {
            Err(invalid("checkpoint drain has no executor"))
        } else {
            Ok(())
        }
    }
    fn settle(
        &mut self,
        state: ManifestProposalState,
        context: &Context<'_>,
        work: &mut Work,
    ) -> Result<()> {
        match state {
            ManifestProposalState::Unknown => {
                work.unknown += 1;
                Ok(())
            }
            ManifestProposalState::Settled(settled) => {
                self.settled = Some(settled);
                self.finish(context, work)
            }
        }
    }
    fn finish(&mut self, context: &Context<'_>, work: &mut Work) -> Result<()> {
        context.check()?;
        let attempt = self
            .attempt
            .as_ref()
            .ok_or_else(|| invalid("settlement has no exact attempt"))?;
        if matches!(self.settled, Some(SettledManifest::WindowRefused { .. })) {
            let owners =
                CheckpointOwners::new(context.root, &attempt.manifest, attempt.predecessor)?;
            owners.close_admission(&context.ledger)?;
            self.retire_after =
                owners.retire_unpublished_page(&context.ledger, self.retire_after)?;
            if self.retire_after.is_some() {
                return Ok(());
            }
        }
        let positive = matches!(
            self.settled,
            Some(
                SettledManifest::MyChangeApplied { .. }
                    | SettledManifest::AlreadyApplied { .. }
                    | SettledManifest::EffectSettled
            )
        );
        if positive {
            // The seam may settle through a later image covering this effect.
            // Adopt the actual winner, whose predecessor is its own generation.
            let (driver, _, range) = context.group.capture_parts();
            let pair = driver.manifest_pair(range.region.0)?;
            let (_, _, bytes) = driver
                .manifest_at(range.region.0, pair.generation)?
                .ok_or_else(|| invalid("settled checkpoint winner missing"))?;
            let winner = CheckpointManifest::decode(&bytes)?;
            if winner.index < attempt.manifest.index {
                return Err(invalid("settled winner does not cover candidate"));
            }
            CheckpointOwners::new(context.root, &attempt.manifest, attempt.predecessor)?
                .after_positive_settlement(&context.ledger)?;
            work.adopted += u64::from(self.adopt(&winner, pair.generation, context)?);
        }
        context.check()?;
        self.candidate
            .as_mut()
            .ok_or_else(|| invalid("settlement lacks reservation"))?
            .finish(&mut self.journal)?;
        if positive {
            if let Some(revision) = self.attempt.as_ref().and_then(|a| a.revision) {
                self.last_revision = revision;
            }
            work.settled_positive += 1;
        } else {
            // No global owner is released by a local negative settlement.
            // Certified abort/old-owner reclamation remains a separate protocol.
            work.refused += 1;
        }
        self.candidate = None;
        self.attempt = None;
        self.settled = None;
        Ok(())
    }
    fn adopt(
        &mut self,
        manifest: &CheckpointManifest,
        generation: u64,
        context: &Context<'_>,
    ) -> Result<bool> {
        if generation <= self.adopted {
            return Ok(false);
        }
        let (_, engine, _) = context.group.capture_parts();
        let predecessor = generation
            .checked_sub(1)
            .ok_or_else(|| invalid("invalid manifest generation"))?;
        context.check_history(manifest)?;
        // Any actual applied replica may finish an interrupted positive handoff.
        // A publisher's death does not leave all followers permanently waiting.
        CheckpointOwners::new(context.root, manifest, predecessor)?
            .after_positive_settlement(&context.ledger)?;
        context.check()?;
        self.verify_image(manifest, predecessor, OwnerKind::Version, context)?;
        context.check()?;
        if !engine.checkpoint_applied(manifest)? {
            let covered = engine.selected_checkpoint()?.is_some_and(|m| {
                m == *manifest
                    || (m.index > manifest.index
                        && m.term >= manifest.term
                        && m.scope.cluster == manifest.scope.cluster
                        && m.scope.region == manifest.scope.region
                        && m.scope.conf_ver >= manifest.scope.conf_ver
                        && m.scope.version >= manifest.scope.version)
            });
            if !covered {
                return Err(invalid("applied checkpoint was not selected"));
            }
        }
        // Only the exact successfully selected image advances this store's
        // prospective floor. A covered older attempt cannot stand in for the
        // newer publication's generation; the next pass processes its history.
        if engine.selected_checkpoint()?.as_ref() != Some(manifest) {
            return Ok(false);
        }
        let activation = context.backend.ttl_context()?;
        activation
            .data_read_fences
            .advance(kv9_common::data_read_fence::ReadFloor {
                identity: activation.local.identity(),
                creation: context.group.target().creation,
                image: crate::checkpoint_retention::data_read_image(manifest)?
                    .ok_or_else(|| invalid("DATA adoption lacks DATA image"))?,
                generation,
            })?;
        let intent = kv9_meta::retention::checkpoint_intent::CheckpointIntent::new(
            context.root,
            manifest,
            predecessor,
        )?;
        let lifetime = context.ledger.local_lifetime()?;
        self.selection_report = Some((
            generation,
            manifest.position(),
            [
                kv9_meta::retention::encode_request(
                    context.root.digest(),
                    &kv9_meta::retention::LedgerRequest::RecordCheckpointSelection {
                        pending: intent.binding(OwnerKind::Pending)?.token(),
                        source: NodeId(lifetime.node),
                        store: kv9_common::StoreIncarnation::from_bytes(lifetime.store),
                    },
                )?,
                kv9_meta::retention::encode_request(
                    context.root.digest(),
                    &kv9_meta::retention::LedgerRequest::RecordDataReadFence {
                        pending: intent.binding(OwnerKind::Pending)?.token(),
                        source: NodeId(lifetime.node),
                        store: kv9_common::StoreIncarnation::from_bytes(lifetime.store),
                        creation: context.group.target().creation,
                    },
                )?,
            ],
        ));
        self.report_selection(context)
    }
    fn report_selection(&mut self, context: &Context<'_>) -> Result<bool> {
        let Some((generation, cut, bytes)) = &self.selection_report else {
            return Ok(false);
        };
        context.check()?;
        let (_, engine, _) = context.group.capture_parts();
        let selected = engine
            .selected_checkpoint()?
            .ok_or_else(|| invalid("pending selection report lost its local reference"))?;
        if selected.position() != *cut {
            if selected.index <= cut.index || selected.term < cut.term {
                return Err(invalid("pending selection report regressed"));
            }
            // A newer local selection is reported through its own exact
            // committed window on this pass. Never retry a superseded query
            // forever, or reinterpret Unknown as proof of the old selection.
            self.selection_report = None;
            return Ok(false);
        }
        for request in bytes {
            context.check()?;
            context.ledger.apply(request.clone())?;
        }
        self.adopted = *generation;
        self.selection_report = None;
        Ok(true)
    }
    fn close_reader(&mut self, ledger: &(impl CheckpointLedger + ?Sized)) -> Result<()> {
        if let Some(reader) = &self.reader {
            reader.finish(ledger)?;
            self.reader = None;
        }
        Ok(())
    }
    fn verify_image(
        &mut self,
        manifest: &CheckpointManifest,
        predecessor: u64,
        source: OwnerKind,
        context: &Context<'_>,
    ) -> Result<()> {
        self.close_reader(&context.ledger)?;
        context.check()?;
        self.reader = Some(CheckpointRead::new(
            context.root,
            manifest,
            predecessor,
            source,
            &context.ledger,
        )?);
        let reader = self.reader.as_mut().unwrap();
        let (result, report) = context.uploader.with_local_read_report(|scoped| {
            reader.before_io(&context.ledger, scoped)?;
            context.check()?;
            let image = scoped.restore(manifest)?;
            context.inspect_image(manifest, &image)
        })?;
        reader.observe_drain(report)?;
        self.close_reader(&context.ledger).and(result)
    }
}
pub(super) struct Context<'a> {
    pub backend: &'a RuntimeBackend,
    pub group: &'a Group,
    pub root: &'a RootDescriptor,
    pub uploader: &'a RemoteUploader,
    pub ledger: RoutedLedger,
    pub publish_new: bool,
    pub cancel: &'a CancellationToken,
    pub deadline: Instant,
}
impl Context<'_> {
    fn check(&self) -> Result<Duration> {
        if self.cancel.is_cancelled() {
            return Err(invalid("checkpoint task cancelled"));
        }
        self.backend.ensure_serving()?;
        self.group.check(self.backend)?;
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| invalid("checkpoint pass deadline expired"))
    }
    fn check_history(&self, manifest: &CheckpointManifest) -> Result<()> {
        self.check()?;
        let (driver, _, range) = self.group.capture_parts();
        if manifest.scope.cluster != self.root.cluster_id.to_string()
            || manifest.scope.region != range.region.0
        {
            return Err(invalid("checkpoint scope differs"));
        }
        if !matches!(
            driver
                .peer()
                .configuration_at_committed(manifest.position())?,
            kv9_raft::storage::ConfigurationLookup::Found(_)
        ) {
            return Err(invalid("checkpoint lacks retained configuration history"));
        }
        Ok(())
    }
    fn inspect_image(
        &self,
        manifest: &CheckpointManifest,
        image: &kv9_engine::MemEngine,
    ) -> Result<()> {
        self.check()?;
        let range = self.group.target();
        let view = image.snapshot()?;
        let image = data_group::inspect_checkpoint(
            view.as_ref(),
            manifest,
            DataCheckpointIdentity {
                cluster: self.root.cluster_id,
                root: range.root,
                region: range.region,
                creation: range.creation,
            },
        )?;
        if let data_group::DataCheckpointState::Import(import) = image.state() {
            self.group
                .capture_parts()
                .0
                .check_ttl_import_checkpoint(import)?;
        }
        let image_range = image.state().target();
        if image_range != range && (image.watermark().is_some() || !range.may_follow(image_range)) {
            return Err(invalid("checkpoint image has incompatible range history"));
        }
        Ok(())
    }
}
fn seam_error(error: kv9_region::manifest::ManifestSeamError) -> Error {
    invalid(&error.to_string())
}
