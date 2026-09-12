use super::*;

fn policy() -> LeasePolicy {
    LeasePolicy {
        node: 1,
        group: 0,
        configuration: 7,
        voters: vec![1, 2, 3],
        promise_ns: 1_000_000_000,
        drift_ppb: 1_000_000,
        margin_ns: 4_005,
    }
}

#[test]
fn sampling_error_margin_covers_leaders_and_recovering_voters() {
    for error_ns in [0, 1, 2, 999, 1_000_000] {
        for drift_ppb in [0, 1, 100_000, 100_000_000, 999_999_999] {
            let bounds = ClockBounds {
                drift_ppb,
                error_ns,
            };
            let margin = bounds.minimum_margin_ns(drift_ppb).unwrap();
            let a = SCALE - u128::from(drift_ppb);
            let b = SCALE + u128::from(drift_ppb);
            let error = u128::from(error_ns);
            assert!(u128::from(margin) * b >= 2 * error * (a + b));
            assert!(u128::from(margin) * a >= 2 * error * (a + b));
            if margin > 0 {
                assert!(u128::from(margin - 1) * a < 2 * error * (a + b));
            }
            for promise in [100, 1_000_000_000, u64::MAX / 4] {
                if let Ok(timing) = Timing::new(promise, drift_ppb, margin) {
                    let e = u128::from(promise);
                    let d = u128::from(timing.leader_ns());
                    let r = u128::from(timing.recovery_ns());
                    assert!(e >= 2 * error);
                    assert!((d + 2 * error) * b <= (e - 2 * error) * a);
                    assert!(r >= 2 * error);
                    assert!((r - 2 * error) * a >= (e + 2 * error) * b);
                }
            }
        }
    }
}

#[test]
fn recovery_requires_more_than_the_leader_only_error_margin() {
    // a=0.9, b=1.1, epsilon=1. Leader needs ceil(40/11)=4;
    // recovery needs ceil(40/9)=5. Reusing 4 expires quarantine too early.
    let bounds = ClockBounds {
        drift_ppb: 100_000_000,
        error_ns: 1,
    };
    let mut p = policy();
    p.drift_ppb = bounds.drift_ppb;
    p.promise_ns = 99;
    p.margin_ns = 4;
    assert_eq!(bounds.minimum_margin_ns(p.drift_ppb), Ok(5));
    assert_eq!(bounds.validate_policy(&p), Err(Refused::InvalidTiming));
    p.margin_ns = 5;
    assert_eq!(bounds.validate_policy(&p), Ok(()));
}

#[test]
fn weaker_or_unrepresentable_clock_contracts_are_refused() {
    let bounds = ClockBounds {
        drift_ppb: 1_000_000,
        error_ns: 1_000,
    };
    let mut p = policy();
    assert_eq!(bounds.validate_policy(&p), Ok(()));
    p.margin_ns -= 1;
    assert_eq!(bounds.validate_policy(&p), Err(Refused::InvalidTiming));
    p = policy();
    p.drift_ppb -= 1;
    assert_eq!(bounds.validate_policy(&p), Err(Refused::InvalidTiming));
    assert_eq!(
        bounds.minimum_margin_ns(1_000_000_000),
        Err(Refused::InvalidTiming)
    );
    let overflow = ClockBounds {
        drift_ppb: 0,
        error_ns: u64::MAX,
    };
    assert_eq!(overflow.minimum_margin_ns(0), Err(Refused::Overflow));
    p = policy();
    p.promise_ns = p.margin_ns;
    assert_eq!(bounds.validate_policy(&p), Err(Refused::InvalidTiming));
}

#[cfg(target_os = "linux")]
#[test]
fn linux_clock_uses_distinct_domains_and_rechecks_installation_policy() {
    use crate::rawnode::LeaseClock;
    let bounds = ClockBounds {
        drift_ppb: 1_000_000,
        error_ns: 1_000,
    };
    let mut p = policy();
    let first = LinuxBoottimeClock::new(&p, bounds).unwrap();
    let second = LinuxBoottimeClock::new(&p, bounds).unwrap();
    let before = first.sample().unwrap();
    assert_ne!(before.domain, second.sample().unwrap().domain);
    assert!(first.sample().unwrap().nanos >= before.nanos);
    assert!(first.resolution_ns() > 0 && first.resolution_ns() <= bounds.error_ns);
    assert_eq!(first.bounds(), bounds);
    p.margin_ns -= 1;
    assert_eq!(first.validate_policy(&p), Err(Refused::InvalidTiming));
    let impossible = ClockBounds {
        error_ns: 0,
        ..bounds
    };
    assert!(matches!(
        LinuxBoottimeClock::new(&policy(), impossible),
        Err(Refused::InvalidTiming)
    ));
}
