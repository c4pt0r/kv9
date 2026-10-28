//! Replay validates complete transfer batches before accepting dependent state.
use super::*;
use kv9_common::raw_ttl::split::ImportRow;
use kv9_engine::{Mutation, WriteBatch};

#[derive(Default, Clone)]
pub(super) struct TailState {
    range: Option<DataRange>,
    watermark: Option<ClockWatermark>,
    seal: Option<SplitSeal>,
    import: Option<ImportState>,
}

#[cfg(test)]
mod tests;

fn put<'a>(m: &'a Mutation, family: ColumnFamily, wanted: &[u8]) -> Option<&'a [u8]> {
    match m {
        Mutation::Put { cf, key, value } if *cf == family && key == wanted => Some(value),
        _ => None,
    }
}
fn transfer_key(m: &Mutation) -> bool {
    let (cf, key) = match m {
        Mutation::Put { cf, key, .. } | Mutation::Delete { cf, key } => (cf, key),
    };
    *cf == ColumnFamily::Default && (key == SEAL_KEY || key == IMPORT_KEY)
}
impl TailState {
    pub(super) fn from_image(
        view: &dyn ReadView,
        range: Option<DataRange>,
        watermark: Option<ClockWatermark>,
        identity: DataCheckpointIdentity,
    ) -> Result<Self> {
        let state = Self {
            range,
            watermark,
            seal: view
                .get(ColumnFamily::Default, SEAL_KEY)?
                .map(|b| SplitSeal::decode(&b))
                .transpose()?,
            import: view
                .get(ColumnFamily::Default, IMPORT_KEY)?
                .map(|b| ImportState::decode(&b))
                .transpose()?,
        };
        state.valid(identity)?;
        Ok(state)
    }
    fn valid(&self, identity: DataCheckpointIdentity) -> Result<()> {
        validate_local(
            (identity.root, identity.region, identity.creation),
            self.range.as_ref(),
            self.watermark,
            self.seal.as_ref(),
            self.import.as_ref(),
        )
    }
    pub(super) fn batch(
        &mut self,
        batch: &WriteBatch,
        at: Option<AppliedPosition>,
        identity: DataCheckpointIdentity,
    ) -> Result<()> {
        if batch.mutations().iter().any(transfer_key) {
            return self.transfer(batch.mutations(), at, identity);
        }
        for mutation in batch.mutations() {
            let (cf, key) = match mutation {
                Mutation::Put { cf, key, .. } | Mutation::Delete { cf, key } => (cf, key),
            };
            if (self.seal.is_some() || self.import.as_ref().is_some_and(|i| !i.complete))
                && decode_key(key).is_ok_and(|k| k.mode == KeyMode::Raw)
            {
                return Err(invalid(
                    "Raw mutation bypasses TTL source seal or import cursor",
                ));
            }
            if *cf == ColumnFamily::Default && key == RANGE_KEY {
                let Some(value) = put(mutation, ColumnFamily::Default, RANGE_KEY) else {
                    return Err(invalid("WAL removes range ownership"));
                };
                let next = DataRange::decode(value)?;
                identity.check_range(&next)?;
                let accepted = match &self.range {
                    Some(current) if current == &next => true,
                    Some(current) => self.watermark.is_none() && next.may_follow(current),
                    None => {
                        self.import.is_none()
                            && !next.sealed
                            && next.conf_ver == 1
                            && next.version == 1
                    }
                };
                if !accepted {
                    return Err(invalid("WAL tail violates the range transition"));
                }
                self.range = Some(next);
            }
            if *cf == ColumnFamily::Default && key == CLOCK_KEY {
                let Some(value) = put(mutation, ColumnFamily::Default, CLOCK_KEY) else {
                    return Err(invalid("WAL removes TTL activation"));
                };
                let next = ClockWatermark::decode(value)?;
                if self.watermark.is_some_and(|prior| next < prior) {
                    return Err(invalid("WAL tail regresses the TTL watermark"));
                }
                self.watermark = Some(next);
            }
            // Normal grouped apply may contain several legacy commands. No
            // later mutation can repair an illegal intermediate transition.
            self.valid(identity)?;
        }
        Ok(())
    }
    fn transfer(
        &mut self,
        mutations: &[Mutation],
        at: Option<AppliedPosition>,
        identity: DataCheckpointIdentity,
    ) -> Result<()> {
        let bad = || invalid("noncanonical or invalid TTL split WAL batch");
        let first = mutations.first().ok_or_else(bad)?;
        if let Some(bytes) = put(first, ColumnFamily::Default, SEAL_KEY) {
            let seal = SplitSeal::decode(bytes)?;
            let range = seal.plan.sealed_range()?;
            if self.seal.is_some()
                || self.range.as_ref() != Some(&seal.plan.parent)
                || self.watermark != Some(seal.watermark)
                || at != Some(seal.cut)
                || mutations.len() != 2
                || put(&mutations[1], ColumnFamily::Default, RANGE_KEY)
                    != Some(range.encode().as_slice())
            {
                return Err(bad());
            }
            self.seal = Some(seal);
            self.range = Some(range);
        } else if let Some(bytes) = put(first, ColumnFamily::Default, IMPORT_KEY) {
            let state = ImportState::decode(bytes)?;
            if mutations.len() != 2 {
                return Err(bad());
            }
            if let Some(prior) = &self.import {
                if prior.complete
                    || prior.finish()? != state
                    || put(&mutations[1], ColumnFamily::Default, RANGE_KEY)
                        != Some(state.spec.target().encode().as_slice())
                {
                    return Err(bad());
                }
                self.range = Some(state.spec.target().clone());
            } else {
                if self.range.is_some()
                    || self.seal.is_some()
                    || self.watermark.is_some()
                    || ImportState::begin(state.spec.clone())? != state
                    || put(&mutations[1], ColumnFamily::Default, CLOCK_KEY)
                        != Some(state.spec.seal.watermark.encode().as_slice())
                {
                    return Err(bad());
                }
                self.watermark = Some(state.spec.seal.watermark);
            }
            self.import = Some(state);
        } else {
            let prior = self
                .import
                .as_ref()
                .filter(|i| !i.complete)
                .ok_or_else(bad)?;
            let (last, pairs) = mutations.split_last().ok_or_else(bad)?;
            let next =
                ImportState::decode(put(last, ColumnFamily::Default, IMPORT_KEY).ok_or_else(bad)?)?;
            if pairs.is_empty()
                || pairs.len() % 2 != 0
                || pairs.len() / 2 > kv9_common::raw_ttl::split::MAX_PAGE_ROWS
            {
                return Err(bad());
            }
            let mut rows = Vec::with_capacity(pairs.len() / 2);
            for pair in pairs.chunks_exact(2) {
                let Mutation::Put {
                    cf: ColumnFamily::Default,
                    key,
                    value,
                } = &pair[0]
                else {
                    return Err(bad());
                };
                let expiry = match &pair[1] {
                    Mutation::Put {
                        cf: ColumnFamily::Write,
                        key: expiry_key,
                        value,
                    } if expiry_key == key => Some(ExpiryRecord::decode(value)?),
                    Mutation::Delete {
                        cf: ColumnFamily::Write,
                        key: expiry_key,
                    } if expiry_key == key => None,
                    _ => return Err(bad()),
                };
                rows.push(ImportRow {
                    key: key.clone(),
                    value: value.clone(),
                    expiry,
                });
            }
            if prior.append(&rows)? != next {
                return Err(bad());
            }
            self.import = Some(next);
        }
        self.valid(identity)
    }
    pub(super) fn finish(&self, engine: &WalEngine) -> Result<()> {
        let range = engine
            .get(ColumnFamily::Default, RANGE_KEY)?
            .map(|b| DataRange::decode(&b))
            .transpose()?;
        let clock = engine
            .get(ColumnFamily::Default, CLOCK_KEY)?
            .map(|b| ClockWatermark::decode(&b))
            .transpose()?;
        let seal = engine
            .get(ColumnFamily::Default, SEAL_KEY)?
            .map(|b| SplitSeal::decode(&b))
            .transpose()?;
        let import = engine
            .get(ColumnFamily::Default, IMPORT_KEY)?
            .map(|b| ImportState::decode(&b))
            .transpose()?;
        if range != self.range
            || clock != self.watermark
            || seal != self.seal
            || import != self.import
        {
            return Err(invalid(
                "recovered state differs from observed WAL transitions",
            ));
        }
        Ok(())
    }
}
