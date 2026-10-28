//! One exact source per existing sweep slot. The cursor is scheduling only;
//! each source's page progress and exact configuration observations are durable.
use super::*;
use kv9_meta::retention::{data_configuration_origins as census, data_store_history};
pub(super) fn pass(
    ledger: &impl CheckpointLedger,
    view: &dyn kv9_engine::ReadView,
    root: &RootDescriptor,
    check: &dyn Fn() -> Result<()>,
    stats: &mut Stats,
) -> Result<()> {
    let Some((next, origin, revision)) =
        data_store_history::next_source(view, root, stats.origins_after.as_ref())?
    else {
        stats.origins_after = None;
        stats.origin_wraps = stats.origin_wraps.saturating_add(1);
        return Ok(());
    };
    // Advance even on a refused/unreachable source, so another store can run.
    stats.origins_after = Some(next);
    let source = census::Source::from_origin(&origin);
    let current = census::observation(view, root, source)?;
    if let Some(p) = &current {
        stats.origin_observed_rounds = stats
            .origin_observed_rounds
            .saturating_add(u64::from(p.round_complete()));
        stats.origin_known_gaps = stats
            .origin_known_gaps
            .saturating_add(u64::from(p.has_known_gaps()));
    }
    let request = LedgerRequest::RecordConfigurationOrigins(census::Request {
        source,
        task: origin.task,
        revision,
        expected: current.map_or(0, |p| p.sequence),
    });
    check()?;
    let result = ledger.apply(encode_request(root.digest(), &request)?);
    if result.is_ok() {
        stats.origin_pages = stats.origin_pages.saturating_add(1);
    } else {
        stats.origin_failures = stats.origin_failures.saturating_add(1);
    }
    result
}
