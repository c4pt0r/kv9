//! Typed bounded TTL payloads. Only ordered apply may turn these into KV effects.
use super::{put_bytes, Reader, RegionFence};
use kv9_common::data_range::DataRange;
use kv9_common::raw_ttl::{
    ClockWatermark, ExpiryPlan, ExpiryRecord, CLOCK_RECORD_BYTES, EXPIRY_PLAN_BYTES,
    EXPIRY_RECORD_BYTES,
};
use kv9_common::{Error, KeyspaceId, Result, RootDigest};

pub const MAX_COMMAND_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_MUTATIONS: usize = 4096;
pub const MAX_KEY_BYTES: usize = 65536;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawTtlMutation {
    Put { key: Vec<u8>, value: Vec<u8> },
    Delete { key: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawTtlAction {
    /// One-way format activation; duplicate activation cannot lower time.
    Enable {
        observed: ClockWatermark,
    },
    Observe {
        observed: ClockWatermark,
    },
    Write {
        plan: ExpiryPlan,
        mutations: Vec<RawTtlMutation>,
    },
    /// Scanning occurs outside apply; these exact observations are rechecked.
    Cleanup {
        candidates: Vec<(Vec<u8>, ExpiryRecord)>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawTtlCommand {
    pub(crate) root: RootDigest,
    pub(crate) creation: RootDigest,
    pub(crate) fence: RegionFence,
    pub(crate) keyspace: KeyspaceId,
    pub(crate) action: RawTtlAction,
}

impl RawTtlCommand {
    pub fn new(range: &DataRange, action: RawTtlAction) -> Result<Self> {
        range.validate()?;
        let command = Self {
            root: range.root,
            creation: range.creation,
            fence: RegionFence {
                region_id: range.region.0,
                conf_ver: range.conf_ver,
                version: range.version,
            },
            keyspace: range.keyspace,
            action,
        };
        command.validate()?;
        if range.sealed || !command.keys().all(|key| range.contains(key)) {
            return Err(Error::StaleEpoch {
                region: range.region,
            });
        }
        Ok(command)
    }

    pub fn action(&self) -> &RawTtlAction {
        &self.action
    }
    pub fn fence(&self) -> RegionFence {
        self.fence
    }

    pub(crate) fn keys(&self) -> Box<dyn Iterator<Item = &[u8]> + '_> {
        match &self.action {
            RawTtlAction::Write { mutations, .. } => {
                Box::new(mutations.iter().map(|mutation| match mutation {
                    RawTtlMutation::Put { key, .. } | RawTtlMutation::Delete { key } => {
                        key.as_slice()
                    }
                }))
            }
            RawTtlAction::Cleanup { candidates } => {
                Box::new(candidates.iter().map(|(key, _)| key.as_slice()))
            }
            _ => Box::new(std::iter::empty()),
        }
    }

    fn validate(&self) -> Result<()> {
        let bad = || Error::Raft("invalid or oversized Raw TTL command".into());
        if self.root.as_bytes() == &[0; 32]
            || self.creation.as_bytes() == &[0; 32]
            || self.fence.region_id < 100
            || self.fence.conf_ver == 0
            || self.fence.version == 0
            || self.keyspace == KeyspaceId::SYSTEM
            || self.keyspace.0 > KeyspaceId::MAX
            || self.keys().any(|key| key.len() > MAX_KEY_BYTES)
        {
            return Err(bad());
        }
        // Includes version/tag, fixed scope and action tag. All arithmetic is
        // checked before encoder casts; a caller cannot bypass decoder budgets.
        let mut bytes = 2usize + 32 + 32 + 24 + 4 + 1;
        let mut charge = |count: usize| -> Result<()> {
            bytes = bytes
                .checked_add(count)
                .filter(|n| *n <= MAX_COMMAND_BYTES)
                .ok_or_else(bad)?;
            Ok(())
        };
        match &self.action {
            RawTtlAction::Enable { .. } | RawTtlAction::Observe { .. } => {
                charge(CLOCK_RECORD_BYTES)?
            }
            RawTtlAction::Write { plan, mutations } => {
                if mutations.is_empty() || mutations.len() > MAX_MUTATIONS {
                    return Err(bad());
                }
                ExpiryPlan::decode(&plan.encode())?;
                charge(EXPIRY_PLAN_BYTES + 4)?;
                for mutation in mutations {
                    match mutation {
                        RawTtlMutation::Put { key, value } => {
                            charge(9)?;
                            charge(key.len())?;
                            charge(value.len())?;
                        }
                        RawTtlMutation::Delete { key } => {
                            charge(5)?;
                            charge(key.len())?;
                        }
                    }
                }
            }
            RawTtlAction::Cleanup { candidates } => {
                if candidates.is_empty() || candidates.len() > MAX_MUTATIONS {
                    return Err(bad());
                }
                charge(4)?;
                for (key, _) in candidates {
                    charge(4 + EXPIRY_RECORD_BYTES)?;
                    charge(key.len())?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn encode_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.root.as_bytes());
        out.extend_from_slice(self.creation.as_bytes());
        for n in [
            self.fence.region_id,
            self.fence.conf_ver,
            self.fence.version,
        ] {
            out.extend_from_slice(&n.to_be_bytes());
        }
        out.extend_from_slice(&self.keyspace.0.to_be_bytes());
        match &self.action {
            RawTtlAction::Enable { observed } | RawTtlAction::Observe { observed } => {
                out.push(if matches!(self.action, RawTtlAction::Enable { .. }) {
                    1
                } else {
                    2
                });
                out.extend_from_slice(&observed.encode());
            }
            RawTtlAction::Write { plan, mutations } => {
                out.push(3);
                out.extend_from_slice(&plan.encode());
                out.extend_from_slice(&(mutations.len() as u32).to_be_bytes());
                for mutation in mutations {
                    match mutation {
                        RawTtlMutation::Put { key, value } => {
                            out.push(1);
                            put_bytes(out, key);
                            put_bytes(out, value);
                        }
                        RawTtlMutation::Delete { key } => {
                            out.push(2);
                            put_bytes(out, key);
                        }
                    }
                }
            }
            RawTtlAction::Cleanup { candidates } => {
                out.push(4);
                out.extend_from_slice(&(candidates.len() as u32).to_be_bytes());
                for (key, record) in candidates {
                    put_bytes(out, key);
                    out.extend_from_slice(&record.encode());
                }
            }
        }
    }

    pub(super) fn decode_from(reader: &mut Reader<'_>) -> Result<Self> {
        let root = RootDigest::from_bytes(reader.take(32)?.try_into().unwrap());
        let creation = RootDigest::from_bytes(reader.take(32)?.try_into().unwrap());
        let fence = RegionFence {
            region_id: reader.u64()?,
            conf_ver: reader.u64()?,
            version: reader.u64()?,
        };
        let keyspace = KeyspaceId(reader.u32()?);
        let action = match reader.u8()? {
            tag @ (1 | 2) => {
                let observed = ClockWatermark::decode(reader.take(CLOCK_RECORD_BYTES)?)?;
                if tag == 1 {
                    RawTtlAction::Enable { observed }
                } else {
                    RawTtlAction::Observe { observed }
                }
            }
            3 => {
                let plan = ExpiryPlan::decode(reader.take(EXPIRY_PLAN_BYTES)?)?;
                let count = bounded_count(reader)?;
                let mut mutations = Vec::new();
                for _ in 0..count {
                    mutations.push(match reader.u8()? {
                        1 => RawTtlMutation::Put {
                            key: reader.bytes()?,
                            value: reader.bytes()?,
                        },
                        2 => RawTtlMutation::Delete {
                            key: reader.bytes()?,
                        },
                        _ => return Err(Error::Raft("unknown Raw TTL mutation".into())),
                    });
                }
                RawTtlAction::Write { plan, mutations }
            }
            4 => {
                let count = bounded_count(reader)?;
                let mut candidates = Vec::new();
                for _ in 0..count {
                    candidates.push((
                        reader.bytes()?,
                        ExpiryRecord::decode(reader.take(EXPIRY_RECORD_BYTES)?)?,
                    ));
                }
                RawTtlAction::Cleanup { candidates }
            }
            _ => return Err(Error::Raft("unknown Raw TTL action".into())),
        };
        let command = Self {
            root,
            creation,
            fence,
            keyspace,
            action,
        };
        command.validate()?;
        Ok(command)
    }
}

fn bounded_count(reader: &mut Reader<'_>) -> Result<usize> {
    let count = reader.u32()? as usize;
    if count == 0 || count > MAX_MUTATIONS {
        return Err(Error::Raft("Raw TTL mutation count exceeds budget".into()));
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Command;
    use kv9_common::raw_ttl::ClockSample;
    use kv9_common::{AppliedPosition, RegionId, TenantId};

    fn range() -> DataRange {
        DataRange {
            root: RootDigest::from_bytes([1; 32]),
            creation: RootDigest::from_bytes([2; 32]),
            region: RegionId(100),
            keyspace: KeyspaceId(100),
            tenant: TenantId(1),
            conf_ver: 1,
            version: 1,
            start: Vec::new(),
            end: Vec::new(),
            sealed: false,
        }
    }
    fn plan() -> ExpiryPlan {
        ExpiryPlan::resolve(
            Some(5),
            ClockSample::with_uncertainty(1000, 5).unwrap(),
            ClockWatermark::default(),
        )
        .unwrap()
    }
    fn command(action: RawTtlAction) -> Command {
        Command::RawTtl(RawTtlCommand::new(&range(), action).unwrap())
    }
    #[test]
    fn ttl_codec_roundtrip_truncation_trailing_and_lowering_refusal() {
        let plan = plan();
        for action in [
            RawTtlAction::Enable {
                observed: plan.observed(),
            },
            RawTtlAction::Observe {
                observed: plan.observed(),
            },
            RawTtlAction::Write {
                plan,
                mutations: vec![
                    RawTtlMutation::Put {
                        key: vec![],
                        value: b"KV9TTL01\0opaque".to_vec(),
                    },
                    RawTtlMutation::Delete {
                        key: b"key".to_vec(),
                    },
                ],
            },
            RawTtlAction::Cleanup {
                candidates: vec![(
                    b"key".to_vec(),
                    plan.at_write(range().creation, AppliedPosition { term: 2, index: 3 })
                        .unwrap()
                        .unwrap(),
                )],
            },
        ] {
            let cmd = command(action);
            let encoded = cmd.encode();
            assert_eq!(Command::decode(&encoded).unwrap(), cmd);
            assert!(cmd.to_write_batch().is_err());
            for cut in 0..encoded.len() {
                assert!(Command::decode(&encoded[..cut]).is_err(), "cut {cut}");
            }
            let mut extra = encoded;
            extra.push(0);
            assert!(Command::decode(&extra).is_err());
        }
    }
    #[test]
    fn ttl_codec_rejects_malformed_scope_tags_counts_and_budgets() {
        let valid = command(RawTtlAction::Write {
            plan: plan(),
            mutations: vec![RawTtlMutation::Delete { key: b"k".to_vec() }],
        })
        .encode();
        let action_offset = 2 + 32 + 32 + 24 + 4;
        let count_offset = action_offset + 1 + EXPIRY_PLAN_BYTES;
        for (offset, bytes) in [
            (2, vec![0; 32]),
            (34, vec![0; 32]),
            (66, 99u64.to_be_bytes().to_vec()),
            (74, vec![0; 8]),
            (82, vec![0; 8]),
            (90, vec![0; 4]),
            (action_offset, vec![99]),
            (count_offset, 0u32.to_be_bytes().to_vec()),
            (count_offset, u32::MAX.to_be_bytes().to_vec()),
            (count_offset + 4, vec![99]),
        ] {
            let mut corrupt = valid.clone();
            corrupt[offset..offset + bytes.len()].copy_from_slice(&bytes);
            assert!(Command::decode(&corrupt).is_err(), "offset {offset}");
        }
        let mut huge = valid;
        huge.resize(MAX_COMMAND_BYTES + 1, 0);
        assert!(Command::decode(&huge).is_err());
        for mutations in [
            vec![],
            vec![RawTtlMutation::Delete { key: vec![] }; MAX_MUTATIONS + 1],
            vec![RawTtlMutation::Delete {
                key: vec![0; MAX_KEY_BYTES + 1],
            }],
            vec![RawTtlMutation::Put {
                key: vec![],
                value: vec![0; MAX_COMMAND_BYTES],
            }],
        ] {
            assert!(RawTtlCommand::new(
                &range(),
                RawTtlAction::Write {
                    plan: plan(),
                    mutations
                }
            )
            .is_err());
        }
        assert!(
            RawTtlCommand::new(&range(), RawTtlAction::Cleanup { candidates: vec![] }).is_err()
        );
        // Exact byte cap is legal; one more byte must fail before encoding.
        let overhead = count_offset + 4 + 9;
        for extra in [0, 1] {
            let result = RawTtlCommand::new(
                &range(),
                RawTtlAction::Write {
                    plan: plan(),
                    mutations: vec![RawTtlMutation::Put {
                        key: vec![],
                        value: vec![0; MAX_COMMAND_BYTES - overhead + extra],
                    }],
                },
            );
            assert_eq!(result.is_ok(), extra == 0);
            if let Ok(cmd) = result {
                assert_eq!(Command::RawTtl(cmd).encode().len(), MAX_COMMAND_BYTES);
            }
        }
    }
    #[test]
    fn ttl_constructor_checks_the_entire_range_batch() {
        let mut scope = range();
        scope.start = b"b".to_vec();
        scope.end = b"m".to_vec();
        for key in [b"a", b"m"] {
            assert!(RawTtlCommand::new(
                &scope,
                RawTtlAction::Write {
                    plan: plan(),
                    mutations: vec![
                        RawTtlMutation::Delete { key: b"c".to_vec() },
                        RawTtlMutation::Delete { key: key.to_vec() },
                    ]
                }
            )
            .is_err());
        }
        scope.sealed = true;
        assert!(RawTtlCommand::new(
            &scope,
            RawTtlAction::Enable {
                observed: plan().observed()
            }
        )
        .is_err());
    }
}
