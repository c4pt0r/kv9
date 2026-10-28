//! Shared runtime compaction passes. Discovery grants no truncation authority:
//! the committed kind-110 floor, all-matched gate and checkpoint history still
//! govern execution. Importing children never enter the public range directory.
use super::*;
use kv9_common::raw_ttl::split::{ImportState, IMPORT_KEY};
use kv9_engine::ColumnFamily;
use std::ops::Bound::{Excluded, Unbounded};

type Driver = NodeDriver<DiskRaftStorage, WalEngine>;
type Handle = (RegionId, Arc<WalEngine>, Arc<Driver>);

#[derive(Clone, Copy)]
pub(super) struct Thresholds {
    pub entries: u64,
    pub bytes: u64,
    pub age: Duration,
}
impl Thresholds {
    pub fn disabled(self) -> bool {
        self.entries == 0 && self.bytes == 0 && self.age == Duration::ZERO
    }
}

/// One ordered local handle, wrapping at the end. No full handle-vector copy,
/// and no driver or engine operations while the directory lock is held.
fn next(backend: &RuntimeBackend, after: Option<u64>) -> Option<Handle> {
    let handles = backend.group_handles.lock().expect("group handles");
    after
        .and_then(|r| handles.range((Excluded(RegionId(r)), Unbounded)).next())
        .or_else(|| handles.first_key_value())
        .map(|(region, (engine, driver))| (*region, engine.clone(), driver.clone()))
}

/// Imports require a real, locally selected image before automatic planning.
/// Its cut is an additional scheduling ceiling, never permission to truncate.
pub(super) fn import_cut(
    backend: &RuntimeBackend,
    region: RegionId,
    engine: &WalEngine,
    driver: &Driver,
) -> Result<Option<AppliedPosition>> {
    let Some(bytes) = engine.get(ColumnFamily::Default, IMPORT_KEY)? else {
        return Ok(None);
    };
    let observed = ImportState::decode(&bytes)?;
    let target = observed.spec.target();
    if target.region != region {
        return Err(Error::Config(
            "compaction import belongs to another group".into(),
        ));
    }
    let committed = kv9_meta::data_groups::split::committed_split_plan(
        &backend.node.meta_raft.store,
        observed.spec.seal.plan.operation,
    )?;
    if committed.plan() != &observed.spec.seal.plan {
        return Err(Error::Config("compaction import split plan differs".into()));
    }
    // Reads validated state from the actual owner; a decoded engine row alone
    // is not sufficient. Finish racing discovery is harmless for this ceiling.
    if driver
        .ttl_import_state(target)?
        .is_none_or(|current| current.spec != observed.spec)
    {
        return Err(Error::Config("compaction import identity changed".into()));
    }
    Ok(engine.selected_checkpoint()?.map(|m| m.position()))
}

pub(super) fn plan(
    backend: &RuntimeBackend,
    thresholds: Thresholds,
    after: &mut Option<u64>,
    age_marks: &mut BTreeMap<u64, (u64, Instant)>,
) -> Result<Option<crate::api::RecordGroupCompactionResult>> {
    if thresholds.disabled() || backend.driver.status().role != Role::Leader {
        return Ok(None);
    }
    let store = &backend.node.meta_raft.store;
    let Some(certified) = kv9_meta::root::certified_root(&store.begin()?)? else {
        return Ok(None);
    };
    // Catalog history discovery remains unpaged; local candidate visits are
    // bounded independently, including unavailable and not-yet-importing groups.
    let bindings = kv9_meta::data_groups::ranges::committed_ranges(store)?
        .into_iter()
        .map(|b| b.range().region)
        .collect::<HashSet<_>>();
    let floors = kv9_meta::data_groups::compaction::committed_compaction_floors(store)?
        .into_iter()
        .map(|d| (d.region(), d.floor().index))
        .collect::<BTreeMap<_, _>>();
    let mut first = None;
    for _ in 0..32 {
        let Some((region, engine, driver)) = next(backend, *after) else {
            break;
        };
        if first == Some(region) {
            break;
        }
        first.get_or_insert(region);
        *after = Some(region.0);
        let status = driver.status();
        if status.fatal.is_some() || status.applied_index == 0 || status.applied_term == 0 {
            continue;
        }
        let import_ceiling = if bindings.contains(&region) {
            None
        } else {
            let Some(cut) = import_cut(backend, region, &engine, &driver)? else {
                continue;
            };
            Some(cut)
        };
        let last_floor = floors.get(&region).copied().unwrap_or(0);
        let retained = status.raft_committed.saturating_sub(status.log_first_index);
        let entries = thresholds.entries > 0
            && retained >= thresholds.entries
            && status.applied_index >= last_floor.saturating_add(thresholds.entries);
        // Until the last floor executes here, its retained bytes must not
        // trigger another proposal for the same uncollected prefix.
        let bytes = thresholds.bytes > 0
            && status.log_first_index > last_floor
            && status.applied_index > last_floor
            && driver.peer().retained_log_bytes()? >= thresholds.bytes;
        let now = Instant::now();
        let mark = age_marks
            .entry(region.0)
            .or_insert((status.log_first_index, now));
        if mark.0 != status.log_first_index {
            *mark = (status.log_first_index, now);
        }
        let age = thresholds.age > Duration::ZERO
            && retained > 0
            && now.duration_since(mark.1) >= thresholds.age
            && status.applied_index > last_floor;
        if !(entries || bytes || age) {
            continue;
        }
        let mut floor = AppliedPosition {
            term: status.applied_term,
            index: status.applied_index,
        };
        let history = engine
            .checkpoint_history()
            .ok_or_else(|| Error::Config("compaction group has no checkpoint history".into()))?;
        for ceiling in history
            .compaction_ceiling()?
            .into_iter()
            .chain(import_ceiling)
        {
            if ceiling.index < floor.index {
                floor = ceiling;
            }
        }
        if floor.index <= last_floor {
            continue;
        }
        return backend
            .record_group_compaction("auto-compact", certified.digest(), region, floor)
            .map(Some);
    }
    Ok(None)
}

/// The confirmation is replicated in the group's own ordered log. An import
/// does not change tag 9 semantics or acquire a range binding by applying it.
fn confirmed(engine: &WalEngine) -> Result<Option<AppliedPosition>> {
    let view = engine.snapshot()?;
    let Some(bytes) = view.get(
        ColumnFamily::Default,
        kv9_common::data_range::COMPACTION_CONFIRMED_KEY,
    )?
    else {
        return Ok(None);
    };
    if bytes.len() != 16 {
        return Err(Error::Config("compaction confirmation is malformed".into()));
    }
    Ok(Some(AppliedPosition {
        term: u64::from_be_bytes(bytes[..8].try_into().unwrap()),
        index: u64::from_be_bytes(bytes[8..].try_into().unwrap()),
    }))
}

pub(super) fn execute(backend: &RuntimeBackend, after: &mut Option<u64>) -> Result<()> {
    let mut floors = kv9_meta::data_groups::compaction::committed_compaction_floors(
        &backend.node.meta_raft.store,
    )?;
    floors.sort_by_key(|d| d.region());
    let start = floors.partition_point(|d| after.is_some_and(|a| d.region().0 <= a));
    for decision in floors[start..]
        .iter()
        .chain(floors[..start].iter())
        .take(32)
    {
        *after = Some(decision.region().0);
        let handle = backend
            .group_handles
            .lock()
            .expect("handles")
            .get(&decision.region())
            .cloned();
        let Some((engine, driver)) = handle else {
            continue;
        };
        let status = driver.status();
        if status.fatal.is_some() || status.applied_index < decision.floor().index {
            continue;
        }
        let published = confirmed(&engine)?;
        let leader_needs = status.role == Role::Leader
            && published.is_none_or(|p| p.index < decision.floor().index);
        let follower_needs = published
            .is_some_and(|p| status.applied_index >= p.index && status.log_first_index <= p.index);
        if !leader_needs && !follower_needs {
            continue;
        }
        if leader_needs {
            engine.sync_applied_now()?;
            driver
                .peer()
                .truncate_retained_log(decision.floor(), &decision.encode())?;
            if confirmed(&engine)?.is_none_or(|p| p.index < decision.floor().index) {
                let proposed = driver.propose(&Command::CompactionConfirmed {
                    region: decision.region().0,
                    floor: decision.floor(),
                })?;
                driver
                    .wait_applied(proposed, Duration::from_secs(10))
                    .map_err(|e| Error::Raft(format!("confirmation publish: {e:?}")))?;
            }
        }
        let Some(floor) = confirmed(&engine)? else {
            break;
        };
        if status.applied_index < floor.index {
            break;
        }
        engine.sync_applied_now()?;
        let mut decision = b"kv9-compaction-confirmed".to_vec();
        decision.extend_from_slice(&floor.term.to_be_bytes());
        decision.extend_from_slice(&floor.index.to_be_bytes());
        driver.peer().compact_confirmed_prefix(floor, &decision)?;
        break;
    }
    Ok(())
}

pub(super) fn reclaim(backend: &RuntimeBackend, bytes: u64, after: &mut Option<u64>) -> Result<()> {
    if bytes == 0 {
        return Ok(());
    }
    let mut first = None;
    for _ in 0..32 {
        let Some((region, _, driver)) = next(backend, *after) else {
            break;
        };
        if first == Some(region) {
            break;
        }
        first.get_or_insert(region);
        *after = Some(region.0);
        let status = driver.status();
        if status.fatal.is_some()
            || status.log_first_index <= 1
            || driver.peer().log_file_bytes() < bytes
        {
            continue;
        }
        // Ok(false) includes snapshot/lease exclusion and unavailable rewrite
        // workspace. Keep the cursor advancing so another group can reclaim.
        driver.peer().reclaim_log()?;
        break;
    }
    Ok(())
}
