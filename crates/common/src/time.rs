//! Timestamps and clocks (DESIGN §8).
//!
//! `txn` keyspaces need a snapshot-isolation timestamp order. In kv9 that order is
//! **per txn group, not global** (DESIGN §8.1). This module defines the timestamp
//! value type plus an `Hlc` (Hybrid Logical Clock) as one of the three oracle
//! realizations described in DESIGN §8.2.

use serde::{Deserialize, Serialize};

/// A monotonically increasing transaction timestamp within a single txn group's
/// timeline (DESIGN §8). `0` is reserved as the "zero / uninitialized" timestamp.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct TimeStamp(pub u64);

impl TimeStamp {
    /// The zero timestamp — precedes all real timestamps.
    pub const ZERO: TimeStamp = TimeStamp(0);

    /// The maximum timestamp — used as an upper snapshot bound.
    pub const MAX: TimeStamp = TimeStamp(u64::MAX);

    #[inline]
    pub fn into_inner(self) -> u64 {
        self.0
    }

    #[inline]
    pub fn next(self) -> TimeStamp {
        TimeStamp(self.0.saturating_add(1))
    }
}

/// A Hybrid Logical Clock value (DESIGN §8.2 option 2).
///
/// Kept as a first-class type so an HLC-backed oracle can be swapped in for a very
/// hot group without changing the `TimestampOracle` trait surface (DESIGN §8.2).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct Hlc {
    /// Physical component (wall-clock derived), monotonic.
    pub physical: u64,
    /// Logical component, breaks ties at equal physical time.
    pub logical: u32,
}

impl Hlc {
    /// Collapse an HLC into a totally ordered [`TimeStamp`] (physical<<16 | logical).
    pub fn to_timestamp(self) -> TimeStamp {
        TimeStamp((self.physical << 16) | u64::from(self.logical & 0xFFFF))
    }
}

/// Abstract clock/uncertainty source (DESIGN §8.2). Shaped so a TrueTime-style
/// bounded-uncertainty source could be swapped in later for external consistency.
pub trait TimeSource: Send + Sync {
    /// Current physical time in nanoseconds since the unix epoch.
    fn now_nanos(&self) -> u64;

    /// Bound on clock uncertainty in nanoseconds (`0` for a plain monotonic clock;
    /// non-zero only for TrueTime-style sources).
    fn uncertainty_nanos(&self) -> u64 {
        0
    }
}
