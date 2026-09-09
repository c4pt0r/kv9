use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::client::ClientConfig;

pub const MAX_CONFIG_BYTES: u64 = 65_536;
pub const MAX_HISTORY_BYTES: u64 = 67_108_864;
pub const MAX_DATASET_BYTES: u64 = 8_388_608;
pub const HEADER_ALLOWANCE: u64 = 65_536;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Correctness,
    Performance,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mix {
    pub get: u8,
    pub put: u8,
    pub delete: u8,
}

/// Tokens and output paths are separate from this reproducible configuration.
/// All operation counts include setup, warmup, measurement and final reads.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkloadConfig {
    pub version: u32,
    pub client: ClientConfig,
    pub mode: Mode,
    pub run_id: String,
    pub keyspace_name: String,
    pub seed: u64,
    pub workers: usize,
    pub keys: usize,
    pub value_bytes: usize,
    pub mix: Mix,
    pub warmup_operations: u64,
    pub max_operations: u64,
    pub measure_ms: u64,
    pub interval_ms: u64,
    pub history_bytes: u64,
}

impl WorkloadConfig {
    pub fn read(path: &Path) -> Result<Self, String> {
        let mut bytes = Vec::new();
        File::open(path)
            .map_err(|_| "cannot open workload configuration")?
            .take(MAX_CONFIG_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "cannot read workload configuration")?;
        if bytes.len() as u64 > MAX_CONFIG_BYTES {
            return Err("workload configuration exceeds 64 KiB".into());
        }
        let config: Self =
            serde_json::from_slice(&bytes).map_err(|_| "invalid workload configuration JSON")?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        self.client.validate()?;
        if self.version != 1 {
            return Err("unsupported workload configuration version");
        }
        let name = |text: &str| {
            !text.is_empty()
                && text.len() <= 64
                && text
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        };
        if !name(&self.run_id) || !name(&self.keyspace_name) {
            return Err("run and keyspace names must be 1..=64 ASCII letters, digits, underscores or hyphens");
        }
        if self.client.keyspace_id == 0
            || self.client.keyspace_id >= 1 << 24
            || self.client.epoch_conf_ver != 1
            || self.client.epoch_version != 1
        {
            return Err(
                "workload requires a fresh raw keyspace with a 24-bit ID and initial epoch 1/1",
            );
        }
        if !(1..=self.client.max_in_flight).contains(&self.workers)
            || !(1..=4096).contains(&self.keys)
            || !(16..=8192).contains(&self.value_bytes)
        {
            return Err("worker, key count or value size is outside workload limits");
        }
        if u16::from(self.mix.get) + u16::from(self.mix.put) + u16::from(self.mix.delete) != 100 {
            return Err("operation percentages must sum to 100");
        }
        if !(1..=3_600_000).contains(&self.measure_ms)
            || self.interval_ms > 1000
            || self.warmup_operations > 100_000
        {
            return Err("workload time or warmup bound is invalid");
        }
        let limit = match self.mode {
            Mode::Correctness => 100_000,
            Mode::Performance => 1_000_000,
        };
        if self.max_operations > limit || self.max_operations <= self.non_measured_operations() {
            return Err("operation limit must reserve setup, warmup, verification and at least one measured operation");
        }
        // Includes an immutable sentinel and conservative per-row overhead;
        // all generated mutations stay inside this fixed dataset. This stays
        // below the current 48 MiB checkpoint boundary, not a storage-capacity fix.
        let dataset = (self.keys as u64 + 1) * (self.key_bytes() + self.value_bytes as u64 + 64);
        if dataset > MAX_DATASET_BYTES {
            return Err("workload dataset exceeds the 8 MiB budget");
        }
        match self.mode {
            Mode::Correctness => {
                if self.history_bytes > MAX_HISTORY_BYTES
                    || self.history_bytes < self.history_reservation()
                {
                    return Err("complete correctness history does not fit its declared budget");
                }
            }
            Mode::Performance if self.history_bytes != 0 => {
                return Err("performance-only mode must declare zero full-history storage");
            }
            Mode::Performance => {}
        }
        Ok(())
    }

    pub fn key_bytes(&self) -> u64 {
        // Prefix plus separator and a fixed sixteen-digit hexadecimal index.
        self.run_id.len() as u64 + 17
    }

    pub fn initialization_operations(&self) -> u64 {
        // Establish absence and then acknowledge one write per key + sentinel.
        2 * (self.keys as u64 + 1)
    }

    pub fn verification_operations(&self) -> u64 {
        self.keys as u64 + 1
    }

    pub fn non_measured_operations(&self) -> u64 {
        self.initialization_operations() + self.warmup_operations + self.verification_operations()
    }

    pub fn event_pair_allowance(&self) -> u64 {
        // Bounded static JSON fields, sixteen attempt records, hex key/value
        // bytes and a receipt. No response prose or credentials are serialized.
        // The recorder additionally enforces this bound on actual event bytes.
        8192 + 2 * self.key_bytes() + 2 * self.value_bytes as u64
    }

    pub fn history_reservation(&self) -> u64 {
        // validate bounds all factors before this is used to authorize a run.
        HEADER_ALLOWANCE + self.max_operations * self.event_pair_allowance()
    }
}

#[cfg(test)]
pub(crate) fn example() -> WorkloadConfig {
    WorkloadConfig {
        version: 1,
        client: ClientConfig {
            version: 1,
            peers: vec![crate::client::Peer {
                node_id: 1,
                address: "127.0.0.1:12345".parse().unwrap(),
            }],
            keyspace_id: 1,
            epoch_conf_ver: 1,
            epoch_version: 1,
            max_in_flight: 4,
            max_attempts: 6,
            deadline_ms: 2000,
            retry_backoff_ms: 10,
        },
        mode: Mode::Correctness,
        run_id: "workload-test".into(),
        keyspace_name: "fresh-test".into(),
        seed: 40,
        workers: 4,
        keys: 8,
        value_bytes: 128,
        mix: Mix {
            get: 50,
            put: 40,
            delete: 10,
        },
        warmup_operations: 16,
        max_operations: 1000,
        measure_ms: 1000,
        interval_ms: 0,
        history_bytes: MAX_HISTORY_BYTES,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_reserves_complete_history_and_all_phases() {
        let mut config = example();
        assert!(config.validate().is_ok());
        config.history_bytes = config.history_reservation() - 1;
        assert_eq!(
            config.validate(),
            Err("complete correctness history does not fit its declared budget")
        );
        config.history_bytes += 1;
        assert!(config.validate().is_ok());
        config.max_operations = config.non_measured_operations();
        assert!(config.validate().is_err());
    }

    #[test]
    fn configuration_rejects_unbounded_and_wrong_scope_inputs() {
        let mut config = example();
        config.keys = usize::MAX;
        assert!(config.validate().is_err());
        config = example();
        config.max_operations = u64::MAX;
        assert!(config.validate().is_err());
        config = example();
        config.value_bytes = 8192;
        config.keys = 4096;
        config.max_operations = 20_000;
        assert_eq!(
            config.validate(),
            Err("workload dataset exceeds the 8 MiB budget")
        );
        config = example();
        config.mode = Mode::Performance;
        assert!(config.validate().is_err());
        config.history_bytes = 0;
        assert!(config.validate().is_ok());
        config.client.epoch_version = 2;
        assert!(config.validate().is_err());
        let mut json = serde_json::to_value(example()).unwrap();
        json["unknown_workload_mode"] = true.into();
        assert!(serde_json::from_value::<WorkloadConfig>(json).is_err());
    }
}
