//! Bounded census of persistent bootstrap grants for an exact metadata image.
//! Current typed terminal grants are distinguishable from legacy/missing origin.
//! No complete deployment, external-reader or Version-release authority follows.
use super::*;
use metadata_read_gate::ReadFence;
use std::collections::BTreeSet;

const MAGIC: &[u8; 8] = b"KV9BCL01";
const STAMP_MAGIC: &[u8; 8] = b"KV9BCM01";
const MAX_BYTES: usize = 224;
fn progress_key(image: [u8; 32], pending: OwnerId) -> Vec<u8> {
    let mut suffix = image.to_vec();
    suffix.extend_from_slice(pending.as_bytes());
    key(34, &suffix)
}
fn stamp_key(image: [u8; 32]) -> Vec<u8> {
    key(33, &image)
}
fn end(mut prefix: Vec<u8>) -> Vec<u8> {
    while prefix.last() == Some(&255) {
        prefix.pop();
    }
    *prefix.last_mut().unwrap() += 1;
    prefix
}
fn image(intent: &CheckpointIntent) -> [u8; 32] {
    *RootDigest::sha256(intent.manifest_bytes()).as_bytes()
}
fn inc(n: &mut u64) -> Result<()> {
    *n = n
        .checked_add(1)
        .ok_or_else(|| invalid("bootstrap closure count exhausted"))?;
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counts {
    pub grants: u64,
    pub retired: u64,
    pub active: u64,
    pub legacy: u64,
    /// Informational subset of typed grants from another exact Pending for
    /// this same image. They are still included in retired/active, never skipped.
    pub other_sources: u64,
}
impl Counts {
    fn numbers(&self) -> [u64; 5] {
        [
            self.grants,
            self.retired,
            self.active,
            self.legacy,
            self.other_sources,
        ]
    }
    fn valid(&self) -> bool {
        self.retired
            .checked_add(self.active)
            .and_then(|v| v.checked_add(self.legacy))
            == Some(self.grants)
            && self
                .retired
                .checked_add(self.active)
                .is_some_and(|n| self.other_sources <= n)
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cursor {
    pub sequence: u64,
    pub source_revision: u64,
    pub fence_revision: u64,
    pub counts: Counts,
    pub done: bool,
    after: Option<OwnerId>,
}
impl Cursor {
    pub fn request(&self, pending: OwnerId) -> LedgerRequest {
        LedgerRequest::AdvanceBootstrapClosure {
            pending: OwnerToken {
                owner: pending,
                generation: NonZeroU64::new(1).unwrap(),
            },
            expected: self.sequence,
        }
    }
    fn encode(&self, root: RootDigest, image: [u8; 32], fence: &ReadFence) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(root.as_bytes());
        b.extend_from_slice(&image);
        b.extend_from_slice(fence.pending.as_bytes());
        for n in [
            fence.closed_at,
            self.fence_revision,
            self.sequence,
            self.source_revision,
        ] {
            uint(&mut b, n);
        }
        b.push(u8::from(self.done));
        b.push(u8::from(self.after.is_some()));
        if let Some(id) = self.after {
            b.extend_from_slice(id.as_bytes());
        }
        for n in self.counts.numbers() {
            uint(&mut b, n);
        }
        seal(b)
    }
}
pub(super) fn check_old_format(view: &dyn ReadView) -> Result<()> {
    if view
        .iter(ColumnFamily::Default, &key(33, &[]), &key(35, &[]))?
        .next()
        .transpose()?
        .is_some()
    {
        return Err(invalid("bootstrap closure under an older ledger header"));
    }
    Ok(())
}
fn stamp(p: &Planner<'_>, image: [u8; 32]) -> Result<Option<u64>> {
    let Some(bytes) = p.view.get(ColumnFamily::Default, &stamp_key(image))? else {
        // A single bounded lookahead detects any aggregate that lost its image
        // stamp. It is not a scan over attempts and never repairs lost authority.
        let prefix = key(34, &image);
        if p.view
            .iter(ColumnFamily::Default, &prefix, &end(prefix.clone()))?
            .next()
            .transpose()?
            .is_some()
        {
            return Err(invalid("bootstrap closure lost its image stamp"));
        }
        return Ok(None);
    };
    if !p
        .view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .is_some_and(|h| bootstrap_closure_format(&h))
    {
        return Err(invalid(
            "bootstrap closure stamp requires version 16 ledger",
        ));
    }
    let mut r = Reader(checked(&bytes, STAMP_MAGIC, 112)?);
    if r.array::<32>()? != *p.root.as_bytes() || r.array::<32>()? != image {
        return Err(invalid("bootstrap closure stamp identity differs"));
    }
    let revision = r.uint()?;
    if !r.0.is_empty() || revision == 0 || revision > p.revision {
        return Err(invalid("bootstrap closure stamp revision differs"));
    }
    Ok(Some(revision))
}
fn write_stamp(p: &mut Planner<'_>, image: [u8; 32]) -> Result<u64> {
    stamp(p, image)?;
    let revision = p
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid("bootstrap closure revision exhausted"))?;
    let mut b = STAMP_MAGIC.to_vec();
    b.extend_from_slice(p.root.as_bytes());
    b.extend_from_slice(&image);
    uint(&mut b, revision);
    p.writes.insert(stamp_key(image), seal(b));
    Ok(revision)
}
/// Inspect only the current bounded mutation batch. All grants for the same
/// image, even from another Pending, share invalidation of their census views.
pub(super) fn mark_changes(p: &mut Planner<'_>) -> Result<()> {
    let mut images = BTreeSet::new();
    for (k, v) in &p.writes {
        let Some(suffix) = k.strip_prefix(PREFIX) else {
            continue;
        };
        let (&tag, id) = suffix
            .split_first()
            .ok_or_else(|| invalid("bootstrap closure write key"))?;
        match tag {
            3 | 5 | 28 | 35 => {
                let owner_id = OwnerId::new(
                    id.try_into()
                        .map_err(|_| invalid("bootstrap closure owner key"))?,
                )
                .map_err(pin_error)?;
                let stored;
                let bytes = if matches!(tag, 28 | 35) {
                    stored = match p.writes.get(&owner_key(owner_id)) {
                        Some(b) => b.clone(),
                        None => p
                            .view
                            .get(ColumnFamily::Default, &owner_key(owner_id))?
                            .ok_or_else(|| invalid("bootstrap origin lacks owner"))?,
                    };
                    &stored
                } else {
                    v
                };
                let owner = decode_owner(bytes, p.root, owner_id)?;
                if owner.binding.descriptor.kind == OwnerKind::BootstrapReader {
                    images.insert(owner.binding.descriptor.subject);
                }
            }
            29 => {
                if id.len() != 48 {
                    return Err(invalid("bootstrap index write key"));
                }
                images.insert(id[..32].try_into().unwrap());
            }
            _ => {}
        }
    }
    for image in images {
        write_stamp(p, image)?;
    }
    Ok(())
}
fn cursor(p: &Planner<'_>, intent: &CheckpointIntent, fence: &ReadFence) -> Result<Cursor> {
    let Some(bytes) = p.view.get(
        ColumnFamily::Default,
        &progress_key(image(intent), intent.pending_id()),
    )?
    else {
        return Ok(Cursor {
            fence_revision: fence.recorded_at,
            ..Cursor::default()
        });
    };
    let mut r = Reader(checked(&bytes, MAGIC, MAX_BYTES)?);
    if r.array::<32>()? != *p.root.as_bytes()
        || r.array::<32>()? != image(intent)
        || r.array::<16>()? != *intent.pending_id().as_bytes()
        || r.uint()? != fence.closed_at
        || r.uint()? != fence.recorded_at
    {
        return Err(invalid("bootstrap closure identity or fence differs"));
    }
    let sequence = r.uint()?;
    let source_revision = r.uint()?;
    let done = r.boolean()?;
    let after = if r.boolean()? {
        Some(OwnerId::new(r.array()?).map_err(pin_error)?)
    } else {
        None
    };
    let counts = Counts {
        grants: r.uint()?,
        retired: r.uint()?,
        active: r.uint()?,
        legacy: r.uint()?,
        other_sources: r.uint()?,
    };
    let c = Cursor {
        sequence,
        source_revision,
        done,
        after,
        counts,
        fence_revision: fence.recorded_at,
    };
    if !r.0.is_empty()
        || sequence == 0
        || sequence > p.revision
        || source_revision == 0
        || source_revision > p.revision
        || source_revision < fence.recorded_at
        || !c.counts.valid()
        || (c.counts.grants == 0) != after.is_none()
        || (!done && after.is_none())
        || c.encode(p.root, image(intent), fence) != bytes
    {
        return Err(invalid("bootstrap closure cursor bounds or encoding"));
    }
    Ok(c)
}
fn context<'a>(
    view: &'a dyn ReadView,
    root: &RootDescriptor,
    pending: OwnerId,
) -> Result<(Planner<'a>, CheckpointIntent)> {
    let intent = checkpoint_intent::checkpoint_intent(view, root, pending)?
        .ok_or_else(|| invalid("bootstrap closure intent absent"))?;
    let h = view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .ok_or_else(|| invalid("bootstrap closure ledger absent"))?;
    let (revision, rows_enabled) = read_header(&h, root.digest())?;
    Ok((
        Planner {
            view,
            root: root.digest(),
            revision,
            rows_enabled,
            writes: BTreeMap::new(),
        },
        intent,
    ))
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Work {
    Open,
    WaitingForCatalog,
    Advance { cursor: Cursor, restart: bool },
    Complete(Cursor),
}
impl Work {
    /// Only the registered grants in this exact current image census. Other
    /// Pending aliases may admit new grants and will invalidate this result.
    pub fn registered_grants_retired(&self) -> bool {
        matches!(self,Self::Complete(c) if c.counts.retired==c.counts.grants)
    }
}
pub fn observe(view: &dyn ReadView, root: &RootDescriptor, pending: OwnerId) -> Result<Work> {
    let Some(fence) = metadata_read_gate::fence(view, root, pending)? else {
        return Ok(Work::Open);
    };
    let (p, intent) = context(view, root, pending)?;
    if intent.manifest()?.scope.region != kv9_common::META_REGION_0.0 {
        return Ok(Work::Open);
    }
    if !matches!(
        bootstrap_index::observe(view, root)?,
        bootstrap_index::Work::Complete
    ) {
        return Ok(Work::WaitingForCatalog);
    }
    let changed = stamp(&p, image(&intent))?;
    let c = cursor(&p, &intent, &fence)?;
    let effective = changed.map(|s| s.max(fence.recorded_at));
    if effective.is_some_and(|s| s < c.source_revision) {
        return Err(invalid("bootstrap closure source stamp regressed"));
    }
    let restart = c.sequence != 0 && effective != Some(c.source_revision);
    Ok(if c.done && !restart {
        Work::Complete(c)
    } else {
        Work::Advance { cursor: c, restart }
    })
}
fn count(
    p: &Planner<'_>,
    root: &RootDescriptor,
    intent: &CheckpointIntent,
    fence: &ReadFence,
    counts: &mut Counts,
    grant: bootstrap_index::Grant,
) -> Result<()> {
    inc(&mut counts.grants)?;
    let (pending, version_token, node, store, operation, entered_at) =
        match (grant.admission, grant.unentered) {
            (Some(a), None) => (
                a.pending,
                a.version,
                a.node,
                a.store,
                a.checkpoint_operation,
                Some(a.entered_at),
            ),
            (None, Some(u)) if grant.owner.phase == PinPhase::Released => (
                u.pending,
                u.version,
                u.node,
                u.store,
                u.checkpoint_operation,
                None,
            ),
            (None, None) => {
                inc(&mut counts.legacy)?;
                return Ok(());
            }
            _ => return Err(invalid("bootstrap census origin or terminal phase differs")),
        };
    // Historical source identity, not a new read authorization. A typed grant
    // may already have retired before its source transfers elsewhere. Decode
    // its retained canonical intent without demanding a still-Published source.
    let bytes = p
        .view
        .get(ColumnFamily::Default, &key(8, pending.owner.as_bytes()))?
        .ok_or_else(|| invalid("bootstrap origin source intent absent"))?;
    let source = CheckpointIntent::decode(&bytes, root, pending.owner)?;
    let manifest = source.manifest()?;
    let canonical = CheckpointIntent::new(root, &manifest, source.predecessor())?;
    let version = source.binding(OwnerKind::Version)?;
    let identity = kv9_common::StoreIdentity {
        root_digest: p.root,
        cluster_id: root.cluster_id,
        node_id: node,
        store_incarnation: store,
    };
    let expected = bootstrap_reader::binding(root, &manifest, source.predecessor(), identity)?;
    if source != canonical
        || source.manifest_bytes() != intent.manifest_bytes()
        || version_token != version.token()
        || operation != version.descriptor.operation
        || grant.owner.binding != expected
        || root.voter(node).is_none()
    {
        return Err(invalid("bootstrap census exact source or binding differs"));
    }
    if pending.owner == intent.pending_id() {
        if entered_at.is_some_and(|at| at >= fence.recorded_at) {
            return Err(invalid("bootstrap entry is not before its read fence"));
        }
    } else {
        inc(&mut counts.other_sources)?;
    }
    match grant.owner.phase {
        PinPhase::Released => inc(&mut counts.retired)?,
        PinPhase::Held | PinPhase::Published => inc(&mut counts.active)?,
        _ => return Err(invalid("typed bootstrap phase differs")),
    };
    Ok(())
}
pub(super) fn advance(
    p: &mut Planner<'_>,
    root: &RootDescriptor,
    pending: OwnerToken,
    expected: u64,
) -> Result<()> {
    if pending.generation.get() != 1 {
        return Err(conflict("bootstrap closure Pending generation"));
    }
    let (mut c, restart) = match observe(p.view, root, pending.owner)? {
        Work::Advance { cursor, restart } => (cursor, restart),
        Work::Complete(c) => (c, false),
        _ => {
            return Err(conflict(
                "bootstrap closure requires read fence and complete bootstrap catalog",
            ))
        }
    };
    if expected.checked_add(1) == Some(c.sequence) {
        return Ok(());
    }
    if expected != c.sequence {
        return Err(conflict("bootstrap closure sequence differs"));
    }
    if c.done && !restart {
        return Ok(());
    }
    let (_, intent) = context(p.view, root, pending.owner)?;
    let fence = metadata_read_gate::fence(p.view, root, pending.owner)?
        .ok_or_else(|| invalid("bootstrap closure fence absent"))?;
    let sequence = c
        .sequence
        .checked_add(1)
        .ok_or_else(|| invalid("bootstrap closure sequence exhausted"))?;
    let source = match stamp(p, image(&intent))? {
        Some(s) => s,
        None => write_stamp(p, image(&intent))?,
    }
    .max(fence.recorded_at);
    if restart {
        c = Cursor {
            fence_revision: fence.recorded_at,
            ..Cursor::default()
        };
    }
    c.sequence = sequence;
    c.source_revision = source;
    let page = bootstrap_index::grants(p.view, root, image(&intent), c.after)?;
    for grant in page.grants {
        c.after = Some(grant.owner.binding.descriptor.id);
        count(p, root, &intent, &fence, &mut c.counts, grant)?;
    }
    c.done = page.next.is_none();
    if !c.done && page.next.map(|a| a.owner) != c.after {
        return Err(invalid("bootstrap closure page cursor differs"));
    }
    p.writes.insert(
        progress_key(image(&intent), pending.owner),
        c.encode(p.root, image(&intent), &fence),
    );
    Ok(())
}
#[cfg(test)]
mod tests;
