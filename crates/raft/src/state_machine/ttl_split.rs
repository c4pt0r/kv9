//! One atomic value/expiry/cursor transition per transfer entry. No source
//! reads, scanning, local clock or network capability is available in apply.
use super::*;
use crate::command::{RawTtlSplit, RawTtlSplitAction};
use kv9_common::data_range::{DataRange, RANGE_KEY};
use kv9_common::raw_ttl::split::{validate_local, ImportState, SplitSeal, IMPORT_KEY, SEAL_KEY};
use kv9_common::raw_ttl::{ClockWatermark, TtlRejection, CLOCK_KEY};
use kv9_common::{Error, RegionId, RootDigest};

struct Local {
    range: Option<DataRange>,
    clock: Option<ClockWatermark>,
    seal: Option<SplitSeal>,
    import: Option<ImportState>,
}
impl<E: ApplyStore> MemStateMachine<E> {
    /// Checkpoint publication has its own import epoch gate. This never grants
    /// ordinary fenced writes or serving ownership to an unbound child.
    pub(crate) fn staging_checkpoint_epoch(
        &self,
        fence: &crate::RegionFence,
    ) -> Result<Option<bool>> {
        let Some(identity) = self.data_identity else {
            return Ok(None);
        };
        let local = self.ttl_split_local(identity)?;
        let Some(import) = local.import else {
            return Ok(None);
        };
        let target = import.spec.target();
        Ok(Some(
            local.range.as_ref().is_none_or(|r| r == target)
                && fence.region_id == target.region.0
                && fence.conf_ver == target.conf_ver
                && fence.version == target.version,
        ))
    }

    pub(crate) fn ttl_split_import_state(&self, target: &DataRange) -> Result<Option<ImportState>> {
        let identity = (target.root, target.region, target.creation);
        if self.data_identity != Some(identity) {
            return Err(Error::StaleEpoch {
                region: target.region,
            });
        }
        Ok(self.ttl_split_local(identity)?.import)
    }

    pub(crate) fn check_ttl_split_source(
        &self,
        plan: &kv9_common::raw_ttl::split::SplitPlan,
    ) -> Result<()> {
        let identity = (plan.parent.root, plan.parent.region, plan.parent.creation);
        if self.data_identity != Some(identity)
            || self
                .ttl_split_local(identity)?
                .seal
                .is_none_or(|s| s.plan != *plan)
        {
            return Err(Error::MetaNotReady(
                "TTL split source seal has not applied".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn check_ttl_split_proposal(&self, command: &RawTtlSplit) -> Result<()> {
        let target = command.target();
        let identity = (target.root, target.region, target.creation);
        if self.data_identity != Some(identity) {
            return Err(Error::StaleEpoch {
                region: target.region,
            });
        }
        let state = self.ttl_split_local(identity)?;
        let allowed = match command.action() {
            RawTtlSplitAction::Seal(plan) => {
                state.clock.is_some()
                    && (state.seal.as_ref().is_some_and(|s| s.plan == *plan)
                        || state.seal.is_none() && state.range.as_ref() == Some(&plan.parent))
            }
            RawTtlSplitAction::Begin { spec, empty_at } => match state.import {
                Some(import) => import.spec == *spec,
                None => {
                    state.range.is_none()
                        && state.clock.is_none()
                        && state.seal.is_none()
                        && self.engine.applied_position()?
                            == DurableAppliedPosition::AppliedThrough(*empty_at)
                }
            },
            RawTtlSplitAction::Append { expected, rows, .. } => state.import.is_some_and(|s| {
                s.spec.target() == target && s.digest() == *expected && s.append(rows).is_ok()
            }),
            RawTtlSplitAction::Finish { expected, .. } => state.import.is_some_and(|s| {
                if s.spec.target() != target {
                    return false;
                }
                let mut before = s.clone();
                before.complete = false;
                (s.digest() == *expected || s.complete && before.digest() == *expected)
                    && s.finish().is_ok()
            }),
        };
        if !allowed {
            return Err(Error::MetaNotReady(
                "TTL split state changed before proposal".into(),
            ));
        }
        Ok(())
    }
    pub(crate) fn check_empty_ttl_import_target(&self, target: &DataRange) -> Result<()> {
        target.validate()?;
        if target.sealed
            || target.conf_ver != 1
            || target.version != 1
            || self.data_identity != Some((target.root, target.region, target.creation))
        {
            return Err(Error::Raft(
                "TTL import target differs from the local empty creation".into(),
            ));
        }
        let state = self.ttl_split_local((target.root, target.region, target.creation))?;
        if state.range.is_some()
            || state.clock.is_some()
            || state.seal.is_some()
            || state.import.is_some()
        {
            return Err(Error::MetaNotReady(
                "TTL import target already has range or transfer state".into(),
            ));
        }
        Ok(())
    }
    fn ttl_split_local(&self, identity: (RootDigest, RegionId, RootDigest)) -> Result<Local> {
        let local = Local {
            range: self
                .engine
                .get(ColumnFamily::Default, RANGE_KEY)?
                .map(|b| DataRange::decode(&b))
                .transpose()?,
            clock: self
                .engine
                .get(ColumnFamily::Default, CLOCK_KEY)?
                .map(|b| ClockWatermark::decode(&b))
                .transpose()?,
            seal: self
                .engine
                .get(ColumnFamily::Default, SEAL_KEY)?
                .map(|b| SplitSeal::decode(&b))
                .transpose()?,
            import: self
                .engine
                .get(ColumnFamily::Default, IMPORT_KEY)?
                .map(|b| ImportState::decode(&b))
                .transpose()?,
        };
        validate_local(
            identity,
            local.range.as_ref(),
            local.clock,
            local.seal.as_ref(),
            local.import.as_ref(),
        )?;
        if self.ttl_enabled != local.clock.is_some() {
            return Err(Error::Raft(
                "TTL split clock differs from apply state".into(),
            ));
        }
        Ok(local)
    }
    pub(crate) fn check_ttl_split_local(
        &self,
        identity: (RootDigest, RegionId, RootDigest),
    ) -> Result<()> {
        self.ttl_split_local(identity).map(|_| ())
    }
    fn split_refuse(&mut self, at: AppliedPosition, region: RegionId) -> Result<ApplyResult> {
        self.engine.write_applied(WriteBatch::new(), at)?;
        self.applied = LogIndex(at.index);
        Ok(ApplyResult::fence_rejected(self.applied, region))
    }
    pub(super) fn apply_ttl_split(
        &mut self,
        at: AppliedPosition,
        command: &RawTtlSplit,
    ) -> Result<ApplyResult> {
        let target = command.target();
        let identity = (target.root, target.region, target.creation);
        if self.data_identity != Some(identity) {
            return self.split_refuse(at, target.region);
        }
        let local = self.ttl_split_local(identity)?;
        let mut batch = WriteBatch::new();
        let mut activates = false;
        match command.action() {
            RawTtlSplitAction::Seal(plan) => {
                if let Some(seal) = local.seal {
                    if seal.plan != *plan {
                        return self.split_refuse(at, target.region);
                    }
                } else {
                    if local.range.as_ref() != Some(&plan.parent) {
                        return self.split_refuse(at, target.region);
                    }
                    let Some(watermark) = local.clock else {
                        return self.ttl_refuse(at, target.region, TtlRejection::NotEnabled);
                    };
                    let seal = SplitSeal {
                        plan: plan.clone(),
                        cut: at,
                        watermark,
                    };
                    // Replay observes authority before its dependent range change.
                    batch.put(ColumnFamily::Default, SEAL_KEY.to_vec(), seal.encode());
                    batch.put(
                        ColumnFamily::Default,
                        RANGE_KEY.to_vec(),
                        plan.sealed_range()?.encode(),
                    );
                }
            }
            RawTtlSplitAction::Begin { spec, empty_at } => {
                if let Some(state) = local.import {
                    if state.spec != *spec {
                        return self.split_refuse(at, target.region);
                    }
                    // A retry never resets progress, time, or a completed child's data.
                } else {
                    if local.range.is_some()
                        || local.clock.is_some()
                        || local.seal.is_some()
                        || self.applied.0 != empty_at.index
                        || self.engine.applied_position()?
                            != DurableAppliedPosition::AppliedThrough(*empty_at)
                    {
                        return self.split_refuse(at, target.region);
                    }
                    // Empty-at is minted by the protected driver snapshot gate.
                    // All ordinary submissions are disabled; a decoded
                    // descriptor alone is never an empty-engine observation.
                    batch.put(
                        ColumnFamily::Default,
                        IMPORT_KEY.to_vec(),
                        ImportState::begin(spec.clone())?.encode(),
                    );
                    batch.put(
                        ColumnFamily::Default,
                        CLOCK_KEY.to_vec(),
                        spec.seal.watermark.encode().to_vec(),
                    );
                    activates = true;
                }
            }
            RawTtlSplitAction::Append { expected, rows, .. } => {
                let Some(state) = local.import else {
                    return self.split_refuse(at, target.region);
                };
                if state.spec.target() != target || state.complete || state.digest() != *expected {
                    return self.split_refuse(at, target.region);
                }
                let next = match state.append(rows) {
                    Ok(next) => next,
                    Err(_) => return self.split_refuse(at, target.region),
                };
                for row in rows {
                    batch.put(ColumnFamily::Default, row.key.clone(), row.value.clone());
                    match row.expiry {
                        Some(expiry) => batch.put(
                            ColumnFamily::Write,
                            row.key.clone(),
                            expiry.encode().to_vec(),
                        ),
                        None => batch.delete(ColumnFamily::Write, row.key.clone()),
                    };
                }
                batch.put(ColumnFamily::Default, IMPORT_KEY.to_vec(), next.encode());
            }
            RawTtlSplitAction::Finish { expected, .. } => {
                let Some(state) = local.import else {
                    return self.split_refuse(at, target.region);
                };
                if state.spec.target() != target {
                    return self.split_refuse(at, target.region);
                }
                if state.complete {
                    let mut before = state.clone();
                    before.complete = false;
                    if *expected != state.digest() && *expected != before.digest() {
                        return self.split_refuse(at, target.region);
                    }
                } else {
                    if state.digest() != *expected {
                        return self.split_refuse(at, target.region);
                    }
                    let next = match state.finish() {
                        Ok(next) => next,
                        Err(_) => return self.split_refuse(at, target.region),
                    };
                    batch.put(ColumnFamily::Default, IMPORT_KEY.to_vec(), next.encode());
                    batch.put(ColumnFamily::Default, RANGE_KEY.to_vec(), target.encode());
                }
            }
        }
        self.engine.write_applied(batch, at)?;
        self.applied = LogIndex(at.index);
        if activates {
            self.ttl_enabled = true;
        }
        Ok(ApplyResult::write_ok(self.applied))
    }
}

#[cfg(test)]
mod tests;
