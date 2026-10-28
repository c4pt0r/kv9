//! Bounded projection of durable actual configurations onto exact historical
//! stores. These are per-source observations, not all-store closure or GC proof.
use super::data_configuration_origins::initial;
use super::data_configuration_timeline as timeline;
use super::data_membership::planner;
pub use super::data_membership::Source;
use super::*;
use kv9_raft::storage::configuration_timeline::{ConfigurationRecord, RecordKind};

mod projection;
pub use projection::{
    Boundary, Member, AMBIGUOUS_POSITION, LEGACY_CHANGE, MISSING_NATIVE, SNAPSHOT_ANCESTRY,
    SOURCE_HISTORY_GAPS, STORE_CONFLICT, TRANSITION_MISMATCH, UNKNOWN_STORE, UNSUPPORTED_STATE,
};
mod query;
mod wire;
pub use query::{intervals, Interval, IntervalCursor, IntervalPage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    pub source: Source,
    pub expected: u64,
    pub timeline_sequence: u64,
    /// Zero means no native initial mapping was present in the input view.
    pub native_at: u64,
}
impl Request {
    pub(super) fn encode(self) -> Result<Vec<u8>> {
        if self.expected == u64::MAX || self.timeline_sequence == 0 {
            return Err(invalid("configuration interval request position"));
        }
        let mut b = self.source.encode()?;
        for n in [self.expected, self.timeline_sequence, self.native_at] {
            uint(&mut b, n);
        }
        Ok(b)
    }
    pub(super) fn decode(r: &mut Reader<'_>) -> Result<Self> {
        let q = Self {
            source: Source::decode(r)?,
            expected: r.uint()?,
            timeline_sequence: r.uint()?,
            native_at: r.uint()?,
        };
        q.encode()?;
        Ok(q)
    }
    fn digest(self) -> Result<RootDigest> {
        Ok(RootDigest::sha256(&self.encode()?))
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub source: Source,
    pub sequence: u64,
    pub recorded_at: u64,
    /// Changes only when native input becomes available; query pages never mix generations.
    pub generation: u64,
    pub timeline_sequence: u64,
    pub native_at: u64,
    pub processed: u64,
    pub retained: u64,
    pub total: u64,
    pub gaps: u16,
    last_request: RootDigest,
}
impl Progress {
    /// Describes this retained source prefix, never all history or live freshness.
    pub fn round_complete(&self) -> bool {
        self.processed == self.total
    }
    pub fn has_known_gaps(&self) -> bool {
        self.gaps != 0
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub ordinal: u64,
    pub generation: u64,
    pub recorded_at: u64,
    pub source_digest: RootDigest,
    pub members: Option<Vec<Member>>,
    pub gaps: u16,
}
fn row_key(source: Source, ordinal: u64) -> Result<Vec<u8>> {
    let mut k = source.key(60)?;
    uint(&mut k, ordinal);
    Ok(k)
}
fn row_end(source: Source) -> Result<Vec<u8>> {
    let mut k = source.key(60)?;
    while k.last() == Some(&255) {
        k.pop();
    }
    *k.last_mut().unwrap() += 1;
    Ok(k)
}
fn actual(p: &Planner<'_>, source: Source, ordinal: u64) -> Result<ConfigurationRecord> {
    timeline::read_entry(p, source, ordinal)?
        .map(|e| e.record)
        .ok_or_else(|| invalid("configuration interval actual source record missing"))
}
fn read_entry(p: &Planner<'_>, source: Source, ordinal: u64) -> Result<Option<Entry>> {
    let key = row_key(source, ordinal)?;
    p.view
        .get(ColumnFamily::Default, &key)?
        .map(|b| Entry::decode(p, source, ordinal, &b))
        .transpose()
}
pub(super) fn check_old_format(view: &dyn ReadView) -> Result<()> {
    if view
        .iter(ColumnFamily::Default, &key(59, &[]), &key(61, &[]))?
        .next()
        .transpose()?
        .is_some()
    {
        return Err(invalid("configuration intervals under older ledger header"));
    }
    Ok(())
}
fn read(p: &Planner<'_>, source: Source) -> Result<Option<Progress>> {
    let value = p.view.get(ColumnFamily::Default, &source.key(59)?)?;
    let out = value.map(|b| Progress::decode(p, source, &b)).transpose()?;
    let retained = out.as_ref().map_or(0, |v| v.retained);
    let mut start = if retained == 0 {
        source.key(60)?
    } else {
        row_key(source, retained)?
    };
    if retained != 0 {
        start.push(0);
    }
    if p.view
        .iter(ColumnFamily::Default, &start, &row_end(source)?)?
        .next()
        .transpose()?
        .is_some()
    {
        return Err(invalid("configuration interval rows ahead of progress"));
    }
    if let Some(v) = &out {
        for n in [v.processed, v.retained] {
            if n == 0 {
                continue;
            }
            let tail = read_entry(p, source, n)?
                .ok_or_else(|| invalid("configuration interval tail missing"))?;
            if tail.recorded_at > v.recorded_at
                || tail.generation > v.generation
                || (n == v.processed && tail.generation != v.generation)
            {
                return Err(invalid("configuration interval tail differs"));
            }
        }
    }
    Ok(out)
}
struct Input {
    timeline: timeline::Progress,
    native: Option<initial::InitialOrigin>,
}
fn input(p: &Planner<'_>, root: &RootDescriptor, source: Source) -> Result<Option<Input>> {
    let Some(timeline) = timeline::read(p, source)? else {
        return Ok(None);
    };
    let native = initial::observation(p.view, root, source)?;
    if let Some(n) = &native {
        let origin = data_store_history::exact_origin(
            p.view,
            p.root,
            source.region,
            source.creation,
            source.node,
            source.store,
            n.creation_task,
        )?;
        let (c, digest) =
            crate::data_groups::store_history::initial_creation(p.view, p.root, &origin)?;
        if digest != n.source_digest || c.task() != n.creation_task || c.replicas() != n.replicas {
            return Err(invalid("configuration interval native creation changed"));
        }
    }
    Ok(Some(Input { timeline, native }))
}
pub fn observation(
    view: &dyn ReadView,
    root: &RootDescriptor,
    source: Source,
) -> Result<Option<Progress>> {
    read(&planner(view, root)?, source)
}
/// At most a few source/record point reads; no walk through earlier configurations.
pub fn next(view: &dyn ReadView, root: &RootDescriptor, source: Source) -> Result<Option<Request>> {
    let p = planner(view, root)?;
    let Some(input) = input(&p, root, source)? else {
        return Ok(None);
    };
    let old = read(&p, source)?;
    let native_at = input.native.as_ref().map_or(0, |n| n.recorded_at);
    if old.as_ref().is_some_and(|o| {
        o.timeline_sequence == input.timeline.sequence
            && o.native_at == native_at
            && o.round_complete()
    }) {
        return Ok(None);
    }
    Ok(Some(Request {
        source,
        expected: old.map_or(0, |o| o.sequence),
        timeline_sequence: input.timeline.sequence,
        native_at,
    }))
}
pub(super) fn advance(p: &mut Planner<'_>, root: &RootDescriptor, q: Request) -> Result<()> {
    let digest = q.digest()?;
    let old = read(p, q.source)?;
    if old
        .as_ref()
        .is_some_and(|o| o.sequence == q.expected + 1 && o.last_request == digest)
    {
        return Ok(());
    }
    if old.as_ref().map_or(0, |o| o.sequence) != q.expected {
        return Err(conflict("configuration interval sequence differs"));
    }
    let input =
        input(p, root, q.source)?.ok_or_else(|| conflict("configuration timeline not observed"))?;
    let native_at = input.native.as_ref().map_or(0, |n| n.recorded_at);
    if q.timeline_sequence != input.timeline.sequence || q.native_at != native_at {
        return Err(conflict("configuration interval inputs changed"));
    }
    if old.as_ref().is_some_and(|o| {
        o.retained > input.timeline.retained_entries
            || o.timeline_sequence > input.timeline.sequence
            || (o.native_at != 0 && o.native_at != native_at)
    }) {
        return Err(conflict("configuration interval inputs regressed"));
    }
    if old.as_ref().is_some_and(|o| {
        o.timeline_sequence == q.timeline_sequence && o.native_at == native_at && o.round_complete()
    }) {
        return Ok(());
    }
    let sequence = q.expected + 1;
    let at = p
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid("ledger revision exhausted"))?;
    let restart = old.as_ref().is_none_or(|o| o.native_at != native_at);
    let generation = if restart {
        sequence
    } else {
        old.as_ref().unwrap().generation
    };
    let mut seen = if restart {
        0
    } else {
        old.as_ref().unwrap().processed
    };
    let mut retained = old.as_ref().map_or(0, |o| o.retained);
    let mut gaps = if restart {
        0
    } else {
        old.as_ref().unwrap().gaps
    };
    if input.timeline.has_known_gaps() {
        gaps |= SOURCE_HISTORY_GAPS;
    }
    let mut previous = if seen == 0 {
        None
    } else {
        let entry = read_entry(p, q.source, seen)?
            .ok_or_else(|| invalid("configuration interval continuation missing"))?;
        Some((actual(p, q.source, seen)?, entry))
    };
    for _ in 0..4 {
        if seen == input.timeline.retained_entries {
            break;
        }
        let ordinal = seen
            .checked_add(1)
            .ok_or_else(|| invalid("configuration interval count exhausted"))?;
        let record = actual(p, q.source, ordinal)?;
        if let Some(origin) = record.origin {
            crate::data_groups::store_history::validate_configuration_origin(
                p.view,
                p.root,
                &origin.origin,
            )?;
        }
        let prior = read_entry(p, q.source, ordinal)?;
        if (prior.is_none() && ordinal <= retained)
            || prior.as_ref().is_some_and(|e| e.generation > generation)
        {
            return Err(invalid("configuration interval retained prefix differs"));
        }
        let (members, local_gaps) = projection::project(
            &record,
            previous.as_ref().map(|(r, e)| (r, e)),
            input.native.as_ref(),
        )?;
        let entry = Entry {
            ordinal,
            generation,
            recorded_at: at,
            source_digest: RootDigest::sha256(&record.encode()?),
            members,
            gaps: local_gaps,
        };
        projection::validate_members(&record, &entry)?;
        p.writes
            .insert(row_key(q.source, ordinal)?, entry.encode(p.root, q.source)?);
        gaps |= local_gaps;
        seen = ordinal;
        retained = retained.max(seen);
        previous = Some((record, entry));
    }
    let progress = Progress {
        source: q.source,
        sequence,
        recorded_at: at,
        generation,
        timeline_sequence: q.timeline_sequence,
        native_at,
        processed: seen,
        retained,
        total: input.timeline.retained_entries,
        gaps,
        last_request: digest,
    };
    p.writes.insert(q.source.key(59)?, progress.encode(p.root)?);
    Ok(())
}

#[cfg(test)]
mod tests;
