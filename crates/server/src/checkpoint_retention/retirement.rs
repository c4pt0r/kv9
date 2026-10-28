use super::*;

impl CheckpointOwners {
    /// At most four executor observations/mutations per worker pass. A failed
    /// page leaves the cursor unchanged; already committed retirements are
    /// observed as Released on retry. Closed admission prevents new Held rows
    /// behind the cursor. This is not a fixed snapshot/global closure audit.
    pub(crate) fn retire_unpublished_page(
        &self,
        api: &(impl CheckpointLedger + ?Sized),
        after: Option<OwnerId>,
    ) -> Result<Option<OwnerId>> {
        let page = api.executors(self.root, self.pending_id(), after, 4)?;
        if !page.admission.is_closed() || page.admission.pending != self.pending_id() {
            return Err(Error::Engine("checkpoint retirement gate differs".into()));
        }
        for owner in page.owners {
            if owner.phase == PinPhase::Held {
                self.apply(
                    api,
                    LedgerRequest::RetireUnpublishedCheckpoint {
                        pending: self.pending.token(),
                        executor: owner.binding.token(),
                    },
                )?;
            }
        }
        Ok(page.next)
    }
}
