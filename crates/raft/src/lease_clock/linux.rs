use super::ClockBounds;
use crate::lease::{ClockReading, Refused, Result};
use crate::lease_policy::LeasePolicy;
use crate::rawnode::LeaseClock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Explicit Linux CLOCK_BOOTTIME adapter. It counts host suspend time, unlike
/// CLOCK_MONOTONIC, but Linux alone does not guarantee the declared rate/error
/// bounds. The deployment must qualify hardware, NTP slew and VM behavior.
///
/// No default constructor, clock fallback, filesystem access, cached samples,
/// mutex or user-space retry loop exists on the read path. Kernel/vDSO work can
/// still be preempted. A sampling error permanently fences this clock object.
/// Regression/domain checks additionally run under each installed peer's gate.
pub struct LinuxBoottimeClock {
    domain: u128,
    bounds: ClockBounds,
    resolution_ns: u64,
    failed: AtomicBool,
}

impl LinuxBoottimeClock {
    /// Supplying bounds is an explicit assertion by the caller, not a
    /// qualification result. Reported resolution is only a necessary check.
    pub fn new(policy: &LeasePolicy, bounds: ClockBounds) -> Result<Self> {
        bounds.validate_policy(policy)?;
        let resolution_ns = read_clock(true)?;
        if resolution_ns == 0 || resolution_ns > bounds.error_ns {
            return Err(Refused::InvalidTiming);
        }
        // Clock domains distinguish objects within this process. They never
        // cross the wire or authorize recovery; the durable peer incarnation
        // and a fresh quarantine provide the separate restart protection.
        static NEXT_DOMAIN: AtomicU64 = AtomicU64::new(1);
        let domain = NEXT_DOMAIN
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| Refused::Overflow)?;
        let clock = Self {
            domain: u128::from(domain),
            bounds,
            resolution_ns,
            failed: AtomicBool::new(false),
        };
        clock.sample()?;
        Ok(clock)
    }

    pub fn bounds(&self) -> ClockBounds {
        self.bounds
    }

    pub fn resolution_ns(&self) -> u64 {
        self.resolution_ns
    }

    fn sample_with(&self, read: impl FnOnce() -> Result<u64>) -> Result<ClockReading> {
        if self.failed.load(Ordering::Acquire) {
            return Err(Refused::Fenced);
        }
        let nanos = match read() {
            Ok(nanos) => nanos,
            Err(reason) => {
                self.failed.store(true, Ordering::Release);
                return Err(reason);
            }
        };
        if self.failed.load(Ordering::Acquire) {
            return Err(Refused::Fenced);
        }
        Ok(ClockReading {
            domain: self.domain,
            nanos,
        })
    }
}

impl LeaseClock for LinuxBoottimeClock {
    fn validate_policy(&self, policy: &LeasePolicy) -> Result<()> {
        // Recheck at installation: a clock created for a conservative policy
        // cannot subsequently be used with a weaker drift or error margin.
        self.bounds.validate_policy(policy)
    }

    fn sample(&self) -> Result<ClockReading> {
        self.sample_with(|| read_clock(false))
    }
}

fn read_clock(resolution: bool) -> Result<u64> {
    let mut value = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: Both libc functions accept this fixed supported clock identifier
    // and a writable, aligned timespec. They retain no pointer. Inspect the
    // return code before using the initialized output; never substitute a clock.
    let status = unsafe {
        if resolution {
            libc::clock_getres(libc::CLOCK_BOOTTIME, &mut value)
        } else {
            libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut value)
        }
    };
    if status != 0 {
        return Err(Refused::ClockUnavailable);
    }
    decode_time(i128::from(value.tv_sec), i128::from(value.tv_nsec))
}

fn decode_time(seconds: i128, nanos: i128) -> Result<u64> {
    if seconds < 0 || !(0..1_000_000_000).contains(&nanos) {
        return Err(Refused::ClockUnavailable);
    }
    let seconds = u64::try_from(seconds).map_err(|_| Refused::Overflow)?;
    seconds
        .checked_mul(1_000_000_000)
        .and_then(|n| n.checked_add(nanos as u64))
        .ok_or(Refused::Overflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_or_overflowing_time_never_becomes_a_sample() {
        assert_eq!(decode_time(7, 42), Ok(7_000_000_042));
        for (sec, ns) in [(-1, 0), (0, -1), (0, 1_000_000_000)] {
            assert_eq!(decode_time(sec, ns), Err(Refused::ClockUnavailable));
        }
        assert_eq!(decode_time(i128::MAX, 0), Err(Refused::Overflow));
        assert_eq!(decode_time(i128::from(u64::MAX), 0), Err(Refused::Overflow));
    }

    #[test]
    fn sampling_failure_is_sticky_even_when_the_source_recovers() {
        let clock = LinuxBoottimeClock {
            domain: 1,
            bounds: ClockBounds {
                drift_ppb: 0,
                error_ns: 1,
            },
            resolution_ns: 1,
            failed: AtomicBool::new(false),
        };
        assert_eq!(clock.sample_with(|| Ok(123)).unwrap().nanos, 123);
        assert_eq!(
            clock.sample_with(|| Err(Refused::ClockUnavailable)),
            Err(Refused::ClockUnavailable)
        );
        assert_eq!(
            clock.sample_with(|| panic!("fenced clock called its source")),
            Err(Refused::Fenced)
        );
    }
}
