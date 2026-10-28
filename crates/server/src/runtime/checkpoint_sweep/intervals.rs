//! One bounded projection page per existing metadata maintenance slot.
use super::*;
use kv9_meta::retention::{data_configuration_intervals as census, data_store_history};
pub(super) fn pass(
    ledger: &impl CheckpointLedger,
    view: &dyn kv9_engine::ReadView,
    root: &RootDescriptor,
    check: &dyn Fn() -> Result<()>,
    stats: &mut Stats,
) -> Result<()> {
    let Some((next, origin, _)) =
        data_store_history::next_source(view, root, stats.intervals_after.as_ref())?
    else {
        stats.intervals_after = None;
        stats.interval_wraps = stats.interval_wraps.saturating_add(1);
        return Ok(());
    };
    stats.intervals_after = Some(next);
    let source = census::Source::from_origin(&origin);
    if let Some(progress) = census::observation(view, root, source)? {
        stats.interval_observed_rounds = stats
            .interval_observed_rounds
            .saturating_add(u64::from(progress.round_complete()));
        stats.interval_known_gaps = stats
            .interval_known_gaps
            .saturating_add(u64::from(progress.has_known_gaps()));
    }
    let Some(request) = census::next(view, root, source)? else {
        return Ok(());
    };
    check()?;
    let result = ledger.apply(encode_request(
        root.digest(),
        &LedgerRequest::AdvanceConfigurationIntervals(request),
    )?);
    if result.is_ok() {
        stats.interval_pages = stats.interval_pages.saturating_add(1);
    } else {
        stats.interval_failures = stats.interval_failures.saturating_add(1);
    }
    result
}
