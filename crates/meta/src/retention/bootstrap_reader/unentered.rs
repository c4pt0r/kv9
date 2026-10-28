//! First independently verified retirement of a grant that never entered.
//! Existing legacy owners cannot acquire this provenance retroactively.
use super::*;
use kv9_common::{NodeId, StoreIncarnation};

pub(crate) const OWNER_MAGIC: &[u8; 8] = b"KV9BUO01";
const MAGIC: &[u8; 8] = b"KV9BUT01";
fn record_key(id: OwnerId) -> Vec<u8> {
    key(35, id.as_bytes())
}
pub(in crate::retention) fn check_old_format(view: &dyn ReadView) -> Result<()> {
    if view
        .iter(ColumnFamily::Default, &key(35, &[]), &key(36, &[]))?
        .next()
        .transpose()?
        .is_some()
    {
        return Err(invalid(
            "bootstrap unentered fact under an older ledger header",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unentered {
    pub pending: OwnerToken,
    pub version: OwnerToken,
    pub node: NodeId,
    pub store: StoreIncarnation,
    pub fenced_at: u64,
    pub checkpoint_operation: [u8; 32],
    binding: RootDigest,
}
impl Unentered {
    fn encode(&self, root: RootDigest) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(root.as_bytes());
        for t in [self.pending, self.version] {
            b.extend_from_slice(t.owner.as_bytes());
            uint(&mut b, t.generation.get());
        }
        uint(&mut b, self.node.0);
        b.extend_from_slice(self.store.as_bytes());
        b.extend_from_slice(self.binding.as_bytes());
        uint(&mut b, self.fenced_at);
        b.extend_from_slice(&self.checkpoint_operation);
        seal(b)
    }
    fn decode(bytes: &[u8], root: RootDigest) -> Result<Self> {
        let mut r = Reader(checked(bytes, MAGIC, 216)?);
        if r.array::<32>()? != *root.as_bytes() {
            return Err(invalid("bootstrap unentered root differs"));
        }
        let token = |r: &mut Reader<'_>| -> Result<OwnerToken> {
            Ok(OwnerToken {
                owner: OwnerId::new(r.array()?).map_err(pin_error)?,
                generation: NonZeroU64::new(r.uint()?)
                    .filter(|g| g.get() == 1)
                    .ok_or_else(|| invalid("bootstrap unentered generation differs"))?,
            })
        };
        let fact = Self {
            pending: token(&mut r)?,
            version: token(&mut r)?,
            node: NodeId(r.uint()?),
            store: StoreIncarnation::from_bytes(r.array()?),
            binding: RootDigest::from_bytes(r.array()?),
            fenced_at: r.uint()?,
            checkpoint_operation: r.array()?,
        };
        if !r.0.is_empty()
            || fact.node.0 == 0
            || fact.store.as_bytes() == &[0; 16]
            || fact.fenced_at == 0
            || fact.pending == fact.version
            || fact.encode(root) != bytes
        {
            return Err(invalid("bootstrap unentered encoding differs"));
        }
        Ok(fact)
    }
}
pub(in crate::retention) fn read(
    p: &Planner<'_>,
    binding: &OwnerBinding,
) -> Result<Option<Unentered>> {
    let Some(bytes) = p
        .view
        .get(ColumnFamily::Default, &record_key(binding.descriptor.id))?
    else {
        return Ok(None);
    };
    if !p
        .view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .is_some_and(|h| bootstrap_unentered_format(&h))
    {
        return Err(invalid(
            "bootstrap unentered fact requires version 17 ledger",
        ));
    }
    let fact = Unentered::decode(&bytes, p.root)?;
    let (id, operation) = identity_parts(p.root, fact.pending, fact.node, fact.store)?;
    if binding.descriptor.kind != OwnerKind::BootstrapReader
        || binding.descriptor.id != id
        || binding.descriptor.operation != operation
        || fact.binding != RootDigest::sha256(&owner_registration(binding))
        || fact.fenced_at > p.revision
        || checkpoint_intent::owner_id(
            p.root,
            &fact.checkpoint_operation,
            b"kv9-checkpoint-pending-v1",
        )? != fact.pending.owner
        || checkpoint_intent::owner_id(
            p.root,
            &fact.checkpoint_operation,
            b"kv9-checkpoint-version-v1",
        )? != fact.version.owner
    {
        return Err(invalid("bootstrap unentered binding differs"));
    }
    Ok(Some(fact))
}
pub(in crate::retention) fn encode(owner: &LedgerOwner) -> Vec<u8> {
    let mut b = encode_owner(owner);
    b.truncate(b.len() - 32);
    b[..8].copy_from_slice(OWNER_MAGIC);
    seal(b)
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
    marked: bool,
) -> Result<()> {
    if owner.binding.descriptor.kind != OwnerKind::BootstrapReader {
        return if marked {
            Err(invalid("bootstrap unentered owner kind differs"))
        } else {
            Ok(())
        };
    }
    match (marked, read(p, &owner.binding)?) {
        (false, None) => Ok(()),
        (true, Some(_)) if owner.phase == PinPhase::Released => Ok(()),
        _ => Err(invalid("bootstrap unentered owner and fact disagree")),
    }
}
pub(super) fn record(
    p: &mut Planner<'_>,
    root: &RootDescriptor,
    pending: OwnerToken,
    store: StoreIdentity,
    binding: &OwnerBinding,
) -> Result<()> {
    store.verify(root, store.node_id)?;
    if root.voter(store.node_id).is_none() {
        return Err(conflict(
            "unentered bootstrap store is not a provisioned metadata voter",
        ));
    }
    // Neither a missing owner nor Released is sufficient evidence of no entry.
    // Refuse all surviving provenance/index/pin fragments; never repair them.
    if p.owner(binding.descriptor.id)?.is_some()
        || admission::read(p, binding)?.is_some()
        || read(p, binding)?.is_some()
    {
        return Err(invalid("bootstrap unentered grant already has history"));
    }
    bootstrap_index::require_unregistered(p, binding)?;
    for resource in &binding.resources {
        if p.resource(*resource)?
            .owner(p.view, binding.descriptor.id)?
            .is_some()
        {
            return Err(invalid("bootstrap unentered grant has orphan pin"));
        }
    }
    let intent = checkpoint_intent::load(p, pending.owner)?
        .ok_or_else(|| invalid("bootstrap unentered intent absent"))?;
    let version = intent.binding(OwnerKind::Version)?;
    let fact = Unentered {
        pending,
        version: version.token(),
        node: store.node_id,
        store: store.store_incarnation,
        fenced_at: p
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("bootstrap revision exhausted"))?,
        checkpoint_operation: version.descriptor.operation,
        binding: RootDigest::sha256(&owner_registration(binding)),
    };
    p.writes
        .insert(record_key(binding.descriptor.id), fact.encode(p.root));
    Ok(())
}
/// Terminal provenance only: this never authorizes a bootstrap GET.
pub fn observe(
    view: &dyn ReadView,
    root: &RootDescriptor,
    token: OwnerToken,
) -> Result<Option<Unentered>> {
    let p = planner(view, root)?;
    let owner = p.exact_owner(token)?;
    if owner.binding.descriptor.kind != OwnerKind::BootstrapReader {
        return Err(conflict("bootstrap unentered owner kind differs"));
    }
    read(&p, &owner.binding)
}

#[cfg(test)]
mod tests;
