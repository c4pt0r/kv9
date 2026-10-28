//! Persistent, bounded census of retained DATA placement origins. Unknown or
//! erased task history remains unknown. This is not all-reader/store closure.
use super::*;
use crate::data_groups::store_history::{self, Origin};
use crate::task_history::{self, Revision};
use kv9_common::{NodeId, RegionId, StoreIncarnation};

pub const PAGE: usize = 4;
const ORIGIN_MAGIC: &[u8; 8] = b"KV9DSH01";
const PROGRESS_MAGIC: &[u8; 8] = b"KV9DSP01";
const COUNT_MAGIC: &[u8; 8] = b"KV9DSC01";
const ORIGIN_BYTES: usize = 184;
fn count_key(region: RegionId, creation: RootDigest) -> Vec<u8> {
    let mut suffix = region.0.to_be_bytes().to_vec();
    suffix.extend_from_slice(creation.as_bytes());
    key(51, &suffix)
}
fn encode_count(root: RootDigest, region: RegionId, creation: RootDigest, count: u64) -> Vec<u8> {
    let mut b = COUNT_MAGIC.to_vec();
    b.extend_from_slice(root.as_bytes());
    uint(&mut b, region.0);
    b.extend_from_slice(creation.as_bytes());
    uint(&mut b, count);
    seal(b)
}
fn decode_count(
    root: RootDigest,
    region: RegionId,
    creation: RootDigest,
    bytes: &[u8],
) -> Result<u64> {
    let mut r = Reader(checked(bytes, COUNT_MAGIC, 120)?);
    if r.array::<32>()? != *root.as_bytes()
        || r.uint()? != region.0
        || r.array::<32>()? != *creation.as_bytes()
    {
        return Err(invalid("DATA origin count scope differs"));
    }
    let count = r.uint()?;
    if count == 0 || !r.0.is_empty() || encode_count(root, region, creation, count) != bytes {
        return Err(invalid("DATA origin count encoding"));
    }
    Ok(count)
}
pub(super) fn origin_key(o: &Origin) -> Vec<u8> {
    let mut suffix = o.region.0.to_be_bytes().to_vec();
    suffix.extend_from_slice(o.creation.as_bytes());
    uint(&mut suffix, o.replica.node.0);
    suffix.extend_from_slice(o.replica.incarnation.as_bytes());
    uint(&mut suffix, o.task);
    key(49, &suffix)
}
pub(super) fn encode_origin(root: RootDigest, o: &Origin) -> Vec<u8> {
    let mut b = ORIGIN_MAGIC.to_vec();
    b.extend_from_slice(root.as_bytes());
    uint(&mut b, o.region.0);
    b.extend_from_slice(o.creation.as_bytes());
    uint(&mut b, o.replica.node.0);
    b.extend_from_slice(o.replica.incarnation.as_bytes());
    uint(&mut b, o.task);
    uint(&mut b, o.kind);
    b.extend_from_slice(o.source_digest.as_bytes());
    seal(b)
}
pub(super) fn decode_origin(root: RootDigest, key: &[u8], bytes: &[u8]) -> Result<Origin> {
    let mut r = Reader(checked(bytes, ORIGIN_MAGIC, ORIGIN_BYTES)?);
    if r.array::<32>()? != *root.as_bytes() {
        return Err(invalid("DATA origin root differs"));
    }
    let o = Origin {
        region: RegionId(r.uint()?),
        creation: RootDigest::from_bytes(r.array()?),
        replica: crate::data_groups::InitialReplica {
            node: NodeId(r.uint()?),
            incarnation: StoreIncarnation::from_bytes(r.array()?),
        },
        task: r.uint()?,
        kind: r.uint()?,
        source_digest: RootDigest::from_bytes(r.array()?),
    };
    if !r.0.is_empty()
        || o.region.0 < 100
        || o.replica.node.0 == 0
        || o.replica.incarnation.as_bytes() == &[0; 16]
        || o.creation.as_bytes() == &[0; 32]
        || o.source_digest.as_bytes() == &[0; 32]
        || o.task < 100
        || !matches!(o.kind, 100 | 103 | 104 | 106 | 108)
        || origin_key(&o) != key
        || encode_origin(root, &o) != bytes
    {
        return Err(invalid("DATA origin key or payload differs"));
    }
    Ok(o)
}
pub(super) fn check_old_format(view: &dyn ReadView) -> Result<()> {
    if view
        .iter(ColumnFamily::Default, &key(49, &[]), &key(52, &[]))?
        .next()
        .transpose()?
        .is_some()
    {
        return Err(invalid("DATA history index under an older ledger header"));
    }
    Ok(())
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Progress {
    pub sequence: u64,
    pub source_revision: u64,
    pub after: Option<u64>,
    pub complete: bool,
    previous_after: Option<u64>,
}
impl Progress {
    fn encode(&self, root: RootDigest) -> Vec<u8> {
        let mut b = PROGRESS_MAGIC.to_vec();
        b.extend_from_slice(root.as_bytes());
        uint(&mut b, self.sequence);
        uint(&mut b, self.source_revision);
        b.push(u8::from(self.complete));
        for cursor in [self.after, self.previous_after] {
            b.push(u8::from(cursor.is_some()));
            if let Some(id) = cursor {
                uint(&mut b, id);
            }
        }
        seal(b)
    }
    pub fn request(&self, revision: u64, after: Option<u64>) -> LedgerRequest {
        LedgerRequest::AdvanceDataStoreHistory {
            expected: self.sequence,
            revision,
            after,
        }
    }
}
fn progress(p: &Planner<'_>) -> Result<Progress> {
    let Some(bytes) = p.view.get(ColumnFamily::Default, &key(50, &[]))? else {
        return Ok(Progress::default());
    };
    let mut r = Reader(checked(&bytes, PROGRESS_MAGIC, 107)?);
    if r.array::<32>()? != *p.root.as_bytes() {
        return Err(invalid("DATA history progress root differs"));
    }
    let sequence = r.uint()?;
    let source_revision = r.uint()?;
    let complete = r.boolean()?;
    let mut cursor = || -> Result<Option<u64>> {
        if r.boolean()? {
            let id = r.uint()?;
            if id < 100 {
                return Err(invalid("DATA history cursor identity"));
            }
            Ok(Some(id))
        } else {
            Ok(None)
        }
    };
    let out = Progress {
        sequence,
        source_revision,
        complete,
        after: cursor()?,
        previous_after: cursor()?,
    };
    if !r.0.is_empty()
        || sequence == 0
        || sequence > p.revision
        || out.encode(p.root) != bytes
        || (!complete && out.after.is_none())
        || out
            .after
            .zip(out.previous_after)
            .is_some_and(|(a, b)| a < b || (a == b && !complete))
    {
        return Err(invalid("DATA history cursor order or encoding"));
    }
    Ok(out)
}
struct TaskPage {
    origins: Vec<Origin>,
    last: Option<u64>,
    complete: bool,
    examined: usize,
}
fn page(view: &dyn ReadView, root: RootDigest, after: Option<u64>) -> Result<TaskPage> {
    let (prefix, end) = crate::codec::row_range(crate::schema::TASKS_DESC.id)?;
    let mut start = after
        .map(store_history::task_key)
        .transpose()?
        .unwrap_or(prefix);
    if after.is_some() {
        start.push(0);
    }
    let mut out = TaskPage {
        origins: Vec::with_capacity(PAGE * 7),
        last: after,
        complete: true,
        examined: 0,
    };
    for entry in view
        .iter(ColumnFamily::Default, &start, &end)?
        .take(PAGE + 1)
    {
        let (key, bytes) = entry?;
        if out.examined == PAGE {
            out.complete = false;
            break;
        }
        let id = store_history::task_id(&key, &bytes)?;
        if out.last.is_some_and(|old| id <= old) {
            return Err(invalid("DATA history task order"));
        }
        out.origins
            .extend(store_history::project(view, root, &key, &bytes)?);
        out.last = Some(id);
        out.examined += 1;
    }
    Ok(out)
}
pub enum Work {
    Uninitialized,
    Upgrade,
    Uncertain(Revision),
    Complete {
        revision: u64,
    },
    Page {
        progress: Progress,
        revision: u64,
        after: Option<u64>,
        examined: usize,
    },
}
pub fn observe(view: &dyn ReadView, root: &RootDescriptor) -> Result<Work> {
    crate::checkpoint::inspect_initial_checkpoint_base(view, root)?;
    let Some(h) = view.get(ColumnFamily::Default, &key(1, &[]))? else {
        check_old_format(view)?;
        return Ok(Work::Uninitialized);
    };
    let (revision, rows_enabled) = read_header(&h, root.digest())?;
    if !h.starts_with(HEADER_MAGIC) {
        check_upgrade(view, &h)?;
        return Ok(Work::Upgrade);
    }
    let p = Planner {
        view,
        root: root.digest(),
        revision,
        rows_enabled,
        writes: BTreeMap::new(),
    };
    let source = task_history::revision(view)?;
    let progress = progress(&p)?;
    if source.sequence < progress.source_revision {
        return Err(invalid(
            "DATA history task revision regressed or disappeared",
        ));
    }
    if source.uncertain {
        return Ok(Work::Uncertain(source));
    }
    if progress.sequence != 0 && progress.source_revision == source.sequence && progress.complete {
        return Ok(Work::Complete {
            revision: source.sequence,
        });
    }
    let after = if progress.source_revision == source.sequence {
        progress.after
    } else {
        None
    };
    let examined = page(view, root.digest(), after)?.examined;
    Ok(Work::Page {
        progress,
        revision: source.sequence,
        after,
        examined,
    })
}
pub(super) fn advance(
    p: &mut Planner<'_>,
    root: &RootDescriptor,
    expected: u64,
    revision: u64,
    after: Option<u64>,
) -> Result<()> {
    if !p
        .view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .is_some_and(|h| h.starts_with(HEADER_MAGIC))
    {
        return Err(conflict(
            "DATA history census requires current ledger format",
        ));
    }
    let current = progress(p)?;
    let actual = task_history::revision(p.view)?;
    if actual.sequence < current.source_revision {
        return Err(invalid(
            "DATA history task revision regressed or disappeared",
        ));
    }
    let sequence = expected
        .checked_add(1)
        .ok_or_else(|| invalid("DATA history sequence exhausted"))?;
    if current.sequence == sequence
        && current.source_revision == revision
        && current.previous_after == after
    {
        return Ok(());
    }
    let start = if current.source_revision == actual.sequence {
        current.after
    } else {
        None
    };
    if actual.uncertain
        || actual.sequence != revision
        || current.sequence != expected
        || start != after
        || (current.complete && current.source_revision == revision)
    {
        return Err(conflict("DATA history source or cursor changed"));
    }
    let page = page(p.view, root.digest(), after)?;
    for origin in page.origins {
        let k = origin_key(&origin);
        let bytes = encode_origin(p.root, &origin);
        let ck = count_key(origin.region, origin.creation);
        let count_bytes = match p.writes.get(&ck) {
            Some(b) => Some(b.clone()),
            None => p.view.get(ColumnFamily::Default, &ck)?,
        };
        let count = count_bytes
            .as_deref()
            .map(|b| decode_count(p.root, origin.region, origin.creation, b))
            .transpose()?;
        match p.view.get(ColumnFamily::Default, &k)? {
            Some(old) if old == bytes && count.is_some() => {}
            Some(_) => return Err(invalid("DATA origin was rewritten")),
            None => {
                p.writes.insert(k, bytes);
                let next = count
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or_else(|| invalid("DATA origin count exhausted"))?;
                p.writes.insert(
                    ck,
                    encode_count(p.root, origin.region, origin.creation, next),
                );
            }
        }
    }
    let next = Progress {
        sequence,
        source_revision: revision,
        after: page.last,
        previous_after: after,
        complete: page.complete,
    };
    p.writes.insert(key(50, &[]), next.encode(p.root));
    Ok(())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    node: NodeId,
    store: StoreIncarnation,
    task: u64,
    // Only a validated page can construct this cursor. Scope and source
    // revision prohibit borrowing a cursor from another group or census.
    region: RegionId,
    creation: RootDigest,
    revision: u64,
    seen: u64,
}
pub struct OriginPage {
    pub origins: Vec<Origin>,
    pub next: Option<Cursor>,
    pub source_revision: u64,
    pub total_origins: u64,
}
/// Coverage is for this one metadata view and the retained placement catalog.
/// Historical actual configurations and all recovery readers remain separate.
pub fn origins(
    view: &dyn ReadView,
    root: &RootDescriptor,
    region: RegionId,
    creation: RootDigest,
    after: Option<Cursor>,
) -> Result<OriginPage> {
    let Work::Complete { revision } = observe(view, root)? else {
        return Err(conflict(
            "DATA retained-placement census incomplete or uncertain",
        ));
    };
    if region.0 < 100
        || creation.as_bytes() == &[0; 32]
        || after.is_some_and(|a| {
            a.node.0 == 0
                || a.store.as_bytes() == &[0; 16]
                || a.task < 100
                || a.region != region
                || a.creation != creation
                || a.revision != revision
                || a.seen == 0
                || a.seen % PAGE as u64 != 0
        })
    {
        return Err(invalid("DATA history query scope"));
    }
    let count_bytes = view
        .get(ColumnFamily::Default, &count_key(region, creation))?
        .ok_or_else(|| invalid("DATA origin count missing or unknown group"))?;
    let total_origins = decode_count(root.digest(), region, creation, &count_bytes)?;
    let previously_seen = after.map_or(0, |a| a.seen);
    if total_origins < 3 || previously_seen >= total_origins {
        return Err(invalid("DATA history cursor exceeds origin count"));
    }
    let mut group = region.0.to_be_bytes().to_vec();
    group.extend_from_slice(creation.as_bytes());
    let prefix = key(49, &group);
    let mut end = prefix.clone();
    while end.last() == Some(&255) {
        end.pop();
    }
    *end.last_mut().unwrap() += 1;
    let mut start = prefix;
    if let Some(a) = after {
        uint(&mut start, a.node.0);
        start.extend_from_slice(a.store.as_bytes());
        uint(&mut start, a.task);
        start.push(0);
    }
    let mut out = OriginPage {
        origins: Vec::with_capacity(PAGE),
        next: None,
        source_revision: revision,
        total_origins,
    };
    for entry in view
        .iter(ColumnFamily::Default, &start, &end)?
        .take(PAGE + 1)
    {
        let (k, v) = entry?;
        let o = decode_origin(root.digest(), &k, &v)?;
        let task_key = store_history::task_key(o.task)?;
        let task = view
            .get(ColumnFamily::Default, &task_key)?
            .ok_or_else(|| invalid("DATA origin source lost"))?;
        if !store_history::project(view, root.digest(), &task_key, &task)?.contains(&o) {
            return Err(invalid("DATA origin no longer matches its source"));
        }
        if out.origins.len() == PAGE {
            let seen = previously_seen
                .checked_add(PAGE as u64)
                .ok_or_else(|| invalid("DATA history result count exhausted"))?;
            out.next = out.origins.last().map(|o| Cursor {
                node: o.replica.node,
                store: o.replica.incarnation,
                task: o.task,
                region,
                creation,
                revision,
                seen,
            });
            break;
        }
        out.origins.push(o);
    }
    let seen = previously_seen
        .checked_add(out.origins.len() as u64)
        .ok_or_else(|| invalid("DATA history result count exhausted"))?;
    if (out.next.is_some() && seen >= total_origins)
        || (out.next.is_none() && seen != total_origins)
    {
        return Err(invalid("DATA origin index and count differ"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests;

fn checked_origin(view: &dyn ReadView, root: RootDigest, k: &[u8], v: &[u8]) -> Result<Origin> {
    let o = decode_origin(root, k, v)?;
    let task_key = store_history::task_key(o.task)?;
    let task = view
        .get(ColumnFamily::Default, &task_key)?
        .ok_or_else(|| invalid("DATA origin task absent"))?;
    if !store_history::project(view, root, &task_key, &task)?.contains(&o) {
        return Err(invalid("DATA origin source differs"));
    }
    let count = view
        .get(ColumnFamily::Default, &count_key(o.region, o.creation))?
        .ok_or_else(|| invalid("DATA origin count missing"))?;
    decode_count(root, o.region, o.creation, &count)?;
    Ok(o)
}
pub(super) fn exact_origin(
    view: &dyn ReadView,
    root: RootDigest,
    region: RegionId,
    creation: RootDigest,
    node: NodeId,
    store: StoreIncarnation,
    task: u64,
) -> Result<Origin> {
    let mut suffix = region.0.to_be_bytes().to_vec();
    suffix.extend_from_slice(creation.as_bytes());
    uint(&mut suffix, node.0);
    suffix.extend_from_slice(store.as_bytes());
    uint(&mut suffix, task);
    let k = key(49, &suffix);
    let v = view
        .get(ColumnFamily::Default, &k)?
        .ok_or_else(|| conflict("DATA membership source has no placement origin"))?;
    checked_origin(view, root, &k, &v)
}
/// One retained placement witness for this node, not a mapping of its entire
/// historical configuration interval to this (or any current) incarnation.
pub(super) fn node_origin(
    view: &dyn ReadView,
    root: RootDigest,
    region: RegionId,
    creation: RootDigest,
    node: NodeId,
) -> Result<Option<Origin>> {
    let mut suffix = region.0.to_be_bytes().to_vec();
    suffix.extend_from_slice(creation.as_bytes());
    uint(&mut suffix, node.0);
    let start = key(49, &suffix);
    let end = prefix_end(&start);
    view.iter(ColumnFamily::Default, &start, &end)?
        .next()
        .transpose()?
        .map(|(k, v)| checked_origin(view, root, &k, &v))
        .transpose()
}
fn prefix_end(prefix: &[u8]) -> Vec<u8> {
    let mut end = prefix.to_vec();
    while end.last() == Some(&255) {
        end.pop();
    }
    *end.last_mut().expect("non-maximal namespace prefix") += 1;
    end
}
#[derive(Debug, Clone)]
pub struct SourceCursor(Vec<u8>);
/// One store per pass, skipping duplicate task origins with an ordered prefix
/// seek. Exhaustion is only a scheduling wrap, never global coverage proof.
pub fn next_source(
    view: &dyn ReadView,
    root: &RootDescriptor,
    after: Option<&SourceCursor>,
) -> Result<Option<(SourceCursor, Origin, u64)>> {
    let Work::Complete { revision } = observe(view, root)? else {
        return Err(conflict("DATA placement census is not current"));
    };
    let start = after
        .map(|a| prefix_end(&a.0))
        .unwrap_or_else(|| key(49, &[]));
    let Some((k, v)) = view
        .iter(ColumnFamily::Default, &start, &key(50, &[]))?
        .next()
        .transpose()?
    else {
        return Ok(None);
    };
    let origin = checked_origin(view, root.digest(), &k, &v)?;
    let prefix = k[..k.len() - 8].to_vec(); // exclude task, retain exact source store
    Ok(Some((SourceCursor(prefix), origin, revision)))
}
