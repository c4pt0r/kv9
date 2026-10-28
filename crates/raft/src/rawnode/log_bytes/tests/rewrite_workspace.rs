use super::*;

#[test]
fn oversized_group_defers_while_another_group_reclaims_and_both_recover() {
    let dir = directory();
    let budget = Arc::new(
        LogBudget::new(64 * 1024 * 1024)
            .unwrap()
            .with_rewrite_limit(4096)
            .unwrap(),
    );
    let mut drivers = Vec::new();
    let mut engines = Vec::new();
    let mut tails = Vec::new();
    for region in 1..=2 {
        let (driver, engine) = open(
            &dir.join(region.to_string()),
            NodeId(1),
            RegionId(region),
            &[1],
            &InProcHub::new(),
            &budget,
        );
        leader(&driver);
        let mut positions = Vec::new();
        for _ in 0..8 {
            positions.push(driver.propose(&command()).unwrap());
            driver.step().unwrap();
        }
        let floor = positions[if region == 1 { 0 } else { 7 }];
        driver
            .peer()
            .truncate_retained_log(
                kv9_common::AppliedPosition {
                    term: floor.term,
                    index: floor.index.0,
                },
                b"fixture committed compaction decision",
            )
            .unwrap();
        tails.push(*positions.last().unwrap());
        drivers.push(driver);
        engines.push(engine);
    }
    let large_before = drivers[0].peer().log_file_bytes();
    let small_before = drivers[1].peer().log_file_bytes();
    assert!(!drivers[0].peer().reclaim_log().unwrap());
    assert_eq!(drivers[0].peer().log_file_bytes(), large_before);
    assert!(drivers[1].peer().reclaim_log().unwrap());
    assert!(drivers[1].peer().log_file_bytes() < small_before);
    assert!(drivers.iter().all(|d| d.status().fatal.is_none()));
    let floor = tails[0];
    drivers[0]
        .peer()
        .truncate_retained_log(
            kv9_common::AppliedPosition {
                term: floor.term,
                index: floor.index.0,
            },
            b"fixture later committed compaction decision",
        )
        .unwrap();
    assert!(drivers[0].peer().reclaim_log().unwrap());
    let stats = budget.snapshot().rewrite_workspace;
    assert_eq!(
        (
            stats.admitted,
            stats.deferred,
            stats.active,
            stats.reserved_bytes
        ),
        (2, 1, 0, 0)
    );
    assert!(stats.peak_bytes <= 4096);
    for driver in &drivers {
        driver.propose(&command()).unwrap();
        driver.step().unwrap();
    }
    drop((drivers, engines));
    for region in 1..=2 {
        let path = dir.join(region.to_string());
        let engine = Arc::new(WalEngine::open(path.join("data.wal")).unwrap().0);
        engine.attach_log_budget(&budget).unwrap();
        let storage = DiskRaftStorage::recover(&path.join("raft")).unwrap();
        let debt = storage.attach_log_budget(&budget).unwrap();
        // The fixture remembers its exact committed compaction decision.
        // Production obtains this authority from RegionManager's catalog.
        let cut = tails[region as usize - 1];
        let base = kv9_common::AppliedPosition {
            term: cut.term,
            index: cut.index.0,
        };
        assert_eq!(storage.compacted_base().unwrap(), Some(base));
        let peer = Arc::new(
            RaftPeer::with_installed_storage(NodeId(1), RegionId(region), storage, base).unwrap(),
        );
        let driver = NodeDriver::with_installed_base(
            peer,
            Arc::new(InProcHub::new().endpoint(NodeId(1))) as Arc<dyn RaftTransport>,
            MemStateMachine::with_engine(engine.clone()).unwrap(),
            base,
        )
        .unwrap();
        driver.attach_log_debt(debt).unwrap();
        assert_eq!(
            engine.get(ColumnFamily::Default, b"key").unwrap(),
            Some(vec![7; 4096])
        );
        assert!(driver.status().fatal.is_none());
        leader(&driver);
        let at = driver
            .propose(&Command::Put {
                cf: 0,
                key: b"after-reopen".to_vec(),
                value: b"ok".to_vec(),
            })
            .unwrap();
        driver.step().unwrap();
        assert!(matches!(
            driver.wait_applied(at, Duration::ZERO).unwrap(),
            ApplyWaitOutcome::Applied(_)
        ));
        assert_eq!(
            engine.get(ColumnFamily::Default, b"after-reopen").unwrap(),
            Some(b"ok".to_vec())
        );
    }
    assert_eq!(budget.snapshot().file_bytes, actual(&dir));
    std::fs::remove_dir_all(dir).unwrap();
}
