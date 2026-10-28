//! Checkpoint ownership is separate from public Raw serving. An importing
//! child is discovered through its actual driver and committed split plan.
use super::*;
use kv9_common::data_range::{DataRange, RANGE_KEY};
use kv9_common::raw_ttl::split::{ImportSpec, ImportState, IMPORT_KEY};
use kv9_engine::ColumnFamily;

pub(super) struct Group {
    driver: Arc<Driver>,
    engine: Arc<WalEngine>,
    target: DataRange,
    source: Source,
}
enum Source {
    Serving(Arc<RawGroup>),
    Transfer(Box<ImportSpec>),
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
        match &self.source {
            Source::Serving(original) => {
                if !backend
                    .raw_directory
                    .by_region(self.target.region)
                    .is_some_and(|g| g.same_owner(original))
                {
                    return Err(Error::StaleEpoch {
                        region: self.target.region,
                    });
                }
            }
            Source::Transfer(spec) => {
                let committed = kv9_meta::data_groups::split::committed_split_plan(
                    &backend.node.meta_raft.store,
                    spec.seal.plan.operation,
                )?;
                if committed.plan() != &spec.seal.plan {
                    return Err(invalid("staging checkpoint split plan differs"));
                }
                let state = self
                    .driver
                    .ttl_import_state(&self.target)?
                    .ok_or_else(|| invalid("staging checkpoint import disappeared"))?;
                if &state.spec != spec.as_ref() {
                    return Err(invalid("staging checkpoint import identity changed"));
                }
            }
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
/// A virgin/unavailable group still advances the dispatcher's visit cursor.
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
    let candidate = (|| -> Result<Option<Arc<Group>>> {
        if let Some(raw) = raw.filter(|g| g.range_binding().region == region) {
            let (driver, engine, target) = raw.capture_parts();
            return Ok(Some(Arc::new(Group {
                driver: driver.clone(),
                engine: engine.clone(),
                target: target.clone(),
                source: Source::Serving(raw),
            })));
        }
        let (_, (engine, driver)) = handle.expect("selected handle");
        let Some(bytes) = engine.get(ColumnFamily::Default, IMPORT_KEY)? else {
            return Ok(None);
        };
        let import = ImportState::decode(&bytes)?;
        let target = import.spec.target();
        let range = engine
            .get(ColumnFamily::Default, RANGE_KEY)?
            .map(|b| DataRange::decode(&b))
            .transpose()?;
        // Finish may race discovery. The worker validates one actual frozen view;
        // discovery grants neither checkpoint publication nor serving authority.
        if target.region != region || range.as_ref().is_some_and(|r| r != target) {
            return Ok(None);
        }
        Ok(Some(Arc::new(Group {
            driver,
            engine,
            target: target.clone(),
            source: Source::Transfer(Box::new(import.spec)),
        })))
    })();
    Some((region, candidate))
}
