//! Persistent metadata bootstrap grants, bound to one store and exact image.
//! These are not per-process Readers or portable recovery anchors. Their full
//! pins remain Published across restarts. Retirement requires independently
//! verified durable read fencing at the exact store, with no active boot jobs.
use super::*;
use kv9_common::StoreIdentity;
use kv9_engine::checkpoint::CheckpointManifest;

fn identity(
    root: &RootDescriptor,
    pending: OwnerToken,
    store: StoreIdentity,
) -> Result<(OwnerId, [u8; 32])> {
    store.verify(root, store.node_id)?;
    if pending.generation.get() != 1 {
        return Err(conflict("bootstrap Pending generation differs"));
    }
    identity_parts(
        root.digest(),
        pending,
        store.node_id,
        store.store_incarnation,
    )
}
fn identity_parts(
    root: RootDigest,
    pending: OwnerToken,
    node: kv9_common::NodeId,
    store: kv9_common::StoreIncarnation,
) -> Result<(OwnerId, [u8; 32])> {
    let mut identity = b"kv9-metadata-bootstrap-reader-v1".to_vec();
    identity.extend_from_slice(pending.owner.as_bytes());
    uint(&mut identity, node.0);
    identity.extend_from_slice(store.as_bytes());
    Ok((
        checkpoint_intent::owner_id(root, &identity, b"bootstrap")?,
        *RootDigest::sha256(&identity).as_bytes(),
    ))
}

pub fn binding(
    root: &RootDescriptor,
    manifest: &CheckpointManifest,
    predecessor: u64,
    store: StoreIdentity,
) -> Result<OwnerBinding> {
    store.verify(root, store.node_id)?;
    if manifest.scope.region != kv9_common::META_REGION_0.0 {
        return Err(invalid("bootstrap reader is only for metadata"));
    }
    let intent = CheckpointIntent::new(root, manifest, predecessor)?;
    let (id, operation) = identity(
        root,
        OwnerToken {
            owner: intent.pending_id(),
            generation: NonZeroU64::new(1).unwrap(),
        },
        store,
    )?;
    let mut binding = intent.binding(OwnerKind::Version)?;
    binding.descriptor.id = id;
    binding.descriptor.operation = operation;
    binding.descriptor.kind = OwnerKind::BootstrapReader;
    binding.validate(root.digest())?;
    Ok(binding)
}

fn planner<'a>(view: &'a dyn ReadView, root: &RootDescriptor) -> Result<Planner<'a>> {
    crate::checkpoint::inspect_initial_checkpoint_base(view, root)?;
    let h = view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .ok_or_else(|| invalid("bootstrap retirement header absent"))?;
    let (revision, rows_enabled) = read_header(&h, root.digest())?;
    if !migration_format(&h) {
        return Err(conflict("bootstrap retirement requires version 8 ledger"));
    }
    Ok(Planner {
        view,
        root: root.digest(),
        revision,
        rows_enabled,
        writes: BTreeMap::new(),
    })
}

fn candidate(
    p: &Planner<'_>,
    root: &RootDescriptor,
    pending: OwnerToken,
    store: StoreIdentity,
) -> Result<OwnerBinding> {
    let intent = checkpoint_intent::load(p, pending.owner)?
        .ok_or_else(|| invalid("bootstrap retirement intent absent"))?;
    if intent.binding(OwnerKind::Pending)?.token() != pending {
        return Err(conflict("bootstrap retirement pending differs"));
    }
    let version = intent.binding(OwnerKind::Version)?;
    let source = p.exact_owner(version.token())?;
    if source.binding != version || source.phase != PinPhase::Published {
        return Err(conflict(
            "bootstrap retirement lacks full Published Version",
        ));
    }
    binding(root, &intent.manifest()?, intent.predecessor(), store)
}

pub fn plan_retry(
    view: &dyn ReadView,
    root: &RootDescriptor,
    pending: OwnerToken,
    store: StoreIdentity,
) -> Result<LedgerPlan> {
    let p = planner(view, root)?;
    let (id, operation) = identity(root, pending, store)?;
    let old = p.exact_owner(OwnerToken {
        owner: id,
        generation: pending.generation,
    })?;
    // This kind cannot be generically released or advance to generation 2.
    // Its exact immutable terminal owner already records the verified fence;
    // acknowledgement must not require a still-Published source or its intent.
    // owner() continues to validate registration and every resource cross-link.
    if old.binding.descriptor.kind != OwnerKind::BootstrapReader
        || old.binding.descriptor.operation != operation
        || old.phase != PinPhase::Released
    {
        return Err(conflict("bootstrap retirement not yet committed"));
    }
    p.finish()
}

/// Trusted runtime entry only, after independently querying the exact store's
/// durable fence, checking completed boot/actual drain and rechecking its
/// endpoint under the catalog lock. Ordinary ledger requests cannot mint this
/// fact. Version keeps the complete old image; this retires only the read grant.
pub fn plan_verified(
    view: &dyn ReadView,
    root: &RootDescriptor,
    pending: OwnerToken,
    store: StoreIdentity,
) -> Result<LedgerPlan> {
    let mut p = planner(view, root)?;
    let binding = candidate(&p, root, pending, store)?;
    // Also terminally fence a prospective grant whose Share/Publish outcome
    // was unknown. Delayed entry cannot resurrect generation 1 after this.
    let managed = admission::read(&p, &binding)?.is_some();
    let old = p.owner(binding.descriptor.id)?;
    if managed && old.is_none() {
        return Err(invalid("orphan bootstrap admission at retirement"));
    }
    if old.is_none() {
        unentered::record(&mut p, root, pending, store, &binding)?;
    }
    checkpoint_reader::finish_covered(&mut p, &binding)?;
    if old.is_none() {
        p.writes.insert(
            owner_registration_key(binding.descriptor.id),
            unentered::registration(&binding),
        );
        p.writes.insert(
            owner_key(binding.descriptor.id),
            unentered::encode(&LedgerOwner {
                binding: binding.clone(),
                phase: PinPhase::Released,
            }),
        );
    }
    if managed && p.writes.contains_key(&owner_key(binding.descriptor.id)) {
        p.writes.insert(
            owner_key(binding.descriptor.id),
            admission::encode(&LedgerOwner {
                binding,
                phase: PinPhase::Released,
            }),
        );
    }
    p.finish()
}

#[cfg(test)]
pub(super) mod tests;

pub mod admission;
pub mod unentered;
