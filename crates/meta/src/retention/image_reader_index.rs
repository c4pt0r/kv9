//! Image-scoped directory of all non-anchor per-process Readers, including
//! legacy registrations and sources with unrelated operations. Index completion
//! certifies catalog coverage only; generations, admission and drain are separate.
use super::*;

const INDEX_MAGIC: &[u8; 8] = b"KV9IRI01";
const PROGRESS_MAGIC: &[u8; 8] = b"KV9IRP01";
pub const PAGE: usize = 4;

fn group_id(binding: &OwnerBinding) -> Vec<u8> {
    group(binding.descriptor.region, binding.descriptor.subject)
}
fn group(region: u64, image: [u8; 32]) -> Vec<u8> {
    let mut bytes = region.to_be_bytes().to_vec();
    bytes.extend_from_slice(&image);
    bytes
}
fn index_key(group: &[u8], owner: OwnerId) -> Vec<u8> {
    let mut id = group.to_vec();
    id.extend_from_slice(owner.as_bytes());
    key(38, &id)
}
fn index(binding: &OwnerBinding) -> Result<Vec<u8>> {
    let mut bytes = INDEX_MAGIC.to_vec();
    bytes.extend_from_slice(binding.descriptor.root.as_bytes());
    bytes.extend_from_slice(&group_id(binding));
    bytes.extend_from_slice(binding.descriptor.id.as_bytes());
    bytes.extend_from_slice(RootDigest::sha256(&owner_registration(binding)).as_bytes());
    Ok(seal(bytes))
}
pub(super) fn record(p: &mut Planner<'_>, binding: &OwnerBinding) -> Result<()> {
    if binding.descriptor.kind != OwnerKind::Reader || binding.descriptor.subject_is_anchor {
        return Ok(());
    }
    let key = index_key(&group_id(binding), binding.descriptor.id);
    let expected = index(binding)?;
    match p.view.get(ColumnFamily::Default, &key)? {
        Some(bytes) if bytes == expected => Ok(()),
        Some(_) => Err(invalid("image Reader reverse index binding differs")),
        None => {
            p.writes.insert(key, expected);
            Ok(())
        }
    }
}
pub(super) fn check(p: &Planner<'_>, binding: &OwnerBinding) -> Result<()> {
    if binding.descriptor.kind != OwnerKind::Reader || binding.descriptor.subject_is_anchor {
        return Ok(());
    }
    match p.view.get(
        ColumnFamily::Default,
        &index_key(&group_id(binding), binding.descriptor.id),
    )? {
        Some(bytes) if bytes == index(binding)? => Ok(()),
        Some(_) => Err(invalid("image Reader reverse index disagrees with owner")),
        None if progress(p)?.stage == 2 => {
            Err(invalid("complete image Reader index lost an owner"))
        }
        None => Ok(()), // Legacy rows remain eligible for the bounded census.
    }
}
pub(super) fn check_old_format(view: &dyn ReadView) -> Result<()> {
    if view
        .iter(ColumnFamily::Default, &key(38, &[]), &key(40, &[]))?
        .next()
        .transpose()?
        .is_some()
    {
        return Err(invalid(
            "image Reader index exists under an older ledger header",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Progress {
    pub sequence: u64,
    /// 0 owner rows, 1 immutable registrations, 2 complete catalog coverage.
    pub stage: u8,
    pub after: Option<OwnerId>,
    previous_stage: u8,
    previous_after: Option<OwnerId>,
}
impl Progress {
    fn encode(&self, root: RootDigest) -> Vec<u8> {
        let mut b = PROGRESS_MAGIC.to_vec();
        b.extend_from_slice(root.as_bytes());
        uint(&mut b, self.sequence);
        b.push(self.stage);
        b.push(self.previous_stage);
        for cursor in [self.after, self.previous_after] {
            b.push(u8::from(cursor.is_some()));
            if let Some(id) = cursor {
                b.extend_from_slice(id.as_bytes());
            }
        }
        seal(b)
    }
    pub fn request(&self, next: Option<OwnerId>) -> LedgerRequest {
        LedgerRequest::AdvanceImageReaderIndex {
            expected: self.sequence,
            stage: self.stage,
            after: self.after,
            next,
        }
    }
}
fn progress(p: &Planner<'_>) -> Result<Progress> {
    let Some(bytes) = p.view.get(ColumnFamily::Default, &key(39, &[]))? else {
        return Ok(Progress::default());
    };
    let mut r = Reader(checked(&bytes, PROGRESS_MAGIC, 116)?);
    if r.array::<32>()? != *p.root.as_bytes() {
        return Err(invalid("image Reader index progress root differs"));
    }
    let sequence = r.uint()?;
    let stage = r.byte()?;
    let previous_stage = r.byte()?;
    let mut cursor = || -> Result<Option<OwnerId>> {
        if r.boolean()? {
            Ok(Some(OwnerId::new(r.array()?).map_err(pin_error)?))
        } else {
            Ok(None)
        }
    };
    let current = Progress {
        sequence,
        stage,
        previous_stage,
        after: cursor()?,
        previous_after: cursor()?,
    };
    if !r.0.is_empty()
        || sequence == 0
        || sequence > p.revision
        || stage > 2
        || previous_stage > 1
        || (current.after.is_some()
            && (stage != previous_stage
                || current
                    .after
                    .zip(current.previous_after)
                    .is_some_and(|(n, o)| n <= o)))
        || (current.after.is_none() && stage != previous_stage + 1)
        || current.encode(p.root) != bytes
    {
        return Err(invalid(
            "image Reader index progress encoding or order differs",
        ));
    }
    Ok(current)
}
fn page(
    view: &dyn ReadView,
    stage: u8,
    after: Option<OwnerId>,
) -> Result<(Vec<OwnerId>, Option<OwnerId>)> {
    let tag = match stage {
        0 => 3,
        1 => 5,
        _ => return Err(invalid("image Reader index stage")),
    };
    let prefix = key(tag, &[]);
    let mut start = after.map_or_else(|| prefix.clone(), |id| key(tag, id.as_bytes()));
    if after.is_some() {
        start.push(0);
    }
    let mut ids = Vec::with_capacity(PAGE);
    for entry in view
        .iter(ColumnFamily::Default, &start, &key(tag + 1, &[]))?
        .take(PAGE + 1)
    {
        let (key, _) = entry?;
        if key.len() != prefix.len() + 16 {
            return Err(invalid("image Reader census owner key length"));
        }
        let id = OwnerId::new(key[prefix.len()..].try_into().unwrap()).map_err(pin_error)?;
        if ids.len() == PAGE {
            let next = ids.last().copied();
            return Ok((ids, next));
        }
        ids.push(id);
    }
    Ok((ids, None))
}
pub enum Work {
    Uninitialized,
    Upgrade,
    Complete,
    Page {
        progress: Progress,
        next: Option<OwnerId>,
        examined: usize,
    },
}
pub fn observe(view: &dyn ReadView, root: &RootDescriptor) -> Result<Work> {
    crate::checkpoint::inspect_initial_checkpoint_base(view, root)?;
    let Some(bytes) = view.get(ColumnFamily::Default, &key(1, &[]))? else {
        let mut end = PREFIX.to_vec();
        *end.last_mut().unwrap() += 1;
        if view
            .iter(ColumnFamily::Default, PREFIX, &end)?
            .next()
            .transpose()?
            .is_some()
        {
            return Err(invalid("image Reader census lost ledger header"));
        }
        return Ok(Work::Uninitialized);
    };
    let (revision, rows_enabled) = read_header(&bytes, root.digest())?;
    if !bytes.starts_with(HEADER_MAGIC) {
        check_upgrade(view, &bytes)?;
        return Ok(Work::Upgrade);
    }
    let p = Planner {
        view,
        root: root.digest(),
        revision,
        rows_enabled,
        writes: BTreeMap::new(),
    };
    let progress = progress(&p)?;
    if progress.stage == 2 {
        return Ok(Work::Complete);
    }
    let (ids, next) = page(view, progress.stage, progress.after)?;
    Ok(Work::Page {
        progress,
        next,
        examined: ids.len(),
    })
}
pub(super) fn advance(
    p: &mut Planner<'_>,
    expected: u64,
    stage: u8,
    after: Option<OwnerId>,
    next: Option<OwnerId>,
) -> Result<()> {
    if !p
        .view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .is_some_and(|b| b.starts_with(HEADER_MAGIC))
    {
        return Err(conflict("image Reader census requires upgraded ledger"));
    }
    let current = progress(p)?;
    let sequence = expected
        .checked_add(1)
        .ok_or_else(|| invalid("image Reader census sequence exhausted"))?;
    let target_stage = stage
        .checked_add(u8::from(next.is_none()))
        .ok_or_else(|| invalid("image Reader census stage exhausted"))?;
    if stage > 1 {
        return Err(invalid("image Reader census stage"));
    }
    if current.sequence == sequence
        && current.previous_stage == stage
        && current.previous_after == after
        && current.stage == target_stage
        && current.after == next
    {
        return Ok(());
    }
    if current.sequence != expected || current.stage != stage || current.after != after {
        return Err(conflict("image Reader census cursor changed"));
    }
    let (ids, actual_next) = page(p.view, stage, after)?;
    if actual_next != next {
        return Err(conflict("image Reader census cannot skip owner rows"));
    }
    for id in ids {
        // Both phases verify state/registration and every resource pin. A lone
        // registration cannot be mistaken for an absent reader at EOF.
        let owner = p
            .owner(id)?
            .ok_or_else(|| invalid("image Reader census owner missing"))?;
        record(p, &owner.binding)?;
    }
    let advanced = Progress {
        sequence,
        stage: target_stage,
        after: next,
        previous_stage: stage,
        previous_after: after,
    };
    p.writes.insert(key(39, &[]), advanced.encode(p.root));
    Ok(())
}

pub struct ReaderPage {
    pub owners: Vec<LedgerOwner>,
    pub next: Option<OwnerId>,
}
/// Includes generic and legacy per-process Readers for this exact region and image.
/// This does not certify their source, individual generations, or actual drain.
/// Callers must use one stable view for all pages of a point-in-time census.
pub fn readers(
    view: &dyn ReadView,
    root: &RootDescriptor,
    region: u64,
    image: [u8; 32],
    after: Option<OwnerId>,
) -> Result<ReaderPage> {
    let group = group(region, image);
    if !matches!(observe(view, root)?, Work::Complete) {
        return Err(conflict("image Reader census incomplete"));
    }
    let bytes = view.get(ColumnFamily::Default, &key(1, &[]))?.unwrap();
    let (revision, rows_enabled) = read_header(&bytes, root.digest())?;
    let p = Planner {
        view,
        root: root.digest(),
        revision,
        rows_enabled,
        writes: BTreeMap::new(),
    };
    let prefix = key(38, &group);
    let mut end = prefix.clone();
    while end.last() == Some(&255) {
        end.pop();
    }
    *end.last_mut().unwrap() += 1;
    let mut start = after.map_or_else(|| prefix.clone(), |id| index_key(&group, id));
    if after.is_some() {
        start.push(0);
    }
    let mut owners = Vec::with_capacity(PAGE);
    let mut next = None;
    for entry in view
        .iter(ColumnFamily::Default, &start, &end)?
        .take(PAGE + 1)
    {
        let (key, bytes) = entry?;
        if key.len() != prefix.len() + 16 {
            return Err(invalid("image Reader reverse index key length"));
        }
        let id = OwnerId::new(key[prefix.len()..].try_into().unwrap()).map_err(pin_error)?;
        let owner = p
            .owner(id)?
            .ok_or_else(|| invalid("image Reader index owner missing"))?;
        if owner.binding.descriptor.kind != OwnerKind::Reader
            || owner.binding.descriptor.region != region
            || owner.binding.descriptor.subject != image
            || owner.binding.descriptor.subject_is_anchor
            || group_id(&owner.binding) != group
            || bytes != index(&owner.binding)?
        {
            return Err(invalid("image Reader reverse index link differs"));
        }
        if owners.len() == PAGE {
            next = owners.last().map(|o: &LedgerOwner| o.binding.descriptor.id);
            break;
        }
        owners.push(owner);
    }
    Ok(ReaderPage { owners, next })
}
