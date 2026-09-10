use crate::client::RawOperation;

use super::WorkloadConfig;

/// A deterministic integer mixer, not a cryptographic random source. The exact
/// wrapping arithmetic is part of configuration version 1's workload sequence.
fn mix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e3779b97f4a7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}

pub struct Generator {
    config: WorkloadConfig,
}

impl Generator {
    pub fn new(config: WorkloadConfig) -> Result<Self, &'static str> {
        config.validate()?;
        Ok(Self { config })
    }

    pub fn key(&self, index: usize) -> Result<Vec<u8>, &'static str> {
        if index > self.config.keys {
            return Err("generated key index exceeds the dataset");
        }
        Ok(format!("{}:{index:016x}", self.config.run_id).into_bytes())
    }

    /// The index immediately after the mutable dataset is a reserved sentinel.
    pub fn sentinel(&self) -> Vec<u8> {
        self.key(self.config.keys)
            .expect("validated sentinel index")
    }

    pub fn value(&self, nonce: u64) -> Vec<u8> {
        let mut value = vec![0; self.config.value_bytes];
        value[..8].copy_from_slice(&nonce.to_be_bytes());
        for (i, chunk) in value[8..].chunks_mut(8).enumerate() {
            let bytes = mix(self.config.seed ^ nonce ^ i as u64).to_be_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
        value
    }

    /// Nonces are assigned once per logical workload invocation. Replays of an
    /// ambiguous write would reuse its immutable value and remain observable.
    /// The runner reserves disjoint nonce ranges for initialization and traffic.
    pub fn operation(&self, nonce: u64) -> RawOperation {
        let random = mix(self.config.seed ^ nonce);
        let key = self
            .key((mix(random) % self.config.keys as u64) as usize)
            .expect("bounded generated index");
        let choice = random % 100;
        if choice < u64::from(self.config.mix.get) {
            RawOperation::Get { key }
        } else if choice < u64::from(self.config.mix.get) + u64::from(self.config.mix.put) {
            RawOperation::Put {
                key,
                value: self.value(nonce),
            }
        } else {
            RawOperation::Delete { key }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_workload_excludes_the_sentinel_and_retains_write_identity() {
        let config = super::super::config::example();
        let generator = Generator::new(config.clone()).unwrap();
        let repeated = Generator::new(config).unwrap();
        let mut kinds = [0; 3];
        for nonce in 0..1000 {
            let operation = generator.operation(nonce);
            assert_eq!(
                format!("{operation:?}"),
                format!("{:?}", repeated.operation(nonce))
            );
            kinds[operation.kind() as usize] += 1;
            let key = match operation {
                RawOperation::Get { key } | RawOperation::Delete { key } => key,
                RawOperation::BatchGet { .. } | RawOperation::BatchPut { .. } => {
                    panic!("point generator emitted a batch")
                }
                RawOperation::Put { key, value } => {
                    assert_eq!(&value[..8], &nonce.to_be_bytes());
                    assert_eq!(value.len(), 128);
                    key
                }
            };
            assert_ne!(key, generator.sentinel());
        }
        assert!(kinds.into_iter().all(|count| count > 0));
        assert_ne!(generator.value(1), generator.value(2));
        assert!(generator.key(9).is_err());
    }
}
