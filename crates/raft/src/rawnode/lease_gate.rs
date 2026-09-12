//! Private installation in the serialized peer. No replacement/uninstall path.

use super::lease_wire::{self, Kind};
use super::*;
use crate::lease::{
    Authority, ClockReading, FixedConfiguration, Grant, LeaderLease, Mode, Progress, Refused,
    Renewal, Timing, VoterLease,
};

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
    configuration: FixedConfiguration,
    timing: Timing,
    last_clock: ClockReading,
    fenced: bool,
    owner: Arc<()>,
    leader: Option<LeaderLease>,
    leader_term: u64,
    next_renewal: u64,
    request: Option<Renewal>,
    grant: Option<Grant>,
    confirmed: Option<Renewal>,
    captured: u64,
}

/// Created only while persisting a peer's Ready under its lock. Neither Clone
/// nor Copy; publishing consumes it through the uniquely owned DrainToken.
pub(super) struct LeaseBatch {
    owner: Arc<()>,
    sequence: u64,
    request: Option<Renewal>,
    grant: Option<Grant>,
    confirmed: Option<Renewal>,
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
        let now = clock.sample().map_err(refusal)?;
        let voter = VoterLease::recover(
            node.0,
            region.0,
            configuration.clone(),
            timing,
            now,
            raw.raft.term,
        )
        .map_err(refusal)?;
        Ok(Self {
            epoch,
            clock,
            voter,
            configuration,
            timing,
            last_clock: now,
            fenced: false,
            owner: Arc::new(()),
            leader: None,
            leader_term: 0,
            next_renewal: 0,
            request: None,
            grant: None,
            confirmed: None,
            captured: 0,
        })
    }

    pub(super) fn fence(&mut self) {
        self.fenced = true;
        self.voter.fence();
        if let Some(leader) = &mut self.leader {
            leader.fence();
        }
        self.request = None;
        self.grant = None;
        self.confirmed = None;
    }

    pub(super) fn incarnation(&self) -> u64 {
        self.epoch.incarnation
    }

    fn may_vote<S: PersistentRaftStorage>(&mut self, raw: &RawNode<S>) -> Result<()> {
        if !matches_configuration(raw, &self.epoch.policy) {
            self.fence();
            return Err(refusal(Refused::InvalidConfiguration));
        }
        let now = self.sample()?;
        self.voter.may_vote(now, raw.raft.term).map_err(refusal)
    }

    fn sample(&mut self) -> Result<ClockReading> {
        if self.fenced {
            return Err(refusal(Refused::Fenced));
        }
        let result = self.clock.sample().and_then(|now| {
            if now.domain != self.last_clock.domain {
                Err(Refused::ClockChanged)
            } else if now.nanos < self.last_clock.nanos {
                Err(Refused::ClockRegressed)
            } else {
                Ok(now)
            }
        });
        match result {
            Ok(now) => {
                self.last_clock = now;
                Ok(now)
            }
            Err(reason) => {
                self.fence();
                Err(refusal(reason))
            }
        }
    }

    pub(super) fn revoke_leader(&mut self) {
        if let Some(leader) = &mut self.leader {
            if leader.mode() == Mode::Leader {
                leader.revoke();
            }
        }
        self.request = None;
        self.confirmed = None;
    }

    fn synchronize<S: PersistentRaftStorage>(&mut self, raw: &RawNode<S>) -> Result<ClockReading> {
        if !matches_configuration(raw, &self.epoch.policy) {
            self.fence();
            return Err(refusal(Refused::InvalidConfiguration));
        }
        let now = self.sample()?;
        if raw.raft.state != StateRole::Leader {
            self.revoke_leader();
        } else if raw.raft.term > self.leader_term {
            let p = &self.epoch.policy;
            self.leader = Some(
                LeaderLease::new(
                    self.configuration.clone(),
                    Authority {
                        group: p.group,
                        configuration: p.configuration,
                        leader: p.node,
                        term: raw.raft.term,
                        incarnation: u128::from(self.epoch.incarnation),
                    },
                    self.timing,
                    now,
                )
                .map_err(refusal)?,
            );
            self.leader_term = raw.raft.term;
            self.next_renewal = now.nanos;
            self.request = None;
            self.confirmed = None;
        } else if raw.raft.term != self.leader_term || self.leader.is_none() {
            self.fence();
            return Err(refusal(Refused::WrongAuthority));
        }
        Ok(now)
    }

    fn progress<S: PersistentRaftStorage>(&self, raw: &RawNode<S>) -> Progress {
        Progress {
            configuration: self.epoch.policy.configuration,
            leader: raw.raft.leader_id,
            term: raw.raft.term,
            committed: raw.raft.raft_log.committed,
            // A zero here refuses the fast path rather than guessing an old term.
            committed_term: if raw.raft.commit_to_current_term() {
                raw.raft.term
            } else {
                0
            },
        }
    }

    pub(super) fn prepare<S: PersistentRaftStorage>(&mut self, raw: &RawNode<S>) {
        let Ok(now) = self.synchronize(raw) else {
            return;
        };
        if raw.raft.state != StateRole::Leader
            || now.nanos < self.next_renewal
            || !raw.raft.commit_to_current_term()
        {
            return;
        }
        let progress = self.progress(raw);
        let Some(leader) = &mut self.leader else {
            return;
        };
        let Ok(renewal) = leader.start(now, progress) else {
            return;
        };
        let Some(next) = now.nanos.checked_add((self.timing.leader_ns() / 2).max(1)) else {
            self.fence();
            return;
        };
        self.next_renewal = next;
        self.request = Some(renewal);
        self.confirmed = None;
        // Self is not a free ACK: it must make the same voting promise.
        if let Ok(grant) = self
            .voter
            .promise(now, raw.raft.term, raw.raft.leader_id, renewal)
        {
            if leader.acknowledge(now, grant).unwrap_or(false) {
                self.confirmed = Some(renewal);
            }
        }
    }

    pub(super) fn receive<S: PersistentRaftStorage>(
        &mut self,
        raw: &mut RawNode<S>,
        message: Message,
    ) {
        let Some((kind, renewal)) = lease_wire::decode(&message, &self.epoch.policy) else {
            return;
        };
        if self.fenced {
            return;
        }
        match kind {
            Kind::Request => {
                // Process only the leader/term observation, never an untrusted
                // commit frontier. Sending a leader's current commit directly
                // in this heartbeat could exceed a lagging follower's log.
                let heartbeat = Message {
                    msg_type: raft::eraftpb::MessageType::MsgHeartbeat,
                    from: message.from,
                    to: message.to,
                    term: message.term,
                    ..Default::default()
                };
                if raw.step(heartbeat).is_err() {
                    return;
                }
                let Ok(now) = self.synchronize(raw) else {
                    return;
                };
                if raw.raft.state != StateRole::Follower {
                    return;
                }
                if let Ok(grant) =
                    self.voter
                        .promise(now, raw.raft.term, raw.raft.leader_id, renewal)
                {
                    self.grant = Some(grant);
                }
            }
            Kind::Grant => {
                let Ok(now) = self.synchronize(raw) else {
                    return;
                };
                if raw.raft.state != StateRole::Leader || raw.raft.term != renewal.authority.term {
                    return;
                }
                if let Some(leader) = &mut self.leader {
                    if leader
                        .acknowledge(
                            now,
                            Grant {
                                renewal,
                                voter: message.from,
                            },
                        )
                        .unwrap_or(false)
                    {
                        self.confirmed = Some(renewal);
                    }
                }
            }
        }
    }

    pub(super) fn capture(&mut self) -> Option<LeaseBatch> {
        if self.fenced {
            return None;
        }
        let Some(sequence) = self.captured.checked_add(1) else {
            self.fence();
            return None;
        };
        self.captured = sequence;
        Some(LeaseBatch {
            owner: self.owner.clone(),
            sequence,
            request: self.request.take(),
            grant: self.grant.take(),
            confirmed: self.confirmed.take(),
        })
    }

    pub(super) fn complete<S: PersistentRaftStorage>(
        &mut self,
        raw: &RawNode<S>,
        batch: LeaseBatch,
    ) -> Result<Vec<Message>> {
        if !Arc::ptr_eq(&batch.owner, &self.owner) || batch.sequence != self.captured {
            return Err(Error::Raft(
                "foreign or superseded lease pump publication".into(),
            ));
        }
        let Ok(now) = self.synchronize(raw) else {
            return Ok(Vec::new());
        };
        let progress = self.progress(raw);
        let p = &self.epoch.policy;
        let mut messages = Vec::new();
        if let Some(grant) = batch.grant {
            // Rechecking a duplicate promise does not extend its deadline.
            if self
                .voter
                .promise(now, raw.raft.term, raw.raft.leader_id, grant.renewal)
                .is_ok()
            {
                messages.push(lease_wire::encode(
                    Kind::Grant,
                    grant.renewal,
                    p,
                    grant.renewal.authority.leader,
                ));
            }
        }
        if raw.raft.state == StateRole::Leader {
            if let Some(leader) = &mut self.leader {
                if let Some(request) = batch.request {
                    if leader.validate_renewal(now, progress, request).is_ok() {
                        messages.extend(
                            p.voters
                                .iter()
                                .filter(|id| **id != p.node)
                                .map(|id| lease_wire::encode(Kind::Request, request, p, *id)),
                        );
                    }
                }
                if let Some(confirmed) = batch.confirmed {
                    let _ = leader.publish(now, progress, confirmed);
                }
            }
        }
        Ok(messages)
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

    pub(super) fn prepare_lease(&mut self) {
        if let Some(lease) = &mut self.lease {
            lease.prepare(&self.raw);
        }
    }

    pub(super) fn receive_lease(&mut self, message: Message) {
        if let Some(lease) = &mut self.lease {
            lease.receive(&mut self.raw, message);
        }
    }
}

#[cfg(test)]
mod tests;
