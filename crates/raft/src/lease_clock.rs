//! Explicit clock contracts for experimental lease installation.
//!
//! These are deployment assumptions, not measurements or an automatic platform
//! qualification. See `docs/LEASE-CLOCK.md` for the sampling-error proof.

use crate::lease::{Refused, Result, Timing};
use crate::lease_policy::LeasePolicy;

const SCALE: u128 = 1_000_000_000;

/// Bounds on a clock's ideal elapsed rate and each returned sample's absolute
/// error, including quantization. They must cover every supported scheduling,
/// suspend, clock-discipline and virtualization condition on every voter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockBounds {
    pub drift_ppb: u32,
    pub error_ns: u64,
}

impl ClockBounds {
    /// Minimum shared margin for BOTH leader containment and restart quarantine.
    /// With |sample - ideal| <= epsilon, a difference has error <= 2*epsilon.
    /// Recovery requires M >= 2*epsilon*(1+b/a) = 4*epsilon/(1-rho).
    /// This also covers the smaller leader requirement 2*epsilon*(1+a/b).
    pub fn minimum_margin_ns(self, policy_drift_ppb: u32) -> Result<u64> {
        if self.drift_ppb > policy_drift_ppb || u128::from(policy_drift_ppb) >= SCALE {
            return Err(Refused::InvalidTiming);
        }
        let margin =
            (4 * u128::from(self.error_ns) * SCALE).div_ceil(SCALE - u128::from(policy_drift_ppb));
        u64::try_from(margin).map_err(|_| Refused::Overflow)
    }

    pub fn validate_policy(self, policy: &LeasePolicy) -> Result<()> {
        policy
            .validate()
            .map_err(|_| Refused::InvalidConfiguration)?;
        if policy.margin_ns < self.minimum_margin_ns(policy.drift_ppb)? {
            return Err(Refused::InvalidTiming);
        }
        Timing::new(policy.promise_ns, policy.drift_ppb, policy.margin_ns)?;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::LinuxBoottimeClock;

#[cfg(test)]
mod tests;
