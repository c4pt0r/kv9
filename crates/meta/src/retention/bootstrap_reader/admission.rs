//! Exact, durable origin of newly admitted store bootstrap readers. This is a
//! point observation, not a census or permission to retire its Version source.
use super::*;
use kv9_common::{NodeId, StoreIncarnation};

pub(crate) const OWNER_MAGIC: &[u8; 8] = b"KV9BRO01";
const MAGIC: &[u8; 8] = b"KV9BRA01";
fn record_key(id: OwnerId) -> Vec<u8> {
    key(28, id.as_bytes())
}
pub(in crate::retention) fn check_old_format(view: &dyn ReadView) -> Result<()> {
    if view
        .iter(ColumnFamily::Default, &key(28, &[]), &key(29, &[]))?
        .next()
        .transpose()?
        .is_some()
    {
        return Err(invalid("bootstrap admissions under an older ledger header"));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Admission {
    pub pending: OwnerToken,
    pub version: OwnerToken,
    pub node: NodeId,
    pub store: StoreIncarnation,
    pub entered_at: u64,
    pub checkpoint_operation: [u8; 32],
    binding: RootDigest,
}
impl Admission {
    fn encode(&self, root: RootDigest) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(root.as_bytes());
        for token in [self.pending, self.version] {
            b.extend_from_slice(token.owner.as_bytes());
            uint(&mut b, token.generation.get());
        }
        uint(&mut b, self.node.0);
        b.extend_from_slice(self.store.as_bytes());
        b.extend_from_slice(self.binding.as_bytes());
        uint(&mut b, self.entered_at);
        b.extend_from_slice(&self.checkpoint_operation);
        seal(b)
    }
    fn decode(bytes: &[u8], root: RootDigest) -> Result<Self> {
        let mut r = Reader(checked(bytes, MAGIC, 216)?);
        if r.array::<32>()? != *root.as_bytes() {
            return Err(invalid("bootstrap admission root differs"));
        }
        let token = |r: &mut Reader<'_>| -> Result<OwnerToken> {
            Ok(OwnerToken {
                owner: OwnerId::new(r.array()?).map_err(pin_error)?,
                generation: NonZeroU64::new(r.uint()?)
                    .filter(|g| g.get() == 1)
                    .ok_or_else(|| invalid("bootstrap admission generation differs"))?,
            })
        };
        let record = Self {
            pending: token(&mut r)?,
            version: token(&mut r)?,
            node: NodeId(r.uint()?),
            store: StoreIncarnation::from_bytes(r.array()?),
            binding: RootDigest::from_bytes(r.array()?),
            entered_at: r.uint()?,
            checkpoint_operation: r.array()?,
        };
        if !r.0.is_empty()
            || record.node.0 == 0
            || record.store.as_bytes() == &[0; 16]
            || record.entered_at == 0
            || record.pending == record.version
            || record.encode(root) != bytes
        {
            return Err(invalid("bootstrap admission encoding differs"));
        }
        Ok(record)
    }
}
pub(in crate::retention) fn read(
    p: &Planner<'_>,
    binding: &OwnerBinding,
) -> Result<Option<Admission>> {
    let Some(bytes) = p
        .view
        .get(ColumnFamily::Default, &record_key(binding.descriptor.id))?
    else {
        return Ok(None);
    };
    if !p
        .view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .is_some_and(|h| bootstrap_admission_format(&h))
    {
        return Err(invalid("bootstrap admission requires version 12 ledger"));
    }
    let record = Admission::decode(&bytes, p.root)?;
    let (id, operation) = identity_parts(p.root, record.pending, record.node, record.store)?;
    if binding.descriptor.kind != OwnerKind::BootstrapReader
        || binding.descriptor.id != id
        || binding.descriptor.operation != operation
        || record.binding != RootDigest::sha256(&owner_registration(binding))
        || record.entered_at > p.revision
        || checkpoint_intent::owner_id(
            p.root,
            &record.checkpoint_operation,
            b"kv9-checkpoint-pending-v1",
        )? != record.pending.owner
        || checkpoint_intent::owner_id(
            p.root,
            &record.checkpoint_operation,
            b"kv9-checkpoint-version-v1",
        )? != record.version.owner
    {
        return Err(invalid("bootstrap admission binding differs"));
    }
    Ok(Some(record))
}
pub(in crate::retention) fn encode(owner: &LedgerOwner) -> Vec<u8> {
    let mut bytes = encode_owner(owner);
    bytes.truncate(bytes.len() - 32);
    bytes[..8].copy_from_slice(OWNER_MAGIC);
    seal(bytes)
}
pub(in crate::retention) fn registration(binding: &OwnerBinding) -> Vec<u8> {
    encode(&LedgerOwner {
        binding: binding.clone(),
        phase: PinPhase::Held,
    })
}
pub(in crate::retention) fn check(
    p: &Planner<'_>,
    owner: &LedgerOwner,
    managed: bool,
) -> Result<()> {
    if owner.binding.descriptor.kind != OwnerKind::BootstrapReader {
        if managed {
            return Err(invalid("bootstrap owner encoding has another kind"));
        }
        return Ok(());
    }
    match (managed, read(p, &owner.binding)?) {
        (false, None) => Ok(()), // Legacy protection is never retroactively enrolled.
        (true, Some(_))
            if matches!(
                owner.phase,
                PinPhase::Held | PinPhase::Published | PinPhase::Released
            ) =>
        {
            Ok(())
        }
        _ => Err(invalid("bootstrap owner and admission disagree")),
    }
}
fn source(p: &Planner<'_>, pending: OwnerToken) -> Result<OwnerBinding> {
    let intent = checkpoint_intent::load(p, pending.owner)?
        .ok_or_else(|| conflict("bootstrap admission intent absent"))?;
    if intent.binding(OwnerKind::Pending)?.token() != pending {
        return Err(conflict("bootstrap admission Pending differs"));
    }
    let version = intent.binding(OwnerKind::Version)?;
    let current = p.exact_owner(version.token())?;
    if current.binding != version || current.phase != PinPhase::Published {
        return Err(conflict(
            "bootstrap admission lacks exact Published Version",
        ));
    }
    checkpoint_refusal::check_reader_entry(p, &version)?;
    Ok(version)
}
pub(in crate::retention) fn begin(
    p: &mut Planner<'_>,
    root: &RootDescriptor,
    pending: OwnerToken,
    store: StoreIdentity,
) -> Result<()> {
    store.verify(root, store.node_id)?;
    // Metadata bootstrap currently supports the provisioned root voters only.
    // A joiner's root-bound local identity is not metadata membership proof.
    if root.voter(store.node_id).is_none() || store.store_incarnation.as_bytes() == &[0; 16] {
        return Err(conflict(
            "bootstrap store is not a provisioned metadata voter",
        ));
    }
    let version = source(p, pending)?;
    let binding = candidate(p, root, pending, store)?;
    let prior = read(p, &binding)?;
    if let Some(old) = p.owner(binding.descriptor.id)? {
        if old.binding == binding
            && matches!(old.phase, PinPhase::Held | PinPhase::Published)
            && prior.is_some_and(|a| {
                a.pending == pending
                    && a.version == version.token()
                    && a.node == store.node_id
                    && a.store == store.store_incarnation
            })
        {
            return Ok(());
        }
        return Err(conflict(
            "legacy or terminal bootstrap grant cannot reenter",
        ));
    }
    if prior.is_some() || unentered::read(p, &binding)?.is_some() {
        return Err(invalid("orphan bootstrap origin"));
    }
    bootstrap_index::require_unregistered(p, &binding)?;
    for resource in &binding.resources {
        if p.resource(*resource)?
            .owner(p.view, binding.descriptor.id)?
            .is_some()
        {
            return Err(invalid("orphan bootstrap pin"));
        }
    }
    let record = Admission {
        pending,
        version: version.token(),
        node: store.node_id,
        store: store.store_incarnation,
        entered_at: p
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("bootstrap revision exhausted"))?,
        binding: RootDigest::sha256(&owner_registration(&binding)),
        checkpoint_operation: version.descriptor.operation,
    };
    let owner = LedgerOwner {
        binding,
        phase: PinPhase::Held,
    };
    p.update_resources(&owner, None, PinPhase::Held)?;
    p.writes.insert(
        owner_registration_key(owner.binding.descriptor.id),
        registration(&owner.binding),
    );
    p.writes
        .insert(owner_key(owner.binding.descriptor.id), encode(&owner));
    p.writes.insert(
        record_key(owner.binding.descriptor.id),
        record.encode(p.root),
    );
    Ok(())
}
pub(in crate::retention) fn publish(p: &mut Planner<'_>, owner: &LedgerOwner) -> Result<()> {
    let record = read(p, &owner.binding)?
        .ok_or_else(|| conflict("legacy bootstrap grant lacks typed admission"))?;
    let mut expected = source(p, record.pending)?;
    if expected.token() != record.version {
        return Err(conflict("bootstrap Version differs"));
    }
    let (id, operation) = identity_parts(p.root, record.pending, record.node, record.store)?;
    expected.descriptor.id = id;
    expected.descriptor.operation = operation;
    expected.descriptor.kind = OwnerKind::BootstrapReader;
    if expected != owner.binding {
        return Err(conflict("bootstrap admission closure differs"));
    }
    if owner.phase == PinPhase::Published {
        return Ok(());
    }
    if owner.phase != PinPhase::Held {
        return Err(conflict("bootstrap grant cannot resume"));
    }
    p.update_resources(
        owner,
        Some(OwnerSlot {
            generation: owner.binding.generation,
            phase: owner.phase,
        }),
        PinPhase::Published,
    )?;
    p.writes.insert(
        owner_key(id),
        encode(&LedgerOwner {
            binding: owner.binding.clone(),
            phase: PinPhase::Published,
        }),
    );
    Ok(())
}
/// The caller must establish one catalog view. Absence means legacy/unknown,
/// never proof that all grants are enumerated or that the source may be freed.
pub fn observe(
    view: &dyn ReadView,
    root: &RootDescriptor,
    token: OwnerToken,
) -> Result<Option<Admission>> {
    let p = planner(view, root)?;
    let owner = p.exact_owner(token)?;
    if owner.binding.descriptor.kind != OwnerKind::BootstrapReader {
        return Err(conflict("bootstrap admission owner kind differs"));
    }
    read(&p, &owner.binding)
}

#[cfg(test)]
mod tests;
