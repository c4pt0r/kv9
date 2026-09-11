use super::common::{self, Load};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

pub const MAX_WIRE: usize = 1_048_576;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReadApi {
    Mget,
    Get,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WriteApi {
    Mset,
    Set,
}

fn deserialize_read_api<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<ReadApi>, D::Error> {
    // Legacy absence is allowed; an explicitly supplied null is not an API.
    ReadApi::deserialize(deserializer).map(Some)
}

fn deserialize_write_api<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<WriteApi>, D::Error> {
    WriteApi::deserialize(deserializer).map(Some)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_read_api"
    )]
    pub read_api: Option<ReadApi>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_write_api"
    )]
    pub write_api: Option<WriteApi>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub get_request_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub get_response_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub set_request_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub set_response_bytes: Option<usize>,
    pub maximum_input_items_in_flight: usize,
    pub maximum_input_payload_bytes_in_flight: usize,
}
fn bulk_size(n: usize) -> usize {
    1 + n.to_string().len() + 2 + n + 2
}
impl Config {
    pub fn validate(&self) -> Result<WireSizes, &'static str> {
        if !matches!(
            (self.version, self.read_api, self.write_api),
            (1, None, None) | (2, Some(_), None) | (3, Some(_), Some(_))
        ) || ((self.effective_read_api() == ReadApi::Get
            || self.effective_write_api() == WriteApi::Set)
            && self.batch_size != 1)
            || (self.version == 2
                && self.effective_read_api() == ReadApi::Get
                && self.read_percent != 100)
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
            get_request_bytes: (self.effective_read_api() == ReadApi::Get)
                .then(|| array(2) + bulk_size(3) + bulk_size(self.run_id.len() + 17)),
            get_response_bytes: (self.effective_read_api() == ReadApi::Get)
                .then(|| bulk_size(self.value_bytes)),
            set_request_bytes: (self.effective_write_api() == WriteApi::Set).then(|| {
                array(3)
                    + bulk_size(3)
                    + bulk_size(self.run_id.len() + 17)
                    + bulk_size(self.value_bytes)
            }),
            set_response_bytes: (self.effective_write_api() == WriteApi::Set).then_some(5),
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
        .chain(sizes.get_request_bytes)
        .chain(sizes.get_response_bytes)
        .chain(sizes.set_request_bytes)
        .chain(sizes.set_response_bytes)
        .any(|n| n > MAX_WIRE)
            || sizes.maximum_input_payload_bytes_in_flight > 64 * 1024 * 1024
        {
            return Err("Redis batch exceeds encoded or pending input bounds");
        }
        Ok(sizes)
    }
    pub fn effective_read_api(&self) -> ReadApi {
        self.read_api.unwrap_or(ReadApi::Mget)
    }
    pub fn effective_write_api(&self) -> WriteApi {
        self.write_api.unwrap_or(WriteApi::Mset)
    }
    pub fn workload_model(&self) -> &'static str {
        if self.version == 3 {
            "bounded_redis_api_performance"
        } else if self.effective_read_api() == ReadApi::Get {
            "bounded_redis_point_get_diagnostic"
        } else {
            "bounded_redis_batch_performance"
        }
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::{json, Value};

    pub fn config_json() -> Value {
        json!({"version":1,"address":"127.0.0.1:6379","deadline_ms":1000,
            "run_id":"point","seed":71,"workers":4,"keys":64,"batch_size":1,
            "value_bytes":128,"read_percent":100,"warmup_calls":8,"measure_ms":1000,
            "max_calls":1000,"load":{"kind":"closed_loop"}})
    }

    #[test]
    fn legacy_config_and_wire_size_schema_are_unchanged() {
        let original = config_json();
        let c: Config = serde_json::from_value(original.clone()).unwrap();
        assert_eq!(serde_json::to_value(&c).unwrap(), original);
        assert_eq!(c.effective_read_api(), ReadApi::Mget);
        assert_eq!(c.effective_write_api(), WriteApi::Mset);
        assert_eq!(c.workload_model(), "bounded_redis_batch_performance");
        let sizes = serde_json::to_value(c.validate().unwrap()).unwrap();
        assert_eq!(sizes.as_object().unwrap().len(), 5);
        assert!(sizes.get("get_request_bytes").is_none());
        assert!(sizes.get("set_request_bytes").is_none());
        let mut explicit = original;
        explicit["version"] = json!(2);
        explicit["read_api"] = json!("mget");
        let c: Config = serde_json::from_value(explicit).unwrap();
        assert_eq!(serde_json::to_value(c.validate().unwrap()).unwrap(), sizes);
    }

    #[test]
    fn explicit_get_configuration_refuses_ambiguous_or_mixed_apis() {
        let mut valid = config_json();
        valid["version"] = json!(2);
        valid["read_api"] = json!("get");
        let c: Config = serde_json::from_value(valid.clone()).unwrap();
        assert_eq!(c.effective_read_api(), ReadApi::Get);
        assert!(c.validate().unwrap().get_request_bytes.is_some());
        for (field, value) in [
            ("version", json!(1)),
            ("version", json!(3)),
            ("version", json!(true)),
            ("read_api", Value::Null),
            ("read_api", json!("GET")),
            ("read_api", json!(false)),
            ("batch_size", json!(2)),
            ("batch_size", json!(true)),
            ("read_percent", json!(99)),
        ] {
            let mut bad = valid.clone();
            bad[field] = value;
            assert!(
                serde_json::from_value::<Config>(bad)
                    .map(|c| c.validate().is_err())
                    .unwrap_or(true),
                "accepted {field}"
            );
        }
        valid.as_object_mut().unwrap().remove("read_api");
        assert!(serde_json::from_value::<Config>(valid)
            .unwrap()
            .validate()
            .is_err());
    }

    #[test]
    fn v3_explicit_apis_allow_point_writes_and_mixed_traffic() {
        for read_api in ["mget", "get"] {
            for write_api in ["mset", "set"] {
                for read_percent in [0, 50, 100] {
                    let mut value = config_json();
                    value["version"] = json!(3);
                    value["read_api"] = json!(read_api);
                    value["write_api"] = json!(write_api);
                    value["read_percent"] = json!(read_percent);
                    let c: Config = serde_json::from_value(value.clone()).unwrap();
                    c.validate().unwrap();
                    assert_eq!(serde_json::to_value(&c).unwrap(), value);
                    assert_eq!(c.workload_model(), "bounded_redis_api_performance");
                }
            }
        }
    }

    #[test]
    fn v3_requires_both_selectors_and_point_cardinality() {
        let mut valid = config_json();
        valid["version"] = json!(3);
        valid["read_api"] = json!("get");
        valid["write_api"] = json!("set");
        valid["read_percent"] = json!(50);
        let rejected = |value| {
            serde_json::from_value::<Config>(value)
                .map(|c| c.validate().is_err())
                .unwrap_or(true)
        };
        for selector in ["read_api", "write_api"] {
            let mut missing = valid.clone();
            missing.as_object_mut().unwrap().remove(selector);
            assert!(rejected(missing), "missing {selector} accepted");
            for bad in [Value::Null, json!(false), json!("unknown"), json!([])] {
                let mut value = valid.clone();
                value[selector] = bad;
                assert!(rejected(value), "invalid {selector} accepted");
            }
        }
        for version in [1, 2, 4] {
            let mut value = valid.clone();
            value["version"] = json!(version);
            assert!(
                rejected(value),
                "write selector accepted by version {version}"
            );
        }
        for (read_api, write_api) in [("get", "mset"), ("mget", "set"), ("get", "set")] {
            let mut value = valid.clone();
            value["read_api"] = json!(read_api);
            value["write_api"] = json!(write_api);
            value["batch_size"] = json!(2);
            assert!(rejected(value), "point selector accepted multiple keys");
        }
        valid["read_api"] = json!("mget");
        valid["write_api"] = json!("mset");
        valid["batch_size"] = json!(2);
        serde_json::from_value::<Config>(valid)
            .unwrap()
            .validate()
            .unwrap();
    }
}
