//! Atomic birth of checkpoint protection. This records a catalog transition,
//! not historical publisher completeness or qualification of remote PUTs.
use super::*;

const MAGIC: &[u8; 8] = b"KV9CBO01";
fn origin_key(pending: OwnerId) -> Vec<u8> {
    key(17, pending.as_bytes())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtomicBirth {
    pub pending: OwnerId,
    pub created_at: u64,
    intent: RootDigest,
}
impl AtomicBirth {
    fn encode(&self, root: RootDigest) -> Vec<u8> {
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(root.as_bytes());
        bytes.extend_from_slice(self.pending.as_bytes());
        bytes.extend_from_slice(self.intent.as_bytes());
        uint(&mut bytes, self.created_at);
        seal(bytes)
    }
}
pub(super) fn check_old_format(view: &dyn ReadView, header: &[u8]) -> Result<()> {
    if view
        .iter(ColumnFamily::Default, &key(17, &[]), &key(18, &[]))?
        .next()
        .transpose()?
        .is_some()
    {
        return Err(invalid(
            "atomic checkpoint birth under an older ledger header",
        ));
    }
    if !header.starts_with(ENROLLMENT_HEADER_MAGIC) {
        checkpoint_admission::enrollment::check_old_format(view)?;
        if !header.starts_with(BUILD_INDEX_HEADER_MAGIC) {
            build_index::check_old_format(view)?;
        }
    }
    Ok(())
}
pub(super) fn reject_legacy_record(p: &Planner<'_>, intent: &CheckpointIntent) -> Result<()> {
    if p.view
        .get(ColumnFamily::Default, &origin_key(intent.pending_id()))?
        .is_some()
    {
        return Err(conflict(
            "atomic checkpoint requires Begin, not legacy Record authorization",
        ));
    }
    Ok(())
}
fn read(p: &Planner<'_>, intent: &CheckpointIntent) -> Result<Option<AtomicBirth>> {
    let Some(bytes) = p
        .view
        .get(ColumnFamily::Default, &origin_key(intent.pending_id()))?
    else {
        return Ok(None);
    };
    if !p
        .view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .is_some_and(|b| {
            (migration_format(&b) || b.starts_with(CLOSURE_HEADER_MAGIC))
                || b.starts_with(ATOMIC_HEADER_MAGIC)
        })
    {
        return Err(invalid("atomic checkpoint birth requires version 6 ledger"));
    }
    let mut r = Reader(checked(&bytes, MAGIC, 128)?);
    if r.array::<32>()? != *p.root.as_bytes() {
        return Err(invalid("checkpoint birth root differs"));
    }
    let record = AtomicBirth {
        pending: OwnerId::new(r.array()?).map_err(pin_error)?,
        intent: RootDigest::from_bytes(r.array()?),
        created_at: r.uint()?,
    };
    let gate = checkpoint_admission::load(p, intent)?;
    if !r.0.is_empty()
        || record.pending != intent.pending_id()
        || record.intent != RootDigest::sha256(&intent.encode())
        || record.created_at == 0
        || record.created_at > p.revision
        || record.created_at != gate.opened_at
        || record.encode(p.root) != bytes
    {
        return Err(invalid("checkpoint birth identity or revision differs"));
    }
    checkpoint_intent::check_owners(p, intent)?;
    Ok(Some(record))
}
pub(super) fn begin(
    p: &mut Planner<'_>,
    root: &RootDescriptor,
    intent: &CheckpointIntent,
) -> Result<()> {
    // Decode before staging any writes; the Pending binding and all resources
    // come from exactly these canonical descriptor bytes.
    let intent = CheckpointIntent::decode(&intent.encode(), root, intent.pending_id())?;
    let intent = CheckpointIntent::new(root, &intent.manifest()?, intent.predecessor())?;
    let pending = intent.binding(OwnerKind::Pending)?;
    let origin = p
        .view
        .get(ColumnFamily::Default, &origin_key(intent.pending_id()))?;
    let existing = p.owner(intent.pending_id())?;
    if let Some(mut owner) = existing {
        if owner.binding != pending {
            return Err(conflict("checkpoint Begin pending binding differs"));
        }
        if origin.is_some() {
            read(p, &intent)?;
        }
        if owner.phase == PinPhase::Held {
            if p.view
                .get(
                    ColumnFamily::Default,
                    &checkpoint_intent::intent_key(intent.pending_id()),
                )?
                .is_some()
                || p.view
                    .get(
                        ColumnFamily::Default,
                        &key(9, intent.pending_id().as_bytes()),
                    )?
                    .is_some()
            {
                return Err(invalid("Held Pending already has an intent or admission"));
            }
            p.update_resources(
                &owner,
                Some(OwnerSlot {
                    generation: pending.generation,
                    phase: PinPhase::Held,
                }),
                PinPhase::Published,
            )?;
            owner.phase = PinPhase::Published;
            p.writes
                .insert(owner_key(intent.pending_id()), encode_owner(&owner));
            checkpoint_intent::persist(p, root, &intent)?;
        } else {
            checkpoint_intent::record(p, root, &intent)?;
        }
        // A pre-existing Pending (even Held with no intent) can have historical
        // publishers. Resumption never fabricates atomic-birth provenance.
        return Ok(());
    }
    if origin.is_some()
        || p.view
            .get(
                ColumnFamily::Default,
                &key(20, intent.pending_id().as_bytes()),
            )?
            .is_some()
        || p.view
            .get(
                ColumnFamily::Default,
                &checkpoint_intent::intent_key(intent.pending_id()),
            )?
            .is_some()
    {
        return Err(invalid("checkpoint birth or intent exists without Pending"));
    }
    if p.owner(intent.binding(OwnerKind::Version)?.descriptor.id)?
        .is_some()
    {
        return Err(invalid(
            "checkpoint Version exists without its original Pending",
        ));
    }
    for identity in &pending.resources {
        let mut record = p.register_resource(*identity)?;
        record.update(
            p.view,
            pending.descriptor.id,
            None,
            OwnerSlot {
                generation: pending.generation,
                phase: PinPhase::Published,
            },
            &mut p.writes,
        )?;
    }
    p.writes.insert(
        owner_registration_key(pending.descriptor.id),
        owner_registration(&pending),
    );
    p.writes.insert(
        owner_key(pending.descriptor.id),
        encode_owner(&LedgerOwner {
            binding: pending,
            phase: PinPhase::Published,
        }),
    );
    checkpoint_intent::persist(p, root, &intent)?;
    let origin = AtomicBirth {
        pending: intent.pending_id(),
        intent: RootDigest::sha256(&intent.encode()),
        created_at: p
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("ledger revision exhausted"))?,
    };
    p.writes
        .insert(origin_key(origin.pending), origin.encode(p.root));
    Ok(())
}
/// Read-only provenance from one established view. None is an explicit legacy
/// or missing-origin blocker, never permission to reconstruct an empty attempt.
pub fn atomic_birth(
    view: &dyn ReadView,
    root: &RootDescriptor,
    pending: OwnerId,
) -> Result<Option<AtomicBirth>> {
    let intent = checkpoint_intent::checkpoint_intent(view, root, pending)?
        .ok_or_else(|| invalid("checkpoint birth intent absent"))?;
    let header = view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .ok_or_else(|| invalid("checkpoint birth header absent"))?;
    let (revision, rows_enabled) = read_header(&header, root.digest())?;
    let p = Planner {
        view,
        root: root.digest(),
        revision,
        rows_enabled,
        writes: BTreeMap::new(),
    };
    read(&p, &intent)
}

#[cfg(test)]
mod tests;
