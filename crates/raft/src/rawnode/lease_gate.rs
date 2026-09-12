//! Private installation in the serialized peer. No replacement/uninstall path.

use super::*;
use crate::lease::{ClockReading, FixedConfiguration, Refused, Timing, VoterLease};

/// Clock supplied to the experimental adapter. A monotonic sample alone does
/// not establish the required real-time rate bound (including process pauses).
/// Sampling failures permanently fence voting in this peer incarnation.
pub trait LeaseClock: Send + Sync {
    fn sample(&self) -> crate::lease::Result<ClockReading>;
}

pub(super) struct InstalledLease {
    epoch: LeaseEpoch,
    clock: Arc<dyn LeaseClock>,
    voter: VoterLease,
}

impl InstalledLease {
    pub(super) fn install<S: PersistentRaftStorage>(
        node: NodeId,
        region: RegionId,
        raw: &RawNode<S>,
        policy: LeasePolicy,
        clock: Arc<dyn LeaseClock>,
    ) -> Result<Self> {
        policy.validate()?;
        if policy.node != node.0 || policy.group != region.0 {
            return Err(Error::Raft("lease policy names another peer".into()));
        }
        let configuration = FixedConfiguration::new(policy.configuration, policy.voters.clone())
            .map_err(refusal)?;
        let timing =
            Timing::new(policy.promise_ns, policy.drift_ppb, policy.margin_ns).map_err(refusal)?;
        if !matches_configuration(raw, &policy) {
            return Err(refusal(Refused::InvalidConfiguration));
        }
        // The exact durable epoch is minted by the exclusively owned store,
        // never supplied as a reusable constructor argument. A failed clock
        // sample leaves the durable opt-in, so ordinary reopen still refuses.
        let epoch = raw.store().begin_lease_incarnation(&policy)?;
        if epoch.policy != policy || epoch.incarnation == 0 {
            return Err(Error::Raft(
                "storage returned an invalid lease epoch".into(),
            ));
        }
        let voter = VoterLease::recover(
            node.0,
            region.0,
            configuration,
            timing,
            clock.sample().map_err(refusal)?,
            raw.raft.term,
        )
        .map_err(refusal)?;
        Ok(Self {
            epoch,
            clock,
            voter,
        })
    }

    pub(super) fn fence(&mut self) {
        self.voter.fence();
    }

    pub(super) fn incarnation(&self) -> u64 {
        self.epoch.incarnation
    }

    fn may_vote<S: PersistentRaftStorage>(&mut self, raw: &RawNode<S>) -> Result<()> {
        if !matches_configuration(raw, &self.epoch.policy) {
            self.fence();
            return Err(refusal(Refused::InvalidConfiguration));
        }
        let now = self.clock.sample().map_err(|cause| {
            self.fence();
            refusal(cause)
        })?;
        self.voter.may_vote(now, raw.raft.term).map_err(refusal)
    }
}

fn matches_configuration<S: PersistentRaftStorage>(raw: &RawNode<S>, policy: &LeasePolicy) -> bool {
    let cs = raw.raft.prs().conf().to_conf_state();
    let mut voters = cs.voters.to_vec();
    voters.sort_unstable();
    voters == policy.voters
        && cs.voters_outgoing.is_empty()
        && cs.learners_next.is_empty()
        && !cs.auto_leave
}

fn refusal(reason: Refused) -> Error {
    Error::Raft(format!("lease voting refused: {reason:?}"))
}

impl<S: PersistentRaftStorage> PeerInner<S> {
    pub(super) fn lease_may_vote(&mut self) -> Result<()> {
        match &mut self.lease {
            Some(lease) => lease.may_vote(&self.raw),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests;
