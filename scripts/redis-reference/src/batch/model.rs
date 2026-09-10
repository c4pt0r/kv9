use super::common::{self, Load};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

pub const MAX_WIRE: usize = 1_048_576;
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub address: SocketAddr,
    pub deadline_ms: u64,
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
#[derive(Serialize)]
pub struct WireSizes {
    pub mget_request_bytes: usize,
    pub mset_request_bytes: usize,
    pub mget_response_bytes: usize,
    pub maximum_input_items_in_flight: usize,
    pub maximum_input_payload_bytes_in_flight: usize,
}
fn bulk_size(n: usize) -> usize {
    1 + n.to_string().len() + 2 + n + 2
}
impl Config {
    pub fn validate(&self) -> Result<WireSizes, &'static str> {
        if self.version != 1
            || self.address.port() == 0
            || !(1..=30_000).contains(&self.deadline_ms)
            || self.run_id.is_empty()
            || self.run_id.len() > 64
            || !self
                .run_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            || !(1..=256).contains(&self.workers)
            || !(1..=65_536).contains(&self.keys)
            || !(1..=256).contains(&self.batch_size)
            || self.batch_size > self.keys
            || !(16..=8192).contains(&self.value_bytes)
            || self.read_percent > 100
            || self.warmup_calls > 100_000
            || !(1..=60_000).contains(&self.measure_ms)
            || !(1..=10_000_000).contains(&self.max_calls)
        {
            return Err("invalid bounded Redis batch reference configuration");
        }
        if let Load::FixedRate { batches_per_second } = self.load {
            if !(1..=2_000_000).contains(&batches_per_second)
                || self.offered_slots().is_none_or(|n| n > self.max_calls)
            {
                return Err("fixed offered load exceeds call budget");
            }
        }
        let n = self.batch_size;
        let array = |n: usize| 1 + n.to_string().len() + 2;
        let sizes = WireSizes {
            mget_request_bytes: array(1 + n) + bulk_size(4) + n * bulk_size(self.run_id.len() + 17),
            mset_request_bytes: array(1 + 2 * n)
                + bulk_size(4)
                + n * (bulk_size(self.run_id.len() + 17) + bulk_size(self.value_bytes)),
            mget_response_bytes: array(n) + n * bulk_size(self.value_bytes),
            maximum_input_items_in_flight: self.workers * n,
            maximum_input_payload_bytes_in_flight: self.workers
                * n
                * (self.run_id.len() + 17 + self.value_bytes),
        };
        if [
            sizes.mget_request_bytes,
            sizes.mset_request_bytes,
            sizes.mget_response_bytes,
        ]
        .into_iter()
        .any(|n| n > MAX_WIRE)
            || sizes.maximum_input_payload_bytes_in_flight > 64 * 1024 * 1024
        {
            return Err("Redis batch exceeds encoded or pending input bounds");
        }
        Ok(sizes)
    }
    pub fn key(&self, index: usize) -> Vec<u8> {
        common::key(&self.run_id, index)
    }
    pub fn value(&self, index: usize, nonce: u64) -> Vec<u8> {
        common::value(self.seed, self.value_bytes, index, nonce)
    }
    pub fn first_key(&self, nonce: u64) -> usize {
        common::first_key(self.seed, nonce, self.keys)
    }
    pub fn is_read(&self, nonce: u64) -> bool {
        common::is_read(self.seed, nonce, self.read_percent)
    }
    pub fn offered_slots(&self) -> Option<u64> {
        common::offered_slots(self.load, self.measure_ms)
    }
    pub fn slot_due_ns(&self, slot: u64) -> Option<u64> {
        common::slot_due_ns(self.load, slot)
    }
    pub fn valid_value(&self, index: usize, value: &[u8], maximum: u64) -> bool {
        if value.len() != self.value_bytes {
            return false;
        }
        let nonce = u64::from_be_bytes(value[8..16].try_into().unwrap());
        nonce <= maximum && value == self.value(index, nonce)
    }
}
