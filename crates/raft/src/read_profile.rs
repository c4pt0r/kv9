//! Diagnostic-only sampled async read lifecycle. No observation is authority.
//! Keep this instrumentation out of performance-candidate comparisons.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use kv9_common::metrics::Outcome;

use crate::driver::DriverMetrics;

pub(crate) struct ReadTrace {
    started: Instant,
    submitted: AtomicU64,
    confirmed: AtomicU64,
    sent: AtomicU64,
    metrics: Arc<DriverMetrics>,
}

impl ReadTrace {
    pub(crate) fn sample(
        context: &[u8; 24],
        started: Instant,
        metrics: &Arc<DriverMetrics>,
    ) -> Option<Arc<Self>> {
        // Mix the checked sequence with the incarnation instead of selecting
        // every 64th member, which can alias with sealed group boundaries.
        let seed = u64::from_be_bytes(context[..8].try_into().unwrap());
        let sequence = u64::from_be_bytes(context[16..].try_into().unwrap());
        let mut mixed = (sequence ^ seed).wrapping_add(0x9e3779b97f4a7c15);
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94d049bb133111eb);
        mixed ^= mixed >> 31;
        if mixed & 63 != 0 {
            return None;
        }
        Some(Arc::new(Self {
            started,
            submitted: AtomicU64::new(0),
            confirmed: AtomicU64::new(0),
            sent: AtomicU64::new(0),
            metrics: metrics.clone(),
        }))
    }

    fn stamp(&self, at: Instant) -> u64 {
        at.checked_duration_since(self.started)
            .and_then(|duration| u64::try_from(duration.as_nanos()).ok())
            .and_then(|nanos| nanos.checked_add(1))
            .unwrap_or(0)
    }

    pub(crate) fn submitted(&self, at: Instant) {
        self.submitted.store(self.stamp(at), Ordering::Release);
    }

    pub(crate) fn confirmed(&self) {
        self.confirmed
            .store(self.stamp(Instant::now()), Ordering::Release);
    }

    pub(crate) fn sent(&self) {
        self.sent
            .store(self.stamp(Instant::now()), Ordering::Release);
    }

    pub(crate) fn observed(&self) {
        // Capture before histogram locks. All five histograms use exactly this
        // successful received sample; recording cost is outside its intervals.
        let received = self.stamp(Instant::now());
        let submitted = self.submitted.load(Ordering::Acquire);
        let confirmed = self.confirmed.load(Ordering::Acquire);
        let sent = self.sent.load(Ordering::Acquire);
        let Some([queue, quorum, apply, notification, total]) =
            intervals([submitted, confirmed, sent, received])
        else {
            self.metrics
                .read_profile_total
                .record(Duration::ZERO, Outcome::Error);
            return;
        };
        for (metric, nanos) in [
            (&self.metrics.read_profile_queue, queue),
            (&self.metrics.read_profile_quorum, quorum),
            (&self.metrics.read_profile_apply, apply),
            (&self.metrics.read_profile_notification, notification),
            (&self.metrics.read_profile_total, total),
        ] {
            metric.record(Duration::from_nanos(nanos), Outcome::Success);
        }
    }
}

fn intervals([submitted, confirmed, sent, received]: [u64; 4]) -> Option<[u64; 5]> {
    if submitted == 0 || submitted > confirmed || confirmed > sent || sent > received {
        return None;
    }
    Some([
        submitted - 1,
        confirmed - submitted,
        sent - confirmed,
        received - sent,
        received - 1,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_share_one_population_and_partition_total_duration() {
        assert_eq!(intervals([11, 41, 61, 101]), Some([10, 30, 20, 40, 100]));
        assert_eq!(intervals([1, 1, 1, 1]), Some([0; 5]));
        for stamps in [[0, 1, 2, 3], [5, 4, 6, 7], [3, 4, 2, 7], [3, 4, 5, 0]] {
            assert_eq!(intervals(stamps), None);
        }
    }
}
