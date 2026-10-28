//! Per-generation Reader admission and actual local drain observations.
//! Legacy generations remain explicitly unqualified. Neither a terminal record
//! nor admission closure proves complete external Reader/Publisher coverage.
use super::*;

pub(super) const OWNER_MAGIC: &[u8; 8] = b"KV9RDO01";
const MAGIC: &[u8; 8] = b"KV9RDG01";
fn record_key(token: OwnerToken) -> Vec<u8> {
    let mut id = token.owner.as_bytes().to_vec();
    uint(&mut id, token.generation.get());
    key(25, &id)
}
pub(super) fn check_old_format(view: &dyn ReadView) -> Result<()> {
    if view
        .iter(ColumnFamily::Default, &key(25, &[]), &key(26, &[]))?
        .next()
        .transpose()?
        .is_some()
    {
        return Err(invalid("Reader generations under an older ledger header"));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Entered = 1,
    Unentered = 2,
    Legacy = 3,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReaderGeneration {
    pub reader: OwnerToken,
    pub source: OwnerToken,
    pub source_kind: OwnerKind,
    pub origin: Origin,
    pub entered_at: u64,
    pub published_at: u64,
    pub finished_at: u64,
    binding: RootDigest,
}
impl ReaderGeneration {
    fn encode(&self, root: RootDigest) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(root.as_bytes());
        for token in [self.reader, self.source] {
            b.extend_from_slice(token.owner.as_bytes());
            uint(&mut b, token.generation.get());
        }
        b.push(self.source_kind as u8);
        b.push(self.origin as u8);
        b.extend_from_slice(self.binding.as_bytes());
        for n in [self.entered_at, self.published_at, self.finished_at] {
            uint(&mut b, n);
        }
        seal(b)
    }
    fn decode(bytes: &[u8], root: RootDigest) -> Result<Self> {
        let mut r = Reader(checked(bytes, MAGIC, 178)?);
        if r.array::<32>()? != *root.as_bytes() {
            return Err(invalid("Reader generation root differs"));
        }
        let token = |r: &mut Reader<'_>| -> Result<OwnerToken> {
            Ok(OwnerToken {
                owner: OwnerId::new(r.array()?).map_err(pin_error)?,
                generation: NonZeroU64::new(r.uint()?)
                    .ok_or_else(|| invalid("Reader generation zero"))?,
            })
        };
        let reader = token(&mut r)?;
        let source = token(&mut r)?;
        let source_kind = match r.byte()? {
            2 => OwnerKind::Pending,
            3 => OwnerKind::Version,
            5 => OwnerKind::Snapshot,
            6 => OwnerKind::Migration,
            _ => return Err(invalid("Reader source kind differs")),
        };
        let origin = match r.byte()? {
            1 => Origin::Entered,
            2 => Origin::Unentered,
            3 => Origin::Legacy,
            _ => return Err(invalid("Reader generation origin differs")),
        };
        let record = Self {
            reader,
            source,
            source_kind,
            origin,
            binding: RootDigest::from_bytes(r.array()?),
            entered_at: r.uint()?,
            published_at: r.uint()?,
            finished_at: r.uint()?,
        };
        if !r.0.is_empty()
            || source.generation.get() != 1
            || (origin == Origin::Entered) != (record.entered_at != 0)
            || (origin != Origin::Entered && (record.published_at != 0 || record.finished_at == 0))
            || (record.published_at != 0 && record.published_at <= record.entered_at)
            || (record.finished_at != 0
                && record.finished_at <= record.entered_at.max(record.published_at))
            || record.encode(root) != bytes
        {
            return Err(invalid(
                "Reader generation encoding or revision order differs",
            ));
        }
        Ok(record)
    }
    fn phase(&self) -> PinPhase {
        if self.finished_at != 0 {
            PinPhase::Released
        } else if self.published_at != 0 {
            PinPhase::Published
        } else {
            PinPhase::Held
        }
    }
}
fn domain(kind: OwnerKind) -> &'static [u8] {
    if matches!(kind, OwnerKind::Pending | OwnerKind::Version) {
        checkpoint_reader::DOMAIN
    } else {
        image_reader::DOMAIN
    }
}
fn read(
    p: &Planner<'_>,
    binding: &OwnerBinding,
    token: OwnerToken,
) -> Result<Option<ReaderGeneration>> {
    let Some(bytes) = p.view.get(ColumnFamily::Default, &record_key(token))? else {
        return Ok(None);
    };
    if !p
        .view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .is_some_and(|h| reader_generation_format(&h))
    {
        return Err(invalid("Reader generation requires version 10 ledger"));
    }
    let record = ReaderGeneration::decode(&bytes, p.root)?;
    if record.reader != token
        || token.owner != binding.descriptor.id
        || binding.descriptor.kind != OwnerKind::Reader
        || record.binding != RootDigest::sha256(&owner_registration(binding))
        || record.entered_at > p.revision
        || record.published_at > p.revision
        || record.finished_at > p.revision
        || checkpoint_reader::reader_id(
            p.root,
            record.source.owner,
            binding
                .descriptor
                .local
                .ok_or_else(|| invalid("Reader lifetime absent"))?,
            domain(record.source_kind),
        )? != token.owner
    {
        return Err(invalid("Reader generation links differ"));
    }
    Ok(Some(record))
}
pub(super) fn encode(owner: &LedgerOwner) -> Vec<u8> {
    let mut b = encode_owner(owner);
    b.truncate(b.len() - 32);
    b[..8].copy_from_slice(OWNER_MAGIC);
    seal(b)
}
pub(super) fn registration(binding: &OwnerBinding) -> Vec<u8> {
    let mut binding = binding.clone();
    binding.generation = NonZeroU64::new(1).unwrap();
    encode(&LedgerOwner {
        binding,
        phase: PinPhase::Held,
    })
}
pub(super) fn check(p: &Planner<'_>, owner: &LedgerOwner, managed: bool) -> Result<()> {
    if owner.binding.descriptor.kind != OwnerKind::Reader {
        if managed {
            return Err(invalid("Reader owner encoding has another kind"));
        }
        return Ok(());
    }
    match (managed, read(p, &owner.binding, owner.binding.token())?) {
        (false, None) => Ok(()),
        (true, Some(record)) if record.phase() == owner.phase => Ok(()),
        _ => Err(invalid("Reader owner and generation disagree")),
    }
}
/// Point observation only. Missing legacy generations remain unknown; callers
/// must not skip holes or treat a Released owner as a dense drain history.
pub fn generation(
    view: &dyn ReadView,
    root: &RootDescriptor,
    token: OwnerToken,
) -> Result<Option<ReaderGeneration>> {
    crate::checkpoint::inspect_initial_checkpoint_base(view, root)?;
    let h = view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .ok_or_else(|| invalid("Reader ledger absent"))?;
    let (revision, rows_enabled) = read_header(&h, root.digest())?;
    let p = Planner {
        view,
        root: root.digest(),
        revision,
        rows_enabled,
        writes: BTreeMap::new(),
    };
    let owner = p
        .owner(token.owner)?
        .ok_or_else(|| conflict("Reader absent"))?;
    if owner.binding.descriptor.kind != OwnerKind::Reader
        || token.generation > owner.binding.generation
    {
        return Err(conflict("Reader history bound differs"));
    }
    read(&p, &owner.binding, token)
}
pub const HISTORY_PAGE: usize = 4;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationObservation {
    pub reader: OwnerToken,
    /// Absence is explicitly unknown; do not skip holes in the dense prefix.
    pub record: Option<ReaderGeneration>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationPage {
    pub current: OwnerToken,
    pub generations: Vec<GenerationObservation>,
    pub next: Option<NonZeroU64>,
}
/// At most four consecutive generations from one established view, including
/// missing rows. This is neither a complete history nor release authority.
pub fn history(
    view: &dyn ReadView,
    root: &RootDescriptor,
    operation: [u8; 32],
    through: OwnerToken,
    after: Option<NonZeroU64>,
) -> Result<GenerationPage> {
    crate::checkpoint::inspect_initial_checkpoint_base(view, root)?;
    let h = view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .ok_or_else(|| invalid("Reader history ledger absent"))?;
    let (revision, rows_enabled) = read_header(&h, root.digest())?;
    let p = Planner {
        view,
        root: root.digest(),
        revision,
        rows_enabled,
        writes: BTreeMap::new(),
    };
    let owner = p
        .owner(through.owner)?
        .ok_or_else(|| invalid("Reader history owner absent"))?;
    if owner.binding.descriptor.kind != OwnerKind::Reader
        || owner.binding.descriptor.operation != operation
        || through.generation > owner.binding.generation
        || after.is_some_and(|a| a >= through.generation)
    {
        return Err(conflict("Reader history owner or cursor differs"));
    }
    let first = after.map_or(1, |a| a.get() + 1);
    let end = first
        .saturating_add(HISTORY_PAGE as u64 - 1)
        .min(through.generation.get());
    let mut generations = Vec::with_capacity(HISTORY_PAGE);
    for generation in first..=end {
        let reader = OwnerToken {
            owner: through.owner,
            generation: NonZeroU64::new(generation).unwrap(),
        };
        generations.push(GenerationObservation {
            reader,
            record: read(&p, &owner.binding, reader)?,
        });
    }
    Ok(GenerationPage {
        current: owner.binding.token(),
        generations,
        next: (end < through.generation.get()).then(|| NonZeroU64::new(end).unwrap()),
    })
}

fn next_revision(p: &Planner<'_>) -> Result<u64> {
    p.revision
        .checked_add(1)
        .ok_or_else(|| invalid("Reader revision exhausted"))
}
fn expected(p: &Planner<'_>, source: OwnerToken, reader: &OwnerBinding) -> Result<LedgerOwner> {
    reader.validate(p.root)?;
    let source = p.exact_owner(source)?;
    let lifetime = reader
        .descriptor
        .local
        .ok_or_else(|| invalid("Reader lifetime absent"))?;
    let mut expected = if matches!(
        source.binding.descriptor.kind,
        OwnerKind::Pending | OwnerKind::Version
    ) {
        checkpoint_reader::binding(&source.binding, lifetime)?
    } else {
        image_reader::binding(&source.binding, lifetime)?
    };
    expected.generation = reader.generation;
    if expected != *reader || source.phase != PinPhase::Published {
        return Err(conflict("Reader admission lacks exact Published source"));
    }
    checkpoint_refusal::check_reader_entry(p, &source.binding)?;
    Ok(source)
}
fn previous(
    p: &Planner<'_>,
    binding: &OwnerBinding,
    old: Option<&LedgerOwner>,
) -> Result<Option<OwnerSlot>> {
    match old {
        None if binding.generation.get() == 1 => {
            for resource in &binding.resources {
                if p.resource(*resource)?
                    .owner(p.view, binding.descriptor.id)?
                    .is_some()
                {
                    return Err(invalid("orphan Reader pin"));
                }
            }
            Ok(None)
        }
        Some(old)
            if old.binding.descriptor == binding.descriptor
                && old.binding.resources == binding.resources
                && old.phase == PinPhase::Released
                && old.binding.generation.get().checked_add(1)
                    == Some(binding.generation.get()) =>
        {
            Ok(Some(OwnerSlot {
                generation: old.binding.generation,
                phase: old.phase,
            }))
        }
        _ => Err(conflict("Reader generation does not follow release")),
    }
}
fn write(
    p: &mut Planner<'_>,
    binding: &OwnerBinding,
    previous: Option<OwnerSlot>,
    record: &ReaderGeneration,
) -> Result<()> {
    let owner = LedgerOwner {
        binding: binding.clone(),
        phase: record.phase(),
    };
    p.update_resources(&owner, previous, owner.phase)?;
    reader_index::record(p, binding)?;
    p.writes.insert(
        owner_registration_key(binding.descriptor.id),
        registration(binding),
    );
    p.writes
        .insert(owner_key(binding.descriptor.id), encode(&owner));
    p.writes
        .insert(record_key(binding.token()), record.encode(p.root));
    Ok(())
}
pub(super) fn share(p: &mut Planner<'_>, source: OwnerToken, reader: &OwnerBinding) -> Result<()> {
    let source = expected(p, source, reader)?;
    let old = p.owner(reader.descriptor.id)?;
    if let Some(old) = &old {
        if old.binding == *reader && matches!(old.phase, PinPhase::Held | PinPhase::Published) {
            let record = read(p, reader, reader.token())?
                .ok_or_else(|| conflict("legacy Reader must finish before new admission"))?;
            if record.source == source.binding.token()
                && record.source_kind == source.binding.descriptor.kind
            {
                return Ok(());
            }
            return Err(conflict("Reader source differs from original admission"));
        }
    }
    let previous = previous(p, reader, old.as_ref())?;
    if read(p, reader, reader.token())?.is_some() {
        return Err(invalid("orphan Reader generation"));
    }
    let record = ReaderGeneration {
        reader: reader.token(),
        source: source.binding.token(),
        source_kind: source.binding.descriptor.kind,
        origin: Origin::Entered,
        entered_at: next_revision(p)?,
        published_at: 0,
        finished_at: 0,
        binding: RootDigest::sha256(&owner_registration(reader)),
    };
    write(p, reader, previous, &record)
}
pub(super) fn publish(p: &mut Planner<'_>, owner: &LedgerOwner) -> Result<()> {
    let mut record = read(p, &owner.binding, owner.binding.token())?
        .ok_or_else(|| conflict("legacy Reader cannot acquire new I/O authority"))?;
    let source = expected(p, record.source, &owner.binding)?;
    if record.source_kind != source.binding.descriptor.kind || record.finished_at != 0 {
        return Err(conflict("Reader source or finished generation differs"));
    }
    if owner.phase == PinPhase::Published {
        return Ok(());
    }
    if owner.phase != PinPhase::Held {
        return Err(conflict("Reader publication requires Held"));
    }
    record.published_at = next_revision(p)?;
    write(
        p,
        &owner.binding,
        Some(OwnerSlot {
            generation: owner.binding.generation,
            phase: owner.phase,
        }),
        &record,
    )
}
/// Called only after the existing protocol-specific finish checked complete
/// Published coverage. Legacy observations never invent an earlier admission.
pub(super) fn finish_covered(
    p: &mut Planner<'_>,
    source: OwnerToken,
    source_kind: OwnerKind,
    reader: &OwnerBinding,
) -> Result<()> {
    let old = p.owner(reader.descriptor.id)?;
    let prior = read(p, reader, reader.token())?;
    let (previous, mut record) = match old.as_ref() {
        Some(old) if old.binding == *reader && old.phase != PinPhase::Released => {
            let record = prior.unwrap_or(ReaderGeneration {
                reader: reader.token(),
                source,
                source_kind,
                origin: Origin::Legacy,
                entered_at: 0,
                published_at: 0,
                finished_at: 0,
                binding: RootDigest::sha256(&owner_registration(reader)),
            });
            (
                Some(OwnerSlot {
                    generation: reader.generation,
                    phase: old.phase,
                }),
                record,
            )
        }
        _ => {
            let previous = previous(p, reader, old.as_ref())?;
            if prior.is_some() {
                return Err(invalid("orphan Reader generation at finish"));
            }
            (
                previous,
                ReaderGeneration {
                    reader: reader.token(),
                    source,
                    source_kind,
                    origin: Origin::Unentered,
                    entered_at: 0,
                    published_at: 0,
                    finished_at: 0,
                    binding: RootDigest::sha256(&owner_registration(reader)),
                },
            )
        }
    };
    if record.source != source || record.source_kind != source_kind || record.finished_at != 0 {
        return Err(conflict("Reader finish source or generation differs"));
    }
    record.finished_at = next_revision(p)?;
    write(p, reader, previous, &record)
}

#[cfg(test)]
mod tests;
