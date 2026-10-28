//! Durable exact-source configuration observations. A completed source round
//! does not establish configuration intervals or snapshot ancestry, and is
//! never an all-store closure, release or garbage-collection capability.
use super::data_membership::{check_source, planner};
pub use super::data_membership::{Request, Source};
use super::*;
use crate::data_groups::store_history::validate_configuration_origin;
use crate::endpoint::NodeEndpoint;
use kv9_common::AppliedPosition;
use kv9_raft::configuration_origin::{ConfigurationOrigin, ORIGIN_BYTES};
use kv9_raft::storage::configuration_origins::{
    AppliedConfigurationOrigin, OriginCursor, OriginPage,
};

pub mod initial;

const PROGRESS_MAGIC: &[u8; 8] = b"KV9DOP01";
const ENTRY_MAGIC: &[u8; 8] = b"KV9DOE01";

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
    pub last_retained_index: u64,
    /// Sticky uncertainty. A later clean page cannot certify missing history.
    pub source_history_unknown: bool,
    pub page: OriginPage,
    last_request: RootDigest,
}
impl Progress {
    pub fn round_complete(&self) -> bool {
        self.page.next.is_none()
    }
    pub fn has_known_gaps(&self) -> bool {
        self.source_history_unknown || self.page.initial.as_ref().is_none_or(|i| i.is_unknown())
    }
    pub fn request(&self, task: u64, revision: u64) -> LedgerRequest {
        LedgerRequest::RecordConfigurationOrigins(Request {
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
            self.last_retained_index,
        ] {
            uint(&mut b, n);
        }
        b.push(u8::from(self.source_history_unknown));
        b.extend_from_slice(self.last_request.as_bytes());
        b.extend_from_slice(&(page.len() as u16).to_be_bytes());
        b.extend_from_slice(&page);
        Ok(seal(b))
    }
}
pub(super) fn check_old_format(view: &dyn ReadView) -> Result<()> {
    if view
        .iter(ColumnFamily::Default, &key(54, &[]), &key(56, &[]))?
        .next()
        .transpose()?
        .is_some()
    {
        return Err(invalid(
            "configuration origins under an older ledger header",
        ));
    }
    Ok(())
}
fn check_entry(root: RootDigest, source: Source, entry: AppliedConfigurationOrigin) -> Result<()> {
    if entry.at.term == 0
        || entry.at.index == 0
        || entry.at.index == u64::MAX
        || entry.origin.root != root
        || entry.origin.region != source.region
        || entry.origin.creation != source.creation
    {
        return Err(invalid("configuration origin position or scope"));
    }
    entry.origin.encode()?;
    Ok(())
}
fn read(p: &Planner<'_>, source: Source) -> Result<Option<Progress>> {
    let Some(bytes) = p.view.get(ColumnFamily::Default, &source.key(54)?)? else {
        return Ok(None);
    };
    let mut r = Reader(checked(&bytes, PROGRESS_MAGIC, 1264)?);
    if r.array::<32>()? != *p.root.as_bytes() || Source::decode(&mut r)? != source {
        return Err(invalid("configuration origin progress scope"));
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
        last_retained_index: r.uint()?,
        source_history_unknown: r.boolean()?,
        last_request: RootDigest::from_bytes(r.array()?),
        page: {
            let len = u16::from_be_bytes(r.array()?) as usize;
            OriginPage::decode(r.take(len)?)?
        },
    };
    if !r.0.is_empty()
        || out.sequence == 0
        || out.sequence > out.recorded_at
        || out.recorded_at > p.revision
        || out.source_revision == 0
        || out.round == 0
        || out.round > out.sequence
        || out.seen < out.page.entries.len() as u64
        || out.seen > out.retained_entries
        || out.retained_entries > out.page.total_known
        || (out.retained_entries == 0) != (out.last_retained_index == 0)
        || out.last_retained_index > out.page.last_configuration_index
        || out
            .page
            .entries
            .last()
            .is_some_and(|e| e.at.index > out.last_retained_index)
        || (out.round_complete() && out.seen != out.page.total_known)
        || out.page.next.is_some_and(|c| c.seen() != out.seen)
        || ((out.page.membership_gaps.any()
            || out.page.legacy_contexts_unknown
            || out.page.initial.as_ref().is_some_and(|i| i.is_unknown()))
            && !out.source_history_unknown)
        || out.last_request.as_bytes() == &[0; 32]
        || out.encode(p.root)? != bytes
    {
        return Err(invalid("configuration origin progress encoding or count"));
    }
    for entry in &out.page.entries {
        check_entry(p.root, source, *entry)?;
    }
    initial::validate_progress(p, &out)?;
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
        return Err(conflict("configuration origin receipt differs"));
    }
    planner(view, root)?.finish()
}
pub fn continuation(
    view: &dyn ReadView,
    root: &RootDescriptor,
    q: Request,
) -> Result<Option<OriginCursor>> {
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
            "configuration origin sequence or revision changed",
        )),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub ordinal: u64,
    pub applied: AppliedConfigurationOrigin,
    pub first_observed: u64,
    pub last_observed: u64,
    pub round: u64,
}
fn entry_key(source: Source, index: u64) -> Result<Vec<u8>> {
    let mut k = source.key(55)?;
    uint(&mut k, index);
    Ok(k)
}
impl Entry {
    fn encode(&self, root: RootDigest, source: Source) -> Result<Vec<u8>> {
        check_entry(root, source, self.applied)?;
        let mut b = ENTRY_MAGIC.to_vec();
        b.extend_from_slice(root.as_bytes());
        b.extend_from_slice(&source.encode()?);
        for n in [self.ordinal, self.applied.at.term, self.applied.at.index] {
            uint(&mut b, n);
        }
        b.extend_from_slice(&self.applied.origin.encode()?);
        for n in [self.first_observed, self.last_observed, self.round] {
            uint(&mut b, n);
        }
        Ok(seal(b))
    }
    fn decode(p: &Planner<'_>, source: Source, k: &[u8], b: &[u8]) -> Result<Self> {
        let mut r = Reader(checked(b, ENTRY_MAGIC, 353)?);
        if r.array::<32>()? != *p.root.as_bytes() || Source::decode(&mut r)? != source {
            return Err(invalid("configuration origin entry scope"));
        }
        let out = Self {
            ordinal: r.uint()?,
            applied: AppliedConfigurationOrigin {
                at: AppliedPosition {
                    term: r.uint()?,
                    index: r.uint()?,
                },
                origin: ConfigurationOrigin::decode(r.take(ORIGIN_BYTES)?)?,
            },
            first_observed: r.uint()?,
            last_observed: r.uint()?,
            round: r.uint()?,
        };
        if !r.0.is_empty()
            || out.ordinal == 0
            || out.first_observed == 0
            || out.first_observed > out.last_observed
            || out.last_observed > p.revision
            || out.round == 0
            || out.round > out.last_observed
            || k != entry_key(source, out.applied.at.index)?
            || out.encode(p.root, source)? != b
        {
            return Err(invalid("configuration origin entry encoding"));
        }
        Ok(out)
    }
}
/// Trusted runtime seam: require authenticated source/cursor correlation and
/// exact endpoint equality under the metadata catalog lock before this call.
/// Original immutable tasks are independently rechecked in this same view.
pub fn plan_verified(
    view: &dyn ReadView,
    root: &RootDescriptor,
    q: Request,
    endpoint: NodeEndpoint,
    cursor: Option<OriginCursor>,
    page: &OriginPage,
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
        return Err(conflict("configuration origin endpoint differs"));
    }
    let current = read(&p, q.source)?;
    if current.as_ref().map_or(0, |v| v.sequence) != q.expected {
        return Err(conflict("configuration origin sequence changed"));
    }
    if current.as_ref().is_some_and(|v| {
        page.last_configuration_index < v.page.last_configuration_index
            || page.total_known < v.page.total_known
    }) {
        return Err(conflict("configuration origin history regressed"));
    }
    page.validate_progress(cursor)?;
    if cursor.is_some()
        && (continuation(view, root, q)? != cursor
            || current
                .as_ref()
                .is_none_or(|v| v.endpoint_generation != endpoint.generation))
    {
        return Err(conflict("configuration origin continuation differs"));
    }
    for entry in &page.entries {
        check_entry(p.root, q.source, *entry)?;
        validate_configuration_origin(view, p.root, &entry.origin)?;
    }
    initial::plan(&mut p, q, page)?;
    if current.as_ref().is_some_and(|v| {
        v.round_complete()
            && v.source_revision == q.revision
            && v.endpoint_generation == endpoint.generation
            && v.page.fingerprint == page.fingerprint
            && v.page.initial == page.initial
            && cursor.is_none()
    }) {
        return p.finish();
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
    let mut last_retained_index = current.as_ref().map_or(0, |v| v.last_retained_index);
    let mut seen = cursor.map_or(0, |c| c.seen());
    for applied in &page.entries {
        seen = seen
            .checked_add(1)
            .ok_or_else(|| invalid("configuration origin count exhausted"))?;
        let k = entry_key(q.source, applied.at.index)?;
        let old = view
            .get(ColumnFamily::Default, &k)?
            .map(|b| Entry::decode(&p, q.source, &k, &b))
            .transpose()?;
        if let Some(old) = &old {
            if old.applied != *applied || old.ordinal != seen {
                return Err(conflict("configuration origin immutable history differs"));
            }
        } else {
            if retained.checked_add(1) != Some(seen) || applied.at.index <= last_retained_index {
                return Err(conflict("configuration origin retained prefix differs"));
            }
            retained = seen;
            last_retained_index = applied.at.index;
        }
        let entry = Entry {
            ordinal: seen,
            applied: *applied,
            first_observed: old.as_ref().map_or(at, |v| v.first_observed),
            last_observed: at,
            round,
        };
        p.writes.insert(k, entry.encode(p.root, q.source)?);
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
        last_retained_index,
        source_history_unknown: page.membership_gaps.any()
            || page.initial.as_ref().is_some_and(|i| i.is_unknown())
            || page.legacy_contexts_unknown
            || current.as_ref().is_some_and(|v| v.source_history_unknown),
        page: page.clone(),
        last_request: q.digest()?,
    };
    p.writes.insert(q.source.key(54)?, progress.encode(p.root)?);
    p.finish()
}

mod entries;
pub use entries::{entries, EntryCursor, EntryPage};
#[cfg(test)]
mod tests;
