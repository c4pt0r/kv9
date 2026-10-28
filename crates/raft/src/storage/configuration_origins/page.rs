use super::*;
use crate::storage::membership_history::{InitialConfiguration, MembershipGaps, MembershipPage};
use kv9_common::RootDigest;
use std::ops::Bound::{Excluded, Unbounded};

pub const ORIGINS_PER_PAGE: usize = 4;
const CURSOR_MAGIC: &[u8; 8] = b"KV9OCP01";
const PAGE_MAGIC: &[u8; 8] = b"KV9OPG01";
const INITIAL_PAGE_MAGIC: &[u8; 8] = b"KV9OPG02";
const ENTRY_BYTES: usize = 16 + ORIGIN_BYTES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OriginCursor {
    fingerprint: RootDigest,
    after: u64,
    seen: u64,
}
impl OriginCursor {
    /// Number of records actually visited before this continuation.
    pub fn seen(self) -> u64 {
        self.seen
    }

    pub fn encode(self) -> Vec<u8> {
        let mut b = CURSOR_MAGIC.to_vec();
        b.extend_from_slice(self.fingerprint.as_bytes());
        b.extend_from_slice(&self.after.to_be_bytes());
        b.extend_from_slice(&self.seen.to_be_bytes());
        b
    }
    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() != 56 || &b[..8] != CURSOR_MAGIC {
            return Err(invalid());
        }
        let out = Self {
            fingerprint: RootDigest::from_bytes(b[8..40].try_into().unwrap()),
            after: u64::from_be_bytes(b[40..48].try_into().unwrap()),
            seen: u64::from_be_bytes(b[48..56].try_into().unwrap()),
        };
        if out.fingerprint.as_bytes() == &[0; 32]
            || out.after == 0
            || out.after == u64::MAX
            || out.seen == 0
            || !out.seen.is_multiple_of(4)
        {
            return Err(invalid());
        }
        Ok(out)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginPage {
    /// None is the legacy page encoding, never a certified initial record.
    pub initial: Option<InitialConfiguration>,
    pub fingerprint: RootDigest,
    pub membership_gaps: MembershipGaps,
    pub legacy_contexts_unknown: bool,
    pub last_configuration_index: u64,
    pub last_snapshot: Option<AppliedPosition>,
    pub total_known: u64,
    pub entries: Vec<AppliedConfigurationOrigin>,
    pub next: Option<OriginCursor>,
}
impl OriginPage {
    fn validate_shape(&self) -> Result<()> {
        if let Some(initial) = &self.initial {
            initial.encode()?;
        }
        if self.fingerprint.as_bytes() == &[0; 32]
            || self.entries.len() > ORIGINS_PER_PAGE
            || self.entries.len() as u64 > self.total_known
            || (self.entries.is_empty() && self.total_known != 0)
            || self.entries.iter().any(|e| {
                e.at.term == 0
                    || e.at.index == 0
                    || e.at.index == u64::MAX
                    || e.at.index > self.last_configuration_index
            })
            || self
                .entries
                .windows(2)
                .any(|w| w[0].at.index >= w[1].at.index)
            || self.last_snapshot.is_some_and(|s| {
                s.term == 0
                    || s.index == 0
                    || s.index > self.last_configuration_index
                    || !self.membership_gaps.snapshot_prefix_unknown
            })
            || self.next.is_some_and(|c| {
                self.entries.len() != ORIGINS_PER_PAGE
                    || c.fingerprint != self.fingerprint
                    || c.seen < ORIGINS_PER_PAGE as u64
                    || !c.seen.is_multiple_of(ORIGINS_PER_PAGE as u64)
                    || c.seen >= self.total_known
                    || self.entries.last().is_none_or(|e| e.at.index != c.after)
            })
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn validate_progress(&self, previous: Option<OriginCursor>) -> Result<()> {
        self.validate_shape()?;
        let seen = previous.map_or(0, |c| c.seen);
        if previous.is_some_and(|c| c.fingerprint != self.fingerprint || c.seen >= self.total_known)
            || previous
                .zip(self.entries.first())
                .is_some_and(|(c, e)| c.after >= e.at.index)
        {
            return Err(invalid());
        }
        let end = seen
            .checked_add(self.entries.len() as u64)
            .ok_or_else(invalid)?;
        match self.next {
            Some(c)
                if self.entries.len() == ORIGINS_PER_PAGE
                    && end < self.total_known
                    && c.seen == end
                    && c.after == self.entries.last().unwrap().at.index
                    && c.fingerprint == self.fingerprint => {}
            None if end == self.total_known => {}
            _ => return Err(invalid()),
        }
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate_shape()?;
        let g = self.membership_gaps;
        let flags = u8::from(g.missing_initial)
            | (u8::from(g.snapshot_prefix_unknown) << 1)
            | (u8::from(g.ambiguous_records) << 2)
            | (u8::from(g.unsupported_configuration) << 3)
            | (u8::from(g.member_limit_exceeded) << 4)
            | (u8::from(self.legacy_contexts_unknown) << 5);
        let mut b = if self.initial.is_some() {
            INITIAL_PAGE_MAGIC
        } else {
            PAGE_MAGIC
        }
        .to_vec();
        b.extend_from_slice(self.fingerprint.as_bytes());
        b.push(flags);
        b.extend_from_slice(&self.last_configuration_index.to_be_bytes());
        let cut = self
            .last_snapshot
            .unwrap_or(AppliedPosition { term: 0, index: 0 });
        for n in [cut.term, cut.index, self.total_known] {
            b.extend_from_slice(&n.to_be_bytes());
        }
        b.push(self.entries.len() as u8);
        for e in &self.entries {
            b.extend_from_slice(&e.observation_bytes()?);
        }
        b.push(u8::from(self.next.is_some()));
        if let Some(c) = self.next {
            b.extend_from_slice(&c.encode());
        }
        if let Some(initial) = &self.initial {
            b.extend_from_slice(&initial.encode()?);
        }
        Ok(b)
    }
    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() < 75
            || b.len() > 75 + 4 * ENTRY_BYTES + 56 + 58
            || (&b[..8] != PAGE_MAGIC && &b[..8] != INITIAL_PAGE_MAGIC)
            || b[40] & !63 != 0
        {
            return Err(invalid());
        }
        let uint = |at| u64::from_be_bytes(b[at..at + 8].try_into().unwrap());
        let flags = b[40];
        let term = uint(49);
        let index = uint(57);
        let n = b[73] as usize;
        if n > ORIGINS_PER_PAGE || b.len() < 75 + n * ENTRY_BYTES || (term == 0) != (index == 0) {
            return Err(invalid());
        }
        let mut entries = Vec::with_capacity(n);
        for i in 0..n {
            let at = 74 + i * ENTRY_BYTES;
            entries.push(AppliedConfigurationOrigin {
                at: AppliedPosition {
                    term: uint(at),
                    index: uint(at + 8),
                },
                origin: ConfigurationOrigin::decode(&b[at + 16..at + ENTRY_BYTES])?,
            });
        }
        let at = 74 + n * ENTRY_BYTES;
        let (next, end) = match b[at] {
            0 => (None, at + 1),
            1 if b.len() >= at + 57 => (Some(OriginCursor::decode(&b[at + 1..at + 57])?), at + 57),
            _ => return Err(invalid()),
        };
        let initial = if &b[..8] == INITIAL_PAGE_MAGIC {
            Some(InitialConfiguration::decode(&b[end..])?)
        } else if end == b.len() {
            None
        } else {
            return Err(invalid());
        };
        let out = Self {
            initial,
            fingerprint: RootDigest::from_bytes(b[8..40].try_into().unwrap()),
            membership_gaps: MembershipGaps {
                missing_initial: flags & 1 != 0,
                snapshot_prefix_unknown: flags & 2 != 0,
                ambiguous_records: flags & 4 != 0,
                unsupported_configuration: flags & 8 != 0,
                member_limit_exceeded: flags & 16 != 0,
            },
            legacy_contexts_unknown: flags & 32 != 0,
            last_configuration_index: uint(41),
            last_snapshot: (term != 0).then_some(AppliedPosition { term, index }),
            total_known: uint(65),
            entries,
            next,
        };
        if out.encode()? != b {
            return Err(invalid());
        }
        Ok(out)
    }
}
impl Origins {
    pub(super) fn page(
        &self,
        history: MembershipPage,
        initial: InitialConfiguration,
        after: Option<OriginCursor>,
    ) -> Result<OriginPage> {
        let mut b = b"KV9 origin page 1".to_vec();
        b.extend_from_slice(self.chain.as_bytes());
        b.extend_from_slice(history.fingerprint.as_bytes());
        b.push(u8::from(self.legacy_unknown));
        b.extend_from_slice(&initial.encode()?);
        let fingerprint = RootDigest::sha256(&b);
        if after.is_some_and(|c| {
            c.fingerprint != fingerprint
                || self
                    .entries
                    .get(&c.after)
                    .is_none_or(|(ordinal, _)| *ordinal != c.seen)
        }) {
            return Err(invalid());
        }
        let entries: Vec<_> = self
            .entries
            .range((Excluded(after.map_or(0, |c| c.after)), Unbounded))
            .take(ORIGINS_PER_PAGE)
            .map(|(_, (_, e))| *e)
            .collect();
        let seen = after.map_or(0, |c| c.seen) + entries.len() as u64;
        let total_known = self.entries.len() as u64;
        let next = (seen < total_known).then(|| OriginCursor {
            fingerprint,
            after: entries.last().unwrap().at.index,
            seen,
        });
        let page = OriginPage {
            initial: Some(initial),
            fingerprint,
            membership_gaps: history.gaps,
            legacy_contexts_unknown: self.legacy_unknown,
            last_configuration_index: history.last_configuration_index,
            last_snapshot: history.last_snapshot,
            total_known,
            entries,
            next,
        };
        page.validate_progress(after)?;
        Ok(page)
    }
}
impl<F: FileSystem> DiskRaftStorage<F> {
    /// At most four known records, plus explicit legacy/ancestry uncertainty.
    /// This enumerates observations, never the complete set of historical stores.
    pub fn configuration_origins_page(&self, after: Option<OriginCursor>) -> Result<OriginPage> {
        let writer = self.file.lock().expect("raft log file poisoned");
        if writer.is_none() {
            return Err(invalid());
        }
        let (history, initial) = {
            let history = self
                .membership_history
                .lock()
                .expect("membership history poisoned");
            (history.page(None)?, history.initial_observation())
        };
        self.configuration_origins
            .lock()
            .expect("configuration origins poisoned")
            .page(history, initial, after)
    }
}
