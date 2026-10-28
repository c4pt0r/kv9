//! GET-only image users require their exact Published source for first finish.
//! Terminal acknowledgements need only the retained Released Reader fact.
//! Neither path closes publishers or releases the migration/snapshot source.
use super::*;

#[cfg(test)]
mod tests;

pub(super) const DOMAIN: &[u8] = b"kv9-image-reader-v1";

pub fn binding(source: &OwnerBinding, lifetime: LocalLifetime) -> Result<OwnerBinding> {
    source.validate(source.descriptor.root)?;
    if !matches!(
        source.descriptor.kind,
        OwnerKind::Snapshot | OwnerKind::Migration
    ) || source.generation.get() != 1
        || source.descriptor.local.is_some()
        || source.descriptor.subject_is_anchor
    {
        return Err(invalid("image reader source kind or generation differs"));
    }
    let mut reader = source.clone();
    reader.descriptor.id = checkpoint_reader::reader_id(
        source.descriptor.root,
        source.descriptor.id,
        lifetime,
        DOMAIN,
    )?;
    reader.descriptor.kind = OwnerKind::Reader;
    reader.descriptor.local = Some(lifetime);
    reader.validate(source.descriptor.root)?;
    Ok(reader)
}

pub(super) fn finish(p: &mut Planner<'_>, source: OwnerToken, reader: &OwnerBinding) -> Result<()> {
    if checkpoint_reader::retry_released(p, source, reader, DOMAIN)? {
        return Ok(());
    }
    let successor = p.exact_owner(source)?;
    let mut expected = binding(
        &successor.binding,
        reader
            .descriptor
            .local
            .ok_or_else(|| invalid("image reader lifetime absent"))?,
    )?;
    expected.generation = reader.generation;
    if expected != *reader || successor.phase != PinPhase::Published {
        return Err(conflict("image reader lacks exact Published coverage"));
    }
    reader_lifecycle::finish_covered(p, source, successor.binding.descriptor.kind, reader)
}
