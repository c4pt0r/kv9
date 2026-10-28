//! Admission for new data entries, serialized with the actual Raft append.
//! This is a retained-entry threshold, not a physical disk quota. Control
//! entries and follower replication must still advance when it is reached.
use super::*;

impl<S: PersistentRaftStorage> PeerInner<S> {
    fn retained_entries(&self) -> u64 {
        let log = &self.raw.raft.raft_log;
        log.last_index()
            .saturating_sub(log.first_index().saturating_sub(1))
    }

    pub(super) fn check_write_pressure(&self, region: RegionId) -> Result<()> {
        if self.max_write_log_entries != 0 && self.retained_entries() >= self.max_write_log_entries
        {
            return Err(Error::WriteBackpressure { region });
        }
        Ok(())
    }
}

impl<S: PersistentRaftStorage> RaftPeer<S> {
    /// Set this peer's local admission threshold. Zero explicitly disables it.
    /// Does not reject replay, Ready persistence or already accepted apply.
    /// Production sets it before starting each data group's driver; changing
    /// it later is also serialized with proposals and may stop new admissions.
    pub fn set_max_write_log_entries(&self, entries: u64) {
        self.lock().max_write_log_entries = entries;
    }

    /// Advisory early refusal only. A successful check reserves no capacity;
    /// typed data proposals recheck under the same lock as their append.
    pub fn check_write_pressure(&self) -> Result<()> {
        let g = self.lock();
        g.check_fatal()?;
        g.check_write_pressure(self.region)
    }
}

#[cfg(test)]
mod tests;
