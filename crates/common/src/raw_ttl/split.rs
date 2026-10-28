//! Canonical TTL split state. These descriptors are data, not metadata/Raft
//! authority. The protected coordinator must certify the source and empty child.
use super::{ClockWatermark, ExpiryRecord};
use crate::codec::{decode_key, KeyMode};
use crate::data_range::{DataRange, MAX_BOUND, MAX_ENCODED_BYTES};
use crate::{AppliedPosition, Error, Result, RootDigest};
use sha2::{Digest, Sha256};

pub const SEAL_KEY: &[u8] = b"s\0\0\0kv9.ttl-split-seal.v1";
pub const IMPORT_KEY: &[u8] = b"s\0\0\0kv9.ttl-split-import.v1";
pub const MAX_RECORD_BYTES: usize = 3 * MAX_ENCODED_BYTES + MAX_BOUND + 512;
pub const MAX_PAGE_ROWS: usize = 256;
pub const MAX_PAGE_BYTES: usize = 16 * 1024 * 1024;

fn invalid() -> Error {
    Error::Raft("invalid TTL split state or transition".into())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitPlan {
    pub operation: [u8; 16],
    /// The exact unsealed predecessor. Sealing increments only its version.
    pub parent: DataRange,
    pub low: DataRange,
    pub high: DataRange,
}
impl SplitPlan {
    pub fn validate(&self) -> Result<()> {
        self.parent.validate()?;
        self.low.validate()?;
        self.high.validate()?;
        let p = &self.parent;
        if self.operation == [0; 16]
            || p.sealed
            || p.version == u64::MAX
            || self.low.start != p.start
            || self.high.end != p.end
            || self.low.end.is_empty()
            || self.low.end != self.high.start
            || self.low.end <= p.start
            || !p.contains(&self.low.end)
            || self.low.region == self.high.region
            || self.low.creation == self.high.creation
        {
            return Err(invalid());
        }
        for c in [&self.low, &self.high] {
            if c.root != p.root
                || c.keyspace != p.keyspace
                || c.tenant != p.tenant
                || c.region == p.region
                || c.creation == p.creation
                || c.sealed
                || c.conf_ver != 1
                || c.version != 1
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
    pub fn sealed_range(&self) -> Result<DataRange> {
        self.validate()?;
        let mut range = self.parent.clone();
        range.sealed = true;
        range.version += 1;
        Ok(range)
    }
    pub fn target(&self, high: bool) -> &DataRange {
        if high {
            &self.high
        } else {
            &self.low
        }
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut b = b"KV9TSP01".to_vec();
        b.extend_from_slice(&self.operation);
        for r in [&self.parent, &self.low, &self.high] {
            put_bytes(&mut b, &r.encode());
        }
        checksum(b)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::record(bytes, b"KV9TSP01")?;
        let operation = r.take(16)?.try_into().unwrap();
        let plan = Self {
            operation,
            parent: DataRange::decode(r.bytes(MAX_ENCODED_BYTES)?)?,
            low: DataRange::decode(r.bytes(MAX_ENCODED_BYTES)?)?,
            high: DataRange::decode(r.bytes(MAX_ENCODED_BYTES)?)?,
        };
        r.finish()?;
        plan.validate()?;
        Ok(plan)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitSeal {
    pub plan: SplitPlan,
    pub cut: AppliedPosition,
    pub watermark: ClockWatermark,
}
impl SplitSeal {
    pub fn validate(&self) -> Result<()> {
        self.plan.validate()?;
        if self.cut.term == 0 || self.cut.index == 0 {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut b = b"KV9TSS01".to_vec();
        put_bytes(&mut b, &self.plan.encode());
        b.extend_from_slice(&self.cut.term.to_be_bytes());
        b.extend_from_slice(&self.cut.index.to_be_bytes());
        b.extend_from_slice(&self.watermark.encode());
        checksum(b)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::record(bytes, b"KV9TSS01")?;
        let seal = Self {
            plan: SplitPlan::decode(r.bytes(MAX_RECORD_BYTES)?)?,
            cut: AppliedPosition {
                term: r.u64()?,
                index: r.u64()?,
            },
            watermark: ClockWatermark::decode(r.take(16)?)?,
        };
        r.finish()?;
        seal.validate()?;
        Ok(seal)
    }
    pub fn digest(&self) -> RootDigest {
        RootDigest::sha256(&self.encode())
    }
}

/// Original expiry bytes and identity travel with the opaque value. No new
/// write position or deadline is manufactured on the destination group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRow {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub expiry: Option<ExpiryRecord>,
}
impl ImportRow {
    pub fn validate(&self, target: &DataRange) -> Result<()> {
        let key = decode_key(&self.key)?;
        if key.mode != KeyMode::Raw
            || key.keyspace != target.keyspace
            || key.user_key.len() > MAX_BOUND
            || !target.contains(key.user_key)
        {
            return Err(invalid());
        }
        if let Some(e) = self.expiry {
            ExpiryRecord::decode(&e.encode())?;
        }
        Ok(())
    }
    pub fn encoded_bytes(&self) -> Result<usize> {
        self.key
            .len()
            .checked_add(self.value.len())
            .and_then(|n| n.checked_add(9 + usize::from(self.expiry.is_some()) * 64))
            .ok_or_else(invalid)
    }
    pub fn fold(&self, prior: RootDigest) -> RootDigest {
        let mut h = Sha256::new();
        h.update(b"kv9.ttl-split.row.v1");
        h.update(prior.as_bytes());
        h.update((self.key.len() as u64).to_be_bytes());
        h.update(&self.key);
        h.update((self.value.len() as u64).to_be_bytes());
        h.update(&self.value);
        h.update([u8::from(self.expiry.is_some())]);
        if let Some(e) = self.expiry {
            h.update(e.encode());
        }
        RootDigest::from_bytes(h.finalize().into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSpec {
    pub seal: SplitSeal,
    pub high: bool,
    pub rows: u64,
    pub digest: RootDigest,
}
impl ImportSpec {
    pub fn target(&self) -> &DataRange {
        self.seal.plan.target(self.high)
    }
    pub fn seed(&self) -> RootDigest {
        let mut b = b"kv9.ttl-split.image.v1".to_vec();
        b.extend_from_slice(self.seal.digest().as_bytes());
        b.push(u8::from(self.high));
        RootDigest::sha256(&b)
    }
    pub fn validate(&self) -> Result<()> {
        self.seal.validate()?;
        if self.digest.as_bytes() == &[0; 32] || (self.rows == 0 && self.digest != self.seed()) {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut b = b"KV9TSI01".to_vec();
        put_bytes(&mut b, &self.seal.encode());
        b.push(u8::from(self.high));
        b.extend_from_slice(&self.rows.to_be_bytes());
        b.extend_from_slice(self.digest.as_bytes());
        checksum(b)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::record(bytes, b"KV9TSI01")?;
        let spec = Self {
            seal: SplitSeal::decode(r.bytes(MAX_RECORD_BYTES)?)?,
            high: r.boolean()?,
            rows: r.u64()?,
            digest: r.digest()?,
        };
        r.finish()?;
        spec.validate()?;
        Ok(spec)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportState {
    pub spec: ImportSpec,
    pub copied: u64,
    /// Exclusive continuation is formed by appending zero to this physical key.
    pub last: Option<Vec<u8>>,
    pub chain: RootDigest,
    pub complete: bool,
}
impl ImportState {
    pub fn begin(spec: ImportSpec) -> Result<Self> {
        spec.validate()?;
        Ok(Self {
            chain: spec.seed(),
            spec,
            copied: 0,
            last: None,
            complete: false,
        })
    }
    pub fn validate(&self) -> Result<()> {
        self.spec.validate()?;
        if self.copied > self.spec.rows
            || self.chain.as_bytes() == &[0; 32]
            || (self.copied == 0) != self.last.is_none()
            || (self.copied == 0 && self.chain != self.spec.seed())
            || (self.complete && (self.copied != self.spec.rows || self.chain != self.spec.digest))
        {
            return Err(invalid());
        }
        if let Some(key) = &self.last {
            ImportRow {
                key: key.clone(),
                value: vec![],
                expiry: None,
            }
            .validate(self.spec.target())?;
        }
        Ok(())
    }
    pub fn append(&self, rows: &[ImportRow]) -> Result<Self> {
        self.validate()?;
        if self.complete || rows.is_empty() || rows.len() > MAX_PAGE_ROWS {
            return Err(invalid());
        }
        let mut next = self.clone();
        let mut bytes = 0usize;
        for row in rows {
            row.validate(self.spec.target())?;
            if next.last.as_ref().is_some_and(|last| last >= &row.key) {
                return Err(invalid());
            }
            bytes = bytes
                .checked_add(row.encoded_bytes()?)
                .ok_or_else(invalid)?;
            if bytes > MAX_PAGE_BYTES {
                return Err(invalid());
            }
            next.copied = next.copied.checked_add(1).ok_or_else(invalid)?;
            next.last = Some(row.key.clone());
            next.chain = row.fold(next.chain);
        }
        next.validate()?;
        Ok(next)
    }
    pub fn finish(&self) -> Result<Self> {
        self.validate()?;
        let mut next = self.clone();
        next.complete = true;
        next.validate()?;
        Ok(next)
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut b = b"KV9TSC01".to_vec();
        put_bytes(&mut b, &self.spec.encode());
        b.extend_from_slice(&self.copied.to_be_bytes());
        b.extend_from_slice(self.chain.as_bytes());
        b.push(u8::from(self.complete));
        b.push(u8::from(self.last.is_some()));
        if let Some(key) = &self.last {
            put_bytes(&mut b, key);
        }
        checksum(b)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::record(bytes, b"KV9TSC01")?;
        let state = Self {
            spec: ImportSpec::decode(r.bytes(MAX_RECORD_BYTES)?)?,
            copied: r.u64()?,
            chain: r.digest()?,
            complete: r.boolean()?,
            last: if r.boolean()? {
                Some(r.bytes(MAX_BOUND + 4)?.to_vec())
            } else {
                None
            },
        };
        r.finish()?;
        state.validate()?;
        Ok(state)
    }
    pub fn digest(&self) -> RootDigest {
        RootDigest::sha256(&self.encode())
    }
}

/// Shared final-state check for live apply, base images and WAL replay. An
/// unbound TTL clock is legal only with an exact unfinished import; a sealed
/// TTL range requires its own immutable source seal. Completed imports may
/// advance time and later split again without changing their original receipt.
pub fn validate_local(
    identity: (RootDigest, crate::RegionId, RootDigest),
    range: Option<&DataRange>,
    clock: Option<ClockWatermark>,
    seal: Option<&SplitSeal>,
    import: Option<&ImportState>,
) -> Result<()> {
    let matches = |r: &DataRange| (r.root, r.region, r.creation) == identity;
    if range.is_some_and(|r| !matches(r)) {
        return Err(invalid());
    }
    if let Some(seal) = seal {
        seal.validate()?;
        if !matches(&seal.plan.parent)
            || range != Some(&seal.plan.sealed_range()?)
            || clock != Some(seal.watermark)
        {
            return Err(invalid());
        }
    }
    if let Some(import) = import {
        import.validate()?;
        let target = import.spec.target();
        if !matches(target) || clock.is_none_or(|c| c < import.spec.seal.watermark) {
            return Err(invalid());
        }
        if import.complete {
            if range != Some(target) && seal.is_none_or(|s| &s.plan.parent != target) {
                return Err(invalid());
            }
        } else if range.is_some() || seal.is_some() || clock != Some(import.spec.seal.watermark) {
            return Err(invalid());
        }
    }
    if clock.is_some()
        && (range.is_none() && import.is_none()
            || range.is_some_and(|r| r.sealed) && seal.is_none())
    {
        return Err(invalid());
    }
    Ok(())
}

fn put_bytes(b: &mut Vec<u8>, value: &[u8]) {
    b.extend_from_slice(&(value.len() as u32).to_be_bytes());
    b.extend_from_slice(value);
}
fn checksum(mut b: Vec<u8>) -> Vec<u8> {
    b.extend_from_slice(RootDigest::sha256(&b).as_bytes());
    b
}
struct Reader<'a> {
    bytes: &'a [u8],
}
impl<'a> Reader<'a> {
    fn record(bytes: &'a [u8], magic: &[u8; 8]) -> Result<Self> {
        if bytes.len() < 40 || bytes.len() > MAX_RECORD_BYTES || &bytes[..8] != magic {
            return Err(invalid());
        }
        let end = bytes.len() - 32;
        if RootDigest::sha256(&bytes[..end]).as_bytes() != &bytes[end..] {
            return Err(invalid());
        }
        Ok(Self {
            bytes: &bytes[8..end],
        })
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.bytes.len() < n {
            return Err(invalid());
        }
        let (head, tail) = self.bytes.split_at(n);
        self.bytes = tail;
        Ok(head)
    }
    fn bytes(&mut self, max: usize) -> Result<&'a [u8]> {
        let n = u32::from_be_bytes(self.take(4)?.try_into().unwrap()) as usize;
        if n > max {
            return Err(invalid());
        }
        self.take(n)
    }
    fn boolean(&mut self) -> Result<bool> {
        match self.take(1)?[0] {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(invalid()),
        }
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn digest(&mut self) -> Result<RootDigest> {
        Ok(RootDigest::from_bytes(self.take(32)?.try_into().unwrap()))
    }
    fn finish(self) -> Result<()> {
        if self.bytes.is_empty() {
            Ok(())
        } else {
            Err(invalid())
        }
    }
}
