//! Inventory only the runtime's fixed journal slots before any checkpoint
//! worker starts. This observes local debt; it never authorizes group recovery.
use super::*;
use kv9_engine::checkpoint_pending_budget::PendingDiskBudget;

pub(super) fn recover(dir: &Path, limit: u64) -> Result<Arc<PendingDiskBudget>> {
    let budget = Arc::new(PendingDiskBudget::new(limit)?);
    budget.inventory_journal(&dir.join("catalog.pending"))?;
    let entries = match std::fs::read_dir(dir.join("data-groups")) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(budget),
        Err(error) => return Err(Error::Engine(format!("pending disk inventory: {error}"))),
    };
    for entry in entries {
        let entry = entry.map_err(|e| Error::Engine(format!("pending disk inventory: {e}")))?;
        // RegionManager independently validates directory names and authority.
        // Include real directories even if quarantined; never follow a symlink.
        if entry
            .file_type()
            .map_err(|e| Error::Engine(format!("pending disk inventory: {e}")))?
            .is_dir()
        {
            inventory_group(&budget, &entry.path())?;
            // Adopted replicas run their engine inside one install generation.
            // Count all real generations, including inactive/corrupt ones, as
            // disk debt; this inventory grants no recovery authority.
            for generation in std::fs::read_dir(entry.path())
                .map_err(|e| Error::Engine(format!("pending disk inventory: {e}")))?
            {
                let generation = generation
                    .map_err(|e| Error::Engine(format!("pending disk inventory: {e}")))?;
                if generation
                    .file_name()
                    .to_string_lossy()
                    .starts_with("install-")
                    && generation
                        .file_type()
                        .map_err(|e| Error::Engine(format!("pending disk inventory: {e}")))?
                        .is_dir()
                {
                    inventory_group(&budget, &generation.path())?;
                }
            }
        }
    }
    Ok(budget)
}

fn inventory_group(budget: &Arc<PendingDiskBudget>, directory: &Path) -> Result<()> {
    for slot in ["data.pending", "capture.plan", "capture-replica.plan"] {
        budget.inventory_journal(&directory.join(slot))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_counts_capture_and_adopted_generation_debt_without_following_links() {
        let dir = std::env::temp_dir().join(format!(
            "kv9-capture-inventory-{}",
            kv9_common::StoreIncarnation::mint().unwrap()
        ));
        let group = dir.join("data-groups/100");
        let generation = group.join("install-test");
        std::fs::create_dir_all(&generation).unwrap();
        for directory in [&group, &generation] {
            for name in [
                "data.pending",
                "data.pending.tmp",
                "capture.plan",
                "capture.pending.tmp",
                "capture-replica.plan",
                "capture-replica.pending.tmp",
            ] {
                std::fs::write(directory.join(name), [0; 11]).unwrap();
            }
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&group, dir.join("data-groups/linked-group")).unwrap();
            std::os::unix::fs::symlink(&generation, group.join("install-link")).unwrap();
        }
        let budget = recover(&dir, 1).unwrap();
        assert_eq!(budget.snapshot().retained_bytes, 132);
        assert_eq!(budget.snapshot().named_paths, 12);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn startup_counts_catalog_all_groups_and_partial_temps_even_above_new_limit() {
        let dir = std::env::temp_dir().join(format!(
            "kv9-pending-inventory-{}",
            kv9_common::StoreIncarnation::mint().unwrap()
        ));
        for name in ["100", "101", "bad-group"] {
            std::fs::create_dir_all(dir.join("data-groups").join(name)).unwrap();
        }
        for (name, bytes) in [
            ("catalog.pending", 7),
            ("catalog.pending.tmp", 11),
            ("data-groups/100/data.pending", 13),
            ("data-groups/101/data.pending.tmp", 17),
            ("data-groups/bad-group/data.pending", 19),
            ("data-groups/100/data.wal", 1024),
        ] {
            std::fs::write(dir.join(name), vec![0; bytes]).unwrap();
        }
        let budget = recover(&dir, 1).unwrap();
        assert_eq!(budget.snapshot().retained_bytes, 67);
        assert_eq!(budget.snapshot().named_paths, 5);
        assert_eq!(budget.snapshot().open_readers, 0);
        assert_eq!(budget.snapshot().open_writers, 0);
        std::fs::create_dir_all(dir.join("data-groups/wrong-type/data.pending")).unwrap();
        assert_eq!(recover(&dir, 1).unwrap().snapshot().retained_bytes, 67);
        // Reobserving an inode is not another copy of its disk bytes.
        budget
            .inventory_journal(&dir.join("catalog.pending"))
            .unwrap();
        assert_eq!(budget.snapshot().retained_bytes, 67);
        assert!(budget
            .observe_removed_journal(&dir.join("catalog.pending"))
            .is_err());
        std::fs::remove_file(dir.join("catalog.pending")).unwrap();
        std::fs::remove_file(dir.join("catalog.pending.tmp")).unwrap();
        budget
            .observe_removed_journal(&dir.join("catalog.pending"))
            .unwrap();
        assert_eq!(budget.snapshot().retained_bytes, 49);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
