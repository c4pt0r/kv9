//! Close future reads of an old metadata checkpoint after every provisioned
//! store has independently reported a strictly later selection. Existing read
//! grants/pins remain intact; this is not complete recovery or release authority.
use super::*;
use kv9_common::META_REGION_0;

const MAGIC: &[u8; 8] = b"KV9MCG01";
pub const PAGE: usize = 4;
fn gate_key(pending: OwnerId) -> Vec<u8> {
    key(32, pending.as_bytes())
}
pub(super) fn check_old_format(view: &dyn ReadView) -> Result<()> {
    if view
        .iter(ColumnFamily::Default, &key(32, &[]), &key(33, &[]))?
        .next()
        .transpose()?
        .is_some()
    {
        return Err(invalid("metadata read gate under an older ledger header"));
    }
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Progress {
    pub sequence: u64,
    pub checked_stores: u64,
    pub stores: u64,
    pub closed_at: u64,
}
impl Progress {
    pub fn is_closed(&self) -> bool {
        self.closed_at != 0
    }
    pub fn request(&self, pending: OwnerId) -> LedgerRequest {
        LedgerRequest::AdvanceMetadataReadGate {
            pending: OwnerToken {
                owner: pending,
                generation: NonZeroU64::new(1).unwrap(),
            },
            expected: self.sequence,
        }
    }
    fn encode(&self, intent: &CheckpointIntent) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(intent.root().as_bytes());
        b.extend_from_slice(intent.pending_id().as_bytes());
        b.extend_from_slice(RootDigest::sha256(&intent.encode()).as_bytes());
        for n in [
            self.sequence,
            self.checked_stores,
            self.stores,
            self.closed_at,
        ] {
            uint(&mut b, n);
        }
        seal(b)
    }
}
pub(super) fn read(
    p: &Planner<'_>,
    root: &RootDescriptor,
    intent: &CheckpointIntent,
) -> Result<Progress> {
    let Some(bytes) = p
        .view
        .get(ColumnFamily::Default, &gate_key(intent.pending_id()))?
    else {
        return Ok(Progress {
            stores: root.voters.len() as u64,
            ..Progress::default()
        });
    };
    if !p
        .view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .is_some_and(|h| metadata_read_gate_format(&h))
    {
        return Err(invalid("metadata read gate requires version 15 ledger"));
    }
    let mut r = Reader(checked(&bytes, MAGIC, 152)?);
    if r.array::<32>()? != *p.root.as_bytes()
        || r.array::<16>()? != *intent.pending_id().as_bytes()
        || r.array::<32>()? != *RootDigest::sha256(&intent.encode()).as_bytes()
    {
        return Err(invalid("metadata read gate identity differs"));
    }
    let g = Progress {
        sequence: r.uint()?,
        checked_stores: r.uint()?,
        stores: r.uint()?,
        closed_at: r.uint()?,
    };
    if !r.0.is_empty()
        || intent.manifest()?.scope.region != META_REGION_0.0
        || g.stores != root.voters.len() as u64
        || g.stores == 0
        || g.stores > 1024
        || g.checked_stores == 0
        || g.checked_stores > g.stores
        || (!g.is_closed() && !g.checked_stores.is_multiple_of(PAGE as u64))
        || g.is_closed() != (g.checked_stores == g.stores)
        || g.sequence != g.checked_stores.div_ceil(PAGE as u64)
        || g.sequence > p.revision
        || (g.is_closed() && g.closed_at < g.sequence)
        || g.closed_at > p.revision
        || g.encode(intent) != bytes
    {
        return Err(invalid("metadata read gate progress or revision differs"));
    }
    Ok(g)
}
fn context<'a>(
    view: &'a dyn ReadView,
    root: &RootDescriptor,
    pending: OwnerId,
) -> Result<(Planner<'a>, CheckpointIntent)> {
    crate::checkpoint::inspect_initial_checkpoint_base(view, root)?;
    let h = view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .ok_or_else(|| invalid("metadata read gate ledger absent"))?;
    let (revision, rows_enabled) = read_header(&h, root.digest())?;
    let p = Planner {
        view,
        root: root.digest(),
        revision,
        rows_enabled,
        writes: BTreeMap::new(),
    };
    let intent = checkpoint_intent::load(&p, pending)?
        .ok_or_else(|| invalid("metadata read gate intent absent"))?;
    Ok((p, intent))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Work {
    /// Data, unpublished or negative attempts do not use this root-store gate.
    Ineligible,
    WaitingForSelection(Progress),
    Advance(Progress),
    Closed(Progress),
}
fn work(p: &Planner<'_>, root: &RootDescriptor, intent: &CheckpointIntent) -> Result<Work> {
    let g = read(p, root, intent)?;
    if g.is_closed() {
        return Ok(Work::Closed(g));
    }
    if intent.manifest()?.scope.region != META_REGION_0.0 {
        return Ok(Work::Ineligible);
    }
    let pending = p.exact_owner(intent.binding(OwnerKind::Pending)?.token())?;
    let version = intent.binding(OwnerKind::Version)?;
    let published = p.owner(version.descriptor.id)?;
    if pending.phase != PinPhase::Released
        || published
            .as_ref()
            .is_none_or(|v| v.binding != version || v.phase != PinPhase::Published)
        || checkpoint_refusal::refusal(p.view, root, intent.pending_id())?.is_some()
    {
        return Ok(Work::Ineligible);
    }
    // Facts are monotone, root/store identities immutable. An already verified
    // prefix cannot regress while later pages wait for a slow/offline store.
    // No census of all attempts, grants or configurations occurs on this path.
    for voter in root
        .voters
        .iter()
        .skip(g.checked_stores as usize)
        .take(PAGE)
    {
        let Some(selected) = checkpoint_selection::selection(
            p.view,
            root,
            voter.node_id,
            voter.store_incarnation,
            META_REGION_0,
        )?
        else {
            return Ok(Work::WaitingForSelection(g));
        };
        if selected.intent() == intent || !selected.covers(intent)? {
            return Ok(Work::WaitingForSelection(g));
        }
    }
    Ok(Work::Advance(g))
}
pub fn observe(view: &dyn ReadView, root: &RootDescriptor, pending: OwnerId) -> Result<Work> {
    let (p, intent) = context(view, root, pending)?;
    work(&p, root, &intent)
}
pub(super) fn advance(
    p: &mut Planner<'_>,
    root: &RootDescriptor,
    pending: OwnerToken,
    expected: u64,
) -> Result<()> {
    let intent = checkpoint_intent::load(p, pending.owner)?
        .ok_or_else(|| invalid("metadata read gate intent absent"))?;
    if intent.binding(OwnerKind::Pending)?.token() != pending {
        return Err(conflict("metadata read gate Pending differs"));
    }
    let mut g = read(p, root, &intent)?;
    // An exact adjacent retry depends only on the retained page, not on a live
    // source or the next page already being eligible. Never acknowledge skips.
    if expected.checked_add(1) == Some(g.sequence) {
        return Ok(());
    }
    if expected != g.sequence {
        return Err(conflict("metadata read gate sequence differs"));
    }
    if g.is_closed() {
        return Ok(());
    }
    if !matches!(work(p, root, &intent)?, Work::Advance(_)) {
        return Err(conflict(
            "metadata read gate requires later selection by every page member",
        ));
    }
    g.sequence = g
        .sequence
        .checked_add(1)
        .ok_or_else(|| invalid("metadata gate sequence exhausted"))?;
    g.checked_stores = (g.checked_stores + PAGE as u64).min(g.stores);
    if g.checked_stores == g.stores {
        g.closed_at = p
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("metadata gate revision exhausted"))?;
    }
    p.writes.insert(gate_key(pending.owner), g.encode(&intent));
    Ok(())
}

/// Common read fence for the existing negative-attempt census and the new
/// positive metadata gate. No caller receives negative settlement from this.
pub(super) struct ReadFence {
    pub pending: OwnerId,
    pub closed_at: u64,
    pub recorded_at: u64,
    /// One image fact covers multiple aliases without rewriting their stamps.
    pub shared_image: bool,
}
pub(super) fn fence(
    view: &dyn ReadView,
    root: &RootDescriptor,
    pending: OwnerId,
) -> Result<Option<ReadFence>> {
    let refusal = checkpoint_refusal::refusal(view, root, pending)?.map(|r| ReadFence {
        pending,
        closed_at: r.closed_at,
        recorded_at: r.recorded_at,
        shared_image: false,
    });
    let image = metadata_image_gate::for_pending(view, root, pending)?.map(|f| ReadFence {
        pending,
        closed_at: f.closed_at,
        recorded_at: f.closed_at,
        shared_image: true,
    });
    let refusal = earliest(refusal, image);
    if view
        .get(ColumnFamily::Default, &gate_key(pending))?
        .is_none()
    {
        return Ok(refusal);
    }
    let (p, intent) = context(view, root, pending)?;
    let g = read(&p, root, &intent)?;
    let gate = g.is_closed().then_some(ReadFence {
        pending,
        closed_at: g.closed_at,
        recorded_at: g.closed_at,
        shared_image: false,
    });
    // A later negative settlement must not change the boundary of a census
    // already started under an earlier positive read fence.
    Ok(earliest(refusal, gate))
}
fn earliest(a: Option<ReadFence>, b: Option<ReadFence>) -> Option<ReadFence> {
    match (a, b) {
        (Some(a), Some(b)) => Some(if a.recorded_at <= b.recorded_at { a } else { b }),
        (a, b) => a.or(b),
    }
}
pub(super) fn check_reader_entry(p: &Planner<'_>, operation: [u8; 32]) -> Result<()> {
    let pending = checkpoint_intent::owner_id(p.root, &operation, b"kv9-checkpoint-pending-v1")?;
    if p.view
        .get(ColumnFamily::Default, &gate_key(pending))?
        .is_none()
    {
        return Ok(());
    }
    // Use the same immutable view, not a fresh engine snapshot or caller count.
    let schema = crate::store::MetaStore::new(std::sync::Arc::new(kv9_engine::MemEngine::new()));
    let txn = schema.begin_at(Box::new(p.view));
    let root = crate::root::certified_root(&txn)?
        .ok_or_else(|| invalid("metadata read gate root absent"))?;
    if root.digest() != p.root {
        return Err(invalid("metadata read gate root differs"));
    }
    let intent = checkpoint_intent::load(p, pending)?
        .ok_or_else(|| invalid("metadata read gate intent absent"))?;
    if read(p, &root, &intent)?.is_closed() {
        return Err(conflict("metadata checkpoint Reader admission is closed"));
    }
    Ok(())
}

#[cfg(test)]
pub(super) mod tests;
