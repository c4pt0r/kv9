use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Debug)]
struct Probe {
    free: AtomicU64,
    failed: AtomicBool,
}
impl DiskSpaceProbe for Probe {
    fn available_bytes(&self) -> std::io::Result<u64> {
        if self.failed.load(Ordering::SeqCst) {
            Err(std::io::Error::other("failed probe"))
        } else {
            Ok(self.free.load(Ordering::SeqCst))
        }
    }
    fn check_path(&self, _: &Path) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn rewrites_can_reclaim_above_data_debt_limit_but_share_a_finite_output_allowance() {
    let budget = Arc::new(LogBudget::new(10).unwrap().with_rewrite_limit(100).unwrap());
    let a = budget.track(Path::new("raft-a"), 200).unwrap();
    let b = budget.track(Path::new("raft-b"), 200).unwrap();
    assert!(a.reserve(1, RegionId(1)).is_err());
    let rewrite = a.reserve_rewrite(70).unwrap();
    assert!(b.reserve_rewrite(31).is_none());
    assert!(b.reserve_rewrite(101).is_none());
    assert!(b.reserve_rewrite(0).is_none());
    let other = b.reserve_rewrite(30).unwrap();
    assert_eq!(budget.snapshot().rewrite_workspace.reserved_bytes, 100);
    assert_eq!(budget.snapshot().peak_accounted_bytes, 500);
    // A failed rewrite retains file debt; dropping a job refunds only future output.
    a.grow(50).unwrap();
    drop((a, rewrite, other));
    assert_eq!(budget.snapshot().file_bytes, 450);
    assert_eq!(budget.snapshot().rewrite_workspace.reserved_bytes, 0);
    assert_eq!(budget.snapshot().rewrite_workspace.active, 0);
}

#[test]
fn actual_space_claims_are_shared_with_data_and_remote_capacity_reports() {
    let probe = Arc::new(Probe {
        free: AtomicU64::new(150),
        failed: AtomicBool::new(false),
    });
    let budget = Arc::new(
        LogBudget::with_disk_headroom(1000, 20, probe.clone())
            .unwrap()
            .with_rewrite_limit(200)
            .unwrap(),
    );
    let debt = budget.track(Path::new("raft"), 100).unwrap();
    let data = debt.reserve(40, RegionId(1)).unwrap();
    let rewrite = debt.reserve_rewrite(60).unwrap();
    assert_eq!(budget.observe_data_capacity(), Some(30));
    assert!(debt.reserve_rewrite(31).is_none());
    assert!(debt.reserve(31, RegionId(1)).is_err());
    assert!(debt.reserve(30, RegionId(1)).is_ok());
    probe.failed.store(true, Ordering::SeqCst);
    assert!(debt.reserve_rewrite(1).is_none());
    assert_eq!(budget.observe_data_capacity(), None);
    probe.failed.store(false, Ordering::SeqCst);
    probe.free.store(1000, Ordering::SeqCst);
    debt.mark_uncertain();
    assert!(debt.reserve_rewrite(1).is_none());
    debt.reconcile(100).unwrap();
    drop((data, rewrite));
    assert!(debt.reserve_rewrite(200).is_some());
}

#[test]
fn concurrent_rewrite_jobs_cannot_reuse_one_observed_free_space_allowance() {
    let probe = Arc::new(Probe {
        free: AtomicU64::new(100),
        failed: AtomicBool::new(false),
    });
    let budget = Arc::new(
        LogBudget::with_disk_headroom(1000, 20, probe)
            .unwrap()
            .with_rewrite_limit(100)
            .unwrap(),
    );
    let debt = budget.track(Path::new("raft"), 0).unwrap();
    let barrier = std::sync::Barrier::new(24);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..24)
            .map(|_| {
                let (debt, barrier) = (&debt, &barrier);
                scope.spawn(move || {
                    barrier.wait();
                    debt.reserve_rewrite(30)
                })
            })
            .collect();
        let claims: Vec<_> = handles
            .into_iter()
            .filter_map(|h| h.join().unwrap())
            .collect();
        assert_eq!(claims.len(), 2);
        assert_eq!(budget.snapshot().rewrite_workspace.peak_bytes, 60);
        assert_eq!(budget.snapshot().rewrite_workspace.deferred, 22);
    });
    assert_eq!(budget.snapshot().rewrite_workspace.reserved_bytes, 0);
}

#[test]
fn writes_overlapping_blocked_rewrite_probe_are_still_deducted_after_completion() {
    #[derive(Debug)]
    struct Gated {
        entered: std::sync::mpsc::Sender<()>,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl DiskSpaceProbe for Gated {
        fn available_bytes(&self) -> std::io::Result<u64> {
            self.entered.send(()).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
            Ok(100)
        }
        fn check_path(&self, _: &Path) -> std::io::Result<()> {
            Ok(())
        }
    }
    let (entered, wait) = std::sync::mpsc::channel();
    let (release, gate) = std::sync::mpsc::channel();
    let budget = Arc::new(
        LogBudget::with_disk_headroom(
            1000,
            20,
            Arc::new(Gated {
                entered,
                release: Mutex::new(gate),
            }),
        )
        .unwrap(),
    );
    let debt = budget.track(Path::new("raft"), 0).unwrap();
    let prior = debt.begin_write(30).unwrap();
    std::thread::scope(|scope| {
        let job = scope.spawn(|| debt.reserve_rewrite(40));
        wait.recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        let during = debt.begin_write(20).unwrap();
        drop((prior, during));
        release.send(()).unwrap();
        assert!(job.join().unwrap().is_none());
    });
    let snapshot = budget.snapshot();
    assert_eq!(snapshot.rewrite_workspace.active, 0);
    assert_eq!(snapshot.disk_headroom.unwrap().last_probe_overlap_bytes, 50);
}
