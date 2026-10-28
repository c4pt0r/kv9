use super::*;
const PROGRESS_MAGIC: &[u8; 8] = b"KV9DIP01";
const ENTRY_MAGIC: &[u8; 8] = b"KV9DIE01";
const MAX_ENTRY_BYTES: usize = 128 * 1024;
impl Progress {
    pub(super) fn encode(&self, root: RootDigest) -> Result<Vec<u8>> {
        let mut b = PROGRESS_MAGIC.to_vec();
        b.extend_from_slice(root.as_bytes());
        b.extend_from_slice(&self.source.encode()?);
        for n in [
            self.sequence,
            self.recorded_at,
            self.generation,
            self.timeline_sequence,
            self.native_at,
            self.processed,
            self.retained,
            self.total,
        ] {
            uint(&mut b, n);
        }
        b.extend_from_slice(&self.gaps.to_be_bytes());
        b.extend_from_slice(self.last_request.as_bytes());
        Ok(seal(b))
    }
    pub(super) fn decode(p: &Planner<'_>, source: Source, b: &[u8]) -> Result<Self> {
        let mut r = Reader(checked(b, PROGRESS_MAGIC, 256)?);
        if r.array::<32>()? != *p.root.as_bytes() || Source::decode(&mut r)? != source {
            return Err(invalid("configuration interval progress scope"));
        }
        let out = Self {
            source,
            sequence: r.uint()?,
            recorded_at: r.uint()?,
            generation: r.uint()?,
            timeline_sequence: r.uint()?,
            native_at: r.uint()?,
            processed: r.uint()?,
            retained: r.uint()?,
            total: r.uint()?,
            gaps: u16::from_be_bytes(r.array()?),
            last_request: RootDigest::from_bytes(r.array()?),
        };
        if !r.0.is_empty()
            || out.generation == 0
            || out.generation > out.sequence
            || out.sequence > out.recorded_at
            || out.recorded_at > p.revision
            || out.timeline_sequence == 0
            || out.native_at > out.recorded_at
            || out.processed > out.retained
            || out.retained > out.total
            || out.gaps & !511 != 0
            || out.last_request.as_bytes() == &[0; 32]
            || out.encode(p.root)? != b
        {
            return Err(invalid("configuration interval progress encoding"));
        }
        Ok(out)
    }
}
impl Entry {
    pub(super) fn encode(&self, root: RootDigest, source: Source) -> Result<Vec<u8>> {
        let mut b = ENTRY_MAGIC.to_vec();
        b.extend_from_slice(root.as_bytes());
        b.extend_from_slice(&source.encode()?);
        for n in [self.ordinal, self.generation, self.recorded_at] {
            uint(&mut b, n);
        }
        b.extend_from_slice(self.source_digest.as_bytes());
        b.extend_from_slice(&self.gaps.to_be_bytes());
        b.push(u8::from(self.members.is_some()));
        if let Some(members) = &self.members {
            if members.len() > projection::MAX_MEMBERS {
                return Err(invalid("configuration interval member count"));
            }
            b.extend_from_slice(&(members.len() as u16).to_be_bytes());
            for m in members {
                uint(&mut b, m.node.0);
                b.push(m.roles);
                b.push(u8::from(m.store.is_some()));
                if let Some(s) = m.store {
                    b.extend_from_slice(s.as_bytes());
                }
            }
        }
        let b = seal(b);
        if b.len() > MAX_ENTRY_BYTES {
            return Err(invalid("configuration interval entry size"));
        }
        Ok(b)
    }
    pub(super) fn decode(p: &Planner<'_>, source: Source, ordinal: u64, b: &[u8]) -> Result<Self> {
        let mut r = Reader(checked(b, ENTRY_MAGIC, MAX_ENTRY_BYTES)?);
        if r.array::<32>()? != *p.root.as_bytes()
            || Source::decode(&mut r)? != source
            || r.uint()? != ordinal
        {
            return Err(invalid("configuration interval entry scope"));
        }
        let generation = r.uint()?;
        let recorded_at = r.uint()?;
        let source_digest = RootDigest::from_bytes(r.array()?);
        let gaps = u16::from_be_bytes(r.array()?);
        let members = if r.boolean()? {
            let count = u16::from_be_bytes(r.array()?) as usize;
            if count > projection::MAX_MEMBERS || count > r.0.len() / 10 {
                return Err(invalid("configuration interval member framing"));
            }
            let mut rows = Vec::with_capacity(count);
            for _ in 0..count {
                rows.push(Member {
                    node: kv9_common::NodeId(r.uint()?),
                    roles: r.byte()?,
                    store: if r.boolean()? {
                        Some(kv9_common::StoreIncarnation::from_bytes(r.array()?))
                    } else {
                        None
                    },
                });
            }
            Some(rows)
        } else {
            None
        };
        let out = Self {
            ordinal,
            generation,
            recorded_at,
            source_digest,
            gaps,
            members,
        };
        if !r.0.is_empty()
            || ordinal == 0
            || generation == 0
            || generation > recorded_at
            || recorded_at > p.revision
            || out.encode(p.root, source)? != b
        {
            return Err(invalid("configuration interval entry encoding"));
        }
        let record = actual(p, source, ordinal)?;
        if RootDigest::sha256(&record.encode()?) != source_digest {
            return Err(invalid("configuration interval actual record changed"));
        }
        projection::validate_members(&record, &out)?;
        Ok(out)
    }
}
