use super::*;
use crate::configuration_origin::{ConfigurationOrigin, ORIGIN_BYTES};
use std::ops::Bound::{Excluded, Unbounded};
pub const RECORDS_PER_PAGE: usize = 4;
pub const MAX_PAGE_BYTES: usize = 144 * 1024;
const MAX_RECORD_BYTES: usize = 33_024;
pub(super) struct Reader<'a>(pub(super) &'a [u8]);
impl<'a> Reader<'a> {
    pub(super) fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.0.len() {
            return Err(invalid());
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    pub(super) fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        Ok(self.take(N)?.try_into().unwrap())
    }
    pub(super) fn uint(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.array()?))
    }
    pub(super) fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub(super) fn boolean(&mut self) -> Result<bool> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(invalid()),
        }
    }
}
fn uint(b: &mut Vec<u8>, n: u64) {
    b.extend_from_slice(&n.to_be_bytes());
}
impl ConfigurationRecord {
    pub fn encode(&self) -> Result<Vec<u8>> {
        if self.ordinal == 0 {
            return Err(invalid());
        }
        let (tag, term, index) = match self.kind {
            RecordKind::Initial => (0, 0, 0),
            RecordKind::Applied { index } => (1, 0, index),
            RecordKind::Snapshot { at } => (2, at.term, at.index),
        };
        if tag == 2 && (term == 0 || index == 0 || index == u64::MAX) {
            return Err(invalid());
        }
        let mut b = b"KV9CTR01".to_vec();
        uint(&mut b, self.ordinal);
        b.push(tag);
        uint(&mut b, term);
        uint(&mut b, index);
        b.push(u8::from(self.origin.is_some()));
        if let Some(origin) = self.origin {
            if tag != 1
                || origin.at.index != index
                || origin.at.term == 0
                || index == 0
                || index == u64::MAX
            {
                return Err(invalid());
            }
            uint(&mut b, origin.at.term);
            uint(&mut b, index);
            b.extend_from_slice(&origin.origin.encode()?);
        }
        b.push(u8::from(self.state.is_some()));
        if let Some(state) = &self.state {
            b.extend_from_slice(&state.encode()?);
        }
        if b.len() > MAX_RECORD_BYTES {
            return Err(invalid());
        }
        Ok(b)
    }
    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() > MAX_RECORD_BYTES {
            return Err(invalid());
        }
        let mut r = Reader(b);
        if r.take(8)? != b"KV9CTR01" {
            return Err(invalid());
        }
        let ordinal = r.uint()?;
        let tag = r.byte()?;
        let term = r.uint()?;
        let index = r.uint()?;
        let kind = match tag {
            0 if term == 0 && index == 0 => RecordKind::Initial,
            1 if term == 0 => RecordKind::Applied { index },
            2 => RecordKind::Snapshot {
                at: AppliedPosition { term, index },
            },
            _ => return Err(invalid()),
        };
        let origin = if r.boolean()? {
            Some(AppliedConfigurationOrigin {
                at: AppliedPosition {
                    term: r.uint()?,
                    index: r.uint()?,
                },
                origin: ConfigurationOrigin::decode(r.take(ORIGIN_BYTES)?)?,
            })
        } else {
            None
        };
        let state = if r.boolean()? {
            let state = ConfigurationState::decode(r.0)?;
            r.0 = &[];
            Some(state)
        } else {
            None
        };
        let out = Self {
            ordinal,
            kind,
            state,
            origin,
        };
        if !r.0.is_empty() || out.encode()? != b {
            return Err(invalid());
        }
        Ok(out)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimelineCursor {
    fingerprint: RootDigest,
    after: u64,
}
impl TimelineCursor {
    pub fn seen(self) -> u64 {
        self.after
    }
    pub fn encode(self) -> Vec<u8> {
        let mut b = b"KV9CTC01".to_vec();
        b.extend_from_slice(self.fingerprint.as_bytes());
        uint(&mut b, self.after);
        b
    }
    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() != 48 || &b[..8] != b"KV9CTC01" {
            return Err(invalid());
        }
        let out = Self {
            fingerprint: RootDigest::from_bytes(b[8..40].try_into().unwrap()),
            after: u64::from_be_bytes(b[40..48].try_into().unwrap()),
        };
        if out.fingerprint.as_bytes() == &[0; 32]
            || out.after == 0
            || !out.after.is_multiple_of(RECORDS_PER_PAGE as u64)
        {
            return Err(invalid());
        }
        Ok(out)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelinePage {
    pub fingerprint: RootDigest,
    pub gaps: MembershipGaps,
    pub total: u64,
    pub records: Vec<ConfigurationRecord>,
    pub next: Option<TimelineCursor>,
}
impl TimelinePage {
    fn validate_shape(&self) -> Result<()> {
        if self.fingerprint.as_bytes()==&[0;32] || self.records.len()>RECORDS_PER_PAGE || self.records.len() as u64>self.total
            || (self.records.is_empty() && self.total!=0)
            || self.records.iter().any(|r|r.ordinal==0 || r.ordinal>self.total
                || (r.state.is_none() && !self.gaps.unsupported_configuration)
                || (matches!(r.kind,RecordKind::Snapshot{..}) && !self.gaps.snapshot_prefix_unknown)
                || ((matches!(r.kind,RecordKind::Initial) && r.ordinal!=1
                    || matches!(r.kind,RecordKind::Applied{index} if index==0 || index==u64::MAX)) && !self.gaps.ambiguous_records))
            || self.records.windows(2).any(|w|w[0].ordinal.checked_add(1)!=Some(w[1].ordinal)) {return Err(invalid());}
        match self.next {
            Some(c)
                if self.records.len() == RECORDS_PER_PAGE
                    && c.fingerprint == self.fingerprint
                    && c.after.is_multiple_of(RECORDS_PER_PAGE as u64)
                    && c.after < self.total
                    && self.records.last().is_some_and(|r| r.ordinal == c.after) => {}
            None if self.records.last().map_or(0, |r| r.ordinal) == self.total => (),
            _ => return Err(invalid()),
        }
        Ok(())
    }
    pub fn validate_progress(&self, after: Option<TimelineCursor>) -> Result<()> {
        self.validate_shape()?;
        let seen = after.map_or(0, |c| c.after);
        if after.is_some_and(|c| c.fingerprint != self.fingerprint || c.after >= self.total)
            || self
                .records
                .first()
                .is_some_and(|r| seen.checked_add(1) != Some(r.ordinal))
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate_shape()?;
        let mut b = b"KV9CTP01".to_vec();
        b.extend_from_slice(self.fingerprint.as_bytes());
        let g = self.gaps;
        b.push(
            u8::from(g.missing_initial)
                | (u8::from(g.snapshot_prefix_unknown) << 1)
                | (u8::from(g.ambiguous_records) << 2)
                | (u8::from(g.unsupported_configuration) << 3)
                | (u8::from(g.member_limit_exceeded) << 4),
        );
        uint(&mut b, self.total);
        b.push(self.records.len() as u8);
        for record in &self.records {
            let bytes = record.encode()?;
            b.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
            b.extend_from_slice(&bytes);
        }
        b.push(u8::from(self.next.is_some()));
        if let Some(c) = self.next {
            b.extend_from_slice(&c.encode());
        }
        if b.len() > MAX_PAGE_BYTES {
            return Err(invalid());
        }
        Ok(b)
    }
    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() > MAX_PAGE_BYTES {
            return Err(invalid());
        }
        let mut r = Reader(b);
        if r.take(8)? != b"KV9CTP01" {
            return Err(invalid());
        }
        let fingerprint = RootDigest::from_bytes(r.array()?);
        let flags = r.byte()?;
        if flags & !31 != 0 {
            return Err(invalid());
        }
        let gaps = MembershipGaps {
            missing_initial: flags & 1 != 0,
            snapshot_prefix_unknown: flags & 2 != 0,
            ambiguous_records: flags & 4 != 0,
            unsupported_configuration: flags & 8 != 0,
            member_limit_exceeded: flags & 16 != 0,
        };
        let total = r.uint()?;
        let n = r.byte()? as usize;
        if n > RECORDS_PER_PAGE {
            return Err(invalid());
        }
        let mut records = Vec::with_capacity(n);
        for _ in 0..n {
            let n = u32::from_be_bytes(r.array()?) as usize;
            records.push(ConfigurationRecord::decode(r.take(n)?)?);
        }
        let next = if r.boolean()? {
            Some(TimelineCursor::decode(r.take(48)?)?)
        } else {
            None
        };
        let out = Self {
            fingerprint,
            gaps,
            total,
            records,
            next,
        };
        if !r.0.is_empty() || out.encode()? != b {
            return Err(invalid());
        }
        Ok(out)
    }
}
impl ConfigurationTimeline {
    pub(in crate::storage) fn page(
        &self,
        fingerprint: RootDigest,
        gaps: MembershipGaps,
        after: Option<TimelineCursor>,
    ) -> Result<TimelinePage> {
        let total = self.records.len() as u64;
        if after.is_some_and(|c| c.fingerprint != fingerprint || c.after >= total) {
            return Err(invalid());
        }
        let records: Vec<_> = self
            .records
            .range((Excluded(after.map_or(0, |c| c.after)), Unbounded))
            .take(RECORDS_PER_PAGE)
            .map(|(_, r)| r.clone())
            .collect();
        let seen = records.last().map_or(0, |r| r.ordinal);
        let next = (seen < total).then_some(TimelineCursor {
            fingerprint,
            after: seen,
        });
        let page = TimelinePage {
            fingerprint,
            gaps,
            total,
            records,
            next,
        };
        page.validate_progress(after)?;
        Ok(page)
    }
}
