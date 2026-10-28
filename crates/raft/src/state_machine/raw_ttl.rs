//! TTL effects are planned from this log's established state, without a clock,
//! scan, or external I/O capability. Each entry has one atomic positioned write.
use super::*;
use crate::command::{RawTtlAction, RawTtlCommand, RawTtlMutation};
use crate::{FencedInner, KvOp};
use kv9_common::codec::{decode_key, encode_key, KeyMode};
use kv9_common::data_range::{DataRange, RANGE_KEY};
use kv9_common::raw_ttl::{ClockWatermark, ExpiryRecord, TtlRejection, CLOCK_KEY};
use kv9_common::{Error, RegionId};

impl<E: ApplyStore> MemStateMachine<E> {
    pub(crate) fn raw_ttl_state(&self) -> Result<(DataRange, ClockWatermark)> {
        let range = self.raw_ttl_activation_range()?;
        let clock =
            self.engine
                .get(ColumnFamily::Default, CLOCK_KEY)?
                .ok_or(Error::RawTtlRejected {
                    region: range.region,
                    reason: TtlRejection::NotEnabled,
                })?;
        Ok((range, ClockWatermark::decode(&clock)?))
    }

    /// Leaf read for activation planning. The returned descriptor is not a
    /// serving permit; concurrent range sealing is adjudicated at ordered apply.
    pub(crate) fn raw_ttl_activation_range(&self) -> Result<DataRange> {
        let identity = self.data_identity.ok_or_else(|| {
            Error::MetaNotReady("Raw TTL activation requires a data group".into())
        })?;
        let range = self
            .engine
            .get(ColumnFamily::Default, RANGE_KEY)?
            .ok_or_else(|| Error::MetaNotReady("Raw TTL range is not applied".into()))?;
        let range = DataRange::decode(&range)?;
        if (range.root, range.region, range.creation) != identity {
            return Err(Error::Raft("Raw TTL found a foreign stored range".into()));
        }
        if range.sealed {
            return Err(Error::StaleEpoch {
                region: range.region,
            });
        }
        Ok(range)
    }

    pub(super) fn ttl_refuse(
        &mut self,
        at: AppliedPosition,
        region: RegionId,
        reason: TtlRejection,
    ) -> Result<ApplyResult> {
        self.engine.write_applied(WriteBatch::new(), at)?;
        self.applied = LogIndex(at.index);
        Ok(ApplyResult::raw_ttl_rejected(self.applied, region, reason))
    }

    pub(super) fn apply_raw_ttl(
        &mut self,
        at: AppliedPosition,
        command: &RawTtlCommand,
    ) -> Result<ApplyResult> {
        let identity = self.data_identity.ok_or_else(|| {
            Error::Raft("Raw TTL apply requires local data group identity".into())
        })?;
        let current = self
            .engine
            .get(ColumnFamily::Default, RANGE_KEY)?
            .map(|bytes| DataRange::decode(&bytes))
            .transpose()?;
        if current
            .as_ref()
            .is_some_and(|r| (r.root, r.region, r.creation) != identity)
        {
            return Err(Error::Raft("Raw TTL found a foreign stored range".into()));
        }
        let region = RegionId(command.fence.region_id);
        if (command.root, region, command.creation) != identity
            || current
                .as_ref()
                .is_some_and(|r| r.keyspace != command.keyspace)
        {
            return self.ttl_refuse(at, region, TtlRejection::InvalidScope);
        }
        let fresh = current.as_ref().is_some_and(|r| {
            !r.sealed
                && r.conf_ver == command.fence.conf_ver
                && r.version == command.fence.version
                && command.keys().all(|key| r.contains(key))
        });
        if !fresh {
            self.engine.write_applied(WriteBatch::new(), at)?;
            self.applied = LogIndex(at.index);
            return Ok(ApplyResult::fence_rejected(self.applied, region));
        }
        let prior = self
            .engine
            .get(ColumnFamily::Default, CLOCK_KEY)?
            .map(|bytes| ClockWatermark::decode(&bytes))
            .transpose()?;
        if prior.is_some() != self.ttl_enabled {
            return Err(Error::Raft(
                "Raw TTL activation differs from durable state".into(),
            ));
        }
        if prior.is_none() && !matches!(command.action, RawTtlAction::Enable { .. }) {
            return self.ttl_refuse(at, region, TtlRejection::NotEnabled);
        }
        let mut clock = prior.unwrap_or_default();
        let mut batch = WriteBatch::new();
        match &command.action {
            RawTtlAction::Enable { observed } | RawTtlAction::Observe { observed } => {
                clock = clock.merge(*observed);
            }
            RawTtlAction::Write { plan, mutations } => {
                clock = clock.merge(plan.observed());
                let expiry = plan.at_write(command.creation, at)?;
                for mutation in mutations {
                    match mutation {
                        RawTtlMutation::Put { key, value } => {
                            let key = encode_key(KeyMode::Raw, command.keyspace, key)?;
                            batch.put(ColumnFamily::Default, key.clone(), value.clone());
                            if let Some(expiry) = expiry {
                                batch.put(ColumnFamily::Write, key, expiry.encode().to_vec());
                            } else {
                                batch.delete(ColumnFamily::Write, key);
                            }
                        }
                        RawTtlMutation::Delete { key } => {
                            let key = encode_key(KeyMode::Raw, command.keyspace, key)?;
                            batch.delete(ColumnFamily::Default, key.clone());
                            batch.delete(ColumnFamily::Write, key);
                        }
                    }
                }
            }
            RawTtlAction::Cleanup { candidates } => {
                for (key, observed) in candidates {
                    let key = encode_key(KeyMode::Raw, command.keyspace, key)?;
                    let current = self
                        .engine
                        .get(ColumnFamily::Write, &key)?
                        .map(|bytes| ExpiryRecord::decode(&bytes))
                        .transpose()?;
                    if observed.permits_cleanup(current, clock) {
                        batch.delete(ColumnFamily::Default, key.clone());
                        batch.delete(ColumnFamily::Write, key);
                    }
                }
            }
        }
        if prior != Some(clock) {
            batch.put(
                ColumnFamily::Default,
                CLOCK_KEY.to_vec(),
                clock.encode().to_vec(),
            );
        }
        self.engine.write_applied(batch, at)?;
        self.applied = LogIndex(at.index);
        self.ttl_enabled = true;
        Ok(ApplyResult::write_ok(self.applied))
    }

    /// Old writes cannot skip expiry removal or manufacture activation. The
    /// default-CF-only Raw grouper cannot touch either reserved namespace;
    /// activation itself is a barrier and disables that grouper afterwards.
    pub(super) fn guard_legacy_ttl(
        &mut self,
        at: AppliedPosition,
        command: &Command,
    ) -> Result<Option<ApplyResult>> {
        let reserved = |cf: u8, key: &[u8]| {
            (cf == crate::cf_code(ColumnFamily::Default)
                && (key == CLOCK_KEY
                    || key == kv9_common::raw_ttl::split::SEAL_KEY
                    || key == kv9_common::raw_ttl::split::IMPORT_KEY))
                || (cf == crate::cf_code(ColumnFamily::Write)
                    && decode_key(key).is_ok_and(|key| key.mode == KeyMode::Raw))
        };
        let reject = match command {
            Command::Put { cf, key, .. } => self.ttl_enabled || reserved(*cf, key),
            Command::Write { ops }
            | Command::CatalogTxn { ops }
            | Command::Fenced {
                inner: FencedInner::Write { ops },
                ..
            } => {
                self.ttl_enabled
                    || ops.iter().any(|op| match op {
                        KvOp::Put { cf, key, .. } | KvOp::Delete { cf, key } => reserved(*cf, key),
                    })
            }
            _ => return Ok(None),
        };
        if !reject {
            return Ok(None);
        }
        if self.ttl_enabled {
            let (_, region, _) = self.data_identity.ok_or_else(|| {
                Error::Raft("TTL-enabled engine lacks local data group identity".into())
            })?;
            return self
                .ttl_refuse(at, region, TtlRejection::LegacyCommand)
                .map(Some);
        }
        Err(Error::Raft(
            "legacy command targets reserved Raw TTL metadata".into(),
        ))
    }
}

#[cfg(test)]
mod tests;
