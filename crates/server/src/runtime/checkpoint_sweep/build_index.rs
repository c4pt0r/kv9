//! Alternate global Build/Reader/bootstrap catalog censuses with attempt maintenance. No per-attempt
//! owner-table scan, no object I/O, and no closure/GC authority from completion.
use super::*;
use kv9_meta::retention::{
    bootstrap_index,
    build_index::{self, Work},
    data_store_history, image_publisher_index, image_reader_index, reader_index,
};

pub(in crate::runtime) fn pass(
    backend: &RuntimeBackend,
    ledger: &impl CheckpointLedger,
    cancel: &CancellationToken,
    deadline: Instant,
    stats: &mut Stats,
) -> Result<()> {
    // Coverage is tied to this snapshot's task revision; never keep a stale
    // success on a later pass that fails before reaching this census family.
    stats.data_store_history_catalog_complete = false;
    stats.data_store_history_source_revision = None;
    stats.data_store_history_uncertain = false;
    let check = || {
        if cancel.is_cancelled() || Instant::now() >= deadline {
            return Err(Error::MetaNotReady(
                "Owner census cancelled or expired".into(),
            ));
        }
        backend.ensure_serving()?;
        if backend.driver.status().role != Role::Leader {
            return Err(Error::NotLeader {
                leader: backend.driver.status().leader_id,
            });
        }
        Ok(())
    };
    check()?;
    backend.driver.read_barrier(
        deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_secs(1)),
    )?;
    let txn = backend.node.meta_raft.store.begin()?;
    let root = kv9_meta::root::certified_root(&txn)?
        .ok_or_else(|| Error::MetaNotReady("Owner census root unavailable".into()))?;
    if root != backend.ttl_context()?.root {
        return Err(Error::Config("Owner census root differs".into()));
    }
    let view = txn.into_view();
    let work = build_index::observe(view.as_ref(), &root)?;
    stats.build_index_catalog_complete = matches!(work, Work::Complete);
    let mut family = 0;
    let (request, rows, upgrade) = match work {
        Work::Uninitialized => return Ok(()),
        Work::Complete => {
            // Build coverage is stable: every new Build is indexed atomically.
            // Reuse the same maintenance slot for one bounded Reader page.
            family = 1;
            let work = reader_index::observe(view.as_ref(), &root)?;
            stats.reader_index_catalog_complete = matches!(work, reader_index::Work::Complete);
            match work {
                reader_index::Work::Uninitialized => return Ok(()),
                reader_index::Work::Complete => {
                    // Persistent bootstrap grants have their own image grouping
                    // and source qualification; use one page in the same slot.
                    family = 2;
                    let work = bootstrap_index::observe(view.as_ref(), &root)?;
                    stats.bootstrap_index_catalog_complete =
                        matches!(work, bootstrap_index::Work::Complete);
                    match work {
                        bootstrap_index::Work::Uninitialized => return Ok(()),
                        bootstrap_index::Work::Complete => {
                            family = 3;
                            let work = image_reader_index::observe(view.as_ref(), &root)?;
                            stats.image_reader_index_catalog_complete =
                                matches!(work, image_reader_index::Work::Complete);
                            match work {
                                image_reader_index::Work::Uninitialized => return Ok(()),
                                image_reader_index::Work::Complete => {
                                    family = 4;
                                    let work =
                                        image_publisher_index::observe(view.as_ref(), &root)?;
                                    stats.image_publisher_index_catalog_complete =
                                        matches!(work, image_publisher_index::Work::Complete);
                                    match work {
                                        image_publisher_index::Work::Uninitialized => return Ok(()),
                                        image_publisher_index::Work::Complete => {
                                            family = 5;
                                            match data_store_history::observe(view.as_ref(), &root)?
                                            {
                                                data_store_history::Work::Uninitialized => {
                                                    return Ok(())
                                                }
                                                data_store_history::Work::Upgrade => {
                                                    (LedgerRequest::Initialize, 0, true)
                                                }
                                                data_store_history::Work::Uncertain(revision) => {
                                                    stats.data_store_history_uncertain = true;
                                                    stats.data_store_history_source_revision =
                                                        Some(revision.sequence);
                                                    return Ok(());
                                                }
                                                data_store_history::Work::Complete { revision } => {
                                                    stats.data_store_history_catalog_complete =
                                                        true;
                                                    stats.data_store_history_source_revision =
                                                        Some(revision);
                                                    let turn = stats.history_turn;
                                                    stats.history_turn = (turn + 1) % 4;
                                                    let pass = match turn {
                                                        0 => super::membership::pass,
                                                        1 => super::origins::pass,
                                                        2 => super::timeline::pass,
                                                        _ => super::intervals::pass,
                                                    };
                                                    return pass(
                                                        ledger,
                                                        view.as_ref(),
                                                        &root,
                                                        &check,
                                                        stats,
                                                    );
                                                }
                                                data_store_history::Work::Page {
                                                    progress,
                                                    revision,
                                                    after,
                                                    examined,
                                                } => {
                                                    stats.data_store_history_source_revision =
                                                        Some(revision);
                                                    (
                                                        progress.request(revision, after),
                                                        examined,
                                                        false,
                                                    )
                                                }
                                            }
                                        }
                                        image_publisher_index::Work::Upgrade => {
                                            (LedgerRequest::Initialize, 0, true)
                                        }
                                        image_publisher_index::Work::Page {
                                            progress,
                                            next,
                                            examined,
                                        } => (progress.request(next), examined, false),
                                    }
                                }
                                image_reader_index::Work::Upgrade => {
                                    (LedgerRequest::Initialize, 0, true)
                                }
                                image_reader_index::Work::Page {
                                    progress,
                                    next,
                                    examined,
                                } => (progress.request(next), examined, false),
                            }
                        }
                        bootstrap_index::Work::Upgrade => (LedgerRequest::Initialize, 0, true),
                        bootstrap_index::Work::Page {
                            progress,
                            next,
                            examined,
                        } => (progress.request(next), examined, false),
                    }
                }
                reader_index::Work::Upgrade => (LedgerRequest::Initialize, 0, true),
                reader_index::Work::Page {
                    progress,
                    next,
                    examined,
                } => (progress.request(next), examined, false),
            }
        }
        Work::Upgrade => (LedgerRequest::Initialize, 0, true),
        Work::Page {
            progress,
            next,
            examined,
        } => (progress.request(next), examined, false),
    };
    drop(view);
    check()?;
    ledger.apply(encode_request(root.digest(), &request)?)?;
    stats.build_index_upgrades = stats
        .build_index_upgrades
        .saturating_add(u64::from(upgrade));
    if family == 5 {
        stats.data_store_history_pages = stats
            .data_store_history_pages
            .saturating_add(u64::from(!upgrade));
        stats.data_store_history_rows = stats.data_store_history_rows.saturating_add(rows as u64);
    } else if family == 4 {
        stats.image_publisher_index_pages = stats
            .image_publisher_index_pages
            .saturating_add(u64::from(!upgrade));
        stats.image_publisher_index_rows =
            stats.image_publisher_index_rows.saturating_add(rows as u64);
    } else if family == 3 {
        stats.image_reader_index_pages = stats
            .image_reader_index_pages
            .saturating_add(u64::from(!upgrade));
        stats.image_reader_index_rows = stats.image_reader_index_rows.saturating_add(rows as u64);
    } else if family == 2 {
        stats.bootstrap_index_pages = stats
            .bootstrap_index_pages
            .saturating_add(u64::from(!upgrade));
        stats.bootstrap_index_rows = stats.bootstrap_index_rows.saturating_add(rows as u64);
    } else if family == 1 {
        stats.reader_index_pages = stats.reader_index_pages.saturating_add(u64::from(!upgrade));
        stats.reader_index_rows = stats.reader_index_rows.saturating_add(rows as u64);
    } else {
        stats.build_index_pages = stats.build_index_pages.saturating_add(u64::from(!upgrade));
        stats.build_index_rows = stats.build_index_rows.saturating_add(rows as u64);
    }
    Ok(())
}
