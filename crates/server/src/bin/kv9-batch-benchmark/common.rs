//! Shared deterministic batch data, offered-slot arithmetic and latency buckets.
//! Both native KV9 and RESP2 Redis clients compile these exact functions.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Load {
    ClosedLoop,
    FixedRate { batches_per_second: u64 },
}

pub fn mix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e3779b97f4a7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}

pub fn key(run_id: &str, index: usize) -> Vec<u8> {
    format!("{run_id}:{index:016x}").into_bytes()
}

pub fn value(seed: u64, bytes: usize, key_index: usize, nonce: u64) -> Vec<u8> {
    let mut result = vec![0; bytes];
    result[..8].copy_from_slice(&(key_index as u64).to_be_bytes());
    result[8..16].copy_from_slice(&nonce.to_be_bytes());
    for (i, chunk) in result[16..].chunks_mut(8).enumerate() {
        let word = mix(seed ^ mix(key_index as u64) ^ mix(nonce) ^ i as u64).to_be_bytes();
        chunk.copy_from_slice(&word[..chunk.len()]);
    }
    result
}

pub fn first_key(seed: u64, nonce: u64, keys: usize) -> usize {
    (mix(seed ^ nonce ^ 0xc6275c213842315b) % keys as u64) as usize
}

pub fn is_read(seed: u64, nonce: u64, read_percent: u8) -> bool {
    mix(seed ^ nonce) % 100 < u64::from(read_percent)
}

pub fn offered_slots(load: Load, measure_ms: u64) -> Option<u64> {
    match load {
        Load::ClosedLoop => None,
        Load::FixedRate { batches_per_second } => {
            Some((measure_ms * batches_per_second).div_ceil(1000))
        }
    }
}

pub fn slot_due_ns(load: Load, slot: u64) -> Option<u64> {
    match load {
        Load::ClosedLoop => None,
        Load::FixedRate { batches_per_second } => {
            Some((u128::from(slot) * 1_000_000_000 / u128::from(batches_per_second)) as u64)
        }
    }
}

pub fn latest_due_slot(
    load: Load,
    measure_ms: u64,
    elapsed_ns: u64,
    worker: usize,
    workers: usize,
) -> Option<u64> {
    let Load::FixedRate { batches_per_second } = load else {
        return None;
    };
    let last = ((u128::from(elapsed_ns) + 1) * u128::from(batches_per_second) - 1) / 1_000_000_000;
    let last = (last as u64).min(offered_slots(load, measure_ms)?.saturating_sub(1));
    let worker = worker as u64;
    (last >= worker).then(|| worker + ((last - worker) / workers as u64) * workers as u64)
}

/// Logarithmic histogram with 64 subdivisions per power of two. Quantiles are
/// reported as bucket bounds, never as fabricated exact samples. Small integer
/// nanosecond values are represented exactly. Allocation is lazy and bounded.
pub const BUCKETS: usize = 64 + (64 - 6) * 64;

#[derive(Clone, Debug, Serialize)]
pub struct Histogram {
    pub count: u64,
    pub sum_ns: u64,
    pub min_ns: Option<u64>,
    pub max_ns: Option<u64>,
    pub valid: bool,
    pub buckets: Vec<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Bounds {
    pub lower_ns: u64,
    pub upper_ns: u64,
}

pub fn bucket(nanos: u64) -> usize {
    if nanos < 64 {
        return nanos as usize;
    }
    let shift = 63 - nanos.leading_zeros() - 6;
    64 + shift as usize * 64 + ((nanos >> shift) as usize - 64)
}

pub fn bounds(index: usize) -> Bounds {
    assert!(index < BUCKETS);
    if index < 64 {
        return Bounds {
            lower_ns: index as u64,
            upper_ns: index as u64,
        };
    }
    let shift = (index - 64) / 64;
    let lower = ((64 + (index - 64) % 64) as u64) << shift;
    Bounds {
        lower_ns: lower,
        upper_ns: lower + ((1u64 << shift) - 1),
    }
}

impl Default for Histogram {
    fn default() -> Self {
        Self {
            count: 0,
            sum_ns: 0,
            min_ns: None,
            max_ns: None,
            valid: true,
            buckets: Vec::new(),
        }
    }
}

impl Histogram {
    pub fn add(&mut self, nanos: u64) {
        if self.buckets.is_empty() {
            self.buckets.resize(BUCKETS, 0);
        }
        let index = bucket(nanos);
        match (
            self.count.checked_add(1),
            self.sum_ns.checked_add(nanos),
            self.buckets[index].checked_add(1),
        ) {
            (Some(count), Some(sum), Some(bucket_count)) => {
                self.count = count;
                self.sum_ns = sum;
                self.buckets[index] = bucket_count;
                self.min_ns = Some(self.min_ns.map_or(nanos, |old| old.min(nanos)));
                self.max_ns = Some(self.max_ns.map_or(nanos, |old| old.max(nanos)));
            }
            _ => self.valid = false,
        }
    }

    pub fn merge(&mut self, other: &Self) {
        self.valid &= other.valid;
        if other.count == 0 {
            return;
        }
        if self.buckets.is_empty() {
            self.buckets.resize(BUCKETS, 0);
        }
        let count = self.count.checked_add(other.count);
        let sum = self.sum_ns.checked_add(other.sum_ns);
        if count.is_none() || sum.is_none() {
            self.valid = false;
            return;
        }
        self.count = count.unwrap();
        self.sum_ns = sum.unwrap();
        self.min_ns = match (self.min_ns, other.min_ns) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        self.max_ns = match (self.max_ns, other.max_ns) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        for (target, source) in self.buckets.iter_mut().zip(&other.buckets) {
            match target.checked_add(*source) {
                Some(sum) => *target = sum,
                None => self.valid = false,
            }
        }
    }

    pub fn quantile(&self, percent: u64) -> Option<Bounds> {
        if !self.valid || self.count == 0 || !(1..=100).contains(&percent) {
            return None;
        }
        let rank = (u128::from(self.count) * u128::from(percent)).div_ceil(100);
        let mut cumulative = 0u128;
        self.buckets
            .iter()
            .position(|n| {
                cumulative += u128::from(*n);
                cumulative >= rank
            })
            .map(bounds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bucket_boundary_round_trips_including_u64_max() {
        for index in 0..BUCKETS {
            let b = bounds(index);
            assert_eq!(bucket(b.lower_ns), index);
            assert_eq!(bucket(b.upper_ns), index);
            if index > 0 {
                assert_eq!(bounds(index - 1).upper_ns.checked_add(1), Some(b.lower_ns));
            }
        }
        assert_eq!(bounds(BUCKETS - 1).upper_ns, u64::MAX);
    }

    #[test]
    fn quantiles_cover_independently_sorted_samples_and_merge() {
        let mut left = Histogram::default();
        let mut right = Histogram::default();
        let mut values: Vec<_> = (0..1001).map(|n| mix(n) % 30_000_000_000).collect();
        for (i, value) in values.iter().enumerate() {
            if i % 2 == 0 {
                left.add(*value);
            } else {
                right.add(*value);
            }
        }
        left.merge(&right);
        values.sort_unstable();
        assert_eq!(left.count, values.len() as u64);
        assert_eq!(left.sum_ns, values.iter().sum::<u64>());
        for percent in [1, 50, 95, 99, 100] {
            let actual = values[(values.len() as u64 * percent).div_ceil(100) as usize - 1];
            let interval = left.quantile(percent).unwrap();
            assert!((interval.lower_ns..=interval.upper_ns).contains(&actual));
        }
    }

    #[test]
    fn overflow_cannot_publish_a_quantile() {
        let mut histogram = Histogram::default();
        histogram.add(u64::MAX);
        histogram.add(1);
        assert!(!histogram.valid);
        assert_eq!(histogram.quantile(99), None);
    }
}
