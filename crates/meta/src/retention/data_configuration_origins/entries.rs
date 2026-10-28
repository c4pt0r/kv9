use super::*;

#[derive(Debug, Clone, Copy)]
pub struct EntryCursor {
    source: Source,
    sequence: u64,
    after: u64,
    seen: u64,
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
        read(&p, source)?.ok_or_else(|| conflict("configuration origins not observed"))?;
    if after.is_some_and(|c| {
        c.source != source
            || c.sequence != progress.sequence
            || c.after == 0
            || c.seen == 0
            || !c.seen.is_multiple_of(4)
            || c.seen >= progress.retained_entries
    }) {
        return Err(conflict("configuration origin query cursor differs"));
    }
    let mut start = source.key(55)?;
    let mut end = start.clone();
    while end.last() == Some(&255) {
        end.pop();
    }
    *end.last_mut().unwrap() += 1;
    if let Some(c) = after {
        uint(&mut start, c.after);
        start.push(0);
    }
    let mut rows = Vec::with_capacity(4);
    let mut more = false;
    let mut seen = after.map_or(0, |c| c.seen);
    for row in view.iter(ColumnFamily::Default, &start, &end)?.take(5) {
        let (k, b) = row?;
        let entry = Entry::decode(&p, source, &k, &b)?;
        if entry.round > progress.round
            || entry.last_observed > progress.recorded_at
            || seen.checked_add(1) != Some(entry.ordinal)
            || entry.applied.at.index > progress.last_retained_index
        {
            return Err(invalid("configuration origin index position differs"));
        }
        if rows.len() == 4 {
            more = true;
            break;
        }
        seen = entry.ordinal;
        rows.push(entry);
    }
    let count_matches = if more {
        seen < progress.retained_entries
    } else {
        seen == progress.retained_entries
            && rows
                .last()
                .is_none_or(|e| e.applied.at.index == progress.last_retained_index)
    };
    if !count_matches {
        return Err(invalid("configuration origin index count differs"));
    }
    let next = more.then(|| EntryCursor {
        source,
        sequence: progress.sequence,
        after: rows.last().unwrap().applied.at.index,
        seen,
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
