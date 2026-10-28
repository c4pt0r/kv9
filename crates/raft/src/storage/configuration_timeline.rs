//! Ordered local configuration observations, including legacy and snapshot
//! boundaries. These are inputs to interval reconciliation, never a proof of
//! complete imported ancestry, historical store coverage or deletion authority.
use super::configuration_origins::AppliedConfigurationOrigin;
use super::membership_history::MembershipGaps;
use super::*;
use kv9_common::{AppliedPosition, RootDigest};
use std::collections::BTreeMap;

mod state;
mod wire;
pub use state::ConfigurationState;
pub use wire::{TimelineCursor, TimelinePage, MAX_PAGE_BYTES, RECORDS_PER_PAGE};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordKind {
    Initial,
    /// Legacy records can lack a term or an independently checked source.
    /// Zero/ambiguous positions remain descriptive and carry history gaps.
    Applied {
        index: u64,
    },
    Snapshot {
        at: AppliedPosition,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationRecord {
    pub ordinal: u64,
    pub kind: RecordKind,
    /// None preserves an unsupported/oversized state, never an empty set.
    pub state: Option<ConfigurationState>,
    pub origin: Option<AppliedConfigurationOrigin>,
}
pub(super) struct ConfigurationTimeline {
    records: BTreeMap<u64, ConfigurationRecord>,
    chain: RootDigest,
}
impl Default for ConfigurationTimeline {
    fn default() -> Self {
        Self {
            records: BTreeMap::new(),
            chain: RootDigest::sha256(b"KV9 configuration timeline 1"),
        }
    }
}
impl ConfigurationTimeline {
    pub(super) fn observe(
        &mut self,
        kind: RecordKind,
        state: &ConfState,
        origin: Option<AppliedConfigurationOrigin>,
    ) {
        let record = ConfigurationRecord {
            ordinal: self.records.len() as u64 + 1,
            kind,
            state: ConfigurationState::observe(state),
            origin,
        };
        // Idempotent reapplication of the same indexed record need not create
        // another interval. Conflicting or out-of-order observations survive;
        // MembershipHistory separately retains their ambiguity flag.
        if let Some((_, last)) = self.records.last_key_value() {
            if matches!(kind, RecordKind::Applied { .. })
                && last.kind == kind
                && last.state == record.state
                && last.origin == origin
            {
                return;
            }
        }
        // Storage rejects malformed origin records before publication. Unknown
        // legacy configurations are represented by None without disrupting apply.
        let bytes = record
            .encode()
            .expect("bounded observed configuration record");
        let mut chain = self.chain.as_bytes().to_vec();
        chain.extend_from_slice(&bytes);
        self.chain = RootDigest::sha256(&chain);
        self.records.insert(record.ordinal, record);
    }
    pub(super) fn fingerprint(&self) -> RootDigest {
        self.chain
    }
}
fn invalid() -> Error {
    Error::Raft("invalid local configuration timeline".into())
}

impl<F: FileSystem> DiskRaftStorage<F> {
    /// Query at most four actual local records. A snapshot is a boundary with
    /// unknown imported ancestry, not a replacement initial configuration.
    pub fn configuration_timeline_page(
        &self,
        after: Option<TimelineCursor>,
    ) -> Result<TimelinePage> {
        let writer = self.file.lock().expect("raft log file poisoned");
        if writer.is_none() {
            return Err(invalid());
        }
        self.membership_history
            .lock()
            .expect("membership history poisoned")
            .timeline_page(after)
    }
}

#[cfg(test)]
mod tests;
