//! Immutable per-generation origin, minted only alongside a new Held or an
//! unentered cancellation. Neither V2 owner bytes nor Released proves origin.
use super::*;

const MAGIC: &[u8; 8] = b"KV9CEN01";
const MAX_BYTES: usize = 161;
pub const HISTORY_PAGE: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Entered,
    Cancelled,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Enrollment {
    pub pending: OwnerId,
    pub executor: OwnerToken,
    pub origin: Origin,
    pub recorded_at: u64,
    opened_at: u64,
    binding: RootDigest,
}
fn record_key(pending: OwnerId, executor: OwnerToken) -> Vec<u8> {
    let mut id = pending.as_bytes().to_vec();
    id.extend_from_slice(executor.owner.as_bytes());
    uint(&mut id, executor.generation.get());
    key(16, &id)
}
impl Enrollment {
    fn encode(&self, root: RootDigest) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(root.as_bytes());
        b.extend_from_slice(self.pending.as_bytes());
        b.extend_from_slice(self.executor.owner.as_bytes());
        uint(&mut b, self.executor.generation.get());
        b.extend_from_slice(self.binding.as_bytes());
        uint(&mut b, self.opened_at);
        uint(&mut b, self.recorded_at);
        b.push(match self.origin {
            Origin::Entered => 0,
            Origin::Cancelled => 1,
        });
        seal(b)
    }
    fn decode(bytes: &[u8], root: RootDigest) -> Result<Self> {
        let mut r = Reader(checked(bytes, MAGIC, MAX_BYTES)?);
        if r.array::<32>()? != *root.as_bytes() {
            return Err(invalid("enrollment root differs"));
        }
        let record = Self {
            pending: OwnerId::new(r.array()?).map_err(pin_error)?,
            executor: OwnerToken {
                owner: OwnerId::new(r.array()?).map_err(pin_error)?,
                generation: NonZeroU64::new(r.uint()?)
                    .ok_or_else(|| invalid("enrollment generation zero"))?,
            },
            binding: RootDigest::from_bytes(r.array()?),
            opened_at: r.uint()?,
            recorded_at: r.uint()?,
            origin: match r.byte()? {
                0 => Origin::Entered,
                1 => Origin::Cancelled,
                _ => return Err(invalid("enrollment origin")),
            },
        };
        if !r.0.is_empty()
            || record.opened_at == 0
            || record.recorded_at <= record.opened_at
            || record.encode(root) != bytes
        {
            return Err(invalid("enrollment encoding or revision differs"));
        }
        Ok(record)
    }
}
pub(in crate::retention) fn check_old_format(view: &dyn ReadView) -> Result<()> {
    if view
        .iter(ColumnFamily::Default, &key(16, &[]), &key(17, &[]))?
        .next()
        .transpose()?
        .is_some()
    {
        return Err(invalid("enrollment exists under an older ledger header"));
    }
    Ok(())
}
fn read(
    p: &Planner<'_>,
    gate: &Admission,
    owner: &LedgerOwner,
    token: OwnerToken,
) -> Result<Option<Enrollment>> {
    let Some(bytes) = p
        .view
        .get(ColumnFamily::Default, &record_key(gate.pending, token))?
    else {
        return Ok(None);
    };
    if !p
        .view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .is_some_and(|b| {
            (migration_format(&b) || b.starts_with(CLOSURE_HEADER_MAGIC))
                || b.starts_with(ATOMIC_HEADER_MAGIC)
                || b.starts_with(ENROLLMENT_HEADER_MAGIC)
        })
    {
        return Err(invalid("enrollment requires version 5 ledger"));
    }
    let record = Enrollment::decode(&bytes, p.root)?;
    if record.pending != gate.pending
        || record.executor != token
        || record.opened_at != gate.opened_at
        || record.recorded_at > p.revision
        || record.binding != RootDigest::sha256(&owner_registration(&owner.binding))
        || token.owner != owner.binding.descriptor.id
        || token.generation > owner.binding.generation
        || (record.origin == Origin::Entered
            && gate.is_closed()
            && record.recorded_at >= gate.closed_at)
        || (record.origin == Origin::Cancelled
            && token.generation == owner.binding.generation
            && owner.phase != PinPhase::Released)
        || !p
            .view
            .get(ColumnFamily::Default, &owner_key(token.owner))?
            .is_some_and(|b| b.starts_with(EXECUTOR_OWNER_MAGIC))
    {
        return Err(invalid("enrollment owner or admission links differ"));
    }
    Ok(Some(record))
}
pub(in crate::retention) fn check(p: &Planner<'_>, owner: &LedgerOwner) -> Result<()> {
    if owner.binding.descriptor.kind == OwnerKind::Build {
        if let Some(gate) = retained(p, &owner.binding)? {
            read(p, &gate, owner, owner.binding.token())?;
        }
    }
    Ok(())
}
pub(in crate::retention) fn require_entered(p: &Planner<'_>, owner: &LedgerOwner) -> Result<()> {
    let gate = managed(p, &owner.binding)?.ok_or_else(|| conflict("executor has no admission"))?;
    require_token_entered(p, &gate, owner, owner.binding.token())
}
pub(super) fn require_token_entered(
    p: &Planner<'_>,
    gate: &Admission,
    owner: &LedgerOwner,
    token: OwnerToken,
) -> Result<()> {
    if !read(p, gate, owner, token)?.is_some_and(|r| r.origin == Origin::Entered) {
        return Err(conflict(
            "executor generation lacks original entry provenance",
        ));
    }
    Ok(())
}
pub(super) fn check_report(
    p: &Planner<'_>,
    gate: &Admission,
    owner: &LedgerOwner,
    issued: u64,
) -> Result<()> {
    check_token_report(p, gate, owner, owner.binding.token(), issued)
}
pub(super) fn check_token_report(
    p: &Planner<'_>,
    gate: &Admission,
    owner: &LedgerOwner,
    token: OwnerToken,
    issued: u64,
) -> Result<()> {
    if issued == 0 {
        return Ok(());
    }
    if let Some(origin) = read(p, gate, owner, token)? {
        if origin.origin == Origin::Cancelled
            || retirement::read(p, gate.pending, token, owner, gate)?.is_some()
        {
            return Err(conflict(
                "unentered or unpublished generation cannot report PUTs",
            ));
        }
    }
    Ok(())
}
pub(in crate::retention) fn record_new(
    p: &mut Planner<'_>,
    binding: &OwnerBinding,
    origin: Origin,
) -> Result<()> {
    let gate =
        managed(p, binding)?.ok_or_else(|| conflict("enrollment requires managed binding"))?;
    if origin == Origin::Entered && gate.is_closed() {
        return Err(conflict("checkpoint admission closed"));
    }
    // Never infer origin from an existing Held/Published/Released row, including
    // an idempotent request that used to promote a legacy owner to V2.
    match p.owner(binding.descriptor.id)? {
        None if binding.generation.get() == 1 => {}
        Some(old)
            if old.phase == PinPhase::Released
                && old.binding.generation.get().checked_add(1)
                    == Some(binding.generation.get())
                && old.binding.descriptor == binding.descriptor
                && old.binding.resources == binding.resources => {}
        _ => {
            return Err(conflict(
                "cannot retroactively enroll an executor generation",
            ))
        }
    }
    let k = record_key(gate.pending, binding.token());
    if p.view.get(ColumnFamily::Default, &k)?.is_some() {
        return Err(invalid("enrollment precedes owner generation"));
    }
    let record = Enrollment {
        pending: gate.pending,
        executor: binding.token(),
        origin,
        opened_at: gate.opened_at,
        recorded_at: p
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("ledger revision exhausted"))?,
        binding: RootDigest::sha256(&owner_registration(binding)),
    };
    p.writes.insert(k, record.encode(p.root));
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenerationState {
    /// Includes old V2 rows; a newer successful report cannot repair this gap.
    MissingOrigin,
    Active(PinPhase),
    MissingTerminal,
    Cancelled,
    RetiredUnpublished,
    /// Even issued == resolved > 0 is not qualified remote PUT termination.
    Drained {
        issued: u64,
        resolved: u64,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationObservation {
    /// Admission revision, absent for missing or never-entered origins.
    pub admitted_at: Option<u64>,
    pub executor: OwnerToken,
    pub state: GenerationState,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationPage {
    pub current: OwnerToken,
    pub admission: Admission,
    pub generations: Vec<GenerationObservation>,
    pub next: Option<NonZeroU64>,
}
/// A dense prefix through the requested token in one established snapshot.
/// Every generation is visited, including absent records. Caller must retain
/// blockers across pages; neither EOF nor a page is a global release capability.
pub fn history(
    view: &dyn ReadView,
    root: &RootDescriptor,
    pending: OwnerId,
    through: OwnerToken,
    after: Option<NonZeroU64>,
) -> Result<GenerationPage> {
    crate::checkpoint::inspect_initial_checkpoint_base(view, root)?;
    let header = view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .ok_or_else(|| invalid("history header missing"))?;
    let (revision, rows_enabled) = read_header(&header, root.digest())?;
    let p = Planner {
        view,
        root: root.digest(),
        revision,
        rows_enabled,
        writes: BTreeMap::new(),
    };
    let intent = checkpoint_intent::retained(&p, pending)?
        .ok_or_else(|| invalid("history intent absent"))?;
    let gate = load(&p, &intent)?;
    let owner = p
        .owner(through.owner)?
        .ok_or_else(|| invalid("history owner absent"))?;
    if retained(&p, &owner.binding)? != Some(gate.clone())
        || through.generation > owner.binding.generation
        || after.is_some_and(|a| a >= through.generation)
    {
        return Err(conflict("history executor or cursor differs"));
    }
    let first = after.map_or(1, |a| a.get() + 1);
    let end = first
        .saturating_add(HISTORY_PAGE as u64 - 1)
        .min(through.generation.get());
    let mut generations = Vec::with_capacity(HISTORY_PAGE);
    for generation in first..=end {
        let token = OwnerToken {
            owner: through.owner,
            generation: NonZeroU64::new(generation).unwrap(),
        };
        let origin = read(&p, &gate, &owner, token)?;
        let admitted_at = origin
            .as_ref()
            .filter(|o| o.origin == Origin::Entered)
            .map(|o| o.recorded_at);
        let state = match origin {
            None => GenerationState::MissingOrigin,
            Some(origin) => {
                let report = drain::read(&p, &gate, token, &owner)?;
                if origin.origin == Origin::Cancelled {
                    if report.is_some_and(|r| r.issued != 0) {
                        return Err(invalid("cancelled generation reports PUTs"));
                    }
                    GenerationState::Cancelled
                } else if retirement::read(&p, pending, token, &owner, &gate)?.is_some() {
                    if report.is_some_and(|r| r.issued != 0) {
                        return Err(invalid("unpublished generation reports PUTs"));
                    }
                    GenerationState::RetiredUnpublished
                } else if let Some(report) = report {
                    GenerationState::Drained {
                        issued: report.issued,
                        resolved: report.resolved,
                    }
                } else if token.generation == owner.binding.generation
                    && owner.phase != PinPhase::Released
                {
                    GenerationState::Active(owner.phase)
                } else {
                    GenerationState::MissingTerminal
                }
            }
        };
        generations.push(GenerationObservation {
            admitted_at,
            executor: token,
            state,
        });
    }
    Ok(GenerationPage {
        current: owner.binding.token(),
        admission: gate,
        generations,
        next: (end < through.generation.get()).then(|| NonZeroU64::new(end).unwrap()),
    })
}

#[cfg(test)]
mod tests;
