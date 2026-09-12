//! Durable opt-in and non-reused process epochs for the experimental lease.
//!
//! Always decoded, including builds without the experimental feature: restarting
//! a lease voter through an ordinary constructor must not erase its voting hold.
//! This format currently permits no policy or membership migration.

use kv9_common::{Error, Result};

/// Stable, per-peer lease declaration. Voters must be in canonical sorted order.
/// Matching declarations on all voters and a qualified clock are deployment
/// obligations; these numbers alone do not qualify a clock platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeasePolicy {
    pub node: u64,
    pub group: u64,
    pub configuration: u128,
    pub voters: Vec<u64>,
    pub promise_ns: u64,
    pub drift_ppb: u32,
    pub margin_ns: u64,
}

impl LeasePolicy {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.node == 0
            || self.configuration == 0
            || self.voters.is_empty()
            || self.voters.len() > 64
            || self.voters[0] == 0
            || self.voters.windows(2).any(|p| p[0] >= p[1])
            || self.voters.binary_search(&self.node).is_err()
            || self.promise_ns == 0
            || self.drift_ppb >= 1_000_000_000
        {
            return Err(Error::Raft("invalid durable lease policy".into()));
        }
        Ok(())
    }
}

/// A synchronized installation record, not a read or grant capability.
/// Incarnations are strictly increasing within the exclusively owned peer log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseEpoch {
    pub policy: LeasePolicy,
    pub incarnation: u64,
}

impl LeaseEpoch {
    pub(crate) fn successor(previous: Option<&Self>, policy: &LeasePolicy) -> Result<Self> {
        policy.validate()?;
        let incarnation = match previous {
            None => 1,
            Some(old) if &old.policy == policy => old
                .incarnation
                .checked_add(1)
                .ok_or_else(|| Error::Raft("lease incarnation exhausted".into()))?,
            Some(_) => return Err(Error::Raft("durable lease policy cannot change".into())),
        };
        Ok(Self {
            policy: policy.clone(),
            incarnation,
        })
    }

    pub(crate) fn encode(&self) -> Vec<u8> {
        let p = &self.policy;
        let mut bytes = Vec::with_capacity(62 + 8 * p.voters.len());
        bytes.push(1); // Exact version; older binaries reject the record kind.
        bytes.extend_from_slice(&self.incarnation.to_be_bytes());
        bytes.extend_from_slice(&p.node.to_be_bytes());
        bytes.extend_from_slice(&p.group.to_be_bytes());
        bytes.extend_from_slice(&p.configuration.to_be_bytes());
        bytes.extend_from_slice(&p.promise_ns.to_be_bytes());
        bytes.extend_from_slice(&p.drift_ppb.to_be_bytes());
        bytes.extend_from_slice(&p.margin_ns.to_be_bytes());
        bytes.push(p.voters.len() as u8);
        for voter in &p.voters {
            bytes.extend_from_slice(&voter.to_be_bytes());
        }
        bytes
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 62
            || bytes[0] != 1
            || bytes[61] > 64
            || bytes.len() != 62 + 8 * usize::from(bytes[61])
        {
            return Err(Error::Raft(
                "invalid lease epoch record shape/version".into(),
            ));
        }
        let u64_at =
            |start| u64::from_be_bytes(bytes[start..start + 8].try_into().expect("checked shape"));
        let epoch = Self {
            incarnation: u64_at(1),
            policy: LeasePolicy {
                node: u64_at(9),
                group: u64_at(17),
                configuration: u128::from_be_bytes(
                    bytes[25..41].try_into().expect("checked shape"),
                ),
                promise_ns: u64_at(41),
                drift_ppb: u32::from_be_bytes(bytes[49..53].try_into().expect("checked shape")),
                margin_ns: u64_at(53),
                voters: (62..bytes.len()).step_by(8).map(u64_at).collect(),
            },
        };
        epoch.policy.validate()?;
        if epoch.incarnation == 0 {
            return Err(Error::Raft("zero lease incarnation".into()));
        }
        Ok(epoch)
    }
}

#[cfg(test)]
mod tests;
