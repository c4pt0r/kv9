//! Replay validates every ownership and TTL-activation transition in the WAL tail.
use super::*;
use kv9_engine::{Mutation, WriteBatch};

#[derive(Default, Clone)]
pub(super) struct TailState {
    range: Option<DataRange>,
    watermark: Option<ClockWatermark>,
}

fn put<'a>(m: &'a Mutation, family: ColumnFamily, wanted: &[u8]) -> Option<&'a [u8]> {
    match m {
        Mutation::Put { cf, key, value } if *cf == family && key == wanted => Some(value),
        _ => None,
    }
}
impl TailState {
    pub(super) fn from_image(
        range: Option<DataRange>,
        watermark: Option<ClockWatermark>,
        identity: DataCheckpointIdentity,
    ) -> Result<Self> {
        let state = Self { range, watermark };
        state.valid(identity)?;
        Ok(state)
    }
    fn valid(&self, identity: DataCheckpointIdentity) -> Result<()> {
        validate_local(
            (identity.root, identity.region, identity.creation),
            self.range.as_ref(),
            self.watermark,
        )
    }
    pub(super) fn batch(
        &mut self,
        batch: &WriteBatch,
        identity: DataCheckpointIdentity,
    ) -> Result<()> {
        for mutation in batch.mutations() {
            let (cf, key) = match mutation {
                Mutation::Put { cf, key, .. } | Mutation::Delete { cf, key } => (cf, key),
            };
            if *cf == ColumnFamily::Default && key == RANGE_KEY {
                let Some(value) = put(mutation, ColumnFamily::Default, RANGE_KEY) else {
                    return Err(invalid("WAL removes range ownership"));
                };
                let next = DataRange::decode(value)?;
                identity.check_range(&next)?;
                let accepted = match &self.range {
                    Some(current) if current == &next => true,
                    Some(current) => self.watermark.is_none() && next.may_follow(current),
                    None => !next.sealed && next.conf_ver == 1 && next.version == 1,
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
    pub(super) fn finish(&self, engine: &WalEngine) -> Result<()> {
        let range = engine
            .get(ColumnFamily::Default, RANGE_KEY)?
            .map(|b| DataRange::decode(&b))
            .transpose()?;
        let clock = engine
            .get(ColumnFamily::Default, CLOCK_KEY)?
            .map(|b| ClockWatermark::decode(&b))
            .transpose()?;
        if range != self.range || clock != self.watermark {
            return Err(invalid(
                "recovered state differs from observed WAL transitions",
            ));
        }
        Ok(())
    }
}
