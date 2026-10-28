//! Bounded, restartable census of every registered Build generation. This
//! composes origin/gate/catalog facts, not deployment qualification or GC.
use super::*;
use checkpoint_admission::{enrollment, Admission};
use std::collections::BTreeSet;

const MAGIC: &[u8; 8] = b"KV9CCL01";
const STAMP_MAGIC: &[u8; 8] = b"KV9CCM01";
const MAX_BYTES: usize = 256;
fn progress_key(pending: OwnerId) -> Vec<u8> {
    key(18, pending.as_bytes())
}
fn stamp_key(pending: OwnerId) -> Vec<u8> {
    key(19, pending.as_bytes())
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
    pub zero_put: u64,
    pub missing_origin: u64,
    pub active: u64,
    pub missing_terminal: u64,
    pub nonzero_put: u64,
    pub unresolved_puts: u64,
}
impl Counts {
    pub(super) fn numbers(&self) -> [u64; 8] {
        [
            self.owners,
            self.generations,
            self.zero_put,
            self.missing_origin,
            self.active,
            self.missing_terminal,
            self.nonzero_put,
            self.unresolved_puts,
        ]
    }
    pub(super) fn add(&mut self, state: enrollment::GenerationState) -> Result<()> {
        use enrollment::GenerationState::*;
        increment(&mut self.generations, 1)?;
        match state {
            MissingOrigin => increment(&mut self.missing_origin, 1)?,
            Active(_) => increment(&mut self.active, 1)?,
            MissingTerminal => increment(&mut self.missing_terminal, 1)?,
            Cancelled
            | RetiredUnpublished
            | Drained {
                issued: 0,
                resolved: 0,
            } => increment(&mut self.zero_put, 1)?,
            Drained { issued, resolved } => {
                increment(&mut self.nonzero_put, 1)?;
                increment(
                    &mut self.unresolved_puts,
                    issued
                        .checked_sub(resolved)
                        .ok_or_else(|| invalid("closure PUT counts"))?,
                )?;
            }
        }
        Ok(())
    }
    pub(super) fn valid(&self) -> bool {
        [
            self.zero_put,
            self.missing_origin,
            self.active,
            self.missing_terminal,
            self.nonzero_put,
        ]
        .into_iter()
        .try_fold(0_u64, |a, b| a.checked_add(b))
            == Some(self.generations)
            && self.owners <= self.generations
            && (self.owners == 0) == (self.generations == 0)
            && (self.nonzero_put != 0 || self.unresolved_puts == 0)
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cursor {
    pub sequence: u64,
    pub source_revision: u64,
    pub birth_revision: u64,
    pub counts: Counts,
    pub done: bool,
    after: Option<OwnerId>,
    current: Option<OwnerToken>,
    after_generation: Option<NonZeroU64>,
}
impl Cursor {
    pub fn request(&self, pending: OwnerId) -> LedgerRequest {
        LedgerRequest::AdvanceCheckpointClosure {
            pending: OwnerToken {
                owner: pending,
                generation: NonZeroU64::new(1).unwrap(),
            },
            expected: self.sequence,
        }
    }
    fn encode(&self, root: RootDigest, gate: &Admission) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(root.as_bytes());
        b.extend_from_slice(gate.pending.as_bytes());
        for n in [
            gate.closed_at,
            self.birth_revision,
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
        .is_some_and(|h| migration_format(&h) || h.starts_with(CLOSURE_HEADER_MAGIC))
    {
        return Err(invalid("closure stamp requires version 7 ledger"));
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
pub(super) fn check_old_format(view: &dyn ReadView, header: &[u8]) -> Result<()> {
    if view
        .iter(ColumnFamily::Default, &key(18, &[]), &key(22, &[]))?
        .next()
        .transpose()?
        .is_some()
    {
        return Err(invalid("closure records under an older ledger header"));
    }
    if !header.starts_with(ATOMIC_HEADER_MAGIC) {
        checkpoint_begin::check_old_format(view, header)?;
    }
    Ok(())
}
/// Central invalidation at the batch boundary covers generic Build changes too.
/// Only this command's bounded writes are examined; progress itself is excluded.
pub(super) fn mark_changes(p: &mut Planner<'_>) -> Result<()> {
    let mut changed = BTreeSet::new();
    for (k, v) in &p.writes {
        let Some(suffix) = k.strip_prefix(PREFIX) else {
            continue;
        };
        let (&tag, id) = suffix
            .split_first()
            .ok_or_else(|| invalid("closure write key"))?;
        let pending = match tag {
            3 | 5 => {
                let id = OwnerId::new(id.try_into().map_err(|_| invalid("closure owner key"))?)
                    .map_err(pin_error)?;
                let owner = decode_owner(v, p.root, id)?;
                if owner.binding.descriptor.kind != OwnerKind::Build {
                    continue;
                }
                checkpoint_intent::owner_id(
                    p.root,
                    &owner.binding.descriptor.operation,
                    b"kv9-checkpoint-pending-v1",
                )?
            }
            8 | 9 | 11 | 13 | 14 | 16 | 17 => OwnerId::new(
                id.get(..16)
                    .ok_or_else(|| invalid("closure fact key"))?
                    .try_into()
                    .unwrap(),
            )
            .map_err(pin_error)?,
            _ => continue,
        };
        // A generic Build can precede its intent. Begin creates the first stamp
        // with its gate; the global census later finds those earlier owners.
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
fn cursor(p: &Planner<'_>, gate: &Admission, birth: u64) -> Result<Cursor> {
    let Some(bytes) = p
        .view
        .get(ColumnFamily::Default, &progress_key(gate.pending))?
    else {
        return Ok(Cursor {
            birth_revision: birth,
            ..Cursor::default()
        });
    };
    let mut r = Reader(checked(&bytes, MAGIC, MAX_BYTES)?);
    if r.array::<32>()? != *p.root.as_bytes()
        || r.array::<16>()? != *gate.pending.as_bytes()
        || r.uint()? != gate.closed_at
        || r.uint()? != birth
    {
        return Err(invalid("closure identity, gate or birth differs"));
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
        zero_put: r.uint()?,
        missing_origin: r.uint()?,
        active: r.uint()?,
        missing_terminal: r.uint()?,
        nonzero_put: r.uint()?,
        unresolved_puts: r.uint()?,
    };
    let c = Cursor {
        sequence,
        source_revision,
        birth_revision: birth,
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
        || source_revision < gate.closed_at
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
    pub fn registered_zero_put_closed(&self) -> bool {
        matches!(self, Self::Complete(c) if c.birth_revision != 0 && c.counts.zero_put == c.counts.generations)
    }
}
fn context<'a>(
    view: &'a dyn ReadView,
    root: &RootDescriptor,
    pending: OwnerId,
) -> Result<(Planner<'a>, Admission, u64)> {
    let intent = checkpoint_intent::checkpoint_intent(view, root, pending)?
        .ok_or_else(|| invalid("closure intent missing"))?;
    let header = view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .ok_or_else(|| invalid("closure header missing"))?;
    let (revision, rows_enabled) = read_header(&header, root.digest())?;
    let p = Planner {
        view,
        root: root.digest(),
        revision,
        rows_enabled,
        writes: BTreeMap::new(),
    };
    let gate = checkpoint_admission::load(&p, &intent)?;
    let birth = checkpoint_begin::atomic_birth(view, root, pending)?.map_or(0, |b| b.created_at);
    Ok((p, gate, birth))
}
pub fn observe(view: &dyn ReadView, root: &RootDescriptor, pending: OwnerId) -> Result<Work> {
    let (p, gate, birth) = context(view, root, pending)?;
    if !gate.is_closed() {
        return Ok(Work::Open);
    }
    if !matches!(
        build_index::observe(view, root)?,
        build_index::Work::Complete
    ) {
        return Ok(Work::WaitingForCatalog);
    }
    let source = stamp(&p, pending)?;
    let c = cursor(&p, &gate, birth)?;
    if source.is_some_and(|s| s < c.source_revision) {
        return Err(invalid("closure change stamp regressed"));
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
                "closure requires Closed gate and complete Build census",
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
        Some(s) => s,
        None => write_stamp(p, pending.owner)?,
    };
    if restart {
        c = Cursor {
            birth_revision: c.birth_revision,
            ..Cursor::default()
        };
    }
    c.source_revision = source;
    c.sequence = sequence;
    if c.current.is_none() {
        let page = build_index::builds(p.view, root, pending.owner, c.after)?;
        if let Some(owner) = page.owners.first() {
            c.current = Some(owner.binding.token());
            increment(&mut c.counts.owners, 1)?;
        } else {
            c.done = true;
        }
    }
    if let Some(through) = c.current {
        let page = enrollment::history(p.view, root, pending.owner, through, c.after_generation)?;
        if page.current != through {
            return Err(conflict(
                "closure source stamp did not track generation change",
            ));
        }
        for generation in page.generations {
            c.counts.add(generation.state)?;
        }
        c.after_generation = page.next;
        if page.next.is_none() {
            c.current = None;
            c.after = Some(through.owner);
        }
    }
    let (_, gate, _) = context(p.view, root, pending.owner)?;
    p.writes
        .insert(progress_key(pending.owner), c.encode(p.root, &gate));
    Ok(())
}

#[cfg(test)]
mod tests;
