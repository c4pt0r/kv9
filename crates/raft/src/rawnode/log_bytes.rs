//! Reservations belong to log positions, never to client futures or owners.
use super::*;
use kv9_common::log_budget::{LogDebt, ProposalClaim};
use std::collections::BTreeMap;

/// Conservative future local Raft + engine WAL growth for a data entry:
/// framing, duplicated TTL keys/expiry records, and one maximum topology
/// publication at rotation. Actual file writes are charged separately until
/// apply consumes the position, so admission deliberately double-counts that
/// overlap. This is not a filesystem/free-block or remote-follower guarantee.
pub(crate) fn proposal_bytes(encoded: usize) -> Result<u64> {
    let layout = kv9_engine::wal_stream::MAX_TOPOLOGY_BYTES as u64
        + (crate::command::ttl::MAX_MUTATIONS * (kv9_common::raw_ttl::EXPIRY_RECORD_BYTES + 64))
            as u64
        + kv9_common::raw_ttl::split::MAX_RECORD_BYTES as u64
        + 4096;
    (encoded as u64)
        .checked_mul(4)
        .and_then(|n| n.checked_add(layout))
        .ok_or_else(|| Error::Raft("proposal log reservation overflow".into()))
}

#[cfg(test)]
mod tests;

impl<S: PersistentRaftStorage> PeerInner<S> {
    pub(super) fn reserve_log_bytes(
        &self,
        region: RegionId,
        bytes: usize,
        data: bool,
    ) -> Result<Option<ProposalClaim>> {
        if data {
            if let Some(pressure) = &self.replica_pressure {
                let conf = self.raw.raft.prs().conf().to_conf_state();
                let members = conf
                    .voters
                    .iter()
                    .chain(&conf.voters_outgoing)
                    .chain(&conf.learners)
                    .chain(&conf.learners_next)
                    .copied();
                if pressure.refuses(members, proposal_bytes(bytes)?) {
                    return Err(Error::WriteBackpressure { region });
                }
            }
        }
        self.log_debt
            .as_ref()
            .filter(|_| data)
            .map(|debt| debt.reserve(proposal_bytes(bytes)?, region))
            .transpose()
    }
    pub(super) fn retain_proposed_bytes(
        &self,
        at: ProposedAt,
        bytes: usize,
        claim: Option<ProposalClaim>,
    ) -> Result<()> {
        if let Some(debt) = &self.log_debt {
            match claim {
                Some(claim) => claim.accept(debt, at.index.0, at.term),
                None => debt.retain_position(at.index.0, at.term, proposal_bytes(bytes)?)?,
            }
        }
        Ok(())
    }
    pub(super) fn retain_ready_bytes(&self, entries: &[Entry]) -> Result<()> {
        if let Some(debt) = &self.log_debt {
            for entry in entries
                .iter()
                .filter(|e| !e.data.is_empty() && e.index > self.applied_reported)
            {
                debt.retain_position(entry.index, entry.term, proposal_bytes(entry.data.len())?)?;
            }
        }
        Ok(())
    }
    pub(super) fn persisted_log_suffix(&self, entries: &[Entry]) -> Result<()> {
        if let (Some(debt), Some(first)) = (&self.log_debt, entries.first()) {
            let next = entries
                .iter()
                .filter(|e| !e.data.is_empty() && e.index > self.applied_reported)
                .map(|e| Ok(((e.index, e.term), proposal_bytes(e.data.len())?)))
                .collect::<Result<BTreeMap<_, _>>>()?;
            debt.replace_pending_suffix(first.index, next)?;
        }
        Ok(())
    }
}

impl<S: PersistentRaftStorage> RaftPeer<S> {
    pub fn set_replica_pressure(&self, pressure: Arc<crate::replica_pressure::ReplicaPressure>) {
        self.lock().replica_pressure = Some(pressure);
    }
    /// Driver supplies its actual recovered state-machine watermark, without
    /// nesting the engine/state-machine lock with this peer lock.
    pub(crate) fn attach_log_debt(&self, debt: LogDebt, applied: u64) -> Result<()> {
        let mut g = self.lock();
        g.check_fatal()?;
        if g.log_debt.is_some() || applied > g.raw.raft.raft_log.committed {
            return Err(Error::Raft(
                "invalid log budget attachment or recovered apply watermark".into(),
            ));
        }
        let log = &g.raw.raft.raft_log;
        let mut low = log.first_index().max(applied.saturating_add(1));
        let high = log
            .last_index()
            .checked_add(1)
            .ok_or_else(|| Error::Raft("log index exhausted".into()))?;
        let mut pending = BTreeMap::new();
        while low < high {
            let entries = log
                .slice(
                    low,
                    high,
                    Some(64 * 1024),
                    raft::GetEntriesContext::empty(false),
                )
                .map_err(raft_err)?;
            let last = entries
                .last()
                .ok_or_else(|| Error::Raft("empty log obligation recovery page".into()))?;
            low = last.index + 1;
            for e in entries.into_iter().filter(|e| !e.data.is_empty()) {
                pending.insert((e.index, e.term), proposal_bytes(e.data.len())?);
            }
        }
        debt.replace_pending_suffix(0, pending)?;
        g.log_debt = Some(debt);
        Ok(())
    }
}
