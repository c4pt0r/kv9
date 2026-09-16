//! Canonical data-group ownership. Decoding is not creation authority.
use crate::{Error, KeyspaceId, RegionId, Result, RootDigest, TenantId};

pub const RANGE_KEY: &[u8] = b"s\0\0\0kv9.data-range.v1";
const MAX_BOUND: usize = 65536;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataRange {
    pub root: RootDigest,
    pub creation: RootDigest,
    pub region: RegionId,
    pub keyspace: KeyspaceId,
    pub tenant: TenantId,
    pub conf_ver: u64,
    pub version: u64,
    pub start: Vec<u8>,
    pub end: Vec<u8>,
    pub sealed: bool,
}

impl DataRange {
    pub fn validate(&self) -> Result<()> {
        if self.root.as_bytes() == &[0; 32]
            || self.creation.as_bytes() == &[0; 32]
            || self.region.0 < 100
            || self.keyspace.0 == 0
            || self.keyspace.0 > KeyspaceId::MAX
            || self.conf_ver == 0
            || self.version == 0
            || self.start.len() > MAX_BOUND
            || self.end.len() > MAX_BOUND
            || (!self.end.is_empty() && self.start >= self.end)
        {
            return Err(Error::Config(
                "invalid data range identity or bounds".into(),
            ));
        }
        Ok(())
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut b = b"KV9RNG01".to_vec();
        b.extend_from_slice(self.root.as_bytes());
        b.extend_from_slice(self.creation.as_bytes());
        b.extend_from_slice(&self.region.0.to_be_bytes());
        b.extend_from_slice(&self.keyspace.0.to_be_bytes());
        for n in [self.tenant.0, self.conf_ver, self.version] {
            b.extend_from_slice(&n.to_be_bytes());
        }
        b.push(u8::from(self.sealed));
        for bound in [&self.start, &self.end] {
            b.extend_from_slice(&(bound.len() as u32).to_be_bytes());
            b.extend_from_slice(bound);
        }
        b
    }
    pub fn decode(b: &[u8]) -> Result<Self> {
        let bad = || Error::Config("invalid data range encoding".into());
        if b.len() < 117 || b.len() > 117 + 2 * MAX_BOUND || &b[..8] != b"KV9RNG01" || b[108] > 1 {
            return Err(bad());
        }
        let mut offset = 109;
        let mut bound = || -> Result<Vec<u8>> {
            let len = u32::from_be_bytes(
                b.get(offset..offset + 4)
                    .ok_or_else(bad)?
                    .try_into()
                    .unwrap(),
            ) as usize;
            offset += 4;
            if len > MAX_BOUND {
                return Err(bad());
            }
            let value = b.get(offset..offset + len).ok_or_else(bad)?.to_vec();
            offset += len;
            Ok(value)
        };
        let start = bound()?;
        let end = bound()?;
        if offset != b.len() {
            return Err(bad());
        }
        let range = Self {
            root: RootDigest::from_bytes(b[8..40].try_into().unwrap()),
            creation: RootDigest::from_bytes(b[40..72].try_into().unwrap()),
            region: RegionId(u64::from_be_bytes(b[72..80].try_into().unwrap())),
            keyspace: KeyspaceId(u32::from_be_bytes(b[80..84].try_into().unwrap())),
            tenant: TenantId(u64::from_be_bytes(b[84..92].try_into().unwrap())),
            conf_ver: u64::from_be_bytes(b[92..100].try_into().unwrap()),
            version: u64::from_be_bytes(b[100..108].try_into().unwrap()),
            sealed: b[108] == 1,
            start,
            end,
        };
        range.validate()?;
        Ok(range)
    }
    pub fn digest(&self) -> RootDigest {
        RootDigest::sha256(&self.encode())
    }
    pub fn contains(&self, key: &[u8]) -> bool {
        key >= self.start.as_slice() && (self.end.is_empty() || key < self.end.as_slice())
    }
    /// Initial publication or one-way sealing only. Movement/split publication
    /// needs its own protocol; this command cannot expand or transfer ownership.
    pub fn may_follow(&self, current: &Self) -> bool {
        let mut sealed = current.clone();
        let Some(next) = current.version.checked_add(1) else {
            return false;
        };
        sealed.version = next;
        sealed.sealed = true;
        !current.sealed && *self == sealed
    }
}
