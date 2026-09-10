use kv9_server::client::{ClientConfig, RawOperation, TransportKind, MAX_MESSAGE_BYTES};
use kv9_server::proto;
use prost::Message;
use serde::{Deserialize, Serialize};

pub const MAX_CALLS: u64 = 10_000_000;
pub const MAX_PENDING_INPUT_BYTES: usize = 64 * 1024 * 1024;

pub use super::common::{Histogram, Load};

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
        super::common::key(&self.run_id, index)
    }

    pub fn value(&self, key_index: usize, nonce: u64) -> Vec<u8> {
        super::common::value(self.seed, self.value_bytes, key_index, nonce)
    }

    pub fn first_key(&self, nonce: u64) -> usize {
        super::common::first_key(self.seed, nonce, self.keys)
    }

    pub fn is_read(&self, nonce: u64) -> bool {
        super::common::is_read(self.seed, nonce, self.read_percent)
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
        super::common::offered_slots(self.load, self.measure_ms)
    }

    pub fn slot_due_ns(&self, slot: u64) -> Option<u64> {
        super::common::slot_due_ns(self.load, slot)
    }
}
