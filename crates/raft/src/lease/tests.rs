use super::*;

fn at(nanos: u64) -> ClockReading {
    ClockReading { domain: 1, nanos }
}

fn voter_at(node: u64, nanos: u64) -> ClockReading {
    ClockReading {
        domain: 10 + u128::from(node),
        nanos,
    }
}

fn authority() -> Authority {
    Authority {
        group: 0,
        configuration: 77,
        leader: 1,
        term: 5,
        incarnation: 91,
    }
}

fn progress(committed: u64) -> Progress {
    Progress {
        configuration: 77,
        leader: 1,
        term: 5,
        committed,
        committed_term: 5,
    }
}

fn configuration() -> FixedConfiguration {
    FixedConfiguration::new(77, vec![3, 1, 2]).unwrap()
}

struct Fixture {
    leader: LeaderLease,
    voters: Vec<VoterLease>,
}

impl Fixture {
    fn new() -> Self {
        let timing = Timing::new(100, 0, 0).unwrap();
        Self {
            leader: LeaderLease::new(configuration(), authority(), timing, at(0)).unwrap(),
            voters: (1..=3)
                .map(|n| {
                    VoterLease::recover(n, 0, configuration(), timing, voter_at(n, 0), 5).unwrap()
                })
                .collect(),
        }
    }

    fn grant(&mut self, renewal: Renewal, node: u64, time: u64) -> Grant {
        self.voters[node as usize - 1]
            .promise(voter_at(node, time), 5, 1, renewal)
            .unwrap()
    }

    fn acquire(&mut self, time: u64) -> Renewal {
        let renewal = self.leader.start(at(time), progress(1)).unwrap();
        for node in 1..=2 {
            let ack = self.grant(renewal, node, time);
            self.leader.acknowledge(at(time), ack).unwrap();
        }
        self.leader.publish(at(time), progress(1), renewal).unwrap();
        renewal
    }
}

#[test]
fn timing_rounds_in_the_safe_direction_and_reserves_margin() {
    let timing = Timing::new(110, 100_000_000, 2).unwrap();
    assert_eq!(timing.promise_ns(), 110);
    assert_eq!(timing.leader_ns(), 88);
    assert_eq!(timing.recovery_ns(), 137);
    for promise in [1, 2, 9, 100, 1_000_000, u64::MAX / 4] {
        for drift in [0, 1, 100_000, 100_000_000, 999_999_999] {
            for margin in [0, 1, 7] {
                if let Ok(timing) = Timing::new(promise, drift, margin) {
                    let a = RATE_SCALE - u128::from(drift);
                    let b = RATE_SCALE + u128::from(drift);
                    assert!(u128::from(timing.leader_ns()) * b <= u128::from(promise) * a);
                    assert!(u128::from(timing.recovery_ns()) * a >= u128::from(promise) * b);
                    assert!(timing.leader_ns() > 0);
                    assert!(timing.recovery_ns() >= promise);
                }
            }
        }
    }
}

#[test]
fn impossible_timing_and_overflow_are_typed_refusals() {
    assert_eq!(Timing::new(0, 0, 0), Err(Refused::InvalidTiming));
    assert_eq!(
        Timing::new(10, 1_000_000_000, 0),
        Err(Refused::InvalidTiming)
    );
    assert_eq!(Timing::new(1, 1, 0), Err(Refused::InvalidTiming));
    assert_eq!(Timing::new(10, 0, 10), Err(Refused::InvalidTiming));
    assert_eq!(Timing::new(u64::MAX, 1, 0), Err(Refused::Overflow));
    assert_eq!(Timing::new(u64::MAX, 0, 1), Err(Refused::Overflow));
}

#[test]
fn fixed_membership_rejects_ambiguous_or_unbounded_quorums() {
    for voters in [vec![], vec![0, 1], vec![1, 1], (1..=65).collect()] {
        assert_eq!(
            FixedConfiguration::new(77, voters),
            Err(Refused::InvalidConfiguration)
        );
    }
    assert_eq!(
        FixedConfiguration::new(0, vec![1]),
        Err(Refused::InvalidConfiguration)
    );
    assert_eq!(configuration().voters(), &[1, 2, 3]);
    assert_eq!(configuration().quorum(), 2);
}

#[test]
fn duplicate_acks_do_not_form_a_quorum_and_receipt_does_not_publish() {
    let mut f = Fixture::new();
    let renewal = f.leader.start(at(200), progress(1)).unwrap();
    let ack = f.grant(renewal, 1, 200);
    for _ in 0..10 {
        assert!(!f.leader.acknowledge(at(200), ack).unwrap());
    }
    assert_eq!(
        f.leader.publish(at(200), progress(1), renewal),
        Err(Refused::NoQuorum)
    );
    assert!(matches!(
        f.leader.begin_read(at(200), progress(1), 400),
        Err(Refused::NoCertificate)
    ));
    let other = f.grant(renewal, 2, 200);
    assert!(f.leader.acknowledge(at(200), other).unwrap());
    assert!(matches!(
        f.leader.begin_read(at(200), progress(1), 400),
        Err(Refused::NoCertificate)
    ));
    f.leader.publish(at(200), progress(1), renewal).unwrap();
    assert!(f.leader.begin_read(at(200), progress(1), 400).is_ok());
}

#[test]
fn outbound_renewal_validation_preserves_the_send_anchor_and_exact_round() {
    let mut f = Fixture::new();
    let first = f.leader.start(at(100), progress(1)).unwrap();
    assert_eq!(
        f.leader.validate_renewal(at(199), progress(1), first),
        Ok(())
    );
    assert_eq!(
        f.leader.validate_renewal(at(200), progress(1), first),
        Err(Refused::Expired)
    );
    let second = f.leader.start(at(200), progress(1)).unwrap();
    assert_eq!(
        f.leader.validate_renewal(at(200), progress(1), first),
        Err(Refused::WrongRound)
    );
    assert_eq!(
        f.leader.validate_renewal(at(200), progress(1), second),
        Ok(())
    );
    f.leader.revoke();
    assert_eq!(
        f.leader.validate_renewal(at(200), progress(1), second),
        Err(Refused::Revoked)
    );
}

#[test]
fn every_round_identity_field_is_checked_and_nonvoters_are_rejected() {
    let mut f = Fixture::new();
    let renewal = f.leader.start(at(200), progress(1)).unwrap();
    let ack = f.grant(renewal, 1, 200);
    let mut wrong = vec![renewal; 8];
    wrong[0].authority.group += 1;
    wrong[1].authority.configuration += 1;
    wrong[2].authority.leader += 1;
    wrong[3].authority.term += 1;
    wrong[4].authority.incarnation += 1;
    wrong[5].generation += 1;
    wrong[6].sequence += 1;
    wrong[7].promise_ns += 1;
    for renewal in wrong {
        assert_eq!(
            f.leader.acknowledge(at(200), Grant { renewal, voter: 1 }),
            Err(Refused::WrongRound)
        );
    }
    assert_eq!(
        f.leader.acknowledge(at(200), Grant { voter: 4, ..ack }),
        Err(Refused::WrongVoter)
    );
    assert!(!f.leader.acknowledge(at(200), ack).unwrap());
}

#[test]
fn late_old_round_acks_cannot_complete_the_new_round() {
    let mut f = Fixture::new();
    let old = f.leader.start(at(200), progress(1)).unwrap();
    let old_ack = f.grant(old, 1, 200);
    let new = f.leader.start(at(201), progress(1)).unwrap();
    assert_eq!(
        f.leader.acknowledge(at(201), old_ack),
        Err(Refused::WrongRound)
    );
    let new_ack = f.grant(new, 2, 201);
    assert!(!f.leader.acknowledge(at(201), new_ack).unwrap());
    assert_eq!(
        f.leader.publish(at(201), progress(1), new),
        Err(Refused::NoQuorum)
    );
}

#[test]
fn delayed_ack_never_rebases_the_send_time_deadline() {
    let mut f = Fixture::new();
    let renewal = f.leader.start(at(200), progress(1)).unwrap();
    let one = f.grant(renewal, 1, 299);
    let two = f.grant(renewal, 2, 299);
    f.leader.acknowledge(at(299), one).unwrap();
    assert_eq!(f.leader.acknowledge(at(300), two), Err(Refused::Expired));
    assert_eq!(
        f.leader.publish(at(300), progress(1), renewal),
        Err(Refused::Expired)
    );
    assert!(matches!(
        f.leader.begin_read(at(300), progress(1), 500),
        Err(Refused::NoCertificate)
    ));
}

#[test]
fn an_old_read_retains_its_own_deadline_across_renewal() {
    let mut f = Fixture::new();
    f.acquire(200);
    let before_expiry = f.leader.begin_read(at(250), progress(1), 500).unwrap();
    let after_expiry = f.leader.begin_read(at(250), progress(1), 500).unwrap();
    f.acquire(260);
    assert!(f
        .leader
        .finish_read(at(299), progress(1), before_expiry, 1)
        .is_ok());
    assert_eq!(
        f.leader.finish_read(at(300), progress(1), after_expiry, 1),
        Err(Refused::Expired)
    );
    let new = f.leader.begin_read(at(300), progress(1), 500).unwrap();
    assert!(f.leader.finish_read(at(300), progress(1), new, 1).is_ok());
}

#[test]
fn expiration_without_quorum_never_returns_read_success() {
    let mut f = Fixture::new();
    f.acquire(200);
    for time in 300..305 {
        assert!(matches!(
            f.leader.begin_read(at(time), progress(1), 1_000),
            Err(Refused::Expired)
        ));
    }
    f.leader.rearm(at(305), progress(1)).unwrap();
    let next = f.leader.start(at(305), progress(1)).unwrap();
    let only_self = f.grant(next, 1, 305);
    f.leader.acknowledge(at(305), only_self).unwrap();
    assert_eq!(
        f.leader.publish(at(305), progress(1), next),
        Err(Refused::NoQuorum)
    );
    assert!(matches!(
        f.leader.begin_read(at(305), progress(1), 1_000),
        Err(Refused::Expired)
    ));
}

#[test]
fn revoke_rearm_and_new_renewal_cannot_resurrect_old_tickets() {
    let mut f = Fixture::new();
    f.acquire(200);
    let ticket = f.leader.begin_read(at(210), progress(1), 500).unwrap();
    f.leader.revoke();
    assert!(matches!(
        f.leader.begin_read(at(211), progress(1), 500),
        Err(Refused::Revoked)
    ));
    f.leader.rearm(at(212), progress(1)).unwrap();
    assert!(matches!(
        f.leader.begin_read(at(212), progress(1), 500),
        Err(Refused::NoCertificate)
    ));
    let new = f.acquire(213);
    assert_eq!(new.generation, 1);
    assert_eq!(new.sequence, 2);
    assert_eq!(
        f.leader.finish_read(at(214), progress(1), ticket, 1),
        Err(Refused::WrongAuthority)
    );
    assert_eq!(
        f.voters[0].may_vote(voter_at(1, 214), 5),
        Err(Refused::VotingPromise)
    );
}

#[test]
fn fresh_frontier_and_exact_view_are_required_after_completed_writes() {
    let mut f = Fixture::new();
    f.acquire(200); // Renewal knew only index 1.
    let ticket = f.leader.begin_read(at(210), progress(10), 500).unwrap();
    assert_eq!(
        f.leader.finish_read(at(211), progress(10), ticket, 9),
        Err(Refused::ApplyLag)
    );
    let ticket = f.leader.begin_read(at(212), progress(10), 500).unwrap();
    let decision = f
        .leader
        .finish_read(at(213), progress(10), ticket, 10)
        .unwrap();
    assert_eq!((decision.committed(), decision.view_index()), (10, 10));
}

#[test]
fn overlapping_write_may_follow_a_reads_retained_prefix() {
    let mut f = Fixture::new();
    f.acquire(200);
    let ticket = f.leader.begin_read(at(210), progress(10), 500).unwrap();
    let decision = f
        .leader
        .finish_read(at(211), progress(11), ticket, 10)
        .unwrap();
    assert_eq!(decision.view_index(), 10);
    let ticket = f.leader.begin_read(at(212), progress(11), 500).unwrap();
    assert_eq!(
        f.leader.finish_read(at(213), progress(11), ticket, 12),
        Err(Refused::InvalidView)
    );
}

#[test]
fn current_term_commit_is_a_gate_and_commit_regression_is_terminal() {
    let mut f = Fixture::new();
    let mut early = progress(1);
    early.committed_term = 4;
    assert_eq!(
        f.leader.start(at(200), early),
        Err(Refused::NoCurrentTermCommit)
    );
    f.acquire(201);
    f.leader.begin_read(at(202), progress(10), 500).unwrap();
    assert!(matches!(
        f.leader.begin_read(at(203), progress(9), 500),
        Err(Refused::CommitRegressed)
    ));
    assert_eq!(f.leader.mode(), Mode::Fenced);
    assert_eq!(f.leader.rearm(at(204), progress(10)), Err(Refused::Fenced));
}

#[test]
fn term_leader_and_configuration_changes_fence_the_final_read() {
    for changed in 0..3 {
        let mut f = Fixture::new();
        f.acquire(200);
        let ticket = f.leader.begin_read(at(210), progress(1), 500).unwrap();
        let mut p = progress(1);
        match changed {
            0 => p.term += 1,
            1 => p.leader += 1,
            _ => p.configuration += 1,
        }
        assert_eq!(
            f.leader.finish_read(at(211), p, ticket, 1),
            Err(Refused::WrongAuthority)
        );
        assert_eq!(f.leader.mode(), Mode::Fenced);
    }
}

#[test]
fn request_deadline_is_not_extended_by_a_live_lease() {
    let mut f = Fixture::new();
    f.acquire(200);
    let ticket = f.leader.begin_read(at(249), progress(1), 250).unwrap();
    assert_eq!(
        f.leader.finish_read(at(250), progress(1), ticket, 1),
        Err(Refused::Deadline)
    );
    assert!(matches!(
        f.leader.begin_read(at(250), progress(1), 250),
        Err(Refused::Deadline)
    ));
}

#[test]
fn a_ticket_from_another_incarnation_is_refused() {
    let mut f = Fixture::new();
    f.acquire(200);
    let ticket = f.leader.begin_read(at(210), progress(1), 500).unwrap();
    let mut epoch = authority();
    epoch.incarnation += 1;
    let mut restarted = LeaderLease::new(configuration(), epoch, f.leader.timing, at(211)).unwrap();
    assert!(matches!(
        restarted.begin_read(at(211), progress(1), 500),
        Err(Refused::NoCertificate)
    ));
    assert_eq!(
        restarted.finish_read(at(211), progress(1), ticket, 1),
        Err(Refused::WrongAuthority)
    );
}

#[test]
fn changed_or_regressed_clock_is_terminal_for_leader_and_voter() {
    for (bad, expected) in [
        (at(199), Refused::ClockRegressed),
        (
            ClockReading {
                domain: 2,
                nanos: 201,
            },
            Refused::ClockChanged,
        ),
    ] {
        let mut f = Fixture::new();
        f.acquire(200);
        assert!(
            matches!(f.leader.begin_read(bad, progress(1), 500), Err(reason) if reason == expected)
        );
        assert_eq!(f.leader.rearm(at(202), progress(1)), Err(Refused::Fenced));
        let voter_bad = ClockReading {
            domain: if bad.domain == 1 { 11 } else { 12 },
            nanos: bad.nanos,
        };
        assert_eq!(f.voters[0].may_vote(voter_bad, 5), Err(expected));
        assert_eq!(
            f.voters[0].may_vote(voter_at(1, 500), 5),
            Err(Refused::Fenced)
        );
    }
}

#[test]
fn self_votes_and_higher_term_traffic_do_not_erase_promises() {
    let mut f = Fixture::new();
    let renewal = f.acquire(200);
    assert_eq!(
        f.voters[0].may_vote(voter_at(1, 299), 6),
        Err(Refused::VotingPromise)
    );
    assert_eq!(
        f.voters[0].promise(voter_at(1, 299), 6, 1, renewal),
        Err(Refused::WrongAuthority)
    );
    assert_eq!(f.voters[0].may_vote(voter_at(1, 300), 6), Ok(()));
}

#[test]
fn duplicate_grants_keep_their_original_expiration() {
    let mut f = Fixture::new();
    let renewal = f.acquire(200);
    assert_eq!(f.grant(renewal, 1, 299).renewal, renewal);
    assert_eq!(f.voters[0].may_vote(voter_at(1, 300), 5), Ok(()));
    assert_eq!(
        f.voters[0].promise(voter_at(1, 300), 5, 1, renewal),
        Err(Refused::Expired)
    );
}

#[test]
fn reordered_requests_do_not_shorten_or_extend_the_wrong_hold() {
    let mut f = Fixture::new();
    let old = f.acquire(200);
    let new = f.acquire(210);
    assert_eq!(
        f.voters[0].promise(voter_at(1, 220), 5, 1, old),
        Err(Refused::WrongRound)
    );
    assert_eq!(
        f.voters[0].may_vote(voter_at(1, 300), 5),
        Err(Refused::VotingPromise)
    );
    assert_eq!(f.voters[0].may_vote(voter_at(1, 310), 5), Ok(()));
    assert!(new.sequence > old.sequence);
}

#[test]
fn recovery_and_repeated_restart_cannot_shorten_prior_obligations() {
    let mut f = Fixture::new();
    let old = f.acquire(200); // Old promise ends at real time 300.
    let timing = f.leader.timing;
    let boot = |domain, nanos| ClockReading { domain, nanos };
    // Crash/restart at real time 201, with a completely new clock origin.
    let mut restarted = VoterLease::recover(2, 0, configuration(), timing, boot(77, 0), 5).unwrap();
    assert_eq!(
        restarted.may_vote(boot(77, 98), 5),
        Err(Refused::RecoveryQuarantine)
    );
    assert_eq!(
        restarted.promise(boot(77, 98), 5, 1, old),
        Err(Refused::RecoveryQuarantine)
    );
    assert_eq!(restarted.may_vote(boot(77, 100), 5), Ok(())); // Real time 301.
                                                              // An additional restart at real time 250 starts a new complete quarantine.
    let mut again = VoterLease::recover(2, 0, configuration(), timing, boot(78, 0), 6).unwrap();
    assert_eq!(
        again.may_vote(boot(78, 50), 6),
        Err(Refused::RecoveryQuarantine)
    );
    assert_eq!(again.may_vote(boot(78, 100), 6), Ok(()));
    assert_eq!(
        again.promise(boot(78, 100), 6, 1, old),
        Err(Refused::WrongAuthority)
    );
}

#[test]
fn new_authority_cannot_receive_a_grant_during_an_old_promise() {
    let mut f = Fixture::new();
    let mut new = f.acquire(200);
    new.authority.term = 6;
    new.authority.leader = 3;
    new.authority.incarnation += 1;
    assert_eq!(
        f.voters[0].promise(voter_at(1, 299), 6, 3, new),
        Err(Refused::VotingPromise)
    );
    assert!(f.voters[0].promise(voter_at(1, 300), 6, 3, new).is_ok());
}

#[test]
fn fastest_voters_cannot_form_an_election_quorum_during_the_slowest_leader_lease() {
    let timing = Timing::new(110, 100_000_000, 0).unwrap();
    let mut leader = LeaderLease::new(configuration(), authority(), timing, at(0)).unwrap();
    let mut voters: Vec<_> = (1..=3)
        .map(|n| VoterLease::recover(n, 0, configuration(), timing, voter_at(n, 0), 5).unwrap())
        .collect();
    // One real step is ten units: leader/self clock advances nine, others eleven.
    let renewal = leader.start(at(20 * 9), progress(1)).unwrap();
    for node in 1..=2 {
        let rate = if node == 1 { 9 } else { 11 };
        let grant = voters[node as usize - 1]
            .promise(voter_at(node, 20 * rate), 5, 1, renewal)
            .unwrap();
        leader.acknowledge(at(20 * 9), grant).unwrap();
    }
    leader.publish(at(20 * 9), progress(1), renewal).unwrap();
    let mut saw_local_read = false;
    let mut saw_election_quorum = false;
    for real_step in 20..=35 {
        let ticket = leader.begin_read(at(real_step * 9), progress(1), 10_000);
        let allowed = voters
            .iter_mut()
            .enumerate()
            .filter_map(|(i, voter)| {
                let node = i as u64 + 1;
                let rate = if node == 1 { 9 } else { 11 };
                voter
                    .may_vote(voter_at(node, real_step * rate), 5)
                    .is_ok()
                    .then_some(node)
            })
            .count();
        if let Ok(ticket) = ticket {
            assert!(allowed < 2);
            leader
                .finish_read(at(real_step * 9), progress(1), ticket, 1)
                .unwrap();
            saw_local_read = true;
        }
        if allowed >= 2 {
            assert!(real_step >= 30);
            assert!(matches!(
                leader.begin_read(at(real_step * 9), progress(1), 10_000),
                Err(Refused::Expired)
            ));
            saw_election_quorum = true;
        }
    }
    assert!(saw_local_read && saw_election_quorum);
}

#[test]
fn generation_and_round_exhaustion_fence_instead_of_wrapping() {
    let mut f = Fixture::new();
    f.leader.generation = u64::MAX - 1;
    f.leader.revoke();
    assert_eq!(f.leader.mode(), Mode::Fenced);
    assert_eq!(f.leader.rearm(at(200), progress(1)), Err(Refused::Fenced));
    let mut f = Fixture::new();
    f.leader.sequence = u64::MAX;
    assert_eq!(f.leader.start(at(200), progress(1)), Err(Refused::Overflow));
    assert_eq!(f.leader.mode(), Mode::Fenced);
}

#[test]
fn deadline_overflow_fences_without_publishing_or_permitting_votes() {
    let mut f = Fixture::new();
    assert_eq!(
        f.leader.start(at(u64::MAX - 1), progress(1)),
        Err(Refused::Overflow)
    );
    assert_eq!(f.leader.mode(), Mode::Fenced);
    let renewal = Fixture::new().leader.start(at(200), progress(1)).unwrap();
    assert_eq!(
        f.voters[0].promise(voter_at(1, u64::MAX - 1), 5, 1, renewal),
        Err(Refused::Overflow)
    );
    assert_eq!(
        f.voters[0].may_vote(voter_at(1, u64::MAX), 5),
        Err(Refused::Fenced)
    );
    assert!(matches!(
        VoterLease::recover(
            1,
            0,
            configuration(),
            f.leader.timing,
            voter_at(1, u64::MAX - 1),
            5
        ),
        Err(Refused::Overflow)
    ));
}
