use super::*;

#[test]
fn startup_inventory_counts_orphans_temps_and_quarantined_layouts_above_lowered_limit() {
    let dir = std::env::temp_dir().join(format!(
        "kv9-log-inventory-{}",
        kv9_common::StoreIncarnation::mint().unwrap()
    ));
    for (name, bytes) in [
        ("catalog.wal", 3),
        ("catalog.tail.tmp", 5),
        ("raft/raft.log", 7),
        ("raft/raft.log.tmp", 11),
        ("data-groups/100/data.wal", 13),
        ("data-groups/100/data.topology.tmp", 17),
        ("data-groups/100/data.segments/foreign/orphan.wal", 19),
        ("data-groups/100/raft/raft.log", 23),
        ("data-groups/bad-group/data.segments/unknown/orphan.wal", 29),
        ("data-groups/temp-only/data.tail.tmp", 31),
        ("data-groups/temp-only/data.upgrade.tmp", 37),
        ("data-groups/migration-only/data.migration/orphan.wal", 41),
        ("data-groups/100/data.pending", 999), // a separate existing budget
    ] {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![0; bytes]).unwrap();
    }
    let budget = inventory(&dir, Arc::new(LogBudget::new(1).unwrap())).unwrap();
    assert_eq!(budget.snapshot().file_bytes, 236);
    assert!(budget.reserve(1, kv9_common::RegionId(100)).is_err());
    std::fs::remove_dir_all(dir.join("data-groups/100")).unwrap();
    std::fs::File::open(dir.join("data-groups"))
        .unwrap()
        .sync_all()
        .unwrap();
    reclaimed(&dir.join("data-groups/100"), &budget).unwrap();
    assert_eq!(budget.snapshot().file_bytes, 164);
    assert_eq!(budget.snapshot().sources, 5);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn finite_configuration_never_silently_disables_log_byte_admission() {
    assert_eq!(
        parse(Err(std::env::VarError::NotPresent)).unwrap(),
        DEFAULT_LOG_BYTES
    );
    assert_eq!(
        parse(Ok((16 * 1024 * 1024).to_string())).unwrap(),
        16 * 1024 * 1024
    );
    for invalid in ["0", "1", "-1", "+42", " 42", "no", "18446744073709551616"] {
        assert!(parse(Ok(invalid.into())).is_err());
    }
}

#[test]
fn disk_headroom_configuration_is_finite_and_never_disabled_by_bad_input() {
    let parse = |v| parse_field(v, "KV9_MIN_FREE_DISK_BYTES", DEFAULT_MIN_FREE_BYTES);
    assert_eq!(
        parse(Err(std::env::VarError::NotPresent)).unwrap(),
        256 * 1024 * 1024
    );
    assert_eq!(
        parse(Ok((16 * 1024 * 1024).to_string())).unwrap(),
        16 * 1024 * 1024
    );
    for invalid in ["0", "1", "-1", "+42", " 42", "no", "18446744073709551616"] {
        let error = parse(Ok(invalid.into())).unwrap_err().to_string();
        assert!(error.contains("KV9_MIN_FREE_DISK_BYTES"));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn production_recovery_keeps_low_space_inventory_and_refuses_data_with_actual_fstatvfs() {
    let dir = std::env::temp_dir().join(format!(
        "kv9-headroom-recovery-{}",
        kv9_common::StoreIncarnation::mint().unwrap()
    ));
    std::fs::create_dir_all(dir.join("raft")).unwrap();
    std::fs::write(dir.join("raft/raft.log"), [0; 23]).unwrap();
    let budget = recover(&dir, 1024 * 1024, u64::MAX, DEFAULT_REWRITE_BYTES).unwrap();
    assert_eq!(budget.snapshot().file_bytes, 23);
    assert!(budget.reserve(1, kv9_common::RegionId(1)).is_err());
    let observed = budget.snapshot().disk_headroom.unwrap();
    assert_eq!(observed.refusals, 1);
    assert_eq!(observed.probe_errors, 0);
    assert!(observed.last_available_bytes.is_some());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rewrite_workspace_configuration_is_finite_and_does_not_share_the_data_debt_limit() {
    let parse = |v| parse_field(v, "KV9_RAFT_REWRITE_BYTES", DEFAULT_REWRITE_BYTES);
    assert_eq!(
        parse(Err(std::env::VarError::NotPresent)).unwrap(),
        512 * 1024 * 1024
    );
    assert_eq!(
        parse(Ok((16 * 1024 * 1024).to_string())).unwrap(),
        16 * 1024 * 1024
    );
    for invalid in ["0", "1", "-1", "+42", " 42", "bad", "18446744073709551616"] {
        assert!(parse(Ok(invalid.into()))
            .unwrap_err()
            .to_string()
            .contains("KV9_RAFT_REWRITE_BYTES"));
    }
}

#[test]
fn inventory_checks_nested_orphan_segments_against_the_probed_device() {
    #[derive(Debug)]
    struct OtherDevice;
    impl kv9_common::log_budget::headroom::DiskSpaceProbe for OtherDevice {
        fn available_bytes(&self) -> std::io::Result<u64> {
            Ok(u64::MAX)
        }
        fn check_path(&self, path: &Path) -> std::io::Result<()> {
            if path.file_name().is_some_and(|name| name == "foreign") {
                Err(std::io::Error::other("another filesystem"))
            } else {
                Ok(())
            }
        }
    }
    let dir = std::env::temp_dir().join(format!(
        "kv9-headroom-device-{}",
        kv9_common::StoreIncarnation::mint().unwrap()
    ));
    std::fs::create_dir_all(dir.join("data.segments/foreign")).unwrap();
    std::fs::write(dir.join("data.segments/foreign/orphan.wal"), [0; 23]).unwrap();
    let budget = Arc::new(LogBudget::with_disk_headroom(1024, 1, Arc::new(OtherDevice)).unwrap());
    assert!(inventory(&dir, budget)
        .unwrap_err()
        .to_string()
        .contains("another filesystem"));
    std::fs::remove_dir_all(dir).unwrap();
}
