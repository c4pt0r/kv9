use super::*;
use kv9_common::raw_ttl::{ClockWatermark, ExpiryPlan};
use std::collections::VecDeque;

pub(super) fn policy() -> TtlClockPolicy {
    TtlClockPolicy {
        max_uncertainty_ms: 100,
        additional_uncertainty_ms: 2,
        max_sample_span_ms: 10,
        boottime_drift_ppb: 500_000,
        boottime_error_ns: 100,
        warmup_ms: 10,
    }
}
struct Script(VecDeque<ClockResult<Observation>>);
impl Source for Script {
    fn read(&mut self) -> ClockResult<Observation> {
        self.0.pop_front().expect("fenced clock read source again")
    }
}
fn reading(unix_ms: u64, boot_ms: u64) -> ClockResult<Observation> {
    Ok(Observation {
        unix_ns: unix_ms * MILLION,
        reported_error_ns: MILLION,
        boot_before_ns: boot_ms * MILLION,
        boot_after_ns: boot_ms * MILLION + 100,
    })
}
fn monitor(readings: Vec<ClockResult<Observation>>) -> Monitor<Script> {
    Monitor::new(policy(), Script(readings.into())).unwrap()
}

#[test]
fn fresh_clock_warms_up_rounds_outward_and_resolves_expiry_once() {
    let clock = monitor(vec![reading(1000, 0), reading(1020, 20), reading(1040, 40)]);
    assert_eq!(clock.sample(), Err(ClockRefusal::WarmingUp));
    let sample = clock.sample().unwrap();
    let (earliest, latest) = sample.interval_ms();
    assert!(earliest <= 1017 && latest >= 1023);
    let plan = ExpiryPlan::resolve(Some(1), sample, ClockWatermark::default()).unwrap();
    assert_eq!(plan.expires_at_ms(), Some(latest + 1000));
    let later = clock.sample().unwrap();
    assert!(later.interval_ms().0 > earliest);
    assert_eq!(ExpiryPlan::decode(&plan.encode()).unwrap(), plan);
    assert_eq!(clock.status().health, ClockHealth::Healthy);
}

#[test]
fn unhealthy_clock_and_read_errors_fence_without_automatic_recovery() {
    for reason in [
        ClockRefusal::Unsynchronized,
        ClockRefusal::SourceFailed,
        ClockRefusal::Discontinuity,
        ClockRefusal::InvalidReading,
    ] {
        let clock = monitor(vec![reading(1000, 0), Err(reason)]);
        assert_eq!(clock.sample(), Err(ClockRefusal::WarmingUp));
        assert_eq!(clock.sample(), Err(reason));
        assert_eq!(clock.sample(), Err(ClockRefusal::Fenced));
        assert_eq!(clock.status().last_refusal, Some(reason));
    }
}

#[test]
fn jumps_regression_latency_and_reported_error_fail_before_returning_time() {
    let base = reading(1020, 20).unwrap();
    for (bad, reason) in [
        (
            Observation {
                unix_ns: 9000 * MILLION,
                ..base
            },
            ClockRefusal::Discontinuity,
        ),
        (
            Observation {
                unix_ns: 500 * MILLION,
                ..base
            },
            ClockRefusal::Discontinuity,
        ),
        (
            Observation {
                boot_before_ns: 0,
                boot_after_ns: 20 * MILLION,
                ..base
            },
            ClockRefusal::SampleTooSlow,
        ),
        (
            Observation {
                reported_error_ns: 100 * MILLION,
                ..base
            },
            ClockRefusal::ExcessiveUncertainty,
        ),
        (
            Observation {
                boot_before_ns: 10,
                boot_after_ns: 9,
                ..base
            },
            ClockRefusal::Discontinuity,
        ),
        (
            Observation {
                unix_ns: u64::MAX,
                ..base
            },
            ClockRefusal::InvalidReading,
        ),
    ] {
        let clock = monitor(vec![reading(1000, 0), Ok(bad)]);
        assert_eq!(clock.sample(), Err(ClockRefusal::WarmingUp));
        assert_eq!(clock.sample(), Err(reason));
        assert_eq!(clock.sample(), Err(ClockRefusal::Fenced));
    }
}

#[test]
fn small_clock_corrections_keep_the_floor_and_cumulative_drift_is_not_rebased() {
    let clock = monitor(vec![reading(1000, 0), reading(1020, 20), reading(1019, 21)]);
    assert_eq!(clock.sample(), Err(ClockRefusal::WarmingUp));
    let floor = clock
        .sample()
        .unwrap()
        .advance(ClockWatermark::default())
        .unwrap();
    assert!(clock.sample().unwrap().advance(floor).unwrap() >= floor);

    let mut rows = vec![reading(1000, 0)];
    for i in 1..30 {
        rows.push(reading(1000 + 21 * i, 20 * i));
    }
    let clock = monitor(rows);
    assert_eq!(clock.sample(), Err(ClockRefusal::WarmingUp));
    assert!((1..30).any(|_| matches!(clock.sample(), Err(ClockRefusal::Discontinuity))));
}

#[test]
fn independent_rate_and_endpoint_error_model_keeps_intervals_conservative() {
    // Independent true-time model. Use different endpoint/UTC errors at the
    // anchor and current observations, including opposite-sign extremes.
    for rate_ppb in [-500_000i128, 0, 500_000] {
        // Integer rate conversion contributes < 1ns of error as well.
        for first_boot_error in [-99i128, 0, 99] {
            for second_boot_error in [-99i128, 0, 99] {
                for first_utc_error in [-(MILLION as i128), 0, MILLION as i128] {
                    for second_utc_error in [-(MILLION as i128), 0, MILLION as i128] {
                        for sampling_ns in [1000, 100_000, 3 * MILLION] {
                            let boot = |true_ns: u64, error: i128| -> u64 {
                                (i128::from(true_ns) * (1_000_000_000 + rate_ppb) / 1_000_000_000
                                    + error) as u64
                            };
                            let row =
                                |elapsed: u64, boot_error: i128, utc_error: i128| Observation {
                                    unix_ns: (10_000_000_000i128 + i128::from(elapsed) + utc_error)
                                        as u64,
                                    reported_error_ns: MILLION,
                                    boot_before_ns: boot(1_000_000_000 + elapsed, -boot_error),
                                    boot_after_ns: boot(
                                        1_000_000_000 + elapsed + sampling_ns,
                                        boot_error,
                                    ),
                                };
                            let clock = monitor(vec![
                                Ok(row(0, first_boot_error, first_utc_error)),
                                Ok(row(100 * MILLION, second_boot_error, second_utc_error)),
                            ]);
                            assert_eq!(clock.sample(), Err(ClockRefusal::WarmingUp));
                            let (low, high) = clock.sample().unwrap().interval_ms();
                            let true_ns = 10_000_000_000 + 100 * MILLION + sampling_ns;
                            assert!(u128::from(low) * u128::from(MILLION) <= u128::from(true_ns));
                            assert!(u128::from(high) * u128::from(MILLION) >= u128::from(true_ns));
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn reconstruction_cannot_reset_a_groups_persisted_time_watermark() {
    let committed = ClockSample::with_uncertainty(2000, 0)
        .unwrap()
        .advance(ClockWatermark::default())
        .unwrap();
    let restarted = monitor(vec![reading(1000, 0), reading(1020, 20)]);
    assert_eq!(restarted.sample(), Err(ClockRefusal::WarmingUp));
    assert!(restarted.sample().unwrap().advance(committed).is_err());
    assert_eq!(committed.unix_ms(), 2000);
}

#[test]
fn policy_has_no_implicit_bounds_and_rejects_unknown_or_invalid_fields() {
    let value = serde_json::to_value(policy()).unwrap();
    let mut missing = value.clone();
    missing
        .as_object_mut()
        .unwrap()
        .remove("additional_uncertainty_ms");
    assert!(serde_json::from_value::<TtlClockPolicy>(missing).is_err());
    let mut unknown = value;
    unknown["assume_synchronized"] = true.into();
    assert!(serde_json::from_value::<TtlClockPolicy>(unknown).is_err());
    let mut bad = policy();
    bad.boottime_drift_ppb = 1_000_000_000;
    assert!(bad.validate().is_err());
    bad = policy();
    bad.additional_uncertainty_ms = bad.max_uncertainty_ms;
    assert!(bad.validate().is_err());
    bad = policy();
    bad.warmup_ms = 0;
    assert!(bad.validate().is_err());
}
