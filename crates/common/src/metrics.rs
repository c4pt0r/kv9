//! Bounded, node-local observation. Recording never returns a business error.
//! Each metric keeps seven fixed outcome histograms, not individual samples.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

pub const BUCKETS: usize = 65;
pub const OUTCOMES: usize = 7;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Success,
    Error,
    Aborted,
    Released,
    Replaced,
    Rejected,
    Unconfirmed,
}

impl Outcome {
    pub const ALL: [Self; OUTCOMES] = [
        Self::Success,
        Self::Error,
        Self::Aborted,
        Self::Released,
        Self::Replaced,
        Self::Rejected,
        Self::Unconfirmed,
    ];
}

/// Bucket 0 is exactly zero; bucket i>0 is [2^(i-1), 2^i-1] ns.
pub fn bucket_index(nanos: u64) -> usize {
    (u64::BITS - nanos.leading_zeros()) as usize
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct BucketBounds {
    pub lower_ns: u64,
    pub upper_ns: u64,
}

pub fn bucket_bounds(index: usize) -> Option<BucketBounds> {
    if index >= BUCKETS {
        return None;
    }
    Some(BucketBounds {
        lower_ns: if index == 0 { 0 } else { 1u64 << (index - 1) },
        upper_ns: if index == 64 {
            u64::MAX
        } else {
            (1u64 << index) - 1
        },
    })
}

#[derive(Clone)]
struct Histogram {
    buckets: [u64; BUCKETS],
    count: u64,
    sum_ns: u64,
    min_ns: Option<u64>,
    max_ns: Option<u64>,
    count_saturated: bool,
    sum_saturated: bool,
    duration_clamped: bool,
}
impl Default for Histogram {
    fn default() -> Self {
        Self {
            buckets: [0; BUCKETS],
            count: 0,
            sum_ns: 0,
            min_ns: None,
            max_ns: None,
            count_saturated: false,
            sum_saturated: false,
            duration_clamped: false,
        }
    }
}

fn add(value: &mut u64, amount: u64) -> bool {
    match value.checked_add(amount) {
        Some(next) => {
            *value = next;
            false
        }
        None => {
            *value = u64::MAX;
            true
        }
    }
}

#[derive(Default)]
struct State {
    histograms: [Histogram; OUTCOMES],
    invalid: bool,
}

#[derive(Default)]
pub struct Latency {
    state: Mutex<State>,
}

impl std::fmt::Debug for Latency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Latency").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct HistogramSnapshot {
    pub outcome: Outcome,
    /// Always exactly 65 entries; allocation exists only in an export snapshot.
    pub buckets: Vec<u64>,
    pub count: u64,
    pub sum_ns: u64,
    pub min_ns: Option<u64>,
    pub max_ns: Option<u64>,
    pub count_saturated: bool,
    pub sum_saturated: bool,
    pub duration_clamped: bool,
    pub p50: Option<BucketBounds>,
    pub p95: Option<BucketBounds>,
    pub p99: Option<BucketBounds>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LatencySnapshot {
    /// False after a poisoned observer lock; such data cannot justify quantiles.
    pub valid: bool,
    pub outcomes: [HistogramSnapshot; OUTCOMES],
}

impl Latency {
    pub fn record(&self, elapsed: Duration, outcome: Outcome) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(error) => {
                let mut state = error.into_inner();
                state.invalid = true;
                state
            }
        };
        let histogram = &mut state.histograms[outcome as usize];
        let nanos = elapsed.as_nanos().min(u128::from(u64::MAX)) as u64;
        histogram.duration_clamped |= elapsed.as_nanos() > u128::from(u64::MAX);
        histogram.count_saturated |= add(&mut histogram.count, 1);
        histogram.count_saturated |= add(&mut histogram.buckets[bucket_index(nanos)], 1);
        histogram.sum_saturated |= add(&mut histogram.sum_ns, nanos);
        histogram.min_ns = Some(histogram.min_ns.map_or(nanos, |old| old.min(nanos)));
        histogram.max_ns = Some(histogram.max_ns.map_or(nanos, |old| old.max(nanos)));
    }

    pub fn start(&self) -> Timer<'_> {
        Timer {
            metric: self,
            started: Instant::now(),
            finished: false,
        }
    }

    /// The callback executes once. Its exact result/error or panic is preserved.
    pub fn measure<T, E>(&self, operation: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
        let timer = self.start();
        let result = operation();
        timer.finish(if result.is_ok() {
            Outcome::Success
        } else {
            Outcome::Error
        });
        result
    }

    /// Observe typed results without converting or replacing the original value.
    pub fn observe<T>(
        &self,
        operation: impl FnOnce() -> T,
        classify: impl FnOnce(&T) -> Outcome,
    ) -> T {
        let timer = self.start();
        let result = operation();
        timer.finish(classify(&result));
        result
    }

    pub fn snapshot(&self) -> LatencySnapshot {
        // Clone only the fixed arrays while locked; quantiles and allocation are
        // outside the lock. No collector ever calls database code under this lock.
        let (histograms, invalid) = match self.state.lock() {
            Ok(state) => (state.histograms.clone(), state.invalid),
            Err(error) => (error.into_inner().histograms.clone(), true),
        };
        LatencySnapshot {
            valid: !invalid,
            outcomes: std::array::from_fn(|i| {
                let h = &histograms[i];
                let percentile = |percent: u128| {
                    if invalid || h.count == 0 || h.count_saturated || h.duration_clamped {
                        return None;
                    }
                    let rank = (u128::from(h.count) * percent).div_ceil(100);
                    let mut cumulative = 0u128;
                    for (index, count) in h.buckets.iter().enumerate() {
                        cumulative += u128::from(*count);
                        if cumulative >= rank {
                            return bucket_bounds(index);
                        }
                    }
                    None
                };
                HistogramSnapshot {
                    outcome: Outcome::ALL[i],
                    buckets: h.buckets.to_vec(),
                    count: h.count,
                    sum_ns: h.sum_ns,
                    min_ns: h.min_ns,
                    max_ns: h.max_ns,
                    count_saturated: h.count_saturated,
                    sum_saturated: h.sum_saturated,
                    duration_clamped: h.duration_clamped,
                    p50: percentile(50),
                    p95: percentile(95),
                    p99: percentile(99),
                }
            }),
        }
    }
}

pub struct Timer<'a> {
    metric: &'a Latency,
    started: Instant,
    finished: bool,
}
impl Timer<'_> {
    pub fn finish(mut self, outcome: Outcome) {
        self.metric.record(self.started.elapsed(), outcome);
        self.finished = true;
    }
}
impl Drop for Timer<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.metric.record(self.started.elapsed(), Outcome::Aborted);
        }
    }
}

#[derive(Debug, Default)]
pub struct WalIoMetrics {
    pub write: Latency,
    pub sync: Latency,
    pub recovery_sync: Latency,
    pub namespace_publish: Latency,
}

#[derive(Serialize)]
pub struct NamedLatency {
    pub name: &'static str,
    pub latency: LatencySnapshot,
}

impl NamedLatency {
    pub fn new(name: &'static str, metric: &Latency) -> Self {
        Self {
            name,
            latency: metric.snapshot(),
        }
    }
}

impl WalIoMetrics {
    pub fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_cover_zero_every_power_boundary_and_machine_maximum() {
        let mut values = vec![0, 1, u64::MAX];
        for bit in 0..64 {
            let n = 1u64 << bit;
            values.extend([n - 1, n, n.saturating_add(1)]);
        }
        for value in values {
            let index = bucket_index(value);
            let bounds = bucket_bounds(index).unwrap();
            assert!(bounds.lower_ns <= value && value <= bounds.upper_ns);
            if index > 0 {
                assert_eq!(
                    bucket_bounds(index - 1).unwrap().upper_ns + 1,
                    bounds.lower_ns
                );
            }
        }
        assert!(bucket_bounds(BUCKETS).is_none());
    }

    #[test]
    fn quantiles_are_explicit_intervals_and_empty_samples_are_absent() {
        let metric = Latency::default();
        assert_eq!(metric.snapshot().outcomes[0].p99, None);
        for nanos in [0, 1, 2, 3, 4, 5, 7, 8, 9, 16] {
            metric.record(Duration::from_nanos(nanos), Outcome::Success);
        }
        let s = metric.snapshot();
        let h = &s.outcomes[0];
        assert_eq!(
            (h.count, h.sum_ns, h.min_ns, h.max_ns),
            (10, 55, Some(0), Some(16))
        );
        assert_eq!(h.buckets.iter().sum::<u64>(), h.count);
        assert_eq!(
            h.p50,
            Some(BucketBounds {
                lower_ns: 4,
                upper_ns: 7
            })
        );
        assert_eq!(
            h.p95,
            Some(BucketBounds {
                lower_ns: 16,
                upper_ns: 31
            })
        );
    }

    #[test]
    fn saturation_and_clamping_are_explicit_without_wrapping() {
        let metric = Latency::default();
        metric.record(Duration::from_nanos(u64::MAX), Outcome::Success);
        metric.record(Duration::from_nanos(1), Outcome::Success);
        let s = metric.snapshot();
        assert!(s.outcomes[0].sum_saturated);
        assert!(s.outcomes[0].p99.is_some());
        metric.state.lock().unwrap().histograms[0].count = u64::MAX;
        metric.record(Duration::ZERO, Outcome::Success);
        let s = metric.snapshot();
        assert!(s.outcomes[0].count_saturated);
        assert_eq!(s.outcomes[0].p50, None);
        metric.record(Duration::from_secs(u64::MAX), Outcome::Error);
        let s = metric.snapshot();
        assert!(s.outcomes[1].duration_clamped);
        assert_eq!(s.outcomes[1].p99, None);
    }

    #[test]
    fn observer_preserves_callback_results_panics_and_poisoned_lock_progress() {
        let metric = Latency::default();
        assert_eq!(metric.measure(|| Ok::<_, u32>(42)), Ok(42));
        assert_eq!(metric.measure(|| Err::<u32, _>(17)), Err(17));
        assert!(std::panic::catch_unwind(
            || metric.measure(|| -> Result<(), ()> { panic!("backend panic") })
        )
        .is_err());
        assert_eq!(metric.snapshot().outcomes[2].count, 1);
        assert!(std::panic::catch_unwind(|| {
            let _guard = metric.state.lock().unwrap();
            panic!("observer poison")
        })
        .is_err());
        assert_eq!(metric.measure(|| Ok::<_, u32>(99)), Ok(99));
        let s = metric.snapshot();
        assert!(!s.valid);
        assert_eq!(s.outcomes[0].count, 2);
        assert_eq!(s.outcomes[0].p99, None);
    }
}
