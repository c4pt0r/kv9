use super::*;

#[test]
fn capacity_reports_account_for_files_claims_and_uncertainty_without_reserving() {
    let budget = Arc::new(LogBudget::new(100).unwrap());
    let debt = budget.track(Path::new("capacity.wal"), 30).unwrap();
    assert_eq!(budget.observe_data_capacity(), Some(70));
    let claim = debt.reserve(20, RegionId(1)).unwrap();
    assert_eq!(budget.observe_data_capacity(), Some(50));
    assert_eq!(budget.snapshot().reserved_bytes, 20);
    debt.mark_uncertain();
    assert_eq!(budget.observe_data_capacity(), None);
    debt.reconcile(80).unwrap();
    assert_eq!(budget.observe_data_capacity(), Some(0));
    drop(claim);
    assert_eq!(budget.observe_data_capacity(), Some(20));
}

#[test]
fn one_node_serializes_admission_and_owner_drop_retains_named_obligations() {
    let budget = Arc::new(LogBudget::new(100).unwrap());
    let a = budget.track(Path::new("a.wal"), 30).unwrap();
    let b = budget.track(Path::new("b.wal"), 20).unwrap();
    let claim = a.reserve(40, RegionId(1)).unwrap();
    assert!(matches!(
        b.reserve(11, RegionId(2)),
        Err(Error::WriteBackpressure { .. })
    ));
    claim.accept(&a, 7, 2);
    drop(a);
    assert_eq!(budget.snapshot().reserved_bytes, 40);
    let a = budget.track(Path::new("a.wal"), 35).unwrap();
    a.retain_position(7, 2, 40).unwrap();
    assert_eq!(budget.snapshot().reserved_bytes, 40);
    a.applied_through(6);
    assert_eq!(budget.snapshot().reserved_bytes, 40);
    a.applied_through(7);
    assert_eq!(budget.snapshot().reserved_bytes, 0);
    assert_eq!(budget.snapshot().file_bytes, 55);
    assert!(b.reserve(45, RegionId(2)).is_ok());
}

#[test]
fn accepted_replication_can_exceed_threshold_and_unknown_accounting_refuses_new_data() {
    let budget = Arc::new(LogBudget::new(10).unwrap());
    let debt = budget.track(Path::new("raft"), 20).unwrap();
    debt.retain_position(1, 1, 100).unwrap();
    debt.grow(20).unwrap();
    assert!(debt.reserve(1, RegionId(1)).is_err());
    debt.applied_through(1);
    debt.reconcile(0).unwrap();
    debt.mark_uncertain();
    assert!(debt.reserve(1, RegionId(1)).is_err());
    debt.reconcile(0).unwrap();
    let provisional = debt.reserve(10, RegionId(1)).unwrap();
    drop(provisional);
    assert_eq!(budget.snapshot().reserved_bytes, 0);
}

#[test]
fn failed_layout_transition_does_not_credit_visible_deletion() {
    let budget = Arc::new(LogBudget::new(100).unwrap());
    let debt = budget.track(Path::new("log"), 70).unwrap();
    let guard = debt.reconciliation(|| Ok(0));
    drop(guard); // no successful directory publication
    assert_eq!(budget.snapshot().file_bytes, 70);
    let mut guard = debt.reconciliation(|| Ok(10));
    guard.completed();
    drop(guard);
    assert_eq!(budget.snapshot().file_bytes, 10);
}

#[test]
fn concurrent_groups_share_the_same_byte_allowance() {
    let budget = Arc::new(LogBudget::new(100).unwrap());
    let debt = budget.track(Path::new("log"), 10).unwrap();
    let barrier = std::sync::Barrier::new(24);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..24)
            .map(|i| {
                let debt = &debt;
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    debt.reserve(30, RegionId(i))
                        .map(|claim| claim.accept(debt, i + 1, 1))
                })
            })
            .collect();
        let accepted = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(Result::is_ok)
            .count();
        assert_eq!(accepted, 3);
    });
    assert_eq!(budget.snapshot().pending_positions, 3);
    assert_eq!(budget.snapshot().reserved_bytes, 90);
}

#[test]
fn short_writes_release_only_unwritten_bytes_and_ambiguous_errors_retain_debt() {
    struct FaultFile {
        data: std::io::Cursor<Vec<u8>>,
        debt: LogDebt,
        fail: bool,
    }
    impl Write for FaultFile {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            assert_eq!(
                self.debt.budget.state.lock().unwrap().writing,
                bytes.len() as u64
            );
            assert!(self.debt.bytes() >= self.data.get_ref().len() as u64 + bytes.len() as u64);
            let n = bytes.len().min(3);
            self.data.write_all(&bytes[..n])?;
            if self.fail {
                Err(std::io::Error::other("ambiguous append failure"))
            } else {
                Ok(n)
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl Read for FaultFile {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            self.data.read(bytes)
        }
    }
    impl Seek for FaultFile {
        fn seek(&mut self, at: SeekFrom) -> std::io::Result<u64> {
            self.data.seek(at)
        }
    }
    impl DurableFile for FaultFile {
        fn sync_data(&self) -> std::io::Result<()> {
            Err(std::io::Error::other("sync failure"))
        }
        fn set_len(&self, _: u64) -> std::io::Result<()> {
            unreachable!()
        }
    }
    let budget = Arc::new(LogBudget::new(10).unwrap());
    let debt = budget.track(Path::new("fault.wal"), 0).unwrap();
    let mut file = FaultFile {
        data: std::io::Cursor::new(Vec::new()),
        debt: debt.clone(),
        fail: false,
    };
    BudgetWriter::new(&mut file, Some(&debt))
        .write_all(&[1; 7])
        .unwrap();
    assert_eq!(debt.bytes(), 7);
    file.fail = true;
    assert!(BudgetWriter::new(&mut file, Some(&debt))
        .write_all(&[2; 5])
        .is_err());
    assert_eq!(file.data.get_ref().len(), 10);
    assert_eq!(debt.bytes(), 12);
    assert_eq!(budget.state.lock().unwrap().writing, 0);
    assert!(BudgetWriter::new(&mut file, Some(&debt))
        .sync_data()
        .is_err());
    assert_eq!(debt.bytes(), 12);
    assert!(debt.reserve(1, RegionId(1)).is_err());
}

#[test]
fn failed_reconciliation_refuses_admission_until_an_exclusive_successful_observation() {
    let budget = Arc::new(LogBudget::new(100).unwrap());
    let debt = budget.track(Path::new("log"), 30).unwrap();
    let mut guard = debt.reconciliation(|| Err(Error::Engine("metadata read failed".into())));
    guard.completed();
    drop(guard);
    assert_eq!(budget.snapshot().file_bytes, 30);
    assert_eq!(budget.snapshot().uncertain_sources, 1);
    assert!(debt.reserve(1, RegionId(1)).is_err());
    debt.reconcile(20).unwrap();
    assert_eq!(budget.snapshot().uncertain_sources, 0);
    assert!(debt.reserve(80, RegionId(1)).is_ok());
}
