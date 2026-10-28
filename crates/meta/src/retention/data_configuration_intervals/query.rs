use super::*;
#[derive(Debug, Clone, Copy)]
pub struct IntervalCursor {
    source: Source,
    sequence: u64,
    after: u64,
}
pub struct Interval {
    pub entry: Entry,
    pub start: Boundary,
    /// None is an open observed suffix, not a claim about future membership.
    pub end: Option<Boundary>,
    /// Includes boundary ambiguity, especially a snapshot skipping unknown configurations.
    pub gaps: u16,
}
pub struct IntervalPage {
    pub progress: Progress,
    pub inputs_current: bool,
    pub intervals: Vec<Interval>,
    pub next: Option<IntervalCursor>,
}
pub fn intervals(
    view: &dyn ReadView,
    root: &RootDescriptor,
    source: Source,
    after: Option<IntervalCursor>,
) -> Result<IntervalPage> {
    let p = planner(view, root)?;
    let progress =
        read(&p, source)?.ok_or_else(|| conflict("configuration intervals not observed"))?;
    if after.is_some_and(|c| {
        c.source != source
            || c.sequence != progress.sequence
            || c.after == 0
            || !c.after.is_multiple_of(4)
            || c.after >= progress.processed
    }) {
        return Err(conflict("configuration interval query cursor changed"));
    }
    let current = input(&p, root, source)?
        .ok_or_else(|| invalid("configuration interval timeline missing"))?;
    let inputs_current = progress.timeline_sequence == current.timeline.sequence
        && progress.native_at == current.native.as_ref().map_or(0, |n| n.recorded_at);
    let mut intervals = Vec::with_capacity(4);
    let mut seen = after.map_or(0, |c| c.after);
    for _ in 0..4 {
        if seen == progress.processed {
            break;
        }
        seen = seen
            .checked_add(1)
            .ok_or_else(|| invalid("configuration interval query exhausted"))?;
        let entry = read_entry(&p, source, seen)?
            .ok_or_else(|| invalid("configuration interval query row missing"))?;
        if entry.generation != progress.generation || entry.recorded_at > progress.recorded_at {
            return Err(invalid("configuration interval query generation differs"));
        }
        let start = projection::boundary(&actual(&p, source, seen)?);
        let end = if seen < progress.total {
            Some(projection::boundary(&actual(&p, source, seen + 1)?))
        } else {
            None
        };
        let mut gaps = entry.gaps;
        if let Some(end) = end {
            if end.index.zip(start.index).is_none_or(|(b, a)| b <= a)
                || end.term.zip(start.term).is_some_and(|(b, a)| b < a)
            {
                gaps |= AMBIGUOUS_POSITION;
            }
            if matches!(end.kind, RecordKind::Snapshot { .. }) {
                gaps |= SNAPSHOT_ANCESTRY;
            }
        }
        intervals.push(Interval {
            entry,
            start,
            end,
            gaps,
        });
    }
    let next = (seen < progress.processed).then_some(IntervalCursor {
        source,
        sequence: progress.sequence,
        after: seen,
    });
    Ok(IntervalPage {
        progress,
        inputs_current,
        intervals,
        next,
    })
}
