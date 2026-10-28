//! A current tracked Held generation behind a closed gate cannot Publish.
//! This proves no upload authorization for that generation, not process death,
//! completion of older generations or global publisher closure.
use super::*;

const MAGIC: &[u8; 8] = b"KV9CUR01";
fn record_key(pending: OwnerId, executor: OwnerToken) -> Vec<u8> {
    let mut id = pending.as_bytes().to_vec();
    id.extend_from_slice(executor.owner.as_bytes());
    uint(&mut id, executor.generation.get());
    key(11, &id)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnpublishedRetirement {
    pub pending: OwnerId,
    pub executor: OwnerToken,
    pub closed_at: u64,
    pub retired_at: u64,
    binding: RootDigest,
}
impl UnpublishedRetirement {
    fn encode(&self, root: RootDigest) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(root.as_bytes());
        b.extend_from_slice(self.pending.as_bytes());
        b.extend_from_slice(self.executor.owner.as_bytes());
        uint(&mut b, self.executor.generation.get());
        b.extend_from_slice(self.binding.as_bytes());
        uint(&mut b, self.closed_at);
        uint(&mut b, self.retired_at);
        seal(b)
    }
    fn decode(bytes: &[u8], root: RootDigest) -> Result<Self> {
        let mut r = Reader(checked(bytes, MAGIC, 160)?);
        if r.array::<32>()? != *root.as_bytes() {
            return Err(invalid("unpublished retirement root differs"));
        }
        let record = Self {
            pending: OwnerId::new(r.array()?).map_err(pin_error)?,
            executor: OwnerToken {
                owner: OwnerId::new(r.array()?).map_err(pin_error)?,
                generation: NonZeroU64::new(r.uint()?)
                    .ok_or_else(|| invalid("retirement generation zero"))?,
            },
            binding: RootDigest::from_bytes(r.array()?),
            closed_at: r.uint()?,
            retired_at: r.uint()?,
        };
        if !r.0.is_empty()
            || record.closed_at == 0
            || record.retired_at <= record.closed_at
            || record.encode(root) != bytes
        {
            return Err(invalid(
                "unpublished retirement revision or encoding differs",
            ));
        }
        Ok(record)
    }
}

pub(super) fn read(
    p: &Planner<'_>,
    pending: OwnerId,
    executor: OwnerToken,
    owner: &LedgerOwner,
    gate: &Admission,
) -> Result<Option<UnpublishedRetirement>> {
    let Some(bytes) = p
        .view
        .get(ColumnFamily::Default, &record_key(pending, executor))?
    else {
        return Ok(None);
    };
    let record = UnpublishedRetirement::decode(&bytes, p.root)?;
    enrollment::require_token_entered(p, gate, owner, executor)?;
    if record.pending != pending
        || record.executor != executor
        || record.closed_at != gate.closed_at
        || record.retired_at > p.revision
        || record.binding != RootDigest::sha256(&owner_registration(&owner.binding))
        || owner.binding.generation < executor.generation
        || owner.phase != PinPhase::Released
    {
        return Err(invalid("unpublished retirement links differ"));
    }
    Ok(Some(record))
}

pub(in crate::retention) fn retire(
    p: &mut Planner<'_>,
    pending: OwnerToken,
    executor: OwnerToken,
) -> Result<()> {
    let intent = checkpoint_intent::load(p, pending.owner)?
        .ok_or_else(|| conflict("checkpoint intent absent"))?;
    if intent.binding(OwnerKind::Pending)?.token() != pending {
        return Err(conflict("retirement pending generation differs"));
    }
    let gate = load(p, &intent)?;
    if !gate.is_closed() {
        return Err(conflict("unpublished retirement requires closed admission"));
    }
    let mut owner = p.exact_owner(executor)?;
    let managed = managed(p, &owner.binding)?
        .ok_or_else(|| conflict("retirement requires checkpoint executor"))?;
    if managed != gate
        || !p
            .view
            .get(ColumnFamily::Default, &owner_key(executor.owner))?
            .is_some_and(|bytes| bytes.starts_with(EXECUTOR_OWNER_MAGIC))
    {
        return Err(conflict("retirement requires matching indexed executor"));
    }
    if read(p, pending.owner, executor, &owner, &gate)?.is_some() {
        return Ok(());
    }
    enrollment::require_entered(p, &owner)?;
    if owner.phase != PinPhase::Held {
        return Err(conflict(
            "only exact unpublished Held generation can retire",
        ));
    }
    let record = UnpublishedRetirement {
        pending: pending.owner,
        executor,
        closed_at: gate.closed_at,
        retired_at: p
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("ledger revision exhausted"))?,
        binding: RootDigest::sha256(&owner_registration(&owner.binding)),
    };
    p.update_resources(
        &owner,
        Some(OwnerSlot {
            generation: executor.generation,
            phase: PinPhase::Held,
        }),
        PinPhase::Released,
    )?;
    owner.phase = PinPhase::Released;
    write_index(p, pending.owner, &owner)?;
    p.writes
        .insert(record_key(pending.owner, executor), record.encode(p.root));
    Ok(())
}

/// Established metadata view required. A record is scoped to one exact
/// generation; Released without this record never reconstructs such evidence.
pub fn unpublished_retirement(
    view: &dyn ReadView,
    root: &RootDescriptor,
    pending: OwnerId,
    executor: OwnerToken,
) -> Result<Option<UnpublishedRetirement>> {
    let intent = checkpoint_intent::checkpoint_intent(view, root, pending)?
        .ok_or_else(|| invalid("checkpoint intent absent"))?;
    let header = view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .ok_or_else(|| invalid("ledger header missing"))?;
    let (revision, rows_enabled) = read_header(&header, root.digest())?;
    let p = Planner {
        view,
        root: root.digest(),
        revision,
        rows_enabled,
        writes: BTreeMap::new(),
    };
    let gate = load(&p, &intent)?;
    let owner = p
        .owner(executor.owner)?
        .ok_or_else(|| invalid("retired executor missing"))?;
    if managed(&p, &owner.binding)? != Some(gate.clone()) {
        return Err(invalid("retirement executor checkpoint differs"));
    }
    read(&p, pending, executor, &owner, &gate)
}

#[cfg(test)]
mod tests;
