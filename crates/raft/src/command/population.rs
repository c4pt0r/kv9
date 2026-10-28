//! A split-copy frame is data, not proof of a committed metadata intent.
//! Runtime supplies that authority; apply checks its exact local target and
//! keeps the ownership barrier independent of caller-side preflight reads.
use super::{put_bytes, Reader};
use kv9_common::codec::{decode_key, KeyMode};
use kv9_common::data_range::DataRange;
use kv9_common::{Error, Result};

pub const MAX_ROWS: usize = 256;
pub const MAX_COMMAND_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPopulation {
    pub(crate) source: DataRange,
    pub(crate) target: DataRange,
    pub(crate) operation: [u8; 16],
    pub(crate) rows: Vec<(Vec<u8>, Vec<u8>)>,
}

impl RawPopulation {
    /// Available bytes for length-prefixed key/value pairs in this exact frame.
    pub fn row_bytes_budget(source: &DataRange, target: &DataRange) -> Result<usize> {
        source.validate()?;
        target.validate()?;
        Ok(
            MAX_COMMAND_BYTES
                - (2 + 16 + 4 + source.encode().len() + 4 + target.encode().len() + 4),
        )
    }
    pub fn new(
        source: DataRange,
        target: DataRange,
        operation: [u8; 16],
        rows: Vec<(Vec<u8>, Vec<u8>)>,
    ) -> Result<Self> {
        let frame = Self {
            source,
            target,
            operation,
            rows,
        };
        frame.validate()?;
        Ok(frame)
    }
    fn validate(&self) -> Result<()> {
        let bad = || Error::Raft("invalid or oversized Raw population".into());
        let (s, t) = (&self.source, &self.target);
        s.validate()?;
        t.validate()?;
        let low = t.start == s.start
            && !t.end.is_empty()
            && t.end > s.start
            && (s.end.is_empty() || t.end < s.end);
        let high = t.end == s.end && t.start > s.start && s.contains(&t.start);
        if !s.sealed
            || t.sealed
            || t.version != 1
            || t.conf_ver != 1
            || t.root != s.root
            || t.keyspace != s.keyspace
            || t.tenant != s.tenant
            || t.region == s.region
            || t.creation == s.creation
            || !(low || high)
            || self.operation == [0; 16]
            || self.rows.is_empty()
            || self.rows.len() > MAX_ROWS
        {
            return Err(bad());
        }
        let budget = Self::row_bytes_budget(s, t)?;
        let mut bytes = 0usize;
        let mut previous: Option<&[u8]> = None;
        for (key, value) in &self.rows {
            let decoded = decode_key(key)?;
            if decoded.mode != KeyMode::Raw
                || decoded.keyspace != t.keyspace
                || !t.contains(decoded.user_key)
                || decoded.user_key.len() > kv9_common::data_range::MAX_BOUND
                || previous.is_some_and(|p| p >= key.as_slice())
            {
                return Err(bad());
            }
            bytes = bytes
                .checked_add(8)
                .and_then(|n| n.checked_add(key.len()))
                .and_then(|n| n.checked_add(value.len()))
                .ok_or_else(bad)?;
            if bytes > budget {
                return Err(bad());
            }
            previous = Some(key);
        }
        Ok(())
    }
    pub(super) fn encode_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.operation);
        put_bytes(out, &self.source.encode());
        put_bytes(out, &self.target.encode());
        out.extend_from_slice(&(self.rows.len() as u32).to_be_bytes());
        for (key, value) in &self.rows {
            put_bytes(out, key);
            put_bytes(out, value);
        }
    }
    pub(super) fn decode_from(r: &mut Reader<'_>) -> Result<Self> {
        let operation = r.take(16)?.try_into().unwrap();
        let source = DataRange::decode(&r.bytes()?)?;
        let target = DataRange::decode(&r.bytes()?)?;
        let count = r.u32()? as usize;
        if count == 0 || count > MAX_ROWS {
            return Err(Error::Raft("invalid Raw population row count".into()));
        }
        let mut rows = Vec::with_capacity(count);
        for _ in 0..count {
            rows.push((r.bytes()?, r.bytes()?));
        }
        Self::new(source, target, operation, rows)
    }
}
