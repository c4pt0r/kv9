//! Close new metadata-image uploads after its all-store read fence. This never
//! cancels an admitted scope or certifies old/unknown remote PUT termination.
use super::*;
const MAGIC: &[u8; 8] = b"KV9MIW01";
const MARKER: &[u8; 8] = b"KV9MWM01";
fn gate_key(image: [u8; 32]) -> Vec<u8> {
    key(42, &image)
}
fn marker_key(image: [u8; 32]) -> Vec<u8> {
    key(43, &image)
}
fn marker(root: RootDigest, image: [u8; 32], gate: &[u8]) -> Vec<u8> {
    let mut b = MARKER.to_vec();
    b.extend_from_slice(root.as_bytes());
    b.extend_from_slice(&image);
    b.extend_from_slice(RootDigest::sha256(gate).as_bytes());
    seal(b)
}
fn bytes(p: &Planner<'_>, image: [u8; 32]) -> Result<Option<Vec<u8>>> {
    match (
        p.view.get(ColumnFamily::Default, &gate_key(image))?,
        p.view.get(ColumnFamily::Default, &marker_key(image))?,
    ) {
        (None, None)
            if p.view
                .get(ColumnFamily::Default, &key(47, &image))?
                .is_none() =>
        {
            Ok(None)
        }
        (Some(g), Some(m)) if m == marker(p.root, image, &g) => Ok(Some(g)),
        _ => Err(invalid("metadata image write gate and marker disagree")),
    }
}
pub(super) fn check_old_format(view: &dyn ReadView) -> Result<()> {
    if view
        .iter(ColumnFamily::Default, &key(42, &[]), &key(44, &[]))?
        .next()
        .transpose()?
        .is_some()
    {
        return Err(invalid("image write gate under older ledger header"));
    }
    Ok(())
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fence {
    pub image: [u8; 32],
    pub source: OwnerId,
    pub read_closed_at: u64,
    pub closed_at: u64,
    read_gate: RootDigest,
}
impl Fence {
    fn encode(&self, root: RootDigest) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(root.as_bytes());
        b.extend_from_slice(&self.image);
        b.extend_from_slice(self.source.as_bytes());
        uint(&mut b, self.read_closed_at);
        b.extend_from_slice(self.read_gate.as_bytes());
        uint(&mut b, self.closed_at);
        seal(b)
    }
}
fn read(p: &Planner<'_>, root: &RootDescriptor, image: [u8; 32]) -> Result<Option<Fence>> {
    let Some(bytes) = bytes(p, image)? else {
        return Ok(None);
    };
    if !p
        .view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .is_some_and(|h| image_publisher_format(&h))
    {
        return Err(invalid("image write gate requires version 20 ledger"));
    }
    let mut r = Reader(checked(&bytes, MAGIC, 168)?);
    if r.array::<32>()? != *p.root.as_bytes() || r.array::<32>()? != image {
        return Err(invalid("image write gate identity differs"));
    }
    let f = Fence {
        image,
        source: OwnerId::new(r.array()?).map_err(pin_error)?,
        read_closed_at: r.uint()?,
        read_gate: RootDigest::from_bytes(r.array()?),
        closed_at: r.uint()?,
    };
    if !r.0.is_empty()
        || f.read_closed_at == 0
        || f.closed_at <= f.read_closed_at
        || f.closed_at > p.revision
        || f.encode(p.root) != bytes
    {
        return Err(invalid("image write gate revision differs"));
    }
    let base = metadata_image_gate::for_pending(p.view, root, f.source)?
        .ok_or_else(|| invalid("image write gate lost read fence"))?;
    let base_bytes = p
        .view
        .get(ColumnFamily::Default, &key(36, &image))?
        .ok_or_else(|| invalid("image read gate absent"))?;
    if base.image != image
        || base.source != f.source
        || base.closed_at != f.read_closed_at
        || RootDigest::sha256(&base_bytes) != f.read_gate
    {
        return Err(invalid("image write gate read proof differs"));
    }
    Ok(Some(f))
}
fn planner<'a>(view: &'a dyn ReadView, root: &RootDescriptor) -> Result<Planner<'a>> {
    crate::checkpoint::inspect_initial_checkpoint_base(view, root)?;
    let h = view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .ok_or_else(|| invalid("image write gate ledger absent"))?;
    let (revision, rows_enabled) = read_header(&h, root.digest())?;
    Ok(Planner {
        view,
        root: root.digest(),
        revision,
        rows_enabled,
        writes: BTreeMap::new(),
    })
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Work {
    Open,
    Ready,
    Closed(Fence),
}
pub fn observe(view: &dyn ReadView, root: &RootDescriptor, pending: OwnerId) -> Result<Work> {
    let Some(base) = metadata_image_gate::for_pending(view, root, pending)? else {
        return Ok(Work::Open);
    };
    let p = planner(view, root)?;
    Ok(match read(&p, root, base.image)? {
        Some(f) => Work::Closed(f),
        None => Work::Ready,
    })
}
pub(super) fn for_pending(
    view: &dyn ReadView,
    root: &RootDescriptor,
    pending: OwnerId,
) -> Result<Option<Fence>> {
    Ok(match observe(view, root, pending)? {
        Work::Closed(f) => Some(f),
        _ => None,
    })
}
pub(super) fn close(p: &mut Planner<'_>, root: &RootDescriptor, pending: OwnerToken) -> Result<()> {
    if pending.generation.get() != 1 {
        return Err(conflict("image write gate pending generation"));
    }
    match observe(p.view, root, pending.owner)? {
        Work::Closed(_) => return Ok(()),
        Work::Ready => {}
        Work::Open => {
            return Err(conflict(
                "image write gate requires closed all-store image read gate",
            ))
        }
    }
    let base = metadata_image_gate::for_pending(p.view, root, pending.owner)?
        .ok_or_else(|| invalid("image read gate disappeared"))?;
    let bytes = p
        .view
        .get(ColumnFamily::Default, &key(36, &base.image))?
        .ok_or_else(|| invalid("image read gate absent"))?;
    let f = Fence {
        image: base.image,
        source: base.source,
        read_closed_at: base.closed_at,
        read_gate: RootDigest::sha256(&bytes),
        closed_at: p
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("image write gate revision exhausted"))?,
    };
    let bytes = f.encode(p.root);
    p.writes
        .insert(marker_key(f.image), marker(p.root, f.image, &bytes));
    p.writes.insert(gate_key(f.image), bytes);
    Ok(())
}
pub(super) fn check_entry(p: &Planner<'_>, binding: &OwnerBinding) -> Result<()> {
    if binding.descriptor.region != kv9_common::META_REGION_0.0
        || binding.descriptor.subject_is_anchor
        || bytes(p, binding.descriptor.subject)?.is_none()
    {
        return Ok(());
    }
    let schema = crate::store::MetaStore::new(std::sync::Arc::new(kv9_engine::MemEngine::new()));
    let txn = schema.begin_at(Box::new(p.view));
    let root = crate::root::certified_root(&txn)?
        .ok_or_else(|| invalid("image write gate root absent"))?;
    if root.digest() != p.root {
        return Err(invalid("image write gate root differs"));
    }
    read(p, &root, binding.descriptor.subject)?
        .ok_or_else(|| invalid("image write gate disappeared"))?;
    Err(conflict("metadata image upload admission is closed"))
}
