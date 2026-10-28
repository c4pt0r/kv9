//! Raw TTL time and record primitives. These do not enable TTL in the public API.
//!
//! A proposer resolves time once; apply and replay consume the resulting numbers
//! without reading a clock. Expiry records are separate from opaque user values.
//! A cleanup decision must compare the exact observed record during ordered apply.

use crate::{AppliedPosition, Error, Result, RootDigest};

pub mod clock;

const EXPIRY_MAGIC: &[u8; 8] = b"KV9TTL01";
const CLOCK_MAGIC: &[u8; 8] = b"KV9CLK01";
const PLAN_MAGIC: &[u8; 8] = b"KV9TPL01";
/// Reserved default-CF record, separate from the Raw user-key namespace.
pub const CLOCK_KEY: &[u8] = b"s\0\0\0kv9.raw-ttl-clock.v1";
pub const EXPIRY_RECORD_BYTES: usize = 64;
pub const CLOCK_RECORD_BYTES: usize = 16;
pub const EXPIRY_PLAN_BYTES: usize = 25;

/// Deterministic refusals from the group's ordered TTL apply path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TtlRejection {
    NotEnabled,
    LegacyCommand,
    InvalidScope,
}

/// A clock provider's interval containing current UNIX time, in milliseconds.
/// Construction checks arithmetic, not whether a host actually meets this bound.
/// The runtime must establish the configured uncertainty before issuing samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockSample {
    earliest_ms: u64,
    latest_ms: u64,
}

impl ClockSample {
    /// Arithmetic validation only; the provider must establish interval coverage.
    pub fn from_interval(earliest_ms: u64, latest_ms: u64) -> Result<Self> {
        if earliest_ms > latest_ms {
            return Err(Error::Config("inverted TTL clock interval".into()));
        }
        Ok(Self {
            earliest_ms,
            latest_ms,
        })
    }

    pub fn interval_ms(self) -> (u64, u64) {
        (self.earliest_ms, self.latest_ms)
    }

    pub fn with_uncertainty(unix_ms: u64, uncertainty_ms: u64) -> Result<Self> {
        let latest_ms = unix_ms.checked_add(uncertainty_ms).ok_or_else(|| {
            Error::Config("TTL clock interval overflows UNIX milliseconds".into())
        })?;
        Ok(Self {
            earliest_ms: unix_ms.saturating_sub(uncertainty_ms),
            latest_ms,
        })
    }

    /// A backwards sample outside its uncertainty cannot authorize expiry.
    /// Small regressions retain the previously committed monotonic floor.
    pub fn advance(self, committed: ClockWatermark) -> Result<ClockWatermark> {
        if self.latest_ms < committed.0 {
            return Err(Error::Config(
                "TTL clock is behind the committed watermark outside its uncertainty".into(),
            ));
        }
        Ok(committed.merge(ClockWatermark(self.earliest_ms)))
    }
}

/// Group-replicated time. Zero is the initial watermark, not a local wall clock.
/// Keep this record in the same atomic engine snapshot as the values it governs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct ClockWatermark(u64);

impl ClockWatermark {
    pub fn unix_ms(self) -> u64 {
        self.0
    }

    /// Ordered apply may receive earlier sampled proposals after newer ones.
    /// Split inheritance and merge must also retain the maximum source floor.
    pub fn merge(self, proposed: Self) -> Self {
        Self(self.0.max(proposed.0))
    }

    pub fn encode(self) -> [u8; CLOCK_RECORD_BYTES] {
        let mut bytes = [0; CLOCK_RECORD_BYTES];
        bytes[..8].copy_from_slice(CLOCK_MAGIC);
        bytes[8..].copy_from_slice(&self.0.to_be_bytes());
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != CLOCK_RECORD_BYTES || &bytes[..8] != CLOCK_MAGIC {
            return Err(Error::Engine("invalid Raw TTL clock record".into()));
        }
        Ok(Self(u64::from_be_bytes(bytes[8..].try_into().unwrap())))
    }
}

/// Immutable time fields to include in one proposed put/batch. Retrying that
/// proposal reuses this plan; an apply/replay path never recomputes its deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpiryPlan {
    observed: ClockWatermark,
    expires_at_ms: Option<u64>,
}

impl ExpiryPlan {
    /// Clearing expiry and expiring at apply need no new UTC observation.
    /// They retain the already committed floor even when the clock is fenced.
    pub fn permanent(committed: ClockWatermark) -> Self {
        Self {
            observed: committed,
            expires_at_ms: None,
        }
    }

    pub fn immediate(committed: ClockWatermark) -> Self {
        Self {
            observed: committed,
            expires_at_ms: Some(committed.0),
        }
    }

    /// `None` clears previous expiry, `Some(0)` expires at this write's apply,
    /// and a positive TTL starts at the first proposal's conservative clock
    /// upper bound. Overflow refuses before any mutation is constructed.
    pub fn resolve(
        ttl_secs: Option<u64>,
        sample: ClockSample,
        committed: ClockWatermark,
    ) -> Result<Self> {
        let observed = sample.advance(committed)?;
        let expires_at_ms = match ttl_secs {
            None => None,
            Some(0) => Some(observed.0),
            Some(seconds) => Some(
                seconds
                    .checked_mul(1000)
                    .and_then(|millis| sample.latest_ms.checked_add(millis))
                    .ok_or_else(|| Error::Config("Raw TTL deadline overflows".into()))?,
            ),
        };
        Ok(Self {
            observed,
            expires_at_ms,
        })
    }

    pub fn observed(self) -> ClockWatermark {
        self.observed
    }

    pub fn expires_at_ms(self) -> Option<u64> {
        self.expires_at_ms
    }

    pub fn encode(self) -> [u8; EXPIRY_PLAN_BYTES] {
        let mut bytes = [0; EXPIRY_PLAN_BYTES];
        bytes[..8].copy_from_slice(PLAN_MAGIC);
        bytes[8..16].copy_from_slice(&self.observed.0.to_be_bytes());
        if let Some(deadline) = self.expires_at_ms {
            bytes[16] = 1;
            bytes[17..].copy_from_slice(&deadline.to_be_bytes());
        }
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != EXPIRY_PLAN_BYTES || &bytes[..8] != PLAN_MAGIC {
            return Err(Error::Engine(
                "invalid Raw TTL proposal time encoding".into(),
            ));
        }
        let observed = ClockWatermark(u64::from_be_bytes(bytes[8..16].try_into().unwrap()));
        let deadline = u64::from_be_bytes(bytes[17..].try_into().unwrap());
        let expires_at_ms = match bytes[16] {
            0 if deadline == 0 => None,
            1 if deadline >= observed.0 => Some(deadline),
            _ => return Err(Error::Engine("invalid Raw TTL proposal deadline".into())),
        };
        Ok(Self {
            observed,
            expires_at_ms,
        })
    }

    /// Mint the cleanup identity at ordered apply, when the exact log position
    /// is known. A repeated key in one atomic batch follows normal last-put wins.
    pub fn at_write(
        self,
        creation: RootDigest,
        position: AppliedPosition,
    ) -> Result<Option<ExpiryRecord>> {
        self.expires_at_ms
            .map(|expires_at_ms| ExpiryRecord::new(creation, position, expires_at_ms))
            .transpose()
    }
}

/// Metadata for one expiring Raw value. This encoding belongs in a separate
/// metadata namespace; arbitrary legacy user bytes must never be sniffed for it.
/// The source creation digest stays unchanged when a version is copied at split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpiryRecord {
    creation: RootDigest,
    position: AppliedPosition,
    expires_at_ms: u64,
}

impl ExpiryRecord {
    pub fn new(
        creation: RootDigest,
        position: AppliedPosition,
        expires_at_ms: u64,
    ) -> Result<Self> {
        if creation.as_bytes() == &[0; 32] || position.term == 0 || position.index == 0 {
            return Err(Error::Engine("invalid Raw TTL write identity".into()));
        }
        Ok(Self {
            creation,
            position,
            expires_at_ms,
        })
    }

    pub fn expired_at(self, watermark: ClockWatermark) -> bool {
        watermark.0 >= self.expires_at_ms
    }

    /// Only a matching current version that is still expired can be removed.
    /// The caller must do this comparison and deletion in the same ordered apply.
    pub fn permits_cleanup(self, current: Option<Self>, watermark: ClockWatermark) -> bool {
        current == Some(self) && self.expired_at(watermark)
    }

    pub fn encode(self) -> [u8; EXPIRY_RECORD_BYTES] {
        let mut bytes = [0; EXPIRY_RECORD_BYTES];
        bytes[..8].copy_from_slice(EXPIRY_MAGIC);
        bytes[8..40].copy_from_slice(self.creation.as_bytes());
        bytes[40..48].copy_from_slice(&self.position.term.to_be_bytes());
        bytes[48..56].copy_from_slice(&self.position.index.to_be_bytes());
        bytes[56..].copy_from_slice(&self.expires_at_ms.to_be_bytes());
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != EXPIRY_RECORD_BYTES || &bytes[..8] != EXPIRY_MAGIC {
            return Err(Error::Engine("invalid Raw TTL expiry record".into()));
        }
        Self::new(
            RootDigest::from_bytes(bytes[8..40].try_into().unwrap()),
            AppliedPosition {
                term: u64::from_be_bytes(bytes[40..48].try_into().unwrap()),
                index: u64::from_be_bytes(bytes[48..56].try_into().unwrap()),
            },
            u64::from_be_bytes(bytes[56..].try_into().unwrap()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(now: u64, uncertainty: u64) -> ClockSample {
        ClockSample::with_uncertainty(now, uncertainty).unwrap()
    }

    fn record(index: u64, expires: u64) -> ExpiryRecord {
        ExpiryRecord::new(
            RootDigest::from_bytes([1; 32]),
            AppliedPosition { term: 1, index },
            expires,
        )
        .unwrap()
    }

    #[test]
    fn clock_rollback_cannot_resurrect_an_expired_version() {
        let current = sample(2000, 50).advance(ClockWatermark::default()).unwrap();
        assert_eq!(current.unix_ms(), 1950);
        let expired = record(1, 1950);
        assert!(expired.expired_at(current));
        let regressed = sample(1940, 50).advance(current).unwrap();
        assert_eq!(regressed, current);
        assert!(expired.expired_at(regressed));
        assert!(sample(1899, 50).advance(current).is_err());
        assert!(ClockSample::with_uncertainty(u64::MAX, 1).is_err());
    }

    #[test]
    fn resolution_handles_absence_zero_boundaries_and_overflow() {
        let current = ClockWatermark::default();
        let clock = sample(1000, 25);
        let absent = ExpiryPlan::resolve(None, clock, current).unwrap();
        assert_eq!(absent.expires_at_ms(), None);
        assert_eq!(
            absent
                .at_write(
                    RootDigest::from_bytes([1; 32]),
                    AppliedPosition { term: 1, index: 1 }
                )
                .unwrap(),
            None
        );
        let zero = ExpiryPlan::resolve(Some(0), clock, current).unwrap();
        assert_eq!(zero.expires_at_ms(), Some(975));
        let positive = ExpiryPlan::resolve(Some(1), clock, current).unwrap();
        assert_eq!(positive.expires_at_ms(), Some(2025));
        assert!(!record(1, 2025).expired_at(ClockWatermark(2024)));
        assert!(record(1, 2025).expired_at(ClockWatermark(2025)));
        assert!(ExpiryPlan::resolve(Some(u64::MAX), clock, current).is_err());
        assert!(ExpiryPlan::resolve(Some(1), sample(u64::MAX - 999, 0), current).is_err());
    }

    #[test]
    fn proposal_encoding_freezes_time_and_rejects_ambiguous_deadlines() {
        for ttl in [None, Some(0), Some(1), Some(86400)] {
            let plan =
                ExpiryPlan::resolve(ttl, sample(1000, 25), ClockWatermark::default()).unwrap();
            let bytes = plan.encode();
            assert_eq!(ExpiryPlan::decode(&bytes).unwrap(), plan);
            for length in 0..bytes.len() {
                assert!(ExpiryPlan::decode(&bytes[..length]).is_err());
            }
            let mut bad = bytes.to_vec();
            bad.push(0);
            assert!(ExpiryPlan::decode(&bad).is_err());
            let mut bad = bytes;
            bad[16] = 2;
            assert!(ExpiryPlan::decode(&bad).is_err());
            let mut bad = bytes;
            bad[16] = 1;
            bad[17..].copy_from_slice(&974u64.to_be_bytes());
            assert!(ExpiryPlan::decode(&bad).is_err());
            let mut bad = bytes;
            bad[16] = 0;
            bad[17..].copy_from_slice(&1u64.to_be_bytes());
            assert!(ExpiryPlan::decode(&bad).is_err());
        }
    }

    #[test]
    fn expiry_and_clock_encodings_reject_truncation_trailing_bytes_and_bad_identity() {
        let original = record(9, 12345);
        let bytes = original.encode();
        assert_eq!(ExpiryRecord::decode(&bytes).unwrap(), original);
        for n in 0..bytes.len() {
            assert!(ExpiryRecord::decode(&bytes[..n]).is_err());
        }
        let mut bad = bytes.to_vec();
        bad.push(0);
        assert!(ExpiryRecord::decode(&bad).is_err());
        for range in [0..8, 8..40, 40..48, 48..56] {
            let mut bad = bytes;
            bad[range].fill(0);
            assert!(ExpiryRecord::decode(&bad).is_err());
        }
        let watermark = ClockWatermark(12345);
        let bytes = watermark.encode();
        assert_eq!(ClockWatermark::decode(&bytes).unwrap(), watermark);
        for n in 0..bytes.len() {
            assert!(ClockWatermark::decode(&bytes[..n]).is_err());
        }
        let mut bad = bytes.to_vec();
        bad.push(0);
        assert!(ClockWatermark::decode(&bad).is_err());
    }

    #[test]
    fn cleanup_refuses_renewal_nonexpiring_overwrite_and_other_group_version() {
        let observed = record(1, 1000);
        let now = ClockWatermark(2000);
        assert!(observed.permits_cleanup(Some(observed), now));
        assert!(!observed.permits_cleanup(Some(observed), ClockWatermark(999)));
        assert!(!observed.permits_cleanup(Some(record(2, 1000)), now));
        assert!(!observed.permits_cleanup(Some(record(2, 3000)), now));
        assert!(!observed.permits_cleanup(None, now));
        let other =
            ExpiryRecord::new(RootDigest::from_bytes([2; 32]), observed.position, 1000).unwrap();
        assert!(!observed.permits_cleanup(Some(other), now));
    }

    #[test]
    fn delayed_proposal_and_split_copy_keep_the_original_deadline() {
        let plan =
            ExpiryPlan::resolve(Some(1), sample(1000, 0), ClockWatermark::default()).unwrap();
        let applied = sample(3000, 0)
            .advance(ClockWatermark::default())
            .unwrap()
            .merge(plan.observed());
        let original = plan
            .at_write(
                RootDigest::from_bytes([1; 32]),
                AppliedPosition { term: 1, index: 9 },
            )
            .unwrap()
            .unwrap();
        assert!(original.expired_at(applied));
        let copied = ExpiryRecord::decode(&original.encode()).unwrap();
        assert!(copied.expired_at(applied));
        assert_eq!(copied, original);
        assert_eq!(applied.merge(ClockWatermark(1000)), applied);
        assert_eq!(ClockWatermark(1000).merge(applied), applied);
    }
}
