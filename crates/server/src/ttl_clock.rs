//! TTL UTC interval provider, separate from Raft lease clocks and transaction
//! timestamps. Policy bounds require deployment qualification. Healthy samples
//! are prepared before proposal; apply/replay never invokes this provider.
use kv9_common::raw_ttl::clock::{ClockRefusal, TtlClockPolicy};
use kv9_common::raw_ttl::ClockSample;
use std::sync::Mutex;

pub(crate) fn configured(
    policy: Option<TtlClockPolicy>,
) -> kv9_common::Result<Option<std::sync::Arc<dyn TtlClock>>> {
    let Some(policy) = policy else {
        return Ok(None);
    };
    #[cfg(target_os = "linux")]
    {
        Ok(Some(std::sync::Arc::new(LinuxTtlClock::new(policy)?)))
    }
    #[cfg(not(target_os = "linux"))]
    {
        policy.validate()?;
        Err(kv9_common::Error::RawTtlClock {
            reason: ClockRefusal::Unavailable,
        })
    }
}

pub type ClockResult<T> = std::result::Result<T, ClockRefusal>;

pub trait TtlClock: Send + Sync {
    /// Return a fresh interval enclosing true UNIX time at this observation,
    /// including source error and sampling latency under a qualified model.
    /// Unknown bounds must refuse; status or an old sample is not a substitute.
    fn sample(&self) -> ClockResult<ClockSample>;
    /// Last observation only, not a current serving or time-advancement permit.
    fn status(&self) -> ClockStatus;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClockHealth {
    WarmingUp,
    Healthy,
    Fenced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ClockStatus {
    pub policy: TtlClockPolicy,
    pub health: ClockHealth,
    pub last_refusal: Option<ClockRefusal>,
    pub observations: u64,
    pub uncertainty_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug)]
struct Observation {
    unix_ns: u64,
    reported_error_ns: u64,
    boot_before_ns: u64,
    boot_after_ns: u64,
}

trait Source: Send {
    fn read(&mut self) -> ClockResult<Observation>;
}

#[derive(Clone, Copy)]
struct Bound {
    lower_ns: u64,
    upper_ns: u64,
    boot_ns: u64,
}

struct State<S> {
    source: S,
    anchor: Option<Bound>,
    previous: Option<Bound>,
    status: ClockStatus,
}

struct Monitor<S> {
    policy: TtlClockPolicy,
    state: Mutex<State<S>>,
}

const SCALE: u128 = 1_000_000_000;
const MILLION: u64 = 1_000_000;

impl<S: Source> Monitor<S> {
    fn new(policy: TtlClockPolicy, source: S) -> kv9_common::Result<Self> {
        policy.validate()?;
        Ok(Self {
            policy,
            state: Mutex::new(State {
                source,
                anchor: None,
                previous: None,
                status: ClockStatus {
                    policy,
                    health: ClockHealth::WarmingUp,
                    last_refusal: Some(ClockRefusal::WarmingUp),
                    observations: 0,
                    uncertainty_ms: None,
                },
            }),
        })
    }

    /// Account for both endpoint sample errors and qualified elapsed rate.
    fn elapsed(&self, delta: u64) -> ClockResult<(u64, u64)> {
        let error = self.policy.boottime_error_ns * 2;
        let drift = u128::from(self.policy.boottime_drift_ppb);
        let lower = u128::from(delta.saturating_sub(error)) * SCALE / (SCALE + drift);
        let upper = ((u128::from(delta) + u128::from(error)) * SCALE).div_ceil(SCALE - drift);
        Ok((
            u64::try_from(lower).map_err(|_| ClockRefusal::InvalidReading)?,
            u64::try_from(upper).map_err(|_| ClockRefusal::InvalidReading)?,
        ))
    }

    fn qualify(&self, state: &mut State<S>, observation: Observation) -> ClockResult<ClockSample> {
        let span = observation
            .boot_after_ns
            .checked_sub(observation.boot_before_ns)
            .ok_or(ClockRefusal::Discontinuity)?;
        let (_, span) = self.elapsed(span)?;
        if span > self.policy.max_sample_span_ms * MILLION {
            return Err(ClockRefusal::SampleTooSlow);
        }
        let radius = observation
            .reported_error_ns
            .checked_add(self.policy.additional_uncertainty_ms * MILLION)
            .and_then(|n| n.checked_add(span))
            .ok_or(ClockRefusal::InvalidReading)?;
        if radius > self.policy.max_uncertainty_ms * MILLION {
            return Err(ClockRefusal::ExcessiveUncertainty);
        }
        let mut current = Bound {
            lower_ns: observation.unix_ns.saturating_sub(radius),
            upper_ns: observation
                .unix_ns
                .checked_add(radius)
                .ok_or(ClockRefusal::InvalidReading)?,
            boot_ns: observation.boot_after_ns,
        };
        // Check the original anchor AND the previous observation: repeatedly
        // rebasing a permissive step threshold must not hide cumulative drift.
        for prior in [state.anchor, state.previous].into_iter().flatten() {
            let delta = current
                .boot_ns
                .checked_sub(prior.boot_ns)
                .ok_or(ClockRefusal::Discontinuity)?;
            let (minimum, maximum) = self.elapsed(delta)?;
            let low = prior
                .lower_ns
                .checked_add(minimum)
                .ok_or(ClockRefusal::InvalidReading)?;
            let high = prior
                .upper_ns
                .checked_add(maximum)
                .ok_or(ClockRefusal::InvalidReading)?;
            current.lower_ns = current.lower_ns.max(low);
            current.upper_ns = current.upper_ns.min(high);
            if current.lower_ns > current.upper_ns {
                return Err(ClockRefusal::Discontinuity);
            }
        }
        let anchor = *state.anchor.get_or_insert(current);
        state.previous = Some(current);
        state.status.observations = state.status.observations.saturating_add(1);
        state.status.uncertainty_ms = Some(radius.div_ceil(MILLION));
        if state.status.observations < 2
            || self.elapsed(current.boot_ns - anchor.boot_ns)?.0 < self.policy.warmup_ms * MILLION
        {
            return Err(ClockRefusal::WarmingUp);
        }
        // Round outwards, never round an earliest bound up or a latest down.
        ClockSample::from_interval(
            current.lower_ns / MILLION,
            current.upper_ns.div_ceil(MILLION),
        )
        .map_err(|_| ClockRefusal::InvalidReading)
    }
}

impl<S: Source> TtlClock for Monitor<S> {
    fn sample(&self) -> ClockResult<ClockSample> {
        let mut state = self.state.lock().map_err(|_| ClockRefusal::Fenced)?;
        if state.status.health == ClockHealth::Fenced {
            return Err(ClockRefusal::Fenced);
        }
        let result = state
            .source
            .read()
            .and_then(|reading| self.qualify(&mut state, reading));
        match result {
            Ok(_) => {
                state.status.health = ClockHealth::Healthy;
                state.status.last_refusal = None;
            }
            Err(ClockRefusal::WarmingUp) => {
                state.status.last_refusal = Some(ClockRefusal::WarmingUp)
            }
            Err(reason) => {
                state.status.health = ClockHealth::Fenced;
                state.status.last_refusal = Some(reason);
            }
        }
        result
    }

    fn status(&self) -> ClockStatus {
        self.state
            .lock()
            .map(|state| state.status)
            .unwrap_or(ClockStatus {
                policy: self.policy,
                health: ClockHealth::Fenced,
                last_refusal: Some(ClockRefusal::Fenced),
                observations: 0,
                uncertainty_ms: None,
            })
    }
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::LinuxTtlClock;

#[cfg(test)]
mod tests;
