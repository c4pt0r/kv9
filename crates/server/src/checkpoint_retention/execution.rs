//! Per-runtime checkpoint uploader records using existing Build owner semantics.
//! Closing transfers protection to Pending/Version; it never removes global
//! coverage or certifies remote cancellation, other executors, or full backfill.
use super::*;
use kv9_engine::checkpoint::{DrainedExecution, ExecutionIdentity, RemoteUploader};
use kv9_meta::retention::LocalLifetime;

pub(crate) struct CheckpointExecution {
    owners: CheckpointOwners,
    binding: OwnerBinding,
    scoped: bool,
    drain: Option<(u64, u64)>,
    report_slot: Option<super::drain_outbox::ReportSlot>,
}

impl CheckpointOwners {
    #[cfg(test)]
    pub(crate) fn execution_id(&self, lifetime: LocalLifetime) -> Result<OwnerId> {
        Ok(self.execution_binding(lifetime)?.descriptor.id)
    }
    fn execution_binding(&self, lifetime: LocalLifetime) -> Result<OwnerBinding> {
        kv9_meta::retention::checkpoint_admission::executor_binding(&self.intent, lifetime)
    }

    /// Only prepares an identity. The caller must save this handle before
    /// `before_io`: an unconfirmed mutation must be retried with this generation.
    pub(crate) fn execution(
        self,
        api: &(impl CheckpointLedger + ?Sized),
    ) -> Result<CheckpointExecution> {
        let mut binding = self.execution_binding(api.local_lifetime()?)?;
        if let Some(bytes) = api.owner(self.root, binding.descriptor.id)? {
            let old = decode_owner_observation(&bytes, self.root, binding.descriptor.id)?;
            if old.binding.descriptor != binding.descriptor
                || old.binding.resources != binding.resources
            {
                return Err(Error::Engine("checkpoint executor binding differs".into()));
            }
            if old.phase != PinPhase::Released {
                return Err(Error::Engine(
                    "checkpoint executor generation has not closed".into(),
                ));
            }
            binding.generation = old
                .binding
                .generation
                .get()
                .checked_add(1)
                .and_then(NonZeroU64::new)
                .ok_or_else(|| Error::Engine("checkpoint executor generation exhausted".into()))?;
        }
        let report_slot = api
            .drain_outbox()?
            .map(|outbox| outbox.reserve(binding.token()))
            .transpose()?;
        Ok(CheckpointExecution {
            owners: self,
            binding,
            scoped: false,
            drain: None,
            report_slot,
        })
    }
}

impl CheckpointExecution {
    fn identity(&self) -> ExecutionIdentity {
        ExecutionIdentity {
            root: self.owners.root,
            pending: self.owners.pending_id(),
            executor: self.binding.token(),
        }
    }
    pub(crate) fn before_io_scoped(
        &mut self,
        api: &(impl CheckpointLedger + ?Sized),
        uploader: &RemoteUploader,
    ) -> Result<()> {
        if self.scoped {
            return Err(Error::Engine(
                "checkpoint executor scope already attached".into(),
            ));
        }
        uploader.bind_execution(self.identity())?;
        self.scoped = true;
        self.before_io(api)
    }
    pub(crate) fn observe_drain(&mut self, report: DrainedExecution) -> Result<()> {
        if report.identity().is_none() {
            return Ok(());
        } // No new execution this pass.
        if report.identity() != Some(self.identity()) || !self.scoped || self.drain.is_some() {
            return Err(Error::Engine(
                "checkpoint drain scope or generation differs".into(),
            ));
        }
        self.drain = Some((report.issued_puts(), report.resolved_puts()));
        Ok(())
    }
    fn phase(&self, api: &(impl CheckpointLedger + ?Sized)) -> Result<Option<PinPhase>> {
        let Some(bytes) = api.owner(self.owners.root, self.binding.descriptor.id)? else {
            return Ok(None);
        };
        let old = decode_owner_observation(&bytes, self.owners.root, self.binding.descriptor.id)?;
        if old.binding == self.binding {
            return Ok(Some(old.phase));
        }
        if old.binding.descriptor == self.binding.descriptor
            && old.binding.resources == self.binding.resources
            && old.phase == PinPhase::Released
            && old.binding.generation.get().checked_add(1) == Some(self.binding.generation.get())
        {
            // This exact next generation may not have been admitted yet. Close
            // it through the ledger before forgetting its unconfirmed Share.
            return Ok(None);
        }
        Err(Error::Engine(
            "checkpoint executor generation or binding differs".into(),
        ))
    }
    fn successor(
        &self,
        api: &(impl CheckpointLedger + ?Sized),
    ) -> Result<kv9_common::retention::OwnerToken> {
        if self.owners.phase(api, &self.owners.pending)? == Some(PinPhase::Published) {
            return Ok(self.owners.pending.token());
        }
        self.owners.published_version(api)?;
        Ok(self.owners.version.token())
    }
    /// Exact committed Share + Publish must precede object I/O. An error grants
    /// nothing; the saved handle still has to be closed after local drain.
    pub(crate) fn before_io(&self, api: &(impl CheckpointLedger + ?Sized)) -> Result<()> {
        self.owners.apply(
            api,
            LedgerRequest::Share {
                from: self.successor(api)?,
                to: self.binding.clone(),
            },
        )?;
        self.owners
            .apply(api, LedgerRequest::Publish(self.binding.token()))
    }
    /// Call only after with_local_execution returned. Even a failed entry is
    /// settled at this exact generation, so a delayed Share cannot revive it.
    /// Any error keeps the handle; no later local upload may pass it.
    pub(crate) fn after_local_drain(&self, api: &(impl CheckpointLedger + ?Sized)) -> Result<()> {
        if self.scoped && self.drain.is_none() {
            return Err(Error::Engine(
                "checkpoint executor lacks its exact drained scope".into(),
            ));
        }
        if let Some((issued, resolved)) = self.drain {
            let bytes = encode_request(
                self.owners.root,
                &LedgerRequest::FinishCheckpointExecution {
                    pending: self.owners.pending.token(),
                    executor: self.binding.clone(),
                    issued,
                    resolved,
                },
            )?;
            // The slot was reserved before this execution could enter I/O.
            // Persist the exact drained observation before submitting it, so
            // loss of the worker/process cannot replace it with new counters.
            if let Some(slot) = &self.report_slot {
                slot.persist(&bytes)?;
            }
            api.apply(bytes.clone())?;
            if let Some(slot) = &self.report_slot {
                slot.acknowledge(&bytes)?;
            }
            return Ok(());
        }
        // Legacy unscoped callers can transfer protection but never invent a
        // scope report. Production uploaders always use the atomic path above.
        self.after_unscoped_drain(api)?;
        if let Some(slot) = &self.report_slot {
            slot.release_unused()?;
        }
        Ok(())
    }
    fn after_unscoped_drain(&self, api: &(impl CheckpointLedger + ?Sized)) -> Result<()> {
        let phase = self.phase(api)?;
        if phase == Some(PinPhase::Released) {
            return Ok(());
        }
        let successor = self.successor(api)?;
        if phase.is_none() {
            // A single terminal fence also works after admission closes. If
            // entry actually committed, the ledger refuses this cancellation;
            // the next pass observes Held/Published and transfers after drain.
            self.owners.apply(
                api,
                LedgerRequest::CancelUnenteredCheckpoint(self.binding.clone()),
            )?;
            return Ok(());
        }
        if phase != Some(PinPhase::Quiesced) {
            self.owners.apply(
                api,
                LedgerRequest::QuiesceAfterTransfer {
                    from: self.binding.token(),
                    to: successor,
                },
            )?;
        }
        self.owners
            .apply(api, LedgerRequest::Release(self.binding.token()))
    }
}
