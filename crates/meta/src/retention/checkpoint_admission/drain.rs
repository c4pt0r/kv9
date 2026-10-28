//! Immutable trusted-executor observations after local drain and exact release.
//! This does not infer process death, prior generations, complete registration
//! or backend qualification. Missing/unknown observations never certify closure.
use super::*;

const MAGIC: &[u8; 8] = b"KV9CDR01";
fn record_key(pending: OwnerId, executor: OwnerToken) -> Vec<u8> {
    let mut id = pending.as_bytes().to_vec();
    id.extend_from_slice(executor.owner.as_bytes());
    uint(&mut id, executor.generation.get());
    key(13, &id)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointDrain {
    pub pending: OwnerId,
    pub executor: OwnerToken,
    pub issued: u64,
    pub resolved: u64,
    pub reported_at: u64,
    opened_at: u64,
    binding: RootDigest,
}
impl CheckpointDrain {
    fn encode(&self, root: RootDigest) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(root.as_bytes());
        b.extend_from_slice(self.pending.as_bytes());
        b.extend_from_slice(self.executor.owner.as_bytes());
        uint(&mut b, self.executor.generation.get());
        b.extend_from_slice(self.binding.as_bytes());
        for n in [self.opened_at, self.reported_at, self.issued, self.resolved] {
            uint(&mut b, n);
        }
        seal(b)
    }
    fn decode(bytes: &[u8], root: RootDigest) -> Result<Self> {
        let mut r = Reader(checked(bytes, MAGIC, 176)?);
        if r.array::<32>()? != *root.as_bytes() {
            return Err(invalid("checkpoint drain root differs"));
        }
        let record = Self {
            pending: OwnerId::new(r.array()?).map_err(pin_error)?,
            executor: OwnerToken {
                owner: OwnerId::new(r.array()?).map_err(pin_error)?,
                generation: NonZeroU64::new(r.uint()?)
                    .ok_or_else(|| invalid("checkpoint drain generation zero"))?,
            },
            binding: RootDigest::from_bytes(r.array()?),
            opened_at: r.uint()?,
            reported_at: r.uint()?,
            issued: r.uint()?,
            resolved: r.uint()?,
        };
        if !r.0.is_empty()
            || record.opened_at == 0
            || record.reported_at <= record.opened_at
            || record.resolved > record.issued
            || record.encode(root) != bytes
        {
            return Err(invalid("checkpoint drain encoding or counters differ"));
        }
        Ok(record)
    }
}

pub(super) fn read(
    p: &Planner<'_>,
    gate: &Admission,
    executor: OwnerToken,
    owner: &LedgerOwner,
) -> Result<Option<CheckpointDrain>> {
    let Some(bytes) = p
        .view
        .get(ColumnFamily::Default, &record_key(gate.pending, executor))?
    else {
        return Ok(None);
    };
    let record = CheckpointDrain::decode(&bytes, p.root)?;
    if record.pending != gate.pending
        || record.executor != executor
        || record.opened_at != gate.opened_at
        || record.reported_at > p.revision
        || record.binding != RootDigest::sha256(&owner_registration(&owner.binding))
        || owner.binding.generation < executor.generation
        || (owner.binding.generation == executor.generation && owner.phase != PinPhase::Released)
    {
        return Err(invalid("checkpoint drain links differ"));
    }
    Ok(Some(record))
}

/// An immutable report can acknowledge an earlier generation while a later
/// one is active. Validate the current registration, pins and index plus the
/// requested report; never transition that later generation or repair a gap.
fn retry_finished(
    p: &Planner<'_>,
    pending: OwnerToken,
    executor: OwnerToken,
    expected: Option<&OwnerBinding>,
    issued: u64,
    resolved: u64,
) -> Result<bool> {
    let Some(owner) = p.owner(executor.owner)? else {
        return Ok(false);
    };
    if owner.binding.descriptor.kind != OwnerKind::Build
        || pending.generation.get() != 1
        || expected.is_some_and(|b| {
            b.token() != executor
                || b.descriptor != owner.binding.descriptor
                || b.resources != owner.binding.resources
        })
    {
        return Err(conflict("checkpoint drain acknowledgement binding differs"));
    }
    if executor.generation > owner.binding.generation {
        // A first/next unentered cancellation is handled by the original
        // covered transition, never acknowledged from an older report.
        return Ok(false);
    }
    let gate = retained(p, &owner.binding)?
        .ok_or_else(|| conflict("checkpoint drain acknowledgement gate absent"))?;
    if gate.pending != pending.owner {
        return Err(conflict("checkpoint drain acknowledgement pending differs"));
    }
    let Some(report) = read(p, &gate, executor, &owner)? else {
        return Ok(false);
    };
    if !p
        .view
        .get(ColumnFamily::Default, &owner_key(executor.owner))?
        .is_some_and(|bytes| bytes.starts_with(EXECUTOR_OWNER_MAGIC))
    {
        return Err(conflict(
            "checkpoint drain acknowledgement requires indexed executor",
        ));
    }
    if report.issued != issued || report.resolved != resolved {
        return Err(conflict("checkpoint drain observation is immutable"));
    }
    enrollment::check_token_report(p, &gate, &owner, executor, issued)?;
    Ok(true)
}
pub(in crate::retention) fn record(
    p: &mut Planner<'_>,
    pending: OwnerToken,
    executor: OwnerToken,
    issued: u64,
    resolved: u64,
) -> Result<()> {
    if resolved > issued {
        return Err(invalid("checkpoint drain resolved exceeds issued"));
    }
    if retry_finished(p, pending, executor, None, issued, resolved)? {
        return Ok(());
    }
    let intent = checkpoint_intent::load(p, pending.owner)?
        .ok_or_else(|| conflict("checkpoint drain intent missing"))?;
    if intent.binding(OwnerKind::Pending)?.token() != pending {
        return Err(conflict("checkpoint drain pending token differs"));
    }
    let gate = load(p, &intent)?;
    let owner = p.exact_owner(executor)?;
    if owner.phase != PinPhase::Released
        || managed(p, &owner.binding)? != Some(gate.clone())
        || !p
            .view
            .get(ColumnFamily::Default, &owner_key(executor.owner))?
            .is_some_and(|bytes| bytes.starts_with(EXECUTOR_OWNER_MAGIC))
    {
        return Err(conflict(
            "checkpoint drain requires exact released indexed executor",
        ));
    }
    enrollment::check_report(p, &gate, &owner, issued)?;
    write_record(p, &gate, &owner, issued, resolved)
}

fn write_record(
    p: &mut Planner<'_>,
    gate: &Admission,
    owner: &LedgerOwner,
    issued: u64,
    resolved: u64,
) -> Result<()> {
    let record = CheckpointDrain {
        pending: gate.pending,
        executor: owner.binding.token(),
        issued,
        resolved,
        opened_at: gate.opened_at,
        reported_at: p
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("ledger revision exhausted"))?,
        binding: RootDigest::sha256(&owner_registration(&owner.binding)),
    };
    p.writes.insert(
        record_key(gate.pending, record.executor),
        record.encode(p.root),
    );
    Ok(())
}

/// The same catalog batch records the drained scope and releases its Build
/// pins. No intermediate Released-without-report state is created by this
/// operation. The caller's exact scope report remains trusted input, as with
/// `record`; neither path infers remote cancellation from local drain.
pub(in crate::retention) fn finish(
    p: &mut Planner<'_>,
    pending: OwnerToken,
    binding: &OwnerBinding,
    issued: u64,
    resolved: u64,
) -> Result<()> {
    binding.validate(p.root)?;
    if resolved > issued {
        return Err(invalid("checkpoint drain resolved exceeds issued"));
    }
    if retry_finished(p, pending, binding.token(), Some(binding), issued, resolved)? {
        return Ok(());
    }
    // Loading the intent verifies its complete Published Pending or Version
    // from this same snapshot, including every resource pin. The canonical
    // managed binding below has exactly that subject and resource closure.
    let intent = checkpoint_intent::load(p, pending.owner)?
        .ok_or_else(|| conflict("checkpoint drain intent missing"))?;
    if intent.binding(OwnerKind::Pending)?.token() != pending {
        return Err(conflict("checkpoint drain pending token differs"));
    }
    let gate = load(p, &intent)?;
    if managed(p, binding)? != Some(gate.clone()) {
        return Err(conflict(
            "checkpoint finish requires exact managed executor",
        ));
    }
    let old = p.owner(binding.descriptor.id)?;
    if old.as_ref().is_some_and(|owner| {
        owner.binding.descriptor != binding.descriptor
            || owner.binding.resources != binding.resources
    }) {
        return Err(conflict("checkpoint finish binding differs"));
    }
    let terminal = LedgerOwner {
        binding: binding.clone(),
        phase: PinPhase::Released,
    };
    match old {
        Some(owner) if owner.binding.generation == binding.generation => {
            if !p
                .view
                .get(ColumnFamily::Default, &owner_key(binding.descriptor.id))?
                .is_some_and(|bytes| bytes.starts_with(EXECUTOR_OWNER_MAGIC))
            {
                return Err(conflict("checkpoint finish requires indexed executor"));
            }
            enrollment::check_report(p, &gate, &owner, issued)?;
            if owner.phase == PinPhase::Held && issued != 0 {
                return Err(conflict("unpublished executor cannot have issued PUTs"));
            }
            if owner.phase != PinPhase::Released {
                p.update_resources(
                    &owner,
                    Some(OwnerSlot {
                        generation: binding.generation,
                        phase: owner.phase,
                    }),
                    PinPhase::Released,
                )?;
                write_index(p, gate.pending, &terminal)?;
            }
        }
        _ => {
            // Entry never committed for this generation. Cancellation checks
            // first/next generation and fences a delayed Share/Publish, even
            // after gate closure. A nonzero report cannot certify no entry.
            if issued != 0 {
                return Err(conflict("unentered executor cannot have issued PUTs"));
            }
            if p.view
                .get(
                    ColumnFamily::Default,
                    &record_key(gate.pending, binding.token()),
                )?
                .is_some()
            {
                return Err(invalid("checkpoint drain exists before its executor"));
            }
            cancel(p, binding)?;
        }
    }
    write_record(p, &gate, &terminal, issued, resolved)
}

/// Historical observation for an exact generation, even while a newer one is
/// Published or the original source has transferred its protection. Retained
/// executor facts remain required; None never proves that no writes were issued.
pub fn checkpoint_drain(
    view: &dyn ReadView,
    root: &RootDescriptor,
    pending: OwnerId,
    executor: OwnerToken,
) -> Result<Option<CheckpointDrain>> {
    crate::checkpoint::inspect_initial_checkpoint_base(view, root)?;
    let header = view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .ok_or_else(|| invalid("checkpoint drain header missing"))?;
    let (revision, rows_enabled) = read_header(&header, root.digest())?;
    let p = Planner {
        view,
        root: root.digest(),
        revision,
        rows_enabled,
        writes: BTreeMap::new(),
    };
    let intent = checkpoint_intent::retained(&p, pending)?
        .ok_or_else(|| invalid("checkpoint drain intent missing"))?;
    let gate = load(&p, &intent)?;
    let owner = p
        .owner(executor.owner)?
        .ok_or_else(|| invalid("checkpoint drain executor missing"))?;
    if retained(&p, &owner.binding)? != Some(gate.clone()) {
        return Err(invalid("checkpoint drain executor differs"));
    }
    read(&p, &gate, executor, &owner)
}

#[cfg(test)]
mod finish_tests;
#[cfg(test)]
mod retry_tests;
#[cfg(test)]
mod tests;
