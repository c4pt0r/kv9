//! Per-process migration upload generations. Only atomic entry grants I/O;
//! actual local drain preserves PUT counts under complete Published coverage.
//! These facts do not certify remote termination or a complete publisher census.
use super::*;
use kv9_raft::snapshot_install::capture::CaptureDescriptor;

const MAGIC: &[u8; 8] = b"KV9MPB01";
fn record_key(token: OwnerToken) -> Vec<u8> {
    let mut id = token.owner.as_bytes().to_vec();
    uint(&mut id, token.generation.get());
    key(22, &id)
}
pub(super) fn check_old_format(view: &dyn ReadView) -> Result<()> {
    if view
        .iter(ColumnFamily::Default, &key(22, &[]), &key(23, &[]))?
        .next()
        .transpose()?
        .is_some()
    {
        return Err(invalid("migration publishers under an older ledger header"));
    }
    Ok(())
}

/// Derive exactly the long-lived pair already used by MigrationOwners: for a
/// migration the source snapshot and destination owners; for a BACKUP range
/// (its request names the synthetic destination, docs/BACKUP-RESTORE.md §12) the
/// source snapshot and the `Backup` owner, salted by the region.
pub fn image_owners(d: &CaptureDescriptor) -> Result<(OwnerBinding, OwnerBinding)> {
    let request = d.request();
    let root = request.range.root;
    let backup = request.destination.node_id == kv9_common::BACKUP_DESTINATION_NODE;
    let operation = if backup {
        crate::data_groups::backup_operation_digest(root, request.operation)
    } else {
        crate::data_groups::evidence::migration_operation_digest(root, request.operation)
    };
    let source = |kind, label: &[u8], salt: &[u8]| -> Result<OwnerBinding> {
        let mut id = label.to_vec();
        id.extend_from_slice(root.as_bytes());
        id.extend_from_slice(&operation);
        id.extend_from_slice(salt);
        let owner = OwnerBinding {
            descriptor: OwnerDescriptor {
                root,
                id: OwnerId::new(RootDigest::sha256(&id).as_bytes()[..16].try_into().unwrap())
                    .map_err(pin_error)?,
                kind,
                region: request.range.region.0,
                conf_ver: request.range.conf_ver,
                version: request.range.version,
                operation,
                subject: *RootDigest::sha256(&d.manifest().encode()?).as_bytes(),
                subject_is_anchor: false,
                local: None,
            },
            generation: NonZeroU64::new(1).unwrap(),
            resources: checkpoint_intent::manifest_resources(root, d.manifest())?,
        };
        owner.validate(root)?;
        Ok(owner)
    };
    if backup {
        return Ok((
            source(OwnerKind::Snapshot, b"kv9-backup-source-v1", &[])?,
            source(OwnerKind::Backup, b"kv9-backup-v1", &request.range.region.0.to_be_bytes())?,
        ));
    }
    Ok((
        source(OwnerKind::Snapshot, b"kv9-migration-source-v1", &[])?,
        source(
            OwnerKind::Migration,
            b"kv9-migration-destination-v1",
            request.destination.store_incarnation.as_bytes(),
        )?,
    ))
}
/// A publisher record names its 16-byte capture operation; its owner carries the
/// 32-byte ledger identity, derived the migration way or the backup way.
fn operation_matches(root: RootDigest, operation: [u8; 16], digest: [u8; 32]) -> bool {
    crate::data_groups::evidence::migration_operation_digest(root, operation) == digest
        || crate::data_groups::backup_operation_digest(root, operation) == digest
}

pub fn binding(d: &CaptureDescriptor, lifetime: LocalLifetime) -> Result<OwnerBinding> {
    let (mut publisher, _) = image_owners(d)?;
    let root = publisher.descriptor.root;
    let mut id = b"kv9-migration-publisher-v1".to_vec();
    id.extend_from_slice(d.digest().as_bytes());
    uint(&mut id, lifetime.node);
    id.extend_from_slice(&lifetime.store);
    id.extend_from_slice(&lifetime.process);
    publisher.descriptor.id = checkpoint_intent::owner_id(root, &id, b"publisher")?;
    publisher.descriptor.kind = OwnerKind::Publisher;
    publisher.descriptor.local = Some(lifetime);
    publisher.validate(root)?;
    Ok(publisher)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublisherGeneration {
    pub operation: [u8; 16],
    pub publisher: OwnerToken,
    /// Zero only for a terminal generation which was never admitted.
    pub admitted_at: u64,
    /// Zero while active; otherwise an actual drained/cancelled observation.
    pub finished_at: u64,
    pub issued: u64,
    pub resolved: u64,
    binding: RootDigest,
}
impl PublisherGeneration {
    fn encode(&self, root: RootDigest) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(root.as_bytes());
        b.extend_from_slice(&self.operation);
        b.extend_from_slice(self.publisher.owner.as_bytes());
        uint(&mut b, self.publisher.generation.get());
        b.extend_from_slice(self.binding.as_bytes());
        for n in [
            self.admitted_at,
            self.finished_at,
            self.issued,
            self.resolved,
        ] {
            uint(&mut b, n);
        }
        seal(b)
    }
    fn decode(bytes: &[u8], root: RootDigest) -> Result<Self> {
        let mut r = Reader(checked(bytes, MAGIC, 176)?);
        if r.array::<32>()? != *root.as_bytes() {
            return Err(invalid("publisher root differs"));
        }
        let record = Self {
            operation: r.array()?,
            publisher: OwnerToken {
                owner: OwnerId::new(r.array()?).map_err(pin_error)?,
                generation: NonZeroU64::new(r.uint()?)
                    .ok_or_else(|| invalid("publisher generation zero"))?,
            },
            binding: RootDigest::from_bytes(r.array()?),
            admitted_at: r.uint()?,
            finished_at: r.uint()?,
            issued: r.uint()?,
            resolved: r.uint()?,
        };
        if !r.0.is_empty()
            || record.operation == [0; 16]
            || record.resolved > record.issued
            || (record.admitted_at == 0 && (record.finished_at == 0 || record.issued != 0))
            || (record.finished_at == 0 && record.issued != 0)
            || (record.finished_at != 0 && record.finished_at <= record.admitted_at)
            || record.encode(root) != bytes
        {
            return Err(invalid("publisher generation encoding or counters differ"));
        }
        Ok(record)
    }
}
fn read(
    p: &Planner<'_>,
    owner: &OwnerBinding,
    token: OwnerToken,
) -> Result<Option<PublisherGeneration>> {
    let Some(bytes) = p.view.get(ColumnFamily::Default, &record_key(token))? else {
        return Ok(None);
    };
    if !p
        .view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .is_some_and(|b| migration_format(&b))
    {
        return Err(invalid("publisher requires version 8 ledger"));
    }
    let record = PublisherGeneration::decode(&bytes, p.root)?;
    if record.publisher != token
        || token.owner != owner.descriptor.id
        || record.binding != RootDigest::sha256(&owner_registration(owner))
        || !operation_matches(p.root, record.operation, owner.descriptor.operation)
        || record.admitted_at > p.revision
        || record.finished_at > p.revision
    {
        return Err(invalid("publisher generation links differ"));
    }
    Ok(Some(record))
}
pub(super) fn check(p: &Planner<'_>, owner: &LedgerOwner) -> Result<()> {
    if owner.binding.descriptor.kind != OwnerKind::Publisher {
        return Ok(());
    }
    let record = read(p, &owner.binding, owner.binding.token())?
        .ok_or_else(|| invalid("publisher enrollment missing"))?;
    let expected = if record.finished_at == 0 {
        PinPhase::Published
    } else {
        PinPhase::Released
    };
    if owner.phase != expected {
        return Err(invalid("publisher phase disagrees with generation"));
    }
    Ok(())
}
/// Bounded point lookup. Missing earlier facts remain unknown, never zero PUT.
pub fn generation(
    view: &dyn ReadView,
    root: &RootDescriptor,
    token: OwnerToken,
) -> Result<Option<PublisherGeneration>> {
    crate::checkpoint::inspect_initial_checkpoint_base(view, root)?;
    let h = view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .ok_or_else(|| invalid("publisher ledger absent"))?;
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
        .ok_or_else(|| conflict("publisher absent"))?;
    if owner.binding.descriptor.kind != OwnerKind::Publisher
        || token.generation > owner.binding.generation
    {
        return Err(conflict("publisher history bound differs"));
    }
    read(&p, &owner.binding, token)
}
fn expected(
    p: &Planner<'_>,
    root: &RootDescriptor,
    operation: [u8; 16],
    publisher: &OwnerBinding,
) -> Result<(LedgerOwner, LedgerOwner)> {
    publisher.validate(p.root)?;
    let capture = crate::data_groups::capture::committed_capture(p.view, root, operation)?
        .ok_or_else(|| conflict("publisher capture catalog missing"))?;
    let d = capture.descriptor();
    let mut wanted = binding(
        d,
        publisher
            .descriptor
            .local
            .ok_or_else(|| invalid("publisher lifetime missing"))?,
    )?;
    wanted.generation = publisher.generation;
    if wanted != *publisher {
        return Err(conflict("publisher differs from exact capture"));
    }
    let (source, destination) = image_owners(d)?;
    let actual_source = p.exact_owner(source.token())?;
    let actual_destination = p.exact_owner(destination.token())?;
    if actual_source.binding != source || actual_destination.binding != destination {
        return Err(conflict("publisher image owner binding differs"));
    }
    Ok((actual_source, actual_destination))
}
fn next_revision(p: &Planner<'_>) -> Result<u64> {
    p.revision
        .checked_add(1)
        .ok_or_else(|| invalid("publisher revision exhausted"))
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
                    return Err(invalid("orphan publisher pin"));
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
        _ => Err(conflict("publisher generation does not follow release")),
    }
}
fn write(
    p: &mut Planner<'_>,
    binding: &OwnerBinding,
    previous: Option<OwnerSlot>,
    record: &PublisherGeneration,
) -> Result<()> {
    let owner = LedgerOwner {
        binding: binding.clone(),
        phase: if record.finished_at == 0 {
            PinPhase::Published
        } else {
            PinPhase::Released
        },
    };
    p.update_resources(&owner, previous, owner.phase)?;
    p.writes.insert(
        owner_registration_key(binding.descriptor.id),
        owner_registration(binding),
    );
    p.writes
        .insert(owner_key(binding.descriptor.id), encode_owner(&owner));
    p.writes
        .insert(record_key(binding.token()), record.encode(p.root));
    Ok(())
}
pub(super) fn enter(
    p: &mut Planner<'_>,
    root: &RootDescriptor,
    operation: [u8; 16],
    binding: &OwnerBinding,
) -> Result<()> {
    metadata_image_write_gate::check_entry(p, binding)?;
    let (source, destination) = expected(p, root, operation, binding)?;
    if source.phase != PinPhase::Published
        || destination.phase != PinPhase::Published
        || crate::data_groups::abort::abort_matches_owner_pair(
            p.view,
            p.root,
            source.binding.descriptor.operation,
            source.binding.descriptor.subject,
        )?
    {
        return Err(conflict(
            "publisher admission lacks live migration protection",
        ));
    }
    let old = p.owner(binding.descriptor.id)?;
    if let Some(old) = &old {
        if old.binding == *binding && old.phase == PinPhase::Published {
            return Ok(());
        }
    }
    let previous = previous(p, binding, old.as_ref())?;
    if read(p, binding, binding.token())?.is_some() {
        return Err(invalid("orphan publisher generation"));
    }
    let record = PublisherGeneration {
        operation,
        publisher: binding.token(),
        binding: RootDigest::sha256(&owner_registration(binding)),
        admitted_at: next_revision(p)?,
        finished_at: 0,
        issued: 0,
        resolved: 0,
    };
    write(p, binding, previous, &record)
}
pub(super) fn finish(
    p: &mut Planner<'_>,
    root: &RootDescriptor,
    operation: [u8; 16],
    binding: &OwnerBinding,
    issued: u64,
    resolved: u64,
) -> Result<()> {
    if resolved > issued {
        return Err(invalid("publisher resolved exceeds issued"));
    }
    binding.validate(p.root)?;
    if binding.descriptor.kind != OwnerKind::Publisher
        || operation == [0; 16]
        || !operation_matches(p.root, operation, binding.descriptor.operation)
    {
        return Err(conflict("publisher drain protocol or operation differs"));
    }
    let old = p.owner(binding.descriptor.id)?;
    let prior = read(p, binding, binding.token())?;
    if let Some(record) = &prior {
        if record.finished_at != 0 {
            if record.operation != operation
                || record.issued != issued
                || record.resolved != resolved
                || !old.as_ref().is_some_and(|o| {
                    o.binding.descriptor == binding.descriptor
                        && o.binding.resources == binding.resources
                        && o.binding.generation >= binding.generation
                })
            {
                return Err(conflict("publisher drain observation is immutable"));
            }
            return Ok(());
        }
    }
    // A first finish still requires the exact capture and complete live source
    // coverage. Only an already committed terminal record bypasses this path.
    let (source, destination) = expected(p, root, operation, binding)?;
    if source.phase != PinPhase::Published && destination.phase != PinPhase::Published {
        return Err(conflict(
            "publisher drain lacks complete Published coverage",
        ));
    }
    let (previous, mut record) = if let Some(old) = &old {
        if old.binding == *binding && old.phase == PinPhase::Published {
            (
                Some(OwnerSlot {
                    generation: binding.generation,
                    phase: old.phase,
                }),
                prior.ok_or_else(|| invalid("publisher enrollment absent"))?,
            )
        } else {
            let previous = previous(p, binding, Some(old))?;
            if prior.is_some() || issued != 0 {
                return Err(conflict("unentered publisher cannot have PUTs"));
            }
            (
                previous,
                PublisherGeneration {
                    operation,
                    publisher: binding.token(),
                    binding: RootDigest::sha256(&owner_registration(binding)),
                    admitted_at: 0,
                    finished_at: 0,
                    issued: 0,
                    resolved: 0,
                },
            )
        }
    } else {
        let previous = previous(p, binding, None)?;
        if prior.is_some() || issued != 0 {
            return Err(conflict("unentered publisher cannot have PUTs"));
        }
        (
            previous,
            PublisherGeneration {
                operation,
                publisher: binding.token(),
                binding: RootDigest::sha256(&owner_registration(binding)),
                admitted_at: 0,
                finished_at: 0,
                issued: 0,
                resolved: 0,
            },
        )
    };
    record.finished_at = next_revision(p)?;
    record.issued = issued;
    record.resolved = resolved;
    write(p, binding, previous, &record)
}

#[cfg(test)]
mod tests;
