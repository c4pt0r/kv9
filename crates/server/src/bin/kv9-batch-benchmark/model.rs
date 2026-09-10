use kv9_server::client::{ClientConfig, RawOperation, TransportKind, MAX_MESSAGE_BYTES};
use kv9_server::proto;
use prost::Message;
use serde::{Deserialize, Serialize};

pub const MAX_CALLS: u64 = 10_000_000;
pub const MAX_PENDING_INPUT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Load {
    ClosedLoop,
    FixedRate { batches_per_second: u64 },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub client: ClientConfig,
    #[serde(default)]
    pub rpc_transport: TransportKind,
    pub run_id: String,
    pub seed: u64,
    pub workers: usize,
    pub keys: usize,
    pub batch_size: usize,
    pub value_bytes: usize,
    pub read_percent: u8,
    pub warmup_calls: u64,
    pub measure_ms: u64,
    pub max_calls: u64,
    pub load: Load,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct WireSizes {
    pub get_request_bytes: usize,
    pub put_request_bytes: usize,
    pub get_response_bytes: usize,
    pub maximum_input_items_in_flight: usize,
    pub maximum_input_payload_bytes_in_flight: usize,
}

pub fn mix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e3779b97f4a7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}

impl Config {
    pub fn validate(&self) -> Result<WireSizes, &'static str> {
        self.client.validate()?;
        if self.version != 1
            || self.run_id.is_empty()
            || self.run_id.len() > 64
            || !self
                .run_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            || !(1..=self.client.max_in_flight).contains(&self.workers)
            || !(1..=65_536).contains(&self.keys)
            || !(1..=256).contains(&self.batch_size)
            || self.batch_size > self.keys
            || !(16..=8192).contains(&self.value_bytes)
            || self.read_percent > 100
            || self.warmup_calls > 100_000
            || !(1..=60_000).contains(&self.measure_ms)
            || !(1..=MAX_CALLS).contains(&self.max_calls)
            || self.client.keyspace_id == 0
            || self.client.keyspace_id >= 1 << 24
            || self.client.epoch_conf_ver != 1
            || self.client.epoch_version != 1
        {
            return Err("invalid bounded native batch benchmark configuration");
        }
        if let Load::FixedRate { batches_per_second } = self.load {
            if !(1..=2_000_000).contains(&batches_per_second)
                || self
                    .offered_slots()
                    .is_none_or(|count| count > self.max_calls)
            {
                return Err("fixed offered load exceeds the declared call budget");
            }
        }
        let context = Some(proto::RequestContext {
            keyspace_id: self.client.keyspace_id,
            region_epoch: Some(proto::RegionEpoch {
                conf_ver: self.client.epoch_conf_ver,
                version: self.client.epoch_version,
            }),
        });
        let keys: Vec<_> = (0..self.batch_size).map(|i| self.key(i)).collect();
        let sizes = WireSizes {
            get_request_bytes: proto::RawBatchGetRequest {
                context,
                keys: keys.clone(),
            }
            .encoded_len(),
            put_request_bytes: proto::RawBatchPutRequest {
                context,
                pairs: keys
                    .into_iter()
                    .enumerate()
                    .map(|(i, key)| proto::KeyValue {
                        key,
                        value: self.value(i, 0),
                    })
                    .collect(),
            }
            .encoded_len(),
            get_response_bytes: proto::RawBatchGetResponse {
                values: (0..self.batch_size)
                    .map(|i| proto::OptionalValue {
                        found: true,
                        value: self.value(i, 0),
                    })
                    .collect(),
            }
            .encoded_len(),
            maximum_input_items_in_flight: self.workers * self.batch_size,
            maximum_input_payload_bytes_in_flight: self.workers
                * self.batch_size
                * (self.run_id.len() + 17 + self.value_bytes),
        };
        if [
            sizes.get_request_bytes,
            sizes.put_request_bytes,
            sizes.get_response_bytes,
        ]
        .into_iter()
        .any(|bytes| bytes > MAX_MESSAGE_BYTES)
            || sizes.maximum_input_payload_bytes_in_flight > MAX_PENDING_INPUT_BYTES
        {
            return Err("batch benchmark exceeds wire or pending input bounds");
        }
        Ok(sizes)
    }

    pub fn key(&self, index: usize) -> Vec<u8> {
        format!("{}:{index:016x}", self.run_id).into_bytes()
    }

    pub fn value(&self, key_index: usize, nonce: u64) -> Vec<u8> {
        let mut result = vec![0; self.value_bytes];
        result[..8].copy_from_slice(&(key_index as u64).to_be_bytes());
        result[8..16].copy_from_slice(&nonce.to_be_bytes());
        for (i, chunk) in result[16..].chunks_mut(8).enumerate() {
            let word = mix(self.seed ^ mix(key_index as u64) ^ mix(nonce) ^ i as u64).to_be_bytes();
            chunk.copy_from_slice(&word[..chunk.len()]);
        }
        result
    }

    pub fn first_key(&self, nonce: u64) -> usize {
        (mix(self.seed ^ nonce ^ 0xc6275c213842315b) % self.keys as u64) as usize
    }

    pub fn is_read(&self, nonce: u64) -> bool {
        mix(self.seed ^ nonce) % 100 < u64::from(self.read_percent)
    }

    pub fn operation(&self, nonce: u64) -> RawOperation {
        let first = self.first_key(nonce);
        if self.is_read(nonce) {
            RawOperation::BatchGet {
                keys: (0..self.batch_size)
                    .map(|i| self.key((first + i) % self.keys))
                    .collect(),
            }
        } else {
            RawOperation::BatchPut {
                pairs: (0..self.batch_size)
                    .map(|i| {
                        let index = (first + i) % self.keys;
                        (self.key(index), self.value(index, nonce))
                    })
                    .collect(),
            }
        }
    }

    pub fn valid_value(&self, index: usize, value: &[u8], maximum_nonce: u64) -> bool {
        if value.len() != self.value_bytes || value.len() < 16 {
            return false;
        }
        let nonce = u64::from_be_bytes(value[8..16].try_into().unwrap());
        nonce <= maximum_nonce && value == self.value(index, nonce)
    }

    /// Number of fixed-rate slots whose due time is strictly before the cutoff.
    pub fn offered_slots(&self) -> Option<u64> {
        match self.load {
            Load::ClosedLoop => None,
            Load::FixedRate { batches_per_second } => {
                Some((self.measure_ms * batches_per_second).div_ceil(1000))
            }
        }
    }

    pub fn slot_due_ns(&self, slot: u64) -> Option<u64> {
        match self.load {
            Load::ClosedLoop => None,
            Load::FixedRate { batches_per_second } => {
                Some((u128::from(slot) * 1_000_000_000 / u128::from(batches_per_second)) as u64)
            }
        }
    }
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
