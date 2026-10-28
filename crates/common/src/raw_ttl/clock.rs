//! Explicit deployment policy for the Linux disciplined TTL clock. These
//! bounds are operator assertions, not automatic NTP/hardware qualification.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TtlClockPolicy {
    /// Maximum accepted radius, including reported error and sampling latency.
    pub max_uncertainty_ms: u64,
    /// Additional qualified UTC error beyond the kernel's reported maxerror.
    /// Covers source/path asymmetry, discipline lag and quantization assumptions.
    pub additional_uncertainty_ms: u64,
    /// Maximum duration of one boottime-bracketed kernel observation.
    pub max_sample_span_ms: u64,
    /// Qualified CLOCK_BOOTTIME rate error, including slew and VM behavior.
    pub boottime_drift_ppb: u32,
    /// Qualified absolute error of one CLOCK_BOOTTIME reading, in nanoseconds.
    /// Reported clock_getres is checked, but is not an accuracy guarantee.
    pub boottime_error_ns: u64,
    /// Two consistent observations must span this interval before issuing time.
    /// This detects some startup anomalies; it does not qualify the time source.
    pub warmup_ms: u64,
}

impl TtlClockPolicy {
    pub fn validate(self) -> crate::Result<()> {
        if !(1..=60_000).contains(&self.max_uncertainty_ms)
            || self.additional_uncertainty_ms == 0
            || self.additional_uncertainty_ms >= self.max_uncertainty_ms
            || !(1..=1_000).contains(&self.max_sample_span_ms)
            || self.boottime_drift_ppb >= 1_000_000_000
            || !(1..=1_000_000).contains(&self.boottime_error_ns)
            || !(1..=60_000).contains(&self.warmup_ms)
        {
            return Err(crate::Error::Config("invalid Raw TTL clock policy".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum ClockRefusal {
    #[error("TTL clock provider is not configured or the platform is unsupported")]
    Unavailable,
    #[error("TTL clock is warming up")]
    WarmingUp,
    #[error("kernel clock is unsynchronized, faulty or adjusting a leap second")]
    Unsynchronized,
    #[error("TTL clock source returned invalid or overflowing time")]
    InvalidReading,
    #[error("TTL clock sampling exceeded its duration bound")]
    SampleTooSlow,
    #[error("TTL clock uncertainty exceeds the configured bound")]
    ExcessiveUncertainty,
    #[error("TTL clock stepped or violates its elapsed-time bound")]
    Discontinuity,
    #[error("TTL clock source failed")]
    SourceFailed,
    #[error("TTL clock is fenced until explicit reconstruction")]
    Fenced,
    #[error("TTL clock is behind the group's committed watermark")]
    BehindWatermark,
}
