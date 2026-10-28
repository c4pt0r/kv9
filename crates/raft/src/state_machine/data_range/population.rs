use super::*;
use crate::command::RawPopulation;

impl<E: ApplyStore> MemStateMachine<E> {
    pub(crate) fn apply_raw_population(
        &mut self,
        at: AppliedPosition,
        frame: &RawPopulation,
    ) -> Result<ApplyResult> {
        let target = &frame.target;
        let identity = (target.root, target.region, target.creation);
        let current = if self.data_identity == Some(identity) {
            RangeFence {
                engine: self.engine.clone(),
                identity,
            }
            .current()?
        } else {
            self.engine.write_applied(WriteBatch::new(), at)?;
            self.applied = LogIndex(at.index);
            return Ok(ApplyResult::fence_rejected(self.applied, target.region));
        };
        // Range publication is terminal for population. A stale/retried batch
        // cannot race the first fenced client write or a later child seal.
        if current.is_some() || self.ttl_enabled {
            self.engine.write_applied(WriteBatch::new(), at)?;
            self.applied = LogIndex(at.index);
            return Ok(ApplyResult::fence_rejected(self.applied, target.region));
        }
        let mut batch = WriteBatch::new();
        for (key, value) in &frame.rows {
            batch.put(ColumnFamily::Default, key.clone(), value.clone());
        }
        self.engine.write_applied(batch, at)?;
        self.applied = LogIndex(at.index);
        Ok(ApplyResult::write_ok(self.applied))
    }
}

#[cfg(test)]
mod tests;
