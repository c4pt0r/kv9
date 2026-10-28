use super::*;

#[derive(Debug, Clone, Copy)]
pub struct EntryCursor {
    source: Source,
    sequence: u64,
    after: u64,
}
pub struct EntryPage {
    pub progress: Progress,
    /// Describes retained placement revision, never live source freshness.
    pub placement_current: bool,
    pub entries: Vec<Entry>,
    pub next: Option<EntryCursor>,
}
pub fn entries(
    view: &dyn ReadView,
    root: &RootDescriptor,
    source: Source,
    after: Option<EntryCursor>,
) -> Result<EntryPage> {
    let p = planner(view, root)?;
    let progress =
        read(&p, source)?.ok_or_else(|| conflict("configuration timeline not observed"))?;
    if after.is_some_and(|c| {
        c.source != source
            || c.sequence != progress.sequence
            || c.after == 0
            || !c.after.is_multiple_of(RECORDS_PER_PAGE as u64)
            || c.after >= progress.retained_entries
    }) {
        return Err(conflict("configuration timeline query cursor differs"));
    }
    let mut start = source.key(58)?;
    if let Some(c) = after {
        uint(&mut start, c.after);
        start.push(0);
    }
    let mut rows = Vec::with_capacity(RECORDS_PER_PAGE);
    let mut more = false;
    let mut seen = after.map_or(0, |c| c.after);
    for row in view
        .iter(ColumnFamily::Default, &start, &entry_end(source)?)?
        .take(RECORDS_PER_PAGE + 1)
    {
        let (k, b) = row?;
        let entry = Entry::decode(&p, source, &k, &b)?;
        if entry.round > progress.round
            || entry.last_observed > progress.recorded_at
            || seen.checked_add(1) != Some(entry.record.ordinal)
            || entry.record.ordinal > progress.retained_entries
        {
            return Err(invalid("configuration timeline entry position differs"));
        }
        if rows.len() == RECORDS_PER_PAGE {
            more = true;
            break;
        }
        seen = entry.record.ordinal;
        rows.push(entry);
    }
    let count_matches = if more {
        seen < progress.retained_entries
    } else {
        seen == progress.retained_entries
    };
    if !count_matches {
        return Err(invalid("configuration timeline entry count differs"));
    }
    let next = more.then_some(EntryCursor {
        source,
        sequence: progress.sequence,
        after: seen,
    });
    let placement_current = matches!(data_store_history::observe(view, root)?,
        data_store_history::Work::Complete { revision } if revision == progress.source_revision);
    Ok(EntryPage {
        progress,
        placement_current,
        entries: rows,
        next,
    })
}
