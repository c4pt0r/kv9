//! Checkpoint ownership is separate from public Raw serving: a group is
//! checkpointed through the serving owner the directory names for its region.
use super::*;
use kv9_common::data_range::DataRange;

pub(super) struct Group {
    driver: Arc<Driver>,
    engine: Arc<WalEngine>,
    target: DataRange,
    source: Arc<RawGroup>,
}
impl Group {
    pub(super) fn capture_parts(&self) -> (&Arc<Driver>, &Arc<WalEngine>, &DataRange) {
        (&self.driver, &self.engine, &self.target)
    }
    pub(super) fn target(&self) -> &DataRange {
        &self.target
    }
    pub(super) fn check(&self, backend: &RuntimeBackend) -> Result<()> {
        if !owner_matches(backend, self.target.region, &self.driver, &self.engine) {
            return Err(Error::StaleEpoch {
                region: self.target.region,
            });
        }
        if !backend
            .raw_directory
            .by_region(self.target.region)
            .is_some_and(|g| g.same_owner(&self.source))
        {
            return Err(Error::StaleEpoch {
                region: self.target.region,
            });
        }
        Ok(())
    }
}

pub(in crate::runtime) fn owner_matches(
    backend: &RuntimeBackend,
    region: RegionId,
    driver: &Arc<Driver>,
    engine: &Arc<WalEngine>,
) -> bool {
    let handle = backend
        .group_handles
        .lock()
        .expect("group handles")
        .get(&region)
        .cloned();
    if let Some((actual_engine, actual_driver)) = handle {
        return Arc::ptr_eq(driver, &actual_driver) && Arc::ptr_eq(engine, &actual_engine);
    }
    backend
        .raw_directory
        .by_region(region)
        .is_some_and(|group| {
            let (actual_driver, actual_engine, _) = group.capture_parts();
            Arc::ptr_eq(driver, actual_driver) && Arc::ptr_eq(engine, actual_engine)
        })
}

/// Visit one region from the union without materializing either directory.
/// A virgin/unavailable group, or a local handle the public directory does not
/// (yet) name, still advances the dispatcher's visit cursor without a candidate.
pub(super) fn next(
    backend: &RuntimeBackend,
    after: Option<RegionId>,
) -> Option<(RegionId, Result<Option<Arc<Group>>>)> {
    use std::ops::Bound::{Excluded, Unbounded};
    let raw = backend.raw_directory.next_after(after);
    let handle = {
        let handles = backend.group_handles.lock().expect("group handles");
        after
            .and_then(|id| handles.range((Excluded(id), Unbounded)).next())
            .or_else(|| handles.first_key_value())
            .map(|(region, pair)| (*region, pair.clone()))
    };
    let region = raw
        .as_ref()
        .map(|g| g.range_binding().region)
        .into_iter()
        .chain(handle.as_ref().map(|(r, _)| *r))
        .min_by_key(|r| (after.is_some_and(|a| *r <= a), *r));
    let region = region?;
    let candidate = raw
        .filter(|g| g.range_binding().region == region)
        .map(|raw| {
            let (driver, engine, target) = raw.capture_parts();
            Arc::new(Group {
                driver: driver.clone(),
                engine: engine.clone(),
                target: target.clone(),
                source: raw,
            })
        });
    Some((region, Ok(candidate)))
}
