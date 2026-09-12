//! Fixed-configuration lease protocol component, pending runtime refinement.
//!
//! This module does not select raft-rs LeaseBased, send network messages, mint
//! a `ReadBarrier`, or qualify a physical clock. The adapter must supply truthful
//! observations under its authority lock and bind decisions to successful
//! persistence/pump publication and the exact immutable applied view.
//! See `docs/LEASE-AUTHORITY-MODEL.md` for the proved abstract protocol.

use std::collections::BTreeSet;

const RATE_SCALE: u128 = 1_000_000_000;
const MAX_VOTERS: usize = 64;

/// Refusal of the lease fast path. These are never successful empty reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    InvalidConfiguration,
    InvalidTiming,
    Overflow,
    ClockChanged,
    ClockRegressed,
    ClockUnavailable,
    Fenced,
    Revoked,
    WrongAuthority,
    NoCurrentTermCommit,
    CommitRegressed,
    NoCertificate,
    Expired,
    Deadline,
    WrongRound,
    WrongVoter,
    NoQuorum,
    ApplyLag,
    InvalidView,
    RecoveryQuarantine,
    VotingPromise,
}

pub type Result<T> = std::result::Result<T, Refused>;

/// Integer nanosecond bounds for one stable cluster lease policy.
///
/// Rates lie in [1-rho, 1+rho], rho = drift_ppb / 1e9. The margin must cover
/// the qualified clock's sampling/quantization error. Supplying a number does
/// not establish that a platform actually satisfies those assumptions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    promise_ns: u64,
    leader_ns: u64,
    recovery_ns: u64,
}

impl Timing {
    pub fn new(promise_ns: u64, drift_ppb: u32, margin_ns: u64) -> Result<Self> {
        if promise_ns == 0 || u128::from(drift_ppb) >= RATE_SCALE {
            return Err(Refused::InvalidTiming);
        }
        let a = RATE_SCALE - u128::from(drift_ppb);
        let b = RATE_SCALE + u128::from(drift_ppb);
        // A u64 times a value below 2e9 fits u128. Floor leader time;
        // ceil recovery time, then reserve the supplied error margin.
        let leader = (u128::from(promise_ns) * a / b)
            .checked_sub(u128::from(margin_ns))
            .filter(|n| *n > 0)
            .ok_or(Refused::InvalidTiming)?;
        let recovery = (u128::from(promise_ns) * b)
            .div_ceil(a)
            .checked_add(u128::from(margin_ns))
            .ok_or(Refused::Overflow)?;
        Ok(Self {
            promise_ns,
            leader_ns: u64::try_from(leader).map_err(|_| Refused::Overflow)?,
            recovery_ns: u64::try_from(recovery).map_err(|_| Refused::Overflow)?,
        })
    }

    pub fn promise_ns(self) -> u64 {
        self.promise_ns
    }

    pub fn leader_ns(self) -> u64 {
        self.leader_ns
    }

    pub fn recovery_ns(self) -> u64 {
        self.recovery_ns
    }
}

/// A sample from one qualified local monotonic clock domain.
/// No timestamp from another node is compared with these nanoseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockReading {
    pub domain: u128,
    pub nanos: u64,
}

#[derive(Debug)]
struct ClockFence {
    last: ClockReading,
}

impl ClockFence {
    fn new(now: ClockReading) -> Result<Self> {
        if now.domain == 0 {
            return Err(Refused::ClockChanged);
        }
        Ok(Self { last: now })
    }

    fn observe(&mut self, now: ClockReading) -> Result<u64> {
        if now.domain != self.last.domain {
            return Err(Refused::ClockChanged);
        }
        if now.nanos < self.last.nanos {
            return Err(Refused::ClockRegressed);
        }
        self.last = now;
        Ok(now.nanos)
    }
}

/// An immutable voting configuration. Its identifier must never be reused for
/// different membership or lease-policy parameters, including across upgrades.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedConfiguration {
    id: u128,
    voters: Box<[u64]>,
}

impl FixedConfiguration {
    pub fn new(id: u128, mut voters: Vec<u64>) -> Result<Self> {
        if id == 0 || voters.is_empty() || voters.len() > MAX_VOTERS {
            return Err(Refused::InvalidConfiguration);
        }
        voters.sort_unstable();
        if voters[0] == 0 || voters.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(Refused::InvalidConfiguration);
        }
        Ok(Self {
            id,
            voters: voters.into_boxed_slice(),
        })
    }

    pub fn id(&self) -> u128 {
        self.id
    }

    pub fn voters(&self) -> &[u64] {
        &self.voters
    }

    fn contains(&self, node: u64) -> bool {
        self.voters.binary_search(&node).is_ok()
    }

    fn quorum(&self) -> usize {
        self.voters.len() / 2 + 1
    }
}

/// One established Raft leader incarnation. Group zero is valid for metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Authority {
    pub group: u64,
    pub configuration: u128,
    pub leader: u64,
    pub term: u64,
    pub incarnation: u128,
}

/// Exact renewal identity; the leader's send-time deadline is kept locally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Renewal {
    pub authority: Authority,
    pub generation: u64,
    pub sequence: u64,
    pub promise_ns: u64,
}

/// A grant decision, not permission to send before the owning pump succeeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grant {
    pub renewal: Renewal,
    pub voter: u64,
}

/// Current Raft observations captured under the adapter's authority lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub configuration: u128,
    pub leader: u64,
    pub term: u64,
    pub committed: u64,
    pub committed_term: u64,
}

#[derive(Debug, Clone, Copy)]
struct Certificate {
    renewal: Renewal,
    end: u64,
}

#[derive(Debug)]
struct Pending {
    certificate: Certificate,
    acknowledgements: BTreeSet<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Leader,
    Revoked,
    Fenced,
}

/// An invocation's captured certificate/frontier/budget; deliberately not Clone
/// or Copy. Final validation consumes it even when the fast path is refused.
#[derive(Debug)]
pub struct ReadTicket {
    certificate: Certificate,
    clock_domain: u128,
    committed: u64,
    deadline: u64,
}

// Reusing one invocation's ticket for another invocation must remain a
// compile-time error, including if a future edit adds a Clone/Copy derive.
#[allow(dead_code)]
const _: () = {
    struct Probe<T>(core::marker::PhantomData<T>);
    trait Fallback {
        const CHECK: () = ();
    }
    impl<T> Fallback for Probe<T> {}
    impl<T: Clone> Probe<T> {
        const CHECK: () = panic!("ReadTicket must be neither Clone nor Copy");
    }
    Probe::<ReadTicket>::CHECK
};

/// A successful algorithm decision. This is not the driver's `ReadBarrier` and
/// cannot establish a production read view by itself.
#[derive(Debug, PartialEq, Eq)]
pub struct ReadDecision {
    committed: u64,
    view_index: u64,
}

impl ReadDecision {
    pub fn committed(&self) -> u64 {
        self.committed
    }

    pub fn view_index(&self) -> u64 {
        self.view_index
    }
}

/// Bounded leader-side state: one pending renewal and one active certificate.
/// A read may retain an older certificate while the next round is published.
#[derive(Debug)]
pub struct LeaderLease {
    configuration: FixedConfiguration,
    authority: Authority,
    timing: Timing,
    clock: ClockFence,
    mode: Mode,
    generation: u64,
    sequence: u64,
    known: u64,
    pending: Option<Pending>,
    active: Option<Certificate>,
}

impl LeaderLease {
    pub fn new(
        configuration: FixedConfiguration,
        authority: Authority,
        timing: Timing,
        now: ClockReading,
    ) -> Result<Self> {
        if authority.configuration != configuration.id
            || !configuration.contains(authority.leader)
            || authority.term == 0
            || authority.incarnation == 0
        {
            return Err(Refused::WrongAuthority);
        }
        Ok(Self {
            configuration,
            authority,
            timing,
            clock: ClockFence::new(now)?,
            mode: Mode::Leader,
            generation: 0,
            sequence: 0,
            known: 0,
            pending: None,
            active: None,
        })
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Fatal errors, transfer or an incarnation/configuration change cannot
    /// later resurrect this controller's certificates.
    pub fn fence(&mut self) {
        self.mode = Mode::Fenced;
        self.generation = u64::MAX;
        self.pending = None;
        self.active = None;
    }

    pub fn revoke(&mut self) {
        if self.mode != Mode::Leader {
            return;
        }
        self.pending = None;
        self.active = None;
        self.generation += 1; // Live generations are strictly below MAX.
        if self.generation == u64::MAX {
            self.fence();
        } else {
            self.mode = Mode::Revoked;
        }
    }

    pub fn rearm(&mut self, now: ClockReading, progress: Progress) -> Result<()> {
        self.observe(now)?;
        self.progress(progress)?;
        match self.mode {
            Mode::Revoked => {
                self.mode = Mode::Leader;
                Ok(())
            }
            Mode::Leader => Ok(()),
            Mode::Fenced => Err(Refused::Fenced),
        }
    }

    fn observe(&mut self, now: ClockReading) -> Result<u64> {
        if self.mode == Mode::Fenced {
            return Err(Refused::Fenced);
        }
        match self.clock.observe(now) {
            Ok(time) => Ok(time),
            Err(reason) => {
                self.fence();
                Err(reason)
            }
        }
    }

    fn progress(&mut self, progress: Progress) -> Result<()> {
        if progress.configuration != self.authority.configuration
            || progress.leader != self.authority.leader
            || progress.term != self.authority.term
        {
            self.fence();
            return Err(Refused::WrongAuthority);
        }
        if progress.committed < self.known {
            self.fence();
            return Err(Refused::CommitRegressed);
        }
        self.known = progress.committed;
        if progress.committed == 0 || progress.committed_term != self.authority.term {
            return Err(Refused::NoCurrentTermCommit);
        }
        Ok(())
    }

    fn live(&self) -> Result<()> {
        match self.mode {
            Mode::Leader => Ok(()),
            Mode::Revoked => Err(Refused::Revoked),
            Mode::Fenced => Err(Refused::Fenced),
        }
    }

    /// Call before sending the request; no received ACK may rebase this time.
    pub fn start(&mut self, now: ClockReading, progress: Progress) -> Result<Renewal> {
        let time = self.observe(now)?;
        self.progress(progress)?;
        self.live()?;
        let Some(end) = time.checked_add(self.timing.leader_ns) else {
            self.fence();
            return Err(Refused::Overflow);
        };
        let Some(sequence) = self.sequence.checked_add(1) else {
            self.fence();
            return Err(Refused::Overflow);
        };
        self.sequence = sequence;
        let renewal = Renewal {
            authority: self.authority,
            generation: self.generation,
            sequence,
            promise_ns: self.timing.promise_ns,
        };
        self.pending = Some(Pending {
            certificate: Certificate { renewal, end },
            acknowledgements: BTreeSet::new(),
        });
        Ok(renewal)
    }

    /// Collect exact grants without publishing read authority. Self counts
    /// only after the local voter actually made the same promise.
    pub fn acknowledge(&mut self, now: ClockReading, grant: Grant) -> Result<bool> {
        let time = self.observe(now)?;
        self.live()?;
        if !self.configuration.contains(grant.voter) {
            return Err(Refused::WrongVoter);
        }
        let pending = self.pending.as_mut().ok_or(Refused::WrongRound)?;
        if pending.certificate.renewal != grant.renewal {
            return Err(Refused::WrongRound);
        }
        if time >= pending.certificate.end {
            return Err(Refused::Expired);
        }
        pending.acknowledgements.insert(grant.voter);
        Ok(pending.acknowledgements.len() >= self.configuration.quorum())
    }

    /// Separate successful-publication event. The adapter must call this only
    /// after the whole owning pump succeeds; this method emits no network I/O.
    pub fn publish(
        &mut self,
        now: ClockReading,
        progress: Progress,
        renewal: Renewal,
    ) -> Result<()> {
        let time = self.observe(now)?;
        self.progress(progress)?;
        self.live()?;
        let pending = self.pending.as_ref().ok_or(Refused::WrongRound)?;
        if pending.certificate.renewal != renewal {
            return Err(Refused::WrongRound);
        }
        if time >= pending.certificate.end {
            return Err(Refused::Expired);
        }
        if pending.acknowledgements.len() < self.configuration.quorum() {
            return Err(Refused::NoQuorum);
        }
        self.active = Some(pending.certificate);
        self.pending = None;
        Ok(())
    }

    /// Check an outbound request against its original pending round/deadline.
    /// A delayed pump may consume all usable time before any send is allowed.
    pub fn validate_renewal(
        &mut self,
        now: ClockReading,
        progress: Progress,
        renewal: Renewal,
    ) -> Result<()> {
        let time = self.observe(now)?;
        self.progress(progress)?;
        self.live()?;
        let pending = self.pending.as_ref().ok_or(Refused::WrongRound)?;
        if pending.certificate.renewal != renewal {
            return Err(Refused::WrongRound);
        }
        if time >= pending.certificate.end {
            return Err(Refused::Expired);
        }
        Ok(())
    }

    pub fn begin_read(
        &mut self,
        now: ClockReading,
        progress: Progress,
        request_deadline: u64,
    ) -> Result<ReadTicket> {
        let time = self.observe(now)?;
        self.progress(progress)?;
        self.live()?;
        if time >= request_deadline {
            return Err(Refused::Deadline);
        }
        let certificate = self.active.ok_or(Refused::NoCertificate)?;
        if time >= certificate.end {
            return Err(Refused::Expired);
        }
        Ok(ReadTicket {
            certificate,
            clock_domain: now.domain,
            committed: progress.committed,
            deadline: request_deadline,
        })
    }

    /// Validate after acquiring the exact immutable view and its position,
    /// with the clock sampled after entering the authority gate. `view_index`
    /// must describe that retained view, not an unrelated applied watermark.
    pub fn finish_read(
        &mut self,
        now: ClockReading,
        progress: Progress,
        ticket: ReadTicket,
        view_index: u64,
    ) -> Result<ReadDecision> {
        let time = self.observe(now)?;
        self.progress(progress)?;
        self.live()?;
        if ticket.certificate.renewal.authority != self.authority
            || ticket.certificate.renewal.generation != self.generation
            || ticket.clock_domain != now.domain
        {
            return Err(Refused::WrongAuthority);
        }
        if time >= ticket.deadline {
            return Err(Refused::Deadline);
        }
        if time >= ticket.certificate.end {
            return Err(Refused::Expired);
        }
        if view_index < ticket.committed {
            return Err(Refused::ApplyLag);
        }
        if view_index > progress.committed {
            return Err(Refused::InvalidView);
        }
        Ok(ReadDecision {
            committed: ticket.committed,
            view_index,
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct HeldGrant {
    renewal: Renewal,
    end: u64,
}

/// Voter-side promise. Every process start conservatively quarantines votes
/// and grants, even when no volatile lease state survived. The stable policy
/// must cover every grant the previous incarnation could have acknowledged.
#[derive(Debug)]
pub struct VoterLease {
    node: u64,
    group: u64,
    configuration: FixedConfiguration,
    timing: Timing,
    clock: ClockFence,
    term: u64,
    hold_until: u64,
    recovery_until: u64,
    last_grant: Option<HeldGrant>,
    fenced: bool,
}

impl VoterLease {
    pub fn recover(
        node: u64,
        group: u64,
        configuration: FixedConfiguration,
        timing: Timing,
        now: ClockReading,
        durable_term: u64,
    ) -> Result<Self> {
        if !configuration.contains(node) {
            return Err(Refused::WrongVoter);
        }
        let recovery_until = now
            .nanos
            .checked_add(timing.recovery_ns)
            .ok_or(Refused::Overflow)?;
        Ok(Self {
            node,
            group,
            configuration,
            timing,
            clock: ClockFence::new(now)?,
            term: durable_term,
            hold_until: 0,
            recovery_until,
            last_grant: None,
            fenced: false,
        })
    }

    pub fn fence(&mut self) {
        self.fenced = true;
    }

    fn observe(&mut self, now: ClockReading, raft_term: u64) -> Result<u64> {
        if self.fenced {
            return Err(Refused::Fenced);
        }
        if raft_term < self.term {
            self.fence();
            return Err(Refused::WrongAuthority);
        }
        // Higher-term traffic must not erase a voting promise.
        self.term = raft_term;
        match self.clock.observe(now) {
            Ok(time) => Ok(time),
            Err(reason) => {
                self.fence();
                Err(reason)
            }
        }
    }

    /// Gate all actual competing votes and self-votes, including forced
    /// campaigns. Returning Ok does not bypass ordinary Raft vote/persistence
    /// rules. Pre-vote itself does not constitute an actual vote.
    pub fn may_vote(&mut self, now: ClockReading, raft_term: u64) -> Result<()> {
        let time = self.observe(now, raft_term)?;
        if time < self.recovery_until {
            return Err(Refused::RecoveryQuarantine);
        }
        if time < self.hold_until {
            return Err(Refused::VotingPromise);
        }
        Ok(())
    }

    /// Establish the hold before returning a grant decision. The adapter must
    /// bind the sender/term/leader observation and publish the ACK only after
    /// successful persistence/pump completion. A network term is not a
    /// substitute for the `raft_term` observed from local Raft state.
    pub fn promise(
        &mut self,
        now: ClockReading,
        raft_term: u64,
        raft_leader: u64,
        renewal: Renewal,
    ) -> Result<Grant> {
        let time = self.observe(now, raft_term)?;
        if time < self.recovery_until {
            return Err(Refused::RecoveryQuarantine);
        }
        let authority = renewal.authority;
        if authority.group != self.group
            || authority.configuration != self.configuration.id
            || !self.configuration.contains(authority.leader)
            || authority.leader != raft_leader
            || authority.term != self.term
            || authority.term == 0
            || authority.incarnation == 0
            || renewal.sequence == 0
            || renewal.generation == u64::MAX
            || renewal.promise_ns != self.timing.promise_ns
        {
            return Err(Refused::WrongAuthority);
        }
        if let Some(last) = self.last_grant {
            if last.renewal.authority == authority {
                if renewal.sequence < last.renewal.sequence
                    || renewal.generation < last.renewal.generation
                {
                    return Err(Refused::WrongRound);
                }
                if renewal.sequence == last.renewal.sequence {
                    if renewal != last.renewal {
                        return Err(Refused::WrongRound);
                    }
                    if time >= last.end {
                        return Err(Refused::Expired);
                    }
                    return Ok(Grant {
                        renewal,
                        voter: self.node,
                    });
                }
            } else if time < self.hold_until {
                return Err(Refused::VotingPromise);
            }
        }
        let Some(end) = time.checked_add(self.timing.promise_ns) else {
            self.fence();
            return Err(Refused::Overflow);
        };
        self.hold_until = self.hold_until.max(end);
        self.last_grant = Some(HeldGrant { renewal, end });
        Ok(Grant {
            renewal,
            voter: self.node,
        })
    }
}

#[cfg(test)]
mod tests;
