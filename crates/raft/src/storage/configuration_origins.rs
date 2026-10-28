//! Atomic configuration/index/origin persistence. Historical facts survive a
//! protocol snapshot locally; the snapshot's imported ancestry stays unknown.
use super::*;
use crate::configuration_origin::{ConfigurationOrigin, ORIGIN_BYTES};
use kv9_common::AppliedPosition;
use raft::prelude::{ConfChangeV2, EntryType};
use raft::Storage as _;
use std::collections::BTreeMap;
mod page;
pub use page::{OriginCursor, OriginPage, ORIGINS_PER_PAGE};

/// Forward-only log record. Pre-origin readers reject this unknown kind on
/// reopen; older live peers can still apply the ordinary ConfChangeV2 but keep
/// their origin unknown. This is not a cluster-wide capability certificate.
pub(super) const RECORD: u8 = 9;
pub(super) struct Origins {
    entries: BTreeMap<u64, (u64, AppliedConfigurationOrigin)>,
    chain: kv9_common::RootDigest,
    legacy_unknown: bool,
}
impl Origins {
    pub(super) fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            chain: kv9_common::RootDigest::sha256(b"KV9 origin chain 1"),
            legacy_unknown: false,
        }
    }
    pub(super) fn get(&self, index: &u64) -> Option<&AppliedConfigurationOrigin> {
        self.entries.get(index).map(|(_, record)| record)
    }
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }
    pub(super) fn observe_legacy(&mut self) {
        self.legacy_unknown = true;
    }
    pub(super) fn legacy_unknown(&self) -> bool {
        self.legacy_unknown
    }
    pub(super) fn validate_insert(&self, record: AppliedConfigurationOrigin) -> Result<()> {
        record.observation_bytes()?;
        if let Some(old) = self.get(&record.at.index) {
            return if *old == record {
                Ok(())
            } else {
                Err(invalid())
            };
        }
        if self
            .entries
            .last_key_value()
            .is_some_and(|(i, _)| *i >= record.at.index)
        {
            return Err(invalid());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppliedConfigurationOrigin {
    pub at: AppliedPosition,
    pub origin: ConfigurationOrigin,
}
impl AppliedConfigurationOrigin {
    fn observation_bytes(self) -> Result<Vec<u8>> {
        if self.at.term == 0 || self.at.index == 0 || self.at.index == u64::MAX {
            return Err(invalid());
        }
        let mut b = self.at.term.to_be_bytes().to_vec();
        b.extend_from_slice(&self.at.index.to_be_bytes());
        b.extend_from_slice(&self.origin.encode()?);
        Ok(b)
    }
    pub(super) fn encode(self, state: &ConfState) -> Result<Vec<u8>> {
        let mut b = self.observation_bytes()?;
        b.extend_from_slice(&state.write_to_bytes().map_err(|_| invalid())?);
        Ok(b)
    }
    pub(super) fn decode(bytes: &[u8]) -> Result<(Self, ConfState)> {
        if bytes.len() < 16 + ORIGIN_BYTES {
            return Err(invalid());
        }
        let record = Self {
            at: AppliedPosition {
                term: u64::from_be_bytes(bytes[..8].try_into().unwrap()),
                index: u64::from_be_bytes(bytes[8..16].try_into().unwrap()),
            },
            origin: ConfigurationOrigin::decode(&bytes[16..16 + ORIGIN_BYTES])?,
        };
        let state =
            ConfState::parse_from_bytes(&bytes[16 + ORIGIN_BYTES..]).map_err(|_| invalid())?;
        if record.encode(&state)? != bytes {
            return Err(invalid());
        }
        Ok((record, state))
    }
}
pub(super) fn insert(origins: &mut Origins, record: AppliedConfigurationOrigin) -> Result<()> {
    origins.validate_insert(record)?;
    if origins.get(&record.at.index).is_some() {
        return Ok(());
    }
    let mut bytes = origins.chain.as_bytes().to_vec();
    bytes.extend_from_slice(&record.observation_bytes()?);
    origins.chain = kv9_common::RootDigest::sha256(&bytes);
    origins
        .entries
        .insert(record.at.index, (origins.entries.len() as u64 + 1, record));
    Ok(())
}
fn invalid() -> Error {
    Error::Raft("invalid durable DATA configuration origin".into())
}
impl<F: FileSystem> DiskRaftStorage<F> {
    /// Look up one exact historical applied position. None means UNKNOWN,
    /// including old records and imported snapshot prefixes; it is not proof
    /// that a store never participated. No lookup consults today's endpoint.
    pub fn configuration_origin_at(
        &self,
        at: AppliedPosition,
    ) -> Result<Option<AppliedConfigurationOrigin>> {
        let writer = self.file.lock().expect("raft log file poisoned");
        if writer.is_none() || at.term == 0 || at.index == 0 || at.index == u64::MAX {
            return Err(invalid());
        }
        let records = self
            .configuration_origins
            .lock()
            .expect("configuration origins poisoned");
        match records.get(&at.index) {
            Some(record) if record.at != at => Err(invalid()),
            record => Ok(record.copied()),
        }
    }
    /// Caller holds the actual storage writer lock. Only an exact committed
    /// configuration entry can contribute an origin; a proposal alone cannot.
    pub(super) fn origin_from_entry(
        &self,
        index: u64,
    ) -> Result<Option<AppliedConfigurationOrigin>> {
        if index == u64::MAX {
            return Err(invalid());
        }
        if index < self.mem.first_index().map_err(|_| invalid())?
            || index > self.mem.last_index().map_err(|_| invalid())?
        {
            return Ok(None);
        }
        // Legacy test/storage callers may have only a ConfState record. Keep
        // it unknown rather than inventing an origin or requiring new input.
        let Ok(entries) = self
            .mem
            .entries(index, index + 1, None, GetEntriesContext::empty(false))
        else {
            return Ok(None);
        };
        let Some(entry) = entries.first() else {
            return Ok(None);
        };
        if entry.entry_type != EntryType::EntryConfChangeV2 {
            return Ok(None);
        }
        let Ok(change) = ConfChangeV2::parse_from_bytes(&entry.data) else {
            return Ok(None);
        };
        let Some(origin) = ConfigurationOrigin::from_change(&change)? else {
            return Ok(None);
        };
        if self
            .mem
            .initial_state()
            .map_err(|_| invalid())?
            .hard_state
            .commit
            < index
        {
            return Err(invalid());
        }
        Ok(Some(AppliedConfigurationOrigin {
            at: AppliedPosition {
                term: entry.term,
                index,
            },
            origin,
        }))
    }
}

#[cfg(test)]
mod tests;
