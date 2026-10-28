//! Bounded, restartable census of every registered per-process Reader generation.
//! Requires a negative refusal or a closed metadata read gate, preserves missing/legacy/active generations, and
//! grants no Pending/Version release or complete external-reader qualification.
use super::*;
use metadata_read_gate::ReadFence;
use reader_lifecycle::{Origin, ReaderGeneration};
use std::collections::BTreeSet;

const MAGIC: &[u8; 8] = b"KV9RCL01";
const STAMP_MAGIC: &[u8; 8] = b"KV9RCM01";
const MAX_BYTES: usize = 256;
fn progress_key(pending: OwnerId) -> Vec<u8> {
    key(26, pending.as_bytes())
}
fn stamp_key(pending: OwnerId) -> Vec<u8> {
    key(27, pending.as_bytes())
}
fn increment(value: &mut u64, n: u64) -> Result<()> {
    *value = value
        .checked_add(n)
        .ok_or_else(|| invalid("closure counter exhausted"))?;
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counts {
    pub owners: u64,
    pub generations: u64,
    pub drained: u64,
    pub unentered: u64,
    pub legacy: u64,
    pub missing: u64,
    pub active: u64,
}
impl Counts {
    pub(super) fn numbers(&self) -> [u64; 7] {
        [
            self.owners,
            self.generations,
            self.drained,
            self.unentered,
            self.legacy,
            self.missing,
            self.active,
        ]
    }
    pub(super) fn add(&mut self, record: Option<ReaderGeneration>, refused_at: u64) -> Result<()> {
        increment(&mut self.generations, 1)?;
        match record {
            None => increment(&mut self.missing, 1)?,
            Some(record) => match record.origin {
                Origin::Legacy => increment(&mut self.legacy, 1)?,
                Origin::Unentered => increment(&mut self.unentered, 1)?,
                Origin::Entered => {
                    if record.entered_at >= refused_at || record.published_at >= refused_at {
                        return Err(invalid("Reader admission is not before refusal"));
                    }
                    if record.finished_at == 0 {
                        increment(&mut self.active, 1)?;
                    } else {
                        increment(&mut self.drained, 1)?;
                    }
                }
            },
        }
        Ok(())
    }
    pub(super) fn valid(&self) -> bool {
        [
            self.drained,
            self.unentered,
            self.legacy,
            self.missing,
            self.active,
        ]
        .into_iter()
        .try_fold(0_u64, |a, b| a.checked_add(b))
            == Some(self.generations)
            && self.owners <= self.generations
            && (self.owners == 0) == (self.generations == 0)
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cursor {
    pub sequence: u64,
    pub source_revision: u64,
    /// Read-admission fence revision; the persisted/public name predates the
    /// positive metadata gate and remains compatible with negative refusals.
    pub refusal_revision: u64,
    pub counts: Counts,
    pub done: bool,
    after: Option<OwnerId>,
    current: Option<OwnerToken>,
    after_generation: Option<NonZeroU64>,
}
impl Cursor {
    pub fn request(&self, pending: OwnerId) -> LedgerRequest {
        LedgerRequest::AdvanceReaderClosure {
            pending: OwnerToken {
                owner: pending,
                generation: NonZeroU64::new(1).unwrap(),
            },
            expected: self.sequence,
        }
    }
    fn encode(&self, root: RootDigest, gate: &ReadFence) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(root.as_bytes());
        b.extend_from_slice(gate.pending.as_bytes());
        for n in [
            gate.closed_at,
            self.refusal_revision,
            self.sequence,
            self.source_revision,
        ] {
            uint(&mut b, n);
        }
        b.push(u8::from(self.done));
        b.push(u8::from(self.after.is_some()));
        if let Some(id) = self.after {
            b.extend_from_slice(id.as_bytes());
        }
        b.push(u8::from(self.current.is_some()));
        if let Some(t) = self.current {
            b.extend_from_slice(t.owner.as_bytes());
            uint(&mut b, t.generation.get());
            uint(&mut b, self.after_generation.map_or(0, NonZeroU64::get));
        }
        for n in self.counts.numbers() {
            uint(&mut b, n);
        }
        seal(b)
    }
}
fn stamp(p: &Planner<'_>, pending: OwnerId) -> Result<Option<u64>> {
    let Some(bytes) = p.view.get(ColumnFamily::Default, &stamp_key(pending))? else {
        if p.view
            .get(ColumnFamily::Default, &progress_key(pending))?
            .is_some()
        {
            return Err(invalid("closure progress lost its change stamp"));
        }
        return Ok(None);
    };
    if !p
        .view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .is_some_and(|h| reader_closure_format(&h))
    {
        return Err(invalid("Reader closure stamp requires version 11 ledger"));
    }
    let mut r = Reader(checked(&bytes, STAMP_MAGIC, 96)?);
    if r.array::<32>()? != *p.root.as_bytes() || r.array::<16>()? != *pending.as_bytes() {
        return Err(invalid("closure stamp identity differs"));
    }
    let revision = r.uint()?;
    if !r.0.is_empty() || revision == 0 || revision > p.revision {
        return Err(invalid("closure stamp revision differs"));
    }
    Ok(Some(revision))
}
fn write_stamp(p: &mut Planner<'_>, pending: OwnerId) -> Result<u64> {
    // Missing stamps cannot repair an existing aggregate.
    stamp(p, pending)?;
    let revision = p
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid("closure revision exhausted"))?;
    let mut b = STAMP_MAGIC.to_vec();
    b.extend_from_slice(p.root.as_bytes());
    b.extend_from_slice(pending.as_bytes());
    uint(&mut b, revision);
    p.writes.insert(stamp_key(pending), seal(b));
    Ok(revision)
}
pub(super) fn check_old_format(view: &dyn ReadView) -> Result<()> {
    if view
        .iter(ColumnFamily::Default, &key(26, &[]), &key(28, &[]))?
        .next()
        .transpose()?
        .is_some()
    {
        return Err(invalid(
            "Reader closure records under an older ledger header",
        ));
    }
    Ok(())
}
/// Only the current bounded batch is examined. Lifecycle writes, legacy
/// transfers and reverse-index backfill all invalidate the same aggregate.
pub(super) fn mark_changes(p: &mut Planner<'_>) -> Result<()> {
    let mut changed = BTreeSet::new();
    for (k, v) in &p.writes {
        let Some(suffix) = k.strip_prefix(PREFIX) else {
            continue;
        };
        let (&tag, id) = suffix
            .split_first()
            .ok_or_else(|| invalid("Reader closure write key"))?;
        let pending = match tag {
            3 | 5 | 25 => {
                let owner_id = OwnerId::new(
                    id.get(..16)
                        .ok_or_else(|| invalid("Reader closure owner key"))?
                        .try_into()
                        .unwrap(),
                )
                .map_err(pin_error)?;
                let stored;
                let bytes = if tag == 25 {
                    stored = match p.writes.get(&owner_key(owner_id)) {
                        Some(b) => b.clone(),
                        None => p
                            .view
                            .get(ColumnFamily::Default, &owner_key(owner_id))?
                            .ok_or_else(|| invalid("Reader generation without owner"))?,
                    };
                    &stored
                } else {
                    v
                };
                let owner = decode_owner(bytes, p.root, owner_id)?;
                if owner.binding.descriptor.kind != OwnerKind::Reader {
                    continue;
                }
                checkpoint_intent::owner_id(
                    p.root,
                    &owner.binding.descriptor.operation,
                    b"kv9-checkpoint-pending-v1",
                )?
            }
            23 | 20 | 32 => {
                let pending = OwnerId::new(
                    id.get(..16)
                        .ok_or_else(|| invalid("Reader closure fact key"))?
                        .try_into()
                        .unwrap(),
                )
                .map_err(pin_error)?;
                // No aggregate can precede refusal. Avoid adding stamps to
                // attempts with no Readers solely because their gate closes.
                if matches!(tag, 20 | 32) && stamp(p, pending)?.is_none() {
                    continue;
                }
                pending
            }
            _ => continue,
        };
        if p.writes.contains_key(&key(9, pending.as_bytes()))
            || p.view
                .get(ColumnFamily::Default, &key(9, pending.as_bytes()))?
                .is_some()
        {
            changed.insert(pending);
        }
    }
    for pending in changed {
        write_stamp(p, pending)?;
    }
    Ok(())
}
fn cursor(p: &Planner<'_>, gate: &ReadFence, refused_at: u64) -> Result<Cursor> {
    let Some(bytes) = p
        .view
        .get(ColumnFamily::Default, &progress_key(gate.pending))?
    else {
        return Ok(Cursor {
            refusal_revision: refused_at,
            ..Cursor::default()
        });
    };
    let mut r = Reader(checked(&bytes, MAGIC, MAX_BYTES)?);
    if r.array::<32>()? != *p.root.as_bytes()
        || r.array::<16>()? != *gate.pending.as_bytes()
        || r.uint()? != gate.closed_at
        || r.uint()? != refused_at
    {
        return Err(invalid("Reader closure identity, gate or refusal differs"));
    }
    let sequence = r.uint()?;
    let source_revision = r.uint()?;
    let done = r.boolean()?;
    let after = if r.boolean()? {
        Some(OwnerId::new(r.array()?).map_err(pin_error)?)
    } else {
        None
    };
    let (current, after_generation) = if r.boolean()? {
        (
            Some(OwnerToken {
                owner: OwnerId::new(r.array()?).map_err(pin_error)?,
                generation: NonZeroU64::new(r.uint()?)
                    .ok_or_else(|| invalid("closure generation zero"))?,
            }),
            NonZeroU64::new(r.uint()?),
        )
    } else {
        (None, None)
    };
    let counts = Counts {
        owners: r.uint()?,
        generations: r.uint()?,
        drained: r.uint()?,
        unentered: r.uint()?,
        legacy: r.uint()?,
        missing: r.uint()?,
        active: r.uint()?,
    };
    let c = Cursor {
        sequence,
        source_revision,
        refusal_revision: refused_at,
        done,
        after,
        current,
        after_generation,
        counts,
    };
    if !r.0.is_empty()
        || sequence == 0
        || sequence > p.revision
        || source_revision == 0
        || source_revision > p.revision
        || source_revision < refused_at
        || !c.counts.valid()
        || (done && current.is_some())
        || current.is_some_and(|t| {
            after.is_some_and(|a| t.owner <= a)
                || after_generation.is_none_or(|g| g >= t.generation)
        })
        || c.encode(p.root, gate) != bytes
    {
        return Err(invalid("closure progress bounds or encoding"));
    }
    Ok(c)
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Work {
    Open,
    WaitingForCatalog,
    Advance { cursor: Cursor, restart: bool },
    Complete(Cursor),
}
impl Work {
    /// A fresh, complete catalog observation, not a release capability.
    pub fn registered_generations_closed(&self) -> bool {
        matches!(self, Self::Complete(c) if c.counts.drained.checked_add(c.counts.unentered) == Some(c.counts.generations))
    }
}
fn context<'a>(
    view: &'a dyn ReadView,
    root: &RootDescriptor,
    pending: OwnerId,
) -> Result<(Planner<'a>, CheckpointIntent)> {
    let intent = checkpoint_intent::checkpoint_intent(view, root, pending)?
        .ok_or_else(|| invalid("Reader closure intent missing"))?;
    let h = view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .ok_or_else(|| invalid("Reader closure header missing"))?;
    let (revision, rows_enabled) = read_header(&h, root.digest())?;
    Ok((
        Planner {
            view,
            root: root.digest(),
            revision,
            rows_enabled,
            writes: BTreeMap::new(),
        },
        intent,
    ))
}
pub fn observe(view: &dyn ReadView, root: &RootDescriptor, pending: OwnerId) -> Result<Work> {
    let Some(refusal) = metadata_read_gate::fence(view, root, pending)? else {
        return Ok(Work::Open);
    };
    let (p, _) = context(view, root, pending)?;
    if !matches!(
        reader_index::observe(view, root)?,
        reader_index::Work::Complete
    ) {
        return Ok(Work::WaitingForCatalog);
    }
    let source = stamp(&p, pending)?;
    if !refusal.shared_image && source.is_some_and(|s| s < refusal.recorded_at) {
        return Err(invalid("Reader closure stamp precedes refusal"));
    }
    let source = source.map(|s| s.max(refusal.recorded_at));
    let c = cursor(&p, &refusal, refusal.recorded_at)?;
    if source.is_some_and(|s| s < c.source_revision) {
        return Err(invalid("Reader closure change stamp regressed"));
    }
    let restart = c.sequence != 0 && source != Some(c.source_revision);
    Ok(if c.done && !restart {
        Work::Complete(c)
    } else {
        Work::Advance { cursor: c, restart }
    })
}
pub(super) fn advance(
    p: &mut Planner<'_>,
    root: &RootDescriptor,
    pending: OwnerToken,
    expected: u64,
) -> Result<()> {
    if pending.generation.get() != 1 {
        return Err(conflict("closure Pending generation differs"));
    }
    let work = observe(p.view, root, pending.owner)?;
    let (mut c, restart) = match work {
        Work::Advance { cursor, restart } => (cursor, restart),
        Work::Complete(c) => (c, false),
        _ => {
            return Err(conflict(
                "Reader closure requires refusal and complete Reader census",
            ))
        }
    };
    if expected.checked_add(1) == Some(c.sequence) {
        return Ok(());
    }
    if expected != c.sequence {
        return Err(conflict("closure sequence differs"));
    }
    if c.done && !restart {
        return Ok(());
    }
    let sequence = c
        .sequence
        .checked_add(1)
        .ok_or_else(|| invalid("closure sequence exhausted"))?;
    let source = match stamp(p, pending.owner)? {
        Some(s) => s.max(c.refusal_revision),
        None => write_stamp(p, pending.owner)?,
    };
    if restart {
        c = Cursor {
            refusal_revision: c.refusal_revision,
            ..Cursor::default()
        };
    }
    c.source_revision = source;
    c.sequence = sequence;
    let (_, intent) = context(p.view, root, pending.owner)?;
    let operation = intent.binding(OwnerKind::Pending)?.descriptor.operation;
    if c.current.is_none() {
        let page = reader_index::readers(p.view, root, operation, c.after)?;
        if let Some(owner) = page.owners.first() {
            c.current = Some(owner.binding.token());
            increment(&mut c.counts.owners, 1)?;
        } else {
            c.done = true;
        }
    }
    if let Some(through) = c.current {
        let page = reader_lifecycle::history(p.view, root, operation, through, c.after_generation)?;
        if page.current != through {
            return Err(conflict(
                "closure source stamp did not track generation change",
            ));
        }
        for generation in page.generations {
            c.counts.add(generation.record, c.refusal_revision)?;
        }
        c.after_generation = page.next;
        if page.next.is_none() {
            c.current = None;
            c.after = Some(through.owner);
        }
    }
    let gate = metadata_read_gate::fence(p.view, root, pending.owner)?
        .ok_or_else(|| invalid("Reader refusal disappeared"))?;
    p.writes
        .insert(progress_key(pending.owner), c.encode(p.root, &gate));
    Ok(())
}

#[cfg(test)]
mod tests;
