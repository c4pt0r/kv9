use super::*;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

/// Explicit Linux provider. Uses only read-only adjtimex (modes=0), BOOTTIME,
/// and a cancel-on-clock-set timerfd. No clock adjustment or CAP_SYS_TIME is
/// requested. The monitor fences on a failed observation; no wall-clock fallback.
pub struct LinuxTtlClock(Monitor<LinuxSource>);

impl LinuxTtlClock {
    pub fn new(policy: TtlClockPolicy) -> kv9_common::Result<Self> {
        policy.validate()?;
        let source =
            LinuxSource::new(policy).map_err(|reason| kv9_common::Error::RawTtlClock { reason })?;
        Ok(Self(Monitor::new(policy, source)?))
    }
}
impl TtlClock for LinuxTtlClock {
    fn sample(&self) -> ClockResult<ClockSample> {
        self.0.sample()
    }
    fn status(&self) -> ClockStatus {
        self.0.status()
    }
}

struct LinuxSource {
    watch: OwnedFd,
}

impl LinuxSource {
    fn new(policy: TtlClockPolicy) -> ClockResult<Self> {
        let mut resolution = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: fixed supported clock and valid writable timespec, no retained pointer.
        if unsafe { libc::clock_getres(libc::CLOCK_BOOTTIME, &mut resolution) } != 0 {
            return Err(ClockRefusal::SourceFailed);
        }
        let resolution = nanos(resolution.tv_sec.into(), resolution.tv_nsec.into())?;
        if resolution == 0 || resolution > policy.boottime_error_ns {
            return Err(ClockRefusal::InvalidReading);
        }
        // SAFETY: creates a new fd with fixed, valid clock and flags.
        let fd = unsafe {
            libc::timerfd_create(libc::CLOCK_REALTIME, libc::TFD_CLOEXEC | libc::TFD_NONBLOCK)
        };
        if fd < 0 {
            return Err(ClockRefusal::SourceFailed);
        }
        // SAFETY: successful timerfd_create transferred a fresh fd to this owner.
        let watch = unsafe { OwnedFd::from_raw_fd(fd) };
        // time_t may be 32-bit on a supported libc target; reject truncation.
        #[allow(clippy::useless_conversion)]
        let expiration = 8_000_000_000i64
            .try_into()
            .map_err(|_| ClockRefusal::Unavailable)?;
        let timer = libc::itimerspec {
            it_interval: libc::timespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
            // Far-future one-shot, within signed 64-bit kernel nanoseconds.
            // Expiration is itself a refusal, never mistaken for "no step".
            it_value: libc::timespec {
                tv_sec: expiration,
                tv_nsec: 0,
            },
        };
        // SAFETY: valid owned timer fd and read-only initialized input; no old-value output.
        if unsafe {
            libc::timerfd_settime(
                watch.as_raw_fd(),
                libc::TFD_TIMER_ABSTIME | libc::TFD_TIMER_CANCEL_ON_SET,
                &timer,
                std::ptr::null_mut(),
            )
        } != 0
        {
            return Err(ClockRefusal::SourceFailed);
        }
        Ok(Self { watch })
    }

    fn check_step(&self) -> ClockResult<()> {
        let mut ticks = 0u64;
        // SAFETY: fd is owned and nonblocking; output points to 8 writable bytes.
        let count =
            unsafe { libc::read(self.watch.as_raw_fd(), (&mut ticks as *mut u64).cast(), 8) };
        let errno = if count < 0 {
            std::io::Error::last_os_error().raw_os_error()
        } else {
            None
        };
        check_watch_result(count, errno)
    }
}

fn check_watch_result(count: isize, errno: Option<i32>) -> ClockResult<()> {
    match (count, errno) {
        (-1, Some(libc::EAGAIN)) => Ok(()),
        (-1, Some(libc::ECANCELED)) | (8, _) => Err(ClockRefusal::Discontinuity),
        _ => Err(ClockRefusal::SourceFailed),
    }
}

impl Source for LinuxSource {
    fn read(&mut self) -> ClockResult<Observation> {
        let before = boottime()?;
        self.check_step()?;
        // SAFETY: zero is a valid initialized timex; modes=0 is strictly read-only.
        let mut value: libc::timex = unsafe { std::mem::zeroed() };
        // SAFETY: points to an initialized writable timex; no pointer retained.
        let state = unsafe { libc::adjtimex(&mut value) };
        if state < 0 {
            return Err(ClockRefusal::SourceFailed);
        }
        let (unix_ns, reported_error_ns) = decode_timex(state, &value)?;
        // Covers a discontinuous set before, during, or between observations.
        self.check_step()?;
        let after = boottime()?;
        Ok(Observation {
            unix_ns,
            reported_error_ns,
            boot_before_ns: before,
            boot_after_ns: after,
        })
    }
}

fn decode_timex(state: i32, value: &libc::timex) -> ClockResult<(u64, u64)> {
    let bad = libc::STA_UNSYNC
        | libc::STA_CLOCKERR
        | libc::STA_INS
        | libc::STA_DEL
        | libc::STA_PPSJITTER
        | libc::STA_PPSWANDER
        | libc::STA_PPSERROR;
    if state != libc::TIME_OK
        || value.status & bad != 0
        || value.status & !0xffff != 0
        || (value.status & (libc::STA_PPSTIME | libc::STA_PPSFREQ) != 0
            && value.status & libc::STA_PPSSIGNAL == 0)
    {
        return Err(ClockRefusal::Unsynchronized);
    }
    let fraction = i128::from(value.time.tv_usec);
    let nanos_fraction = if value.status & libc::STA_NANO != 0 {
        fraction
    } else {
        if !(0..1_000_000).contains(&fraction) {
            return Err(ClockRefusal::InvalidReading);
        }
        fraction * 1000
    };
    let unix = nanos(value.time.tv_sec.into(), nanos_fraction)?;
    let maximum = u64::try_from(value.maxerror).map_err(|_| ClockRefusal::InvalidReading)?;
    let precision = u64::try_from(value.precision).map_err(|_| ClockRefusal::InvalidReading)?;
    // maxerror and precision are microseconds even with STA_NANO. esterror is
    // deliberately not substituted for the maximum-error field.
    let error = maximum
        .checked_add(precision)
        .and_then(|n| n.checked_add(1))
        .and_then(|n| n.checked_mul(1000))
        .ok_or(ClockRefusal::InvalidReading)?;
    Ok((unix, error))
}

fn boottime() -> ClockResult<u64> {
    let mut value = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: fixed clock and valid writable timespec; no retained pointer.
    if unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut value) } != 0 {
        return Err(ClockRefusal::SourceFailed);
    }
    nanos(value.tv_sec.into(), value.tv_nsec.into())
}

fn nanos(seconds: i128, nanos: i128) -> ClockResult<u64> {
    if seconds < 0 || !(0..1_000_000_000).contains(&nanos) {
        return Err(ClockRefusal::InvalidReading);
    }
    let value = seconds
        .checked_mul(1_000_000_000)
        .and_then(|n| n.checked_add(nanos))
        .ok_or(ClockRefusal::InvalidReading)?;
    u64::try_from(value).map_err(|_| ClockRefusal::InvalidReading)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_units_flags_and_overflow_cannot_shrink_uncertainty() {
        // SAFETY: zero is a valid timex, only local decoding occurs in this test.
        let mut t: libc::timex = unsafe { std::mem::zeroed() };
        t.time.tv_sec = 10;
        t.time.tv_usec = 123;
        t.maxerror = 100;
        t.esterror = 0;
        t.precision = 1;
        assert_eq!(
            decode_timex(libc::TIME_OK, &t).unwrap(),
            (10_000_123_000, 102_000)
        );
        t.status = libc::STA_NANO;
        assert_eq!(
            decode_timex(libc::TIME_OK, &t).unwrap(),
            (10_000_000_123, 102_000)
        );
        for flag in [
            libc::STA_UNSYNC,
            libc::STA_CLOCKERR,
            libc::STA_INS,
            libc::STA_DEL,
            libc::STA_PPSFREQ,
            libc::STA_PPSJITTER,
            0x10000,
        ] {
            t.status = flag;
            assert_eq!(
                decode_timex(libc::TIME_OK, &t),
                Err(ClockRefusal::Unsynchronized)
            );
        }
        t.status = 0;
        for state in [
            libc::TIME_ERROR,
            libc::TIME_INS,
            libc::TIME_DEL,
            libc::TIME_OOP,
            libc::TIME_WAIT,
            42,
        ] {
            assert_eq!(decode_timex(state, &t), Err(ClockRefusal::Unsynchronized));
        }
        t.maxerror = -1;
        assert_eq!(
            decode_timex(libc::TIME_OK, &t),
            Err(ClockRefusal::InvalidReading)
        );
        assert_eq!(nanos(i128::MAX, 0), Err(ClockRefusal::InvalidReading));
        assert_eq!(nanos(0, 1_000_000_000), Err(ClockRefusal::InvalidReading));
    }

    #[test]
    fn discontinuity_watch_never_treats_cancellation_or_expiration_as_healthy() {
        assert_eq!(check_watch_result(-1, Some(libc::EAGAIN)), Ok(()));
        for (n, errno) in [(-1, Some(libc::ECANCELED)), (8, None)] {
            assert_eq!(
                check_watch_result(n, errno),
                Err(ClockRefusal::Discontinuity)
            );
        }
        for (n, errno) in [
            (0, None),
            (7, None),
            (-1, Some(libc::EINTR)),
            (-1, Some(libc::EBADF)),
        ] {
            assert_eq!(
                check_watch_result(n, errno),
                Err(ClockRefusal::SourceFailed)
            );
        }
    }

    #[test]
    fn live_adapter_only_observes_the_host_and_never_qualifies_it() {
        let clock = LinuxTtlClock::new(super::super::tests::policy()).unwrap();
        // No host time mutation. Healthy or unsynchronized host both produce a
        // refusal on the first sample; never assert UTC accuracy from this run.
        assert!(matches!(
            clock.sample(),
            Err(ClockRefusal::WarmingUp
                | ClockRefusal::Unsynchronized
                | ClockRefusal::ExcessiveUncertainty
                | ClockRefusal::SampleTooSlow)
        ));
    }
}
