//! Durable actual configuration-state observations from one exact store.
//! Completion describes one source round, never complete store intervals,
//! imported ancestry, DATA closure or authority to release retained objects.
use super::data_membership::{check_source, planner};
pub use super::data_membership::{Request, Source};
use super::*;
use crate::data_groups::store_history::validate_configuration_origin;
use crate::endpoint::NodeEndpoint;
use kv9_raft::storage::configuration_timeline::{
    ConfigurationRecord, RecordKind, TimelineCursor, TimelinePage, MAX_PAGE_BYTES, RECORDS_PER_PAGE,
};

const PROGRESS_MAGIC: &[u8; 8] = b"KV9DTP01";
const ENTRY_MAGIC: &[u8; 8] = b"KV9DTE01";
const MAX_PROGRESS_BYTES: usize = MAX_PAGE_BYTES + 229;
const MAX_ENTRY_BYTES: usize = 33_188;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub source: Source,
    pub sequence: u64,
    pub source_revision: u64,
    pub endpoint_generation: u64,
    pub recorded_at: u64,
    pub round: u64,
    pub seen: u64,
    pub retained_entries: u64,
    /// Sticky uncertainty includes applied legacy records with no exact origin.
    pub source_history_unknown: bool,
    pub page: TimelinePage,
    last_request: RootDigest,
}
fn has_gaps(page: &TimelinePage) -> bool {
    page.total == 0
        || page.gaps.any()
        || page.records.iter().any(|r| {
            r.state.is_none() || matches!(r.kind, RecordKind::Applied { .. }) && r.origin.is_none()
        })
}
impl Progress {
    pub fn round_complete(&self) -> bool {
        self.page.next.is_none()
    }
    pub fn has_known_gaps(&self) -> bool {
        self.source_history_unknown
    }
    pub fn request(&self, task: u64, revision: u64) -> LedgerRequest {
        LedgerRequest::RecordConfigurationTimeline(Request {
            source: self.source,
            task,
            revision,
            expected: self.sequence,
        })
    }
    fn encode(&self, root: RootDigest) -> Result<Vec<u8>> {
        let page = self.page.encode()?;
        let mut b = PROGRESS_MAGIC.to_vec();
        b.extend_from_slice(root.as_bytes());
        b.extend_from_slice(&self.source.encode()?);
        for n in [
            self.sequence,
            self.source_revision,
            self.endpoint_generation,
            self.recorded_at,
            self.round,
            self.seen,
            self.retained_entries,
        ] {
            uint(&mut b, n);
        }
        b.push(u8::from(self.source_history_unknown));
        b.extend_from_slice(self.last_request.as_bytes());
        b.extend_from_slice(&(page.len() as u32).to_be_bytes());
        b.extend_from_slice(&page);
        Ok(seal(b))
    }
}
pub(super) fn check_old_format(view: &dyn ReadView) -> Result<()> {
    if view
        .iter(ColumnFamily::Default, &key(57, &[]), &key(59, &[]))?
        .next()
        .transpose()?
        .is_some()
    {
        return Err(invalid("configuration timeline under older ledger header"));
    }
    Ok(())
}
fn check_record(root: RootDigest, source: Source, record: &ConfigurationRecord) -> Result<()> {
    record.encode()?;
    if record.origin.is_some_and(|o| {
        o.origin.root != root
            || o.origin.region != source.region
            || o.origin.creation != source.creation
    }) {
        return Err(invalid("configuration timeline record scope"));
    }
    Ok(())
}
fn entry_key(source: Source, ordinal: u64) -> Result<Vec<u8>> {
    let mut k = source.key(58)?;
    uint(&mut k, ordinal);
    Ok(k)
}
fn entry_end(source: Source) -> Result<Vec<u8>> {
    let mut end = source.key(58)?;
    while end.last() == Some(&255) {
        end.pop();
    }
    *end.last_mut().unwrap() += 1;
    Ok(end)
}
fn check_no_extra(p: &Planner<'_>, source: Source, retained: u64) -> Result<()> {
    let mut start = entry_key(source, retained)?;
    // Without progress even malformed/zero ordinal rows must be refused.
    if retained == 0 {
        start = source.key(58)?;
    } else {
        start.push(0);
    }
    if p.view
        .iter(ColumnFamily::Default, &start, &entry_end(source)?)?
        .next()
        .transpose()?
        .is_some()
    {
        return Err(invalid("configuration timeline rows ahead of progress"));
    }
    Ok(())
}
pub(super) fn read(p: &Planner<'_>, source: Source) -> Result<Option<Progress>> {
    let Some(bytes) = p.view.get(ColumnFamily::Default, &source.key(57)?)? else {
        check_no_extra(p, source, 0)?;
        return Ok(None);
    };
    let mut r = Reader(checked(&bytes, PROGRESS_MAGIC, MAX_PROGRESS_BYTES)?);
    if r.array::<32>()? != *p.root.as_bytes() || Source::decode(&mut r)? != source {
        return Err(invalid("configuration timeline progress scope"));
    }
    let out = Progress {
        source,
        sequence: r.uint()?,
        source_revision: r.uint()?,
        endpoint_generation: r.uint()?,
        recorded_at: r.uint()?,
        round: r.uint()?,
        seen: r.uint()?,
        retained_entries: r.uint()?,
        source_history_unknown: r.boolean()?,
        last_request: RootDigest::from_bytes(r.array()?),
        page: {
            let len = u32::from_be_bytes(r.array()?) as usize;
            TimelinePage::decode(r.take(len)?)?
        },
    };
    if !r.0.is_empty()
        || out.sequence == 0
        || out.sequence > out.recorded_at
        || out.recorded_at > p.revision
        || out.source_revision == 0
        || out.round == 0
        || out.round > out.sequence
        || out.seen != out.page.records.last().map_or(0, |r| r.ordinal)
        || out.seen < out.page.records.len() as u64
        || out.seen > out.retained_entries
        || out.retained_entries > out.page.total
        || (out.round_complete() && out.seen != out.page.total)
        || out.page.next.is_some_and(|c| c.seen() != out.seen)
        || (has_gaps(&out.page) && !out.source_history_unknown)
        || out.last_request.as_bytes() == &[0; 32]
        || out.encode(p.root)? != bytes
    {
        return Err(invalid("configuration timeline progress encoding or count"));
    }
    // Check the actual atomic page, not just its embedded copy in progress.
    for record in &out.page.records {
        let entry = read_entry(p, source, record.ordinal)?
            .ok_or_else(|| invalid("configuration timeline retained page missing"))?;
        if entry.record != *record
            || entry.last_observed != out.recorded_at
            || entry.round != out.round
        {
            return Err(invalid("configuration timeline retained page differs"));
        }
    }
    if out.retained_entries > out.seen {
        let tail = read_entry(p, source, out.retained_entries)?
            .ok_or_else(|| invalid("configuration timeline retained tail missing"))?;
        if tail.last_observed > out.recorded_at || tail.round > out.round {
            return Err(invalid("configuration timeline retained tail ahead"));
        }
    }
    check_no_extra(p, source, out.retained_entries)?;
    Ok(Some(out))
}
pub fn observation(
    view: &dyn ReadView,
    root: &RootDescriptor,
    source: Source,
) -> Result<Option<Progress>> {
    read(&planner(view, root)?, source)
}
pub fn is_retry(view: &dyn ReadView, root: &RootDescriptor, q: Request) -> Result<bool> {
    let digest = q.digest()?;
    Ok(read(&planner(view, root)?, q.source)?
        .is_some_and(|p| p.sequence == q.expected + 1 && p.last_request == digest))
}
pub fn plan_retry(view: &dyn ReadView, root: &RootDescriptor, q: Request) -> Result<LedgerPlan> {
    if !is_retry(view, root, q)? {
        return Err(conflict("configuration timeline receipt differs"));
    }
    planner(view, root)?.finish()
}
pub fn continuation(
    view: &dyn ReadView,
    root: &RootDescriptor,
    q: Request,
) -> Result<Option<TimelineCursor>> {
    let p = planner(view, root)?;
    check_source(&p, root, q)?;
    match read(&p, q.source)? {
        None if q.expected == 0 => Ok(None),
        Some(old) if old.sequence == q.expected && old.source_revision <= q.revision => {
            Ok(if old.source_revision == q.revision {
                old.page.next
            } else {
                None
            })
        }
        _ => Err(conflict(
            "configuration timeline sequence or revision changed",
        )),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub record: ConfigurationRecord,
    pub first_observed: u64,
    pub last_observed: u64,
    pub round: u64,
}
impl Entry {
    fn encode(&self, root: RootDigest, source: Source) -> Result<Vec<u8>> {
        check_record(root, source, &self.record)?;
        let record = self.record.encode()?;
        let mut b = ENTRY_MAGIC.to_vec();
        b.extend_from_slice(root.as_bytes());
        b.extend_from_slice(&source.encode()?);
        for n in [self.first_observed, self.last_observed, self.round] {
            uint(&mut b, n);
        }
        b.extend_from_slice(&(record.len() as u32).to_be_bytes());
        b.extend_from_slice(&record);
        Ok(seal(b))
    }
    fn decode(p: &Planner<'_>, source: Source, k: &[u8], b: &[u8]) -> Result<Self> {
        let mut r = Reader(checked(b, ENTRY_MAGIC, MAX_ENTRY_BYTES)?);
        if r.array::<32>()? != *p.root.as_bytes() || Source::decode(&mut r)? != source {
            return Err(invalid("configuration timeline entry scope"));
        }
        let first_observed = r.uint()?;
        let last_observed = r.uint()?;
        let round = r.uint()?;
        let len = u32::from_be_bytes(r.array()?) as usize;
        let record = ConfigurationRecord::decode(r.take(len)?)?;
        let out = Self {
            record,
            first_observed,
            last_observed,
            round,
        };
        if !r.0.is_empty()
            || out.first_observed == 0
            || out.first_observed > out.last_observed
            || out.last_observed > p.revision
            || out.round == 0
            || out.round > out.last_observed
            || k != entry_key(source, out.record.ordinal)?
            || out.encode(p.root, source)? != b
        {
            return Err(invalid("configuration timeline entry encoding"));
        }
        Ok(out)
    }
}
pub(super) fn read_entry(p: &Planner<'_>, source: Source, ordinal: u64) -> Result<Option<Entry>> {
    let k = entry_key(source, ordinal)?;
    p.view
        .get(ColumnFamily::Default, &k)?
        .map(|b| Entry::decode(p, source, &k, &b))
        .transpose()
}
/// Trusted runtime seam: require authenticated cursor/source correlation and
/// exact endpoint equality under the catalog lock. Recheck every known origin
/// against its immutable original task in this same metadata view.
pub fn plan_verified(
    view: &dyn ReadView,
    root: &RootDescriptor,
    q: Request,
    endpoint: NodeEndpoint,
    cursor: Option<TimelineCursor>,
    page: &TimelinePage,
) -> Result<LedgerPlan> {
    let mut p = planner(view, root)?;
    check_source(&p, root, q)?;
    if !endpoint.active
        || endpoint.node != q.source.node
        || endpoint.incarnation != q.source.store
        || endpoint.address.port() == 0
        || endpoint.address.ip().is_unspecified()
        || root
            .voter(endpoint.node)
            .is_some_and(|v| v.store_incarnation != endpoint.incarnation)
    {
        return Err(conflict("configuration timeline endpoint differs"));
    }
    let current = read(&p, q.source)?;
    if current.as_ref().map_or(0, |v| v.sequence) != q.expected {
        return Err(conflict("configuration timeline sequence changed"));
    }
    if current
        .as_ref()
        .is_some_and(|v| page.total < v.page.total || v.source_revision > q.revision)
    {
        return Err(conflict("configuration timeline history regressed"));
    }
    page.encode()?;
    page.validate_progress(cursor)?;
    if cursor.is_some()
        && (continuation(view, root, q)? != cursor
            || current
                .as_ref()
                .is_none_or(|v| v.endpoint_generation != endpoint.generation))
    {
        return Err(conflict("configuration timeline continuation differs"));
    }
    let sequence = q.expected + 1;
    let at = p
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid("ledger revision exhausted"))?;
    let round = if cursor.is_none() {
        sequence
    } else {
        current.as_ref().unwrap().round
    };
    let mut retained = current.as_ref().map_or(0, |v| v.retained_entries);
    let mut seen = cursor.map_or(0, |c| c.seen());
    let mut updates = Vec::with_capacity(RECORDS_PER_PAGE);
    for record in &page.records {
        check_record(p.root, q.source, record)?;
        if let Some(origin) = &record.origin {
            validate_configuration_origin(view, p.root, &origin.origin)?;
        }
        seen = seen
            .checked_add(1)
            .ok_or_else(|| invalid("configuration timeline count exhausted"))?;
        if seen != record.ordinal {
            return Err(conflict("configuration timeline ordinal differs"));
        }
        let old = read_entry(&p, q.source, record.ordinal)?;
        if let Some(old) = &old {
            if old.record != *record || record.ordinal > retained {
                return Err(conflict("configuration timeline immutable prefix differs"));
            }
        } else {
            if retained.checked_add(1) != Some(seen) {
                return Err(conflict("configuration timeline retained prefix missing"));
            }
            retained = seen;
        }
        updates.push(Entry {
            record: record.clone(),
            first_observed: old.as_ref().map_or(at, |v| v.first_observed),
            last_observed: at,
            round,
        });
    }
    // Confirm the prefix above even on a stable round; no-op does not refresh
    // historical timestamps, nor repair a missing/corrupt durable record.
    if current.as_ref().is_some_and(|v| {
        v.round_complete()
            && v.source_revision == q.revision
            && v.endpoint_generation == endpoint.generation
            && v.page.fingerprint == page.fingerprint
            && v.page.total == page.total
            && (!has_gaps(page) || v.source_history_unknown)
            && cursor.is_none()
    }) {
        return p.finish();
    }
    for entry in updates {
        p.writes.insert(
            entry_key(q.source, entry.record.ordinal)?,
            entry.encode(p.root, q.source)?,
        );
    }
    let progress = Progress {
        source: q.source,
        sequence,
        source_revision: q.revision,
        endpoint_generation: endpoint.generation,
        recorded_at: at,
        round,
        seen,
        retained_entries: retained,
        source_history_unknown: has_gaps(page)
            || current.as_ref().is_some_and(|v| v.source_history_unknown),
        page: page.clone(),
        last_request: q.digest()?,
    };
    p.writes.insert(q.source.key(57)?, progress.encode(p.root)?);
    p.finish()
}

mod entries;
pub use entries::{entries, EntryCursor, EntryPage};
#[cfg(test)]
mod tests;
