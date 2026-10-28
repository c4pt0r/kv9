//! Applied-source and empty-child observations for protected TTL transfer.
use super::*;
use crate::command::{RawTtlSplit, RawTtlSplitAction};
use crate::rawnode::ttl_split::{PreparedTtlSplit, TtlSplitProbe};
use kv9_common::data_range::{DataRange, COMPACTION_CONFIRMED_KEY};
use kv9_common::raw_ttl::split::{
    validate_local, ImportState, SplitPlan, SplitSeal, IMPORT_KEY, SEAL_KEY,
};
use kv9_common::raw_ttl::{ClockWatermark, CLOCK_KEY};
use kv9_common::AppliedPosition;
use kv9_engine::{ColumnFamily, DurableAppliedPosition, ReplicatedEngine, WalEngine};

mod source;
pub use source::SealedTtlSplitSource;

/// Minted only from this driver's real positioned engine snapshot. Not Clone or
/// deserializable. It is a prefix observation, not submission authority; Begin
/// must still compare this exact position inside ordered apply.
pub struct EmptyTtlImport {
    target: DataRange,
    position: AppliedPosition,
}
impl EmptyTtlImport {
    pub fn target(&self) -> &DataRange {
        &self.target
    }
    pub fn position(&self) -> AppliedPosition {
        self.position
    }
}
impl<S: PersistentRaftStorage> NodeDriver<S, WalEngine> {
    /// The runtime must supply a plan from its validated committed metadata
    /// intent. This method captures only local data and full-configuration facts.
    pub fn begin_ttl_split_seal(
        &self,
        plan: SplitPlan,
        deadline: Instant,
    ) -> Result<TtlSplitProbe> {
        self.ttl_split_probe(RawTtlSplitAction::Seal(plan), deadline)
    }

    /// A follower's committed applied seal is sufficient: the sealed source is
    /// immutable. Its local snapshot, identity, cut and clock are rechecked.
    pub fn capture_ttl_split_source(
        &self,
        plan: &SplitPlan,
        deadline: Instant,
    ) -> Result<SealedTtlSplitSource> {
        plan.validate()?;
        source::unexpired(deadline)?;
        let engine = {
            let state = self.sm.lock().expect("sm poisoned");
            state.check_ttl_split_source(plan)?;
            state.engine().clone()
        };
        let snapshot = engine
            .try_positioned_resident_snapshot()
            .ok_or_else(|| Error::MetaNotReady("TTL source snapshot busy".into()))?;
        let at = snapshot
            .position()
            .ok_or_else(|| Error::MetaNotReady("TTL source has no applied cut".into()))?;
        let view = snapshot.into_view();
        let range = view
            .get(ColumnFamily::Default, kv9_common::data_range::RANGE_KEY)?
            .map(|b| DataRange::decode(&b))
            .transpose()?;
        let clock = view
            .get(ColumnFamily::Default, CLOCK_KEY)?
            .map(|b| ClockWatermark::decode(&b))
            .transpose()?;
        let seal = view
            .get(ColumnFamily::Default, SEAL_KEY)?
            .map(|b| SplitSeal::decode(&b))
            .transpose()?
            .ok_or_else(|| Error::MetaNotReady("TTL source seal missing".into()))?;
        let import = view
            .get(ColumnFamily::Default, IMPORT_KEY)?
            .map(|b| ImportState::decode(&b))
            .transpose()?;
        validate_local(
            (plan.parent.root, plan.parent.region, plan.parent.creation),
            range.as_ref(),
            clock,
            Some(&seal),
            import.as_ref(),
        )?;
        if seal.plan != *plan || seal.cut.term > at.term || seal.cut.index > at.index {
            return Err(Error::Raft(
                "TTL source identity or durable cut differs".into(),
            ));
        }
        SealedTtlSplitSource::capture(view, seal, deadline)
    }

    /// Compare a verified staging image with this driver's actual immutable
    /// import identity and monotone progress. This does not grant publication.
    pub fn check_ttl_import_checkpoint(&self, image: &ImportState) -> Result<()> {
        image.validate()?;
        let current = self
            .ttl_import_state(image.spec.target())?
            .ok_or_else(|| Error::MetaNotReady("checkpoint has no matching local import".into()))?;
        if image.complete
            || image.spec != current.spec
            || image.copied > current.copied
            || (image.copied == current.copied
                && (image.last != current.last || image.chain != current.chain))
            || (image.copied < current.copied
                && image
                    .last
                    .as_ref()
                    .is_some_and(|last| current.last.as_ref().is_none_or(|now| last >= now)))
        {
            return Err(Error::MetaNotReady(
                "checkpoint import identity or progress differs from actual driver".into(),
            ));
        }
        Ok(())
    }

    pub fn ttl_import_state(&self, target: &DataRange) -> Result<Option<ImportState>> {
        self.sm
            .lock()
            .expect("sm poisoned")
            .ttl_split_import_state(target)
    }

    pub fn verify_ttl_split_target(
        &self,
        source: &SealedTtlSplitSource,
        high: bool,
        deadline: Instant,
    ) -> Result<()> {
        let engine = {
            let state = self.sm.lock().expect("sm poisoned");
            state.ttl_split_import_state(source.spec(high).target())?;
            state.engine().clone()
        };
        let snapshot = engine
            .try_positioned_resident_snapshot()
            .ok_or_else(|| Error::MetaNotReady("TTL target snapshot busy".into()))?;
        source.verify_target(snapshot.into_view(), high, deadline)
    }

    /// Bind the actual source commitment and an actual empty target prefix.
    /// An exact existing import is a retry; it never resets its durable cursor.
    pub fn begin_ttl_split_import(
        &self,
        source: &SealedTtlSplitSource,
        high: bool,
        deadline: Instant,
    ) -> Result<TtlSplitProbe> {
        let spec = source.spec(high);
        let empty_at = if let Some(state) = self.ttl_import_state(spec.target())? {
            if state.spec != *spec {
                return Err(Error::MetaNotReady(
                    "TTL import source commitment differs".into(),
                ));
            }
            // Only a no-op retry; still record a real local applied position.
            let state = self.sm.lock().expect("sm poisoned");
            match ReplicatedEngine::applied_position(state.engine().as_ref())? {
                DurableAppliedPosition::AppliedThrough(at) => at,
                DurableAppliedPosition::AppliedNothing | DurableAppliedPosition::Volatile => {
                    return Err(Error::MetaNotReady(
                        "existing TTL import has no applied position".into(),
                    ))
                }
            }
        } else {
            self.inspect_empty_ttl_import(spec.target())?.position()
        };
        self.ttl_split_probe(
            RawTtlSplitAction::Begin {
                spec: spec.clone(),
                empty_at,
            },
            deadline,
        )
    }

    /// Prepare the next source-derived page, or Finish at the exact commitment.
    pub fn begin_ttl_split_copy(
        &self,
        source: &SealedTtlSplitSource,
        high: bool,
        deadline: Instant,
    ) -> Result<TtlSplitProbe> {
        let spec = source.spec(high);
        let state = self
            .ttl_import_state(spec.target())?
            .filter(|s| s.spec == *spec)
            .ok_or_else(|| Error::MetaNotReady("TTL import must begin from this source".into()))?;
        let action = if state.copied == spec.rows {
            state.finish()?;
            RawTtlSplitAction::Finish {
                target: spec.target().clone(),
                expected: state.digest(),
            }
        } else {
            RawTtlSplitAction::Append {
                target: spec.target().clone(),
                expected: state.digest(),
                rows: source.next_page(&state, deadline)?,
            }
        };
        self.ttl_split_probe(action, deadline)
    }

    fn ttl_split_probe(
        &self,
        action: RawTtlSplitAction,
        deadline: Instant,
    ) -> Result<TtlSplitProbe> {
        if self.async_applies.snapshot().stopped {
            return Err(Error::Raft("TTL split owner is stopped".into()));
        }
        let command = RawTtlSplit::new(action)?;
        self.sm
            .lock()
            .expect("sm poisoned")
            .check_ttl_split_proposal(&command)?;
        self.peer.begin_ttl_split_confirmation(command, deadline)
    }

    pub fn propose_ttl_split(&self, prepared: PreparedTtlSplit) -> Result<RawTtlSubmission> {
        let reservation = self.async_applies.reserve(prepared.deadline())?;
        self.sm
            .lock()
            .expect("sm poisoned")
            .check_ttl_split_proposal(prepared.command())?;
        let at = self.metrics.proposal_submission.observe(
            || self.peer.propose_ttl_split_confirmed(prepared),
            classify_proposal,
        )?;
        Ok(self.raw_ttl_submission(at, reservation))
    }

    pub fn inspect_empty_ttl_import(&self, target: &DataRange) -> Result<EmptyTtlImport> {
        let engine = {
            let state = self.sm.lock().expect("sm poisoned");
            state.check_empty_ttl_import_target(target)?;
            state.engine().clone()
        };
        let snapshot = engine
            .try_positioned_resident_snapshot()
            .ok_or_else(|| Error::MetaNotReady("TTL import snapshot busy".into()))?;
        let position = snapshot.position().ok_or_else(|| {
            Error::MetaNotReady("TTL import requires an applied command barrier".into())
        })?;
        let view = snapshot.into_view();
        for cf in ColumnFamily::ALL {
            let count = view.resident_len(cf).ok_or_else(|| {
                Error::MetaNotReady("TTL import requires exact resident CF counts".into())
            })?;
            if count == 0 {
                continue;
            }
            if cf != ColumnFamily::Default || count != 1 {
                return Err(Error::MetaNotReady("TTL import target is not empty".into()));
            }
            let value = view
                .get_resident(cf, COMPACTION_CONFIRMED_KEY)
                .flatten()
                .filter(|value| value.len() == 16)
                .ok_or_else(|| Error::MetaNotReady("TTL import target is not empty".into()))?;
            let term = u64::from_be_bytes(value[..8].try_into().unwrap());
            let index = u64::from_be_bytes(value[8..].try_into().unwrap());
            if term == 0 || index == 0 || term > position.term || index > position.index {
                return Err(Error::Raft(
                    "TTL import target compaction floor is invalid".into(),
                ));
            }
        }
        Ok(EmptyTtlImport {
            target: target.clone(),
            position,
        })
    }
}
