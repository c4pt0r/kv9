//! Bounded census of persistent BootstrapReader grants. Image subjects group
//! legacy and managed grants alike; only an admission row identifies the exact
//! Pending/Version/store. Coverage never certifies read fencing or source release.
use super::*;

const INDEX_MAGIC: &[u8; 8] = b"KV9BRI01";
const OLD_PROGRESS_MAGIC: &[u8; 8] = b"KV9BRP01";
const PROGRESS_MAGIC: &[u8; 8] = b"KV9BRP02";
pub const PAGE: usize = 4;

fn group_id(binding: &OwnerBinding) -> [u8; 32] {
    binding.descriptor.subject
}
fn index_key(group: [u8; 32], owner: OwnerId) -> Vec<u8> {
    let mut id = group.to_vec();
    id.extend_from_slice(owner.as_bytes());
    key(29, &id)
}
pub(super) fn require_unregistered(p: &Planner<'_>, binding: &OwnerBinding) -> Result<()> {
    if p.view
        .get(
            ColumnFamily::Default,
            &index_key(group_id(binding), binding.descriptor.id),
        )?
        .is_some()
    {
        return Err(invalid(
            "absent bootstrap owner still has reverse index history",
        ));
    }
    Ok(())
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
    if binding.descriptor.kind != OwnerKind::BootstrapReader {
        return Ok(());
    }
    let key = index_key(group_id(binding), binding.descriptor.id);
    let expected = index(binding)?;
    match p.view.get(ColumnFamily::Default, &key)? {
        Some(bytes) if bytes == expected => {
            require_format(p)?;
            Ok(())
        }
        Some(_) => Err(invalid("Bootstrap reverse index binding differs")),
        None => {
            p.writes.insert(key, expected);
            Ok(())
        }
    }
}
pub(super) fn check(p: &Planner<'_>, binding: &OwnerBinding) -> Result<()> {
    if binding.descriptor.kind != OwnerKind::BootstrapReader {
        return Ok(());
    }
    match p.view.get(
        ColumnFamily::Default,
        &index_key(group_id(binding), binding.descriptor.id),
    )? {
        Some(bytes) if bytes == index(binding)? => {
            require_format(p)?;
            Ok(())
        }
        Some(_) => Err(invalid("Bootstrap reverse index disagrees with owner")),
        // The original three passes already established every owner's index.
        // The new fourth pass adds terminal provenance coverage, never weakens
        // that existing missing-index check during a v16 upgrade.
        None if progress(p)?.stage >= 3 => Err(invalid("complete Bootstrap index lost an owner")),
        None => Ok(()), // Legacy rows remain eligible for the bounded census.
    }
}
pub(super) fn check_old_format(view: &dyn ReadView) -> Result<()> {
    if view
        .iter(ColumnFamily::Default, &key(29, &[]), &key(31, &[]))?
        .next()
        .transpose()?
        .is_some()
    {
        return Err(invalid(
            "Bootstrap index exists under an older ledger header",
        ));
    }
    Ok(())
}
pub(super) fn check_old_unentered_format(view: &dyn ReadView) -> Result<()> {
    if view
        .get(ColumnFamily::Default, &key(30, &[]))?
        .is_some_and(|b| b.starts_with(PROGRESS_MAGIC))
    {
        return Err(invalid(
            "bootstrap four-pass census under an older ledger header",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Progress {
    pub sequence: u64,
    /// 0 owners, 1 registrations, 2 admissions, 3 unentered facts, 4 complete.
    pub stage: u8,
    pub after: Option<OwnerId>,
    previous_stage: u8,
    previous_after: Option<OwnerId>,
}
impl Progress {
    fn encode(&self, root: RootDigest) -> Vec<u8> {
        self.encode_with(root, PROGRESS_MAGIC)
    }
    fn encode_with(&self, root: RootDigest, magic: &[u8; 8]) -> Vec<u8> {
        let mut b = magic.to_vec();
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
        LedgerRequest::AdvanceBootstrapIndex {
            expected: self.sequence,
            stage: self.stage,
            after: self.after,
            next,
        }
    }
}
fn require_format(p: &Planner<'_>) -> Result<()> {
    if !p
        .view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .is_some_and(|h| bootstrap_index_format(&h))
    {
        return Err(invalid("bootstrap index requires version 13 ledger"));
    }
    Ok(())
}
fn progress(p: &Planner<'_>) -> Result<Progress> {
    let Some(bytes) = p.view.get(ColumnFamily::Default, &key(30, &[]))? else {
        return Ok(Progress::default());
    };
    require_format(p)?;
    let legacy = bytes.starts_with(OLD_PROGRESS_MAGIC);
    let magic = if legacy {
        OLD_PROGRESS_MAGIC
    } else {
        PROGRESS_MAGIC
    };
    if !legacy
        && !p
            .view
            .get(ColumnFamily::Default, &key(1, &[]))?
            .is_some_and(|b| bootstrap_unentered_format(&b))
    {
        return Err(invalid(
            "bootstrap four-pass census requires version 17 ledger",
        ));
    }
    let mut r = Reader(checked(&bytes, magic, 116)?);
    if r.array::<32>()? != *p.root.as_bytes() {
        return Err(invalid("Bootstrap index progress root differs"));
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
        || stage > if legacy { 3 } else { 4 }
        || previous_stage > if legacy { 2 } else { 3 }
        || (current.after.is_some()
            && (stage != previous_stage
                || current
                    .after
                    .zip(current.previous_after)
                    .is_some_and(|(n, o)| n <= o)))
        || (current.after.is_none() && stage != previous_stage + 1)
        || current.encode_with(p.root, magic) != bytes
    {
        return Err(invalid(
            "Bootstrap index progress encoding or order differs",
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
        2 => 28,
        3 => 35,
        _ => return Err(invalid("Bootstrap index stage")),
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
            return Err(invalid("Bootstrap census owner key length"));
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
            return Err(invalid("Bootstrap census lost ledger header"));
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
    if progress.stage == 4 {
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
        return Err(conflict("Bootstrap census requires upgraded ledger"));
    }
    let current = progress(p)?;
    let sequence = expected
        .checked_add(1)
        .ok_or_else(|| invalid("Bootstrap census sequence exhausted"))?;
    let target_stage = stage
        .checked_add(u8::from(next.is_none()))
        .ok_or_else(|| invalid("Bootstrap census stage exhausted"))?;
    if stage > 3 {
        return Err(invalid("Bootstrap census stage"));
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
        return Err(conflict("Bootstrap census cursor changed"));
    }
    let (ids, actual_next) = page(p.view, stage, after)?;
    if actual_next != next {
        return Err(conflict("Bootstrap census cannot skip owner rows"));
    }
    for id in ids {
        // Every pass verifies state/registration and every resource pin. A lone
        // registration cannot be mistaken for an absent reader at EOF.
        let owner = p
            .owner(id)?
            .ok_or_else(|| invalid("Bootstrap census owner missing"))?;
        if stage == 2
            && (owner.binding.descriptor.kind != OwnerKind::BootstrapReader
                || bootstrap_reader::admission::read(p, &owner.binding)?.is_none())
        {
            return Err(invalid("bootstrap admission census link differs"));
        }
        if stage == 3
            && (owner.binding.descriptor.kind != OwnerKind::BootstrapReader
                || bootstrap_reader::unentered::read(p, &owner.binding)?.is_none())
        {
            return Err(invalid("bootstrap unentered census link differs"));
        }
        record(p, &owner.binding)?;
    }
    let advanced = Progress {
        sequence,
        stage: target_stage,
        after: next,
        previous_stage: stage,
        previous_after: after,
    };
    p.writes.insert(key(30, &[]), advanced.encode(p.root));
    Ok(())
}

/// Add every newly written bootstrap owner to the index in its own atomic
/// batch, including direct terminal fencing before entry. The write set is
/// bounded by the ledger batch limit; no owner-table scan happens here.
pub(super) fn record_changes(p: &mut Planner<'_>) -> Result<()> {
    let prefix = key(3, &[]);
    let bindings = p
        .writes
        .iter()
        .filter(|(k, _)| k.starts_with(&prefix))
        .map(|(k, v)| {
            if k.len() != prefix.len() + 16 {
                return Err(invalid("bootstrap owner write key length"));
            }
            let id = OwnerId::new(k[prefix.len()..].try_into().unwrap()).map_err(pin_error)?;
            Ok(decode_owner(v, p.root, id)?.binding)
        })
        .collect::<Result<Vec<_>>>()?;
    for binding in bindings {
        record(p, &binding)?;
    }
    Ok(())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub image: [u8; 32],
    pub owner: OwnerId,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub owner: LedgerOwner,
    /// None does not prove absence of entry. Check the separate unentered fact;
    /// when both are absent the history remains legacy/unknown.
    pub admission: Option<bootstrap_reader::admission::Admission>,
    pub unentered: Option<bootstrap_reader::unentered::Unentered>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantPage {
    pub grants: Vec<Grant>,
    pub next: Option<Cursor>,
}
fn query(
    view: &dyn ReadView,
    root: &RootDescriptor,
    image: Option<[u8; 32]>,
    after: Option<Cursor>,
) -> Result<GrantPage> {
    if !matches!(observe(view, root)?, Work::Complete) {
        return Err(conflict("bootstrap census incomplete"));
    }
    if image == Some([0; 32])
        || after.is_some_and(|a| a.image == [0; 32] || image.is_some_and(|i| i != a.image))
    {
        return Err(invalid("bootstrap census query cursor differs"));
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
    let prefix = key(29, image.as_ref().map_or(&[][..], |v| &v[..]));
    let mut end = prefix.clone();
    while end.last() == Some(&255) {
        end.pop();
    }
    *end.last_mut().unwrap() += 1;
    let mut start = after.map_or_else(|| prefix.clone(), |a| index_key(a.image, a.owner));
    if after.is_some() {
        start.push(0);
    }
    let mut grants = Vec::with_capacity(PAGE);
    let mut next = None;
    let id_start = key(29, &[]).len();
    for entry in view
        .iter(ColumnFamily::Default, &start, &end)?
        .take(PAGE + 1)
    {
        let (key, bytes) = entry?;
        if key.len() != id_start + 48 {
            return Err(invalid("bootstrap reverse index key length"));
        }
        let subject: [u8; 32] = key[id_start..id_start + 32].try_into().unwrap();
        let id = OwnerId::new(key[id_start + 32..].try_into().unwrap()).map_err(pin_error)?;
        let owner = p
            .owner(id)?
            .ok_or_else(|| invalid("bootstrap index owner missing"))?;
        if owner.binding.descriptor.kind != OwnerKind::BootstrapReader
            || subject != owner.binding.descriptor.subject
            || image.is_some_and(|i| i != subject)
            || bytes != index(&owner.binding)?
        {
            return Err(invalid("bootstrap reverse index link differs"));
        }
        if grants.len() == PAGE {
            next = grants.last().map(|g: &Grant| Cursor {
                image: g.owner.binding.descriptor.subject,
                owner: g.owner.binding.descriptor.id,
            });
            break;
        }
        let admission = bootstrap_reader::admission::read(&p, &owner.binding)?;
        let unentered = bootstrap_reader::unentered::read(&p, &owner.binding)?;
        grants.push(Grant {
            owner,
            admission,
            unentered,
        });
    }
    Ok(GrantPage { grants, next })
}
/// Enumerate every registered bootstrap grant from one stable catalog view,
/// including unknown legacy subjects. A completed index is not a drain proof.
pub fn catalog(
    view: &dyn ReadView,
    root: &RootDescriptor,
    after: Option<Cursor>,
) -> Result<GrantPage> {
    query(view, root, None, after)
}
/// Enumerate grants with this exact immutable image subject. Do not derive a
/// Pending from a legacy operation or interpret this page as a Version fence.
pub fn grants(
    view: &dyn ReadView,
    root: &RootDescriptor,
    image: [u8; 32],
    after: Option<OwnerId>,
) -> Result<GrantPage> {
    query(
        view,
        root,
        Some(image),
        after.map(|owner| Cursor { image, owner }),
    )
}

#[cfg(test)]
mod tests;
