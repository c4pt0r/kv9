//! Point-addressed resource pins. Released rows are retained as generation
//! fences; no operation scans the growing owner history or evicts tombstones.
//!
//! A legacy KV9PIN01 value becomes an immutable, checksum-bound base. Keeping
//! that bounded value intact avoids expanding 64 full legacy resources into
//! tens of MiB of rows in a single transaction. Later writes override just one
//! owner. Legacy fallback reads remain bounded by the old 4096-owner codec.
use super::*;
use kv9_common::retention::RetentionRecord;

const RESOURCE_MAGIC: &[u8; 8] = b"KV9RES02";
const PIN_MAGIC: &[u8; 8] = b"KV9RPO02";
const MAX_RESOURCE_BYTES: usize = 163;
const PIN_BYTES: usize = 154;

fn pin_prefix(identity: &ResourceIdentity) -> Vec<u8> {
    key(6, &resource_id(identity))
}
fn pin_key(identity: &ResourceIdentity, owner: OwnerId) -> Vec<u8> {
    let mut k = pin_prefix(identity);
    k.extend_from_slice(owner.as_bytes());
    k
}
fn base_key(identity: &ResourceIdentity) -> Vec<u8> {
    key(7, &resource_id(identity))
}
fn identity_bytes(b: &mut Vec<u8>, identity: ResourceIdentity) {
    b.extend_from_slice(identity.root().as_bytes());
    b.extend_from_slice(&resource_id(&identity));
    b.extend_from_slice(identity.content());
}
fn read_identity(r: &mut Reader<'_>, expected: ResourceIdentity) -> Result<()> {
    if r.array::<32>()? != *expected.root().as_bytes()
        || r.array::<17>()? != resource_id(&expected)
        || r.array::<32>()? != *expected.content()
    {
        return Err(invalid("resource row identity differs"));
    }
    Ok(())
}

pub(super) struct ResourceRecord {
    identity: ResourceIdentity,
    revision: u64,
    retired: bool,
    base: Option<RetentionRecord>,
    base_digest: Option<RootDigest>,
    /// Only present before the original monolithic value has been moved.
    legacy_bytes: Option<Vec<u8>>,
}
impl ResourceRecord {
    pub(super) fn new(identity: ResourceIdentity) -> Self {
        Self {
            identity,
            revision: 0,
            retired: false,
            base: None,
            base_digest: None,
            legacy_bytes: None,
        }
    }

    /// No namespace repair: a headerless/original-format resource must not
    /// already have a base or point rows. One iterator step is sufficient to
    /// refuse, and never interprets a history scan as release authority.
    pub(super) fn check_empty(view: &dyn ReadView, identity: ResourceIdentity) -> Result<()> {
        if view
            .get(ColumnFamily::Default, &base_key(&identity))?
            .is_some()
        {
            return Err(invalid("resource has an unexpected legacy base"));
        }
        let prefix = pin_prefix(&identity);
        let mut end = prefix.clone();
        // Increment the complete prefix, including all-FF instance IDs.
        while end.last() == Some(&u8::MAX) {
            end.pop();
        }
        *end.last_mut().unwrap() += 1;
        if view
            .iter(ColumnFamily::Default, &prefix, &end)?
            .next()
            .transpose()?
            .is_some()
        {
            return Err(invalid("resource has orphan point rows"));
        }
        Ok(())
    }

    pub(super) fn load(
        view: &dyn ReadView,
        identity: ResourceIdentity,
        bytes: &[u8],
        rows_enabled: bool,
    ) -> Result<Self> {
        if !bytes.starts_with(RESOURCE_MAGIC) {
            let base = RetentionRecord::decode(bytes, identity).map_err(pin_error)?;
            Self::check_empty(view, identity)?;
            return Ok(Self {
                identity,
                revision: base.revision(),
                retired: base.is_retired(),
                base: Some(base),
                base_digest: Some(RootDigest::sha256(bytes)),
                legacy_bytes: Some(bytes.to_vec()),
            });
        }
        if !rows_enabled {
            return Err(invalid("point rows require the version 2 ledger header"));
        }
        let mut r = Reader(checked(bytes, RESOURCE_MAGIC, MAX_RESOURCE_BYTES)?);
        read_identity(&mut r, identity)?;
        let revision = r.uint()?;
        let retired = r.boolean()?;
        let base_digest = if r.boolean()? {
            Some(RootDigest::from_bytes(r.array()?))
        } else {
            None
        };
        if !r.0.is_empty() {
            return Err(invalid("resource header trailing data"));
        }
        let base_bytes = view.get(ColumnFamily::Default, &base_key(&identity))?;
        let base = match (base_digest, base_bytes) {
            (None, None) if !retired => None,
            (Some(digest), Some(bytes)) if RootDigest::sha256(&bytes) == digest => {
                let base = RetentionRecord::decode(&bytes, identity).map_err(pin_error)?;
                if base.revision() > revision || base.is_retired() != retired {
                    return Err(invalid("legacy base revision/retirement disagrees"));
                }
                Some(base)
            }
            _ => return Err(invalid("resource legacy base missing or inconsistent")),
        };
        Ok(Self {
            identity,
            revision,
            retired,
            base,
            base_digest,
            legacy_bytes: None,
        })
    }

    pub(super) fn is_retired(&self) -> bool {
        self.retired
    }

    pub(super) fn owner(&self, view: &dyn ReadView, id: OwnerId) -> Result<Option<OwnerSlot>> {
        let Some(bytes) = view.get(ColumnFamily::Default, &pin_key(&self.identity, id))? else {
            return Ok(self.base.as_ref().and_then(|base| base.owner(id)));
        };
        let mut r = Reader(checked(&bytes, PIN_MAGIC, PIN_BYTES)?);
        read_identity(&mut r, self.identity)?;
        if r.array::<16>()? != *id.as_bytes() {
            return Err(invalid("resource pin owner identity differs"));
        }
        let generation =
            NonZeroU64::new(r.uint()?).ok_or_else(|| invalid("zero pin generation"))?;
        let phase = match r.byte()? {
            1 => PinPhase::Held,
            2 => PinPhase::Published,
            3 => PinPhase::Quiesced,
            4 => PinPhase::Released,
            _ => return Err(invalid("unknown resource pin phase")),
        };
        let revision = r.uint()?;
        if !r.0.is_empty()
            || revision > self.revision
            || generation.get() > revision
            || self
                .base
                .as_ref()
                .is_some_and(|base| revision <= base.revision())
            || (self.retired && phase != PinPhase::Released)
        {
            return Err(invalid("resource pin revision/phase disagrees"));
        }
        Ok(Some(OwnerSlot { generation, phase }))
    }

    pub(super) fn encode(&self) -> Vec<u8> {
        let mut b = RESOURCE_MAGIC.to_vec();
        identity_bytes(&mut b, self.identity);
        uint(&mut b, self.revision);
        b.push(u8::from(self.retired));
        b.push(u8::from(self.base_digest.is_some()));
        if let Some(digest) = self.base_digest {
            b.extend_from_slice(digest.as_bytes());
        }
        seal(b)
    }

    /// Register can upgrade all 64 legacy resources atomically without
    /// materializing their 262144 possible pins as separate writes.
    pub(super) fn normalize(&self, writes: &mut BTreeMap<Vec<u8>, Vec<u8>>) {
        if let Some(bytes) = &self.legacy_bytes {
            writes.insert(base_key(&self.identity), bytes.clone());
            writes.insert(resource_key(&self.identity), self.encode());
        }
    }

    /// The enclosing planner already checked the complete owner binding,
    /// allowed transition and source/successor. Compare each exact forward
    /// link again before emitting the indivisible cross-resource write batch.
    pub(super) fn update(
        &mut self,
        view: &dyn ReadView,
        id: OwnerId,
        previous: Option<OwnerSlot>,
        next: OwnerSlot,
        writes: &mut BTreeMap<Vec<u8>, Vec<u8>>,
    ) -> Result<()> {
        if self.owner(view, id)? != previous {
            return Err(invalid("resource pin changed or disagrees with owner"));
        }
        if self.retired {
            return Err(conflict("resource instance permanently retired"));
        }
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("resource revision exhausted"))?;
        if next.generation.get() > self.revision {
            return Err(invalid("resource pin generation exceeds revision"));
        }
        self.normalize(writes);
        let mut bytes = PIN_MAGIC.to_vec();
        identity_bytes(&mut bytes, self.identity);
        bytes.extend_from_slice(id.as_bytes());
        uint(&mut bytes, next.generation.get());
        bytes.push(next.phase as u8);
        uint(&mut bytes, self.revision);
        writes.insert(pin_key(&self.identity, id), seal(bytes));
        writes.insert(resource_key(&self.identity), self.encode());
        Ok(())
    }
}

#[cfg(test)]
mod tests;
