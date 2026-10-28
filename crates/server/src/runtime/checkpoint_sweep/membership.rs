//! One exact source per existing sweep slot. The cursor is scheduling only;
//! each source's page progress and unknown membership witnesses are durable.
use super::*;
use kv9_meta::retention::{data_membership as census, data_store_history};
pub(super) fn pass(
    ledger: &impl CheckpointLedger,
    view: &dyn kv9_engine::ReadView,
    root: &RootDescriptor,
    check: &dyn Fn() -> Result<()>,
    stats: &mut Stats,
) -> Result<()> {
    let Some((next, origin, revision)) =
        data_store_history::next_source(view, root, stats.membership_after.as_ref())?
    else {
        stats.membership_after = None;
        stats.membership_wraps = stats.membership_wraps.saturating_add(1);
        return Ok(());
    };
    // Advance even on a refused/unreachable source, so another store can run.
    stats.membership_after = Some(next);
    let source = census::Source::from_origin(&origin);
    let current = census::observation(view, root, source)?;
    if let Some(p) = &current {
        stats.membership_observed_rounds = stats
            .membership_observed_rounds
            .saturating_add(u64::from(p.round_complete()));
        stats.membership_known_gaps = stats
            .membership_known_gaps
            .saturating_add(u64::from(p.has_known_gaps()));
    }
    let request = LedgerRequest::RecordDataMembership(census::Request {
        source,
        task: origin.task,
        revision,
        expected: current.map_or(0, |p| p.sequence),
    });
    check()?;
    let result = ledger.apply(encode_request(root.digest(), &request)?);
    if result.is_ok() {
        stats.membership_pages = stats.membership_pages.saturating_add(1);
    } else {
        stats.membership_failures = stats.membership_failures.saturating_add(1);
    }
    result
}
