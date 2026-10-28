//! Bounded image-wide Publisher generation census across all source operations.
//! Requires an immutable metadata image write gate and complete image directory.
//! Active, missing and legacy generations block closure; no release authority.
use super::*;
pub use checkpoint_closure::Counts;
use metadata_image_write_gate::Fence;
use std::collections::BTreeMap;

const MAGIC: &[u8; 8] = b"KV9IPL01";
const STAMP_MAGIC: &[u8; 8] = b"KV9IPM01";
const MAX_BYTES: usize = 280;
fn progress_key(image: [u8; 32]) -> Vec<u8> {
    key(47, &image)
}
fn stamp_key(image: [u8; 32]) -> Vec<u8> {
    key(46, &image)
}
fn increment(value: &mut u64, n: u64) -> Result<()> {
    *value = value
        .checked_add(n)
        .ok_or_else(|| invalid("image closure counter exhausted"))?;
    Ok(())
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cursor {
    pub sequence: u64,
    pub source_revision: u64,
    /// Immutable exact-image upload-admission fence revision.
    pub fence_revision: u64,
    pub counts: Counts,
    pub done: bool,
    after: Option<OwnerId>,
    current: Option<OwnerToken>,
    after_generation: Option<NonZeroU64>,
}
impl Cursor {
    pub fn request(&self, pending: OwnerId) -> LedgerRequest {
        LedgerRequest::AdvanceImagePublisherClosure {
            pending: OwnerToken {
                owner: pending,
                generation: NonZeroU64::new(1).unwrap(),
            },
            expected: self.sequence,
        }
    }
    fn encode(&self, root: RootDigest, gate: &Fence) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(root.as_bytes());
        b.extend_from_slice(&gate.image);
        for n in [
            gate.closed_at,
            self.fence_revision,
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
#[derive(Clone, Copy, Default)]
struct Stamp {
    revision: u64,
    owners: u64,
}
fn stamp(p: &Planner<'_>, image: [u8; 32]) -> Result<Option<Stamp>> {
    let Some(bytes) = p.view.get(ColumnFamily::Default, &stamp_key(image))? else {
        if p.view
            .get(ColumnFamily::Default, &progress_key(image))?
            .is_some()
        {
            return Err(invalid("closure progress lost its change stamp"));
        }
        // Every metadata image index insertion commits its count atomically.
        // Losing the count before the first census cannot turn surviving links
        // into an untracked legacy set or let a later mutation rebuild it.
        let mut group = kv9_common::META_REGION_0.0.to_be_bytes().to_vec();
        group.extend_from_slice(&image);
        let prefix = key(44, &group);
        let mut end = prefix.clone();
        while end.last() == Some(&255) {
            end.pop();
        }
        *end.last_mut().unwrap() += 1;
        if p.view
            .iter(ColumnFamily::Default, &prefix, &end)?
            .next()
            .transpose()?
            .is_some()
        {
            return Err(invalid("image Publisher directory lost its count stamp"));
        }
        return Ok(None);
    };
    if !p
        .view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .is_some_and(|h| image_publisher_format(&h))
    {
        return Err(invalid(
            "image Publisher closure stamp requires version 20 ledger",
        ));
    }
    let mut r = Reader(checked(&bytes, STAMP_MAGIC, 120)?);
    if r.array::<32>()? != *p.root.as_bytes() || r.array::<32>()? != image {
        return Err(invalid("closure stamp identity differs"));
    }
    let revision = r.uint()?;
    let owners = r.uint()?;
    if !r.0.is_empty() || revision == 0 || revision > p.revision {
        return Err(invalid("closure stamp revision differs"));
    }
    Ok(Some(Stamp { revision, owners }))
}
fn write_stamp(p: &mut Planner<'_>, image: [u8; 32], added: u64) -> Result<Stamp> {
    // Missing stamps cannot repair an existing aggregate.
    let previous = stamp(p, image)?.unwrap_or_default();
    let owners = previous
        .owners
        .checked_add(added)
        .ok_or_else(|| invalid("image Publisher owner count exhausted"))?;
    let revision = p
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid("closure revision exhausted"))?;
    let mut b = STAMP_MAGIC.to_vec();
    b.extend_from_slice(p.root.as_bytes());
    b.extend_from_slice(&image);
    uint(&mut b, revision);
    uint(&mut b, owners);
    p.writes.insert(stamp_key(image), seal(b));
    Ok(Stamp { revision, owners })
}
pub(super) fn check_old_format(view: &dyn ReadView) -> Result<()> {
    if view
        .iter(ColumnFamily::Default, &key(46, &[]), &key(48, &[]))?
        .next()
        .transpose()?
        .is_some()
    {
        return Err(invalid(
            "image Publisher closure records under an older ledger header",
        ));
    }
    Ok(())
}
/// Only the current bounded batch is examined. Lifecycle writes, legacy
/// transfers and reverse-index backfill all invalidate the same aggregate.
pub(super) fn mark_changes(p: &mut Planner<'_>) -> Result<()> {
    let mut changed = BTreeMap::<[u8; 32], u64>::new();
    for (k, v) in &p.writes {
        let Some(suffix) = k.strip_prefix(PREFIX) else {
            continue;
        };
        let (&tag, id) = suffix
            .split_first()
            .ok_or_else(|| invalid("image Publisher change key"))?;
        let (region, image, anchor) = match tag {
            3 | 5 | 13 | 16 | 22 => {
                let owner_id = OwnerId::new(
                    id.get(if matches!(tag, 13 | 16) {
                        16..32
                    } else {
                        0..16
                    })
                    .ok_or_else(|| invalid("image Publisher owner key"))?
                    .try_into()
                    .unwrap(),
                )
                .map_err(pin_error)?;
                let stored;
                let bytes = if matches!(tag, 13 | 16 | 22) {
                    stored = match p.writes.get(&owner_key(owner_id)) {
                        Some(b) => b.clone(),
                        None => p
                            .view
                            .get(ColumnFamily::Default, &owner_key(owner_id))?
                            .ok_or_else(|| invalid("image Publisher generation without owner"))?,
                    };
                    &stored
                } else {
                    v
                };
                let owner = decode_owner(bytes, p.root, owner_id)?;
                if !matches!(
                    owner.binding.descriptor.kind,
                    OwnerKind::Build | OwnerKind::Publisher
                ) {
                    continue;
                }
                let d = owner.binding.descriptor;
                (d.region, d.subject, d.subject_is_anchor)
            }
            44 if id.len() == 56 => (
                u64::from_be_bytes(id[..8].try_into().unwrap()),
                id[8..40].try_into().unwrap(),
                false,
            ),
            44 => return Err(invalid("image Publisher directory key length")),
            _ => continue,
        };
        if region == kv9_common::META_REGION_0.0 && !anchor {
            let added = changed.entry(image).or_default();
            if tag == 44 {
                increment(added, 1)?;
            }
        }
    }
    for (image, added) in changed {
        write_stamp(p, image, added)?;
    }
    Ok(())
}
fn cursor(p: &Planner<'_>, gate: &Fence, refused_at: u64) -> Result<Cursor> {
    let Some(bytes) = p
        .view
        .get(ColumnFamily::Default, &progress_key(gate.image))?
    else {
        return Ok(Cursor {
            fence_revision: refused_at,
            ..Cursor::default()
        });
    };
    let mut r = Reader(checked(&bytes, MAGIC, MAX_BYTES)?);
    if r.array::<32>()? != *p.root.as_bytes()
        || r.array::<32>()? != gate.image
        || r.uint()? != gate.closed_at
        || r.uint()? != refused_at
    {
        return Err(invalid(
            "image Publisher closure identity, gate or refusal differs",
        ));
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
        fence_revision: refused_at,
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
    pub fn registered_zero_put_closed(&self) -> bool {
        matches!(self, Self::Complete(c) if c.counts.zero_put == c.counts.generations)
    }
}
fn context<'a>(view: &'a dyn ReadView, root: &RootDescriptor) -> Result<Planner<'a>> {
    crate::checkpoint::inspect_initial_checkpoint_base(view, root)?;
    let h = view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .ok_or_else(|| invalid("image Publisher closure header missing"))?;
    let (revision, rows_enabled) = read_header(&h, root.digest())?;
    Ok(Planner {
        view,
        root: root.digest(),
        revision,
        rows_enabled,
        writes: BTreeMap::new(),
    })
}
pub fn observe(view: &dyn ReadView, root: &RootDescriptor, pending: OwnerId) -> Result<Work> {
    let Some(gate) = metadata_image_write_gate::for_pending(view, root, pending)? else {
        return Ok(Work::Open);
    };
    let p = context(view, root)?;
    if !matches!(
        image_publisher_index::observe(view, root)?,
        image_publisher_index::Work::Complete
    ) {
        return Ok(Work::WaitingForCatalog);
    }
    let current = stamp(&p, gate.image)?;
    let source = current.map(|s| s.revision.max(gate.closed_at));
    let c = cursor(&p, &gate, gate.closed_at)?;
    if source.is_some_and(|s| s < c.source_revision) {
        return Err(invalid("image Publisher closure change stamp regressed"));
    }
    let restart = c.sequence != 0 && source != Some(c.source_revision);
    if c.done && !restart && c.counts.owners != current.map_or(0, |s| s.owners) {
        return Err(invalid(
            "image Publisher census owner count differs from directory",
        ));
    }
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
    let (mut c, restart) =
        match work {
            Work::Advance { cursor, restart } => (cursor, restart),
            Work::Complete(c) => (c, false),
            _ => return Err(conflict(
                "image Publisher closure requires image gate and complete image Publisher census",
            )),
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
    let gate = metadata_image_write_gate::for_pending(p.view, root, pending.owner)?
        .ok_or_else(|| invalid("image gate disappeared"))?;
    let source = match stamp(p, gate.image)? {
        Some(s) => s,
        None => write_stamp(p, gate.image, 0)?,
    };
    if restart {
        c = Cursor {
            fence_revision: c.fence_revision,
            ..Cursor::default()
        };
    }
    c.source_revision = source.revision.max(c.fence_revision);
    c.sequence = sequence;
    if c.current.is_none() {
        let page = image_publisher_index::publishers(
            p.view,
            root,
            kv9_common::META_REGION_0.0,
            gate.image,
            c.after,
        )?;
        if let Some(owner) = page.owners.first() {
            c.current = Some(owner.binding.token());
            increment(&mut c.counts.owners, 1)?;
        } else {
            c.done = true;
        }
    }
    if let Some(through) = c.current {
        let owner = p.exact_owner(through)?;
        let d = &owner.binding.descriptor;
        if !matches!(d.kind, OwnerKind::Build | OwnerKind::Publisher)
            || d.region != kv9_common::META_REGION_0.0
            || d.subject != gate.image
            || d.subject_is_anchor
        {
            return Err(invalid("image Publisher cursor points to another image"));
        }
        let next = generations(
            p,
            root,
            &owner,
            through,
            c.after_generation,
            c.fence_revision,
            &mut c.counts,
        )?;
        c.after_generation = next;
        if next.is_none() {
            c.current = None;
            c.after = Some(through.owner);
        }
    }
    if c.done && c.counts.owners != source.owners {
        return Err(invalid("image Publisher census lost directory entries"));
    }
    p.writes
        .insert(progress_key(gate.image), c.encode(p.root, &gate));
    Ok(())
}

fn generations(
    p: &Planner<'_>,
    root: &RootDescriptor,
    owner: &LedgerOwner,
    through: OwnerToken,
    after: Option<NonZeroU64>,
    fence: u64,
    counts: &mut Counts,
) -> Result<Option<NonZeroU64>> {
    use checkpoint_admission::enrollment::{self, GenerationState};
    if owner.binding.descriptor.kind == OwnerKind::Build {
        if let Some(gate) = checkpoint_admission::retained(p, &owner.binding)? {
            let page = enrollment::history(p.view, root, gate.pending, through, after)?;
            if page.current != through {
                return Err(conflict(
                    "Publisher generation changed without source stamp",
                ));
            }
            for g in page.generations {
                if g.admitted_at.is_some_and(|r| r >= fence) {
                    return Err(invalid("Build admitted after image upload fence"));
                }
                counts.add(g.state)?;
            }
            return Ok(page.next);
        }
    }
    let first = after.map_or(1, |n| n.get() + 1);
    let end = first
        .saturating_add(image_publisher_index::PAGE as u64 - 1)
        .min(through.generation.get());
    for generation in first..=end {
        let record = if owner.binding.descriptor.kind == OwnerKind::Publisher {
            migration_publisher::generation(
                p.view,
                root,
                OwnerToken {
                    owner: through.owner,
                    generation: NonZeroU64::new(generation).unwrap(),
                },
            )?
        } else {
            None
        };
        let state = match record {
            None => GenerationState::MissingOrigin,
            Some(g) => {
                if g.admitted_at >= fence {
                    return Err(invalid("Publisher admitted after image upload fence"));
                }
                if g.finished_at == 0 {
                    GenerationState::Active(PinPhase::Published)
                } else if g.admitted_at == 0 {
                    GenerationState::Cancelled
                } else {
                    GenerationState::Drained {
                        issued: g.issued,
                        resolved: g.resolved,
                    }
                }
            }
        };
        counts.add(state)?;
    }
    Ok((end < through.generation.get()).then(|| NonZeroU64::new(end).unwrap()))
}

#[cfg(test)]
mod tests;
