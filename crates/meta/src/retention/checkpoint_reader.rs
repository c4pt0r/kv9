//! Read-only checkpoint users. First finish keeps complete Published coverage.
//! An exact already-Released generation can be acknowledged without its source.
//! Neither path retires that source or authorizes object deletion.
use super::*;

pub(super) const DOMAIN: &[u8] = b"kv9-checkpoint-reader-v1";

pub(super) fn reader_id(
    root: RootDigest,
    source: OwnerId,
    lifetime: LocalLifetime,
    domain: &[u8],
) -> Result<OwnerId> {
    let mut identity = domain.to_vec();
    identity.extend_from_slice(source.as_bytes());
    uint(&mut identity, lifetime.node);
    identity.extend_from_slice(&lifetime.store);
    identity.extend_from_slice(&lifetime.process);
    checkpoint_intent::owner_id(root, &identity, b"reader")
}

/// Acknowledge only the exact already-terminal generation. Its retained owner,
/// registration and resource links supply the fact; source metadata is not an
/// acknowledgement dependency. This cannot terminate absent or active readers.
pub(super) fn retry_released(
    p: &Planner<'_>,
    source: OwnerToken,
    reader: &OwnerBinding,
    domain: &[u8],
) -> Result<bool> {
    reader.validate(p.root)?;
    let lifetime = reader
        .descriptor
        .local
        .ok_or_else(|| invalid("reader retry lifetime absent"))?;
    if reader.descriptor.kind != OwnerKind::Reader
        || source.generation.get() != 1
        || reader_id(p.root, source.owner, lifetime, domain)? != reader.descriptor.id
    {
        return Err(conflict("reader retry source or protocol differs"));
    }
    Ok(p.owner(reader.descriptor.id)?
        .is_some_and(|old| old.binding == *reader && old.phase == PinPhase::Released))
}

pub fn binding(source: &OwnerBinding, lifetime: LocalLifetime) -> Result<OwnerBinding> {
    if !matches!(
        source.descriptor.kind,
        OwnerKind::Pending | OwnerKind::Version
    ) || source.generation.get() != 1
        || source.descriptor.local.is_some()
    {
        return Err(invalid(
            "checkpoint reader source kind or generation differs",
        ));
    }
    source.validate(source.descriptor.root)?;
    let id = reader_id(
        source.descriptor.root,
        source.descriptor.id,
        lifetime,
        DOMAIN,
    )?;
    let mut reader = source.clone();
    reader.descriptor.id = id;
    reader.descriptor.kind = OwnerKind::Reader;
    reader.descriptor.local = Some(lifetime);
    reader.validate(source.descriptor.root)?;
    Ok(reader)
}

/// The observation is supplied only after the actual local read jobs drained.
/// As with existing owner transfers, the full resource set stays Published
/// under the successor. A failed/uncertain entry is fenced at this generation.
pub(super) fn finish(p: &mut Planner<'_>, source: OwnerToken, reader: &OwnerBinding) -> Result<()> {
    if retry_released(p, source, reader, DOMAIN)? {
        return Ok(());
    }
    let successor = p.exact_owner(source)?;
    let mut expected = binding(
        &successor.binding,
        reader
            .descriptor
            .local
            .ok_or_else(|| invalid("checkpoint reader lifetime absent"))?,
    )?;
    expected.generation = reader.generation;
    if expected != *reader {
        return Err(conflict("checkpoint reader lacks exact published coverage"));
    }
    if successor.phase != PinPhase::Published {
        // A concurrent positive handoff may retire Pending while its reader
        // drains. Validate the original Pending and its exact Published Version
        // from this same view; never substitute a newer/different image.
        let intent = checkpoint_intent::load(p, source.owner)?
            .ok_or_else(|| conflict("checkpoint reader successor is not published"))?;
        if successor.binding != intent.binding(OwnerKind::Pending)? {
            return Err(conflict("checkpoint reader successor differs"));
        }
    }
    reader_lifecycle::finish_covered(p, source, successor.binding.descriptor.kind, reader)
}

/// Caller checked the exact full Published successor in this same view.
pub(super) fn finish_covered(p: &mut Planner<'_>, reader: &OwnerBinding) -> Result<()> {
    let previous = match p.owner(reader.descriptor.id)? {
        None if reader.generation.get() == 1 => {
            for resource in &reader.resources {
                if p.resource(*resource)?
                    .owner(p.view, reader.descriptor.id)?
                    .is_some()
                {
                    return Err(invalid("orphan checkpoint reader pin"));
                }
            }
            p.writes.insert(
                owner_registration_key(reader.descriptor.id),
                owner_registration(reader),
            );
            None
        }
        Some(old) if old.binding == *reader => {
            if old.phase == PinPhase::Released {
                return Ok(());
            }
            Some(OwnerSlot {
                generation: old.binding.generation,
                phase: old.phase,
            })
        }
        Some(old)
            if old.binding.descriptor == reader.descriptor
                && old.binding.resources == reader.resources
                && old.phase == PinPhase::Released
                && old.binding.generation.get().checked_add(1) == Some(reader.generation.get()) =>
        {
            Some(OwnerSlot {
                generation: old.binding.generation,
                phase: old.phase,
            })
        }
        _ => return Err(conflict("checkpoint reader generation or binding differs")),
    };
    reader_index::record(p, reader)?;
    let owner = LedgerOwner {
        binding: reader.clone(),
        phase: PinPhase::Released,
    };
    p.update_resources(&owner, previous, PinPhase::Released)?;
    p.writes
        .insert(owner_key(reader.descriptor.id), encode_owner(&owner));
    Ok(())
}

#[cfg(test)]
mod retry_tests;
#[cfg(test)]
mod tests;
