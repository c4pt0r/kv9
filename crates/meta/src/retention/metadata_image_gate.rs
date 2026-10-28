//! Immutable closure of new catalog reads for one exact metadata image.
//! A closed all-root-store checkpoint gate is the prerequisite. Existing grants
//! still require independent retirement/drain; no Version or GC authority follows.
use super::*;

const MAGIC: &[u8; 8] = b"KV9MIG01";
const MARKER_MAGIC: &[u8; 8] = b"KV9MIM01";
fn gate_key(image: [u8; 32]) -> Vec<u8> {
    key(36, &image)
}
fn marker_key(image: [u8; 32]) -> Vec<u8> {
    key(37, &image)
}
fn marker(root: RootDigest, image: [u8; 32], gate: &[u8]) -> Vec<u8> {
    let mut b = MARKER_MAGIC.to_vec();
    b.extend_from_slice(root.as_bytes());
    b.extend_from_slice(&image);
    b.extend_from_slice(RootDigest::sha256(gate).as_bytes());
    seal(b)
}
fn record_bytes(p: &Planner<'_>, image: [u8; 32]) -> Result<Option<Vec<u8>>> {
    match (
        p.view.get(ColumnFamily::Default, &gate_key(image))?,
        p.view.get(ColumnFamily::Default, &marker_key(image))?,
    ) {
        (None, None) => Ok(None),
        (Some(gate), Some(m)) if m == marker(p.root, image, &gate) => Ok(Some(gate)),
        _ => Err(invalid("metadata image gate and closure marker disagree")),
    }
}
fn subject(intent: &CheckpointIntent) -> [u8; 32] {
    *RootDigest::sha256(intent.manifest_bytes()).as_bytes()
}
pub(super) fn check_old_format(view: &dyn ReadView) -> Result<()> {
    if view
        .iter(ColumnFamily::Default, &key(36, &[]), &key(38, &[]))?
        .next()
        .transpose()?
        .is_some()
    {
        return Err(invalid("metadata image gate under an older ledger header"));
    }
    Ok(())
}
fn planner<'a>(view: &'a dyn ReadView, root: &RootDescriptor) -> Result<Planner<'a>> {
    crate::checkpoint::inspect_initial_checkpoint_base(view, root)?;
    let h = view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .ok_or_else(|| invalid("image gate ledger absent"))?;
    let (revision, rows_enabled) = read_header(&h, root.digest())?;
    Ok(Planner {
        view,
        root: root.digest(),
        revision,
        rows_enabled,
        writes: BTreeMap::new(),
    })
}
fn intent(
    p: &Planner<'_>,
    root: &RootDescriptor,
    pending: OwnerId,
) -> Result<Option<CheckpointIntent>> {
    let Some(bytes) = p
        .view
        .get(ColumnFamily::Default, &key(8, pending.as_bytes()))?
    else {
        return Ok(None);
    };
    let i = CheckpointIntent::decode(&bytes, root, pending)?;
    if i.encode() != bytes {
        return Err(invalid("metadata image gate intent is not canonical"));
    }
    Ok(Some(i))
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fence {
    pub image: [u8; 32],
    pub source: OwnerId,
    pub source_gate_revision: u64,
    pub closed_at: u64,
    intent_digest: RootDigest,
}
impl Fence {
    fn encode(&self, root: RootDigest) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(root.as_bytes());
        b.extend_from_slice(&self.image);
        b.extend_from_slice(self.source.as_bytes());
        b.extend_from_slice(self.intent_digest.as_bytes());
        uint(&mut b, self.source_gate_revision);
        uint(&mut b, self.closed_at);
        seal(b)
    }
}
fn read(p: &Planner<'_>, root: &RootDescriptor, image: [u8; 32]) -> Result<Option<Fence>> {
    let Some(bytes) = record_bytes(p, image)? else {
        return Ok(None);
    };
    if !p
        .view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .is_some_and(|h| metadata_image_gate_format(&h))
    {
        return Err(invalid("metadata image gate requires version 18 ledger"));
    }
    let mut r = Reader(checked(&bytes, MAGIC, 168)?);
    if r.array::<32>()? != *p.root.as_bytes() || r.array::<32>()? != image {
        return Err(invalid("metadata image gate root or image differs"));
    }
    let f = Fence {
        image,
        source: OwnerId::new(r.array()?).map_err(pin_error)?,
        intent_digest: RootDigest::from_bytes(r.array()?),
        source_gate_revision: r.uint()?,
        closed_at: r.uint()?,
    };
    if !r.0.is_empty()
        || f.source_gate_revision == 0
        || f.closed_at <= f.source_gate_revision
        || f.closed_at > p.revision
        || f.encode(p.root) != bytes
    {
        return Err(invalid("metadata image gate revision or encoding differs"));
    }
    // The immutable proof is historical. A source Version transfer must not
    // reopen admissions or turn acknowledgement into a new source dependency.
    let i = intent(p, root, f.source)?.ok_or_else(|| invalid("image gate source intent absent"))?;
    if subject(&i) != image
        || RootDigest::sha256(&i.encode()) != f.intent_digest
        || i.manifest()?.scope.region != kv9_common::META_REGION_0.0
    {
        return Err(invalid("metadata image gate source differs"));
    }
    let g = metadata_read_gate::read(p, root, &i)?;
    if !g.is_closed() || g.closed_at != f.source_gate_revision {
        return Err(invalid("metadata image gate lost its all-store proof"));
    }
    Ok(Some(f))
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Work {
    Ineligible,
    Ready,
    Closed(Fence),
}
pub fn observe(view: &dyn ReadView, root: &RootDescriptor, pending: OwnerId) -> Result<Work> {
    let p = planner(view, root)?;
    let i =
        intent(&p, root, pending)?.ok_or_else(|| invalid("image gate candidate intent absent"))?;
    if i.manifest()?.scope.region != kv9_common::META_REGION_0.0 {
        return Ok(Work::Ineligible);
    }
    if let Some(f) = read(&p, root, subject(&i))? {
        return Ok(Work::Closed(f));
    }
    Ok(
        if matches!(
            metadata_read_gate::observe(view, root, pending)?,
            metadata_read_gate::Work::Closed(_)
        ) {
            Work::Ready
        } else {
            Work::Ineligible
        },
    )
}
pub(super) fn close(p: &mut Planner<'_>, root: &RootDescriptor, pending: OwnerToken) -> Result<()> {
    if pending.generation.get() != 1 {
        return Err(conflict("metadata image gate Pending generation"));
    }
    match observe(p.view, root, pending.owner)? {
        Work::Closed(_) => return Ok(()),
        Work::Ready => {}
        Work::Ineligible => {
            return Err(conflict(
                "metadata image gate requires a closed all-store read gate",
            ))
        }
    }
    let i = checkpoint_intent::load(p, pending.owner)?
        .ok_or_else(|| invalid("image gate source intent absent"))?;
    let source = p.exact_owner(i.binding(OwnerKind::Version)?.token())?;
    if source.binding != i.binding(OwnerKind::Version)?
        || source.phase != PinPhase::Published
        || p.exact_owner(pending)?.phase != PinPhase::Released
    {
        return Err(conflict(
            "image gate requires exact released Pending and Published Version",
        ));
    }
    let gate = metadata_read_gate::read(p, root, &i)?;
    let f = Fence {
        image: subject(&i),
        source: pending.owner,
        source_gate_revision: gate.closed_at,
        closed_at: p
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("metadata image gate revision exhausted"))?,
        intent_digest: RootDigest::sha256(&i.encode()),
    };
    let bytes = f.encode(p.root);
    p.writes
        .insert(marker_key(f.image), marker(p.root, f.image, &bytes));
    p.writes.insert(gate_key(f.image), bytes);
    Ok(())
}
/// Include an image-wide fence in an alias's census without moving any earlier
/// per-attempt refusal/read-gate boundary. No alias scan or marker rewrite.
pub(super) fn for_pending(
    view: &dyn ReadView,
    root: &RootDescriptor,
    pending: OwnerId,
) -> Result<Option<Fence>> {
    if view
        .get(ColumnFamily::Default, &key(8, pending.as_bytes()))?
        .is_none()
    {
        return Ok(None);
    }
    let p = planner(view, root)?;
    let i = intent(&p, root, pending)?.ok_or_else(|| invalid("image gate alias intent absent"))?;
    if i.manifest()?.scope.region != kv9_common::META_REGION_0.0 {
        return Ok(None);
    }
    read(&p, root, subject(&i))
}
pub(super) fn check_reader_entry(p: &Planner<'_>, source: &OwnerBinding) -> Result<()> {
    if source.descriptor.region != kv9_common::META_REGION_0.0
        || source.descriptor.subject_is_anchor
        || record_bytes(p, source.descriptor.subject)?.is_none()
    {
        return Ok(());
    }
    let schema = crate::store::MetaStore::new(std::sync::Arc::new(kv9_engine::MemEngine::new()));
    let txn = schema.begin_at(Box::new(p.view));
    let root = crate::root::certified_root(&txn)?
        .ok_or_else(|| invalid("metadata image gate root absent"))?;
    if root.digest() != p.root {
        return Err(invalid("metadata image gate root differs"));
    }
    read(p, &root, source.descriptor.subject)?
        .ok_or_else(|| invalid("metadata image gate disappeared"))?;
    Err(conflict(
        "metadata image Reader and Bootstrap admission is closed",
    ))
}

#[cfg(test)]
mod tests;
