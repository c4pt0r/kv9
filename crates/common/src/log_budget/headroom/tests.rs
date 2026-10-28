use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Debug)]
struct Probe {
    free: AtomicU64,
    failed: AtomicBool,
}
impl Probe {
    fn new(free: u64) -> Arc<Self> {
        Arc::new(Self {
            free: AtomicU64::new(free),
            failed: AtomicBool::new(false),
        })
    }
}
impl DiskSpaceProbe for Probe {
    fn available_bytes(&self) -> std::io::Result<u64> {
        if self.failed.load(Ordering::SeqCst) {
            Err(std::io::Error::other("probe I/O failure"))
        } else {
            Ok(self.free.load(Ordering::SeqCst))
        }
    }
    fn check_path(&self, _: &Path) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn capacity_report_uses_fresh_space_and_does_not_count_a_rejected_proposal() {
    let probe = Probe::new(100);
    let budget = Arc::new(LogBudget::with_disk_headroom(1000, 20, probe.clone()).unwrap());
    let debt = budget.track(Path::new("capacity"), 900).unwrap();
    assert_eq!(budget.observe_data_capacity(), Some(80));
    let claim = debt.reserve(30, RegionId(1)).unwrap();
    assert_eq!(budget.observe_data_capacity(), Some(50));
    probe.free.store(10, Ordering::SeqCst);
    assert_eq!(budget.observe_data_capacity(), Some(0));
    probe.failed.store(true, Ordering::SeqCst);
    assert_eq!(budget.observe_data_capacity(), None);
    assert_eq!(budget.snapshot().reserved_bytes, 30);
    let snapshot = budget.snapshot().disk_headroom.unwrap();
    assert_eq!(snapshot.refusals, 0);
    assert_eq!(snapshot.probe_errors, 1);
    drop(claim);
}

#[test]
fn capacity_report_deducts_completed_writes_that_overlapped_the_probe() {
    let (entered, wait) = std::sync::mpsc::channel();
    let (release, gate) = std::sync::mpsc::channel();
    let budget = Arc::new(
        LogBudget::with_disk_headroom(
            1000,
            20,
            Arc::new(GatedProbe {
                enter: entered,
                release: Mutex::new(gate),
            }),
        )
        .unwrap(),
    );
    let debt = budget.track(Path::new("capacity"), 0).unwrap();
    let prior = debt.begin_write(30).unwrap();
    std::thread::scope(|scope| {
        let report = scope.spawn(|| budget.observe_data_capacity());
        wait.recv_timeout(Duration::from_secs(1)).unwrap();
        let during = debt.begin_write(20).unwrap();
        drop(prior);
        drop(during);
        release.send(()).unwrap();
        assert_eq!(report.join().unwrap(), Some(30));
    });
    assert_eq!(budget.snapshot().disk_headroom.unwrap().refusals, 0);
}

#[test]
fn fresh_free_space_accounts_for_future_claims_without_subtracting_old_files_twice() {
    let probe = Probe::new(100);
    let budget = Arc::new(LogBudget::with_disk_headroom(1000, 20, probe.clone()).unwrap());
    let debt = budget.track(Path::new("raft"), 500).unwrap();
    let a = budget.reserve(40, RegionId(1)).unwrap();
    a.accept(&debt, 1, 1);
    let b = budget.reserve(40, RegionId(2)).unwrap();
    b.accept(&debt, 2, 1);
    assert!(budget.reserve(1, RegionId(3)).is_err());
    // Closing an owner does not give capacity back.
    drop(debt);
    assert!(budget.reserve(1, RegionId(3)).is_err());
    let debt = budget.track(Path::new("raft"), 500).unwrap();
    debt.applied_through(1);
    assert!(budget.reserve(40, RegionId(3)).is_ok());
    // External consumption is visible without waiting for a local log write.
    probe.free.store(59, Ordering::SeqCst);
    assert!(budget.reserve(1, RegionId(3)).is_err());
    probe.failed.store(true, Ordering::SeqCst);
    assert!(budget.reserve(1, RegionId(3)).is_err());
    assert_eq!(budget.snapshot().disk_headroom.unwrap().probe_errors, 1);
    // Accepted/control work bypasses free-space refusal, but remains debt.
    debt.retain_position(3, 1, 200).unwrap();
    debt.grow(100).unwrap();
    debt.applied_through(3);
    probe.failed.store(false, Ordering::SeqCst);
    probe.free.store(100, Ordering::SeqCst);
    assert!(budget.reserve(80, RegionId(3)).is_ok());
}

#[test]
fn stale_or_overflowing_space_observation_cannot_admit_data() {
    let mut observation = Observation::default();
    let old = Sample {
        began: Instant::now() - MAX_PROBE_AGE - Duration::from_millis(1),
        available: Ok(u64::MAX),
    };
    assert!(!observation.admit(1, old, 0, 0, 1));
    assert_eq!(observation.errors, 1);
    let sample = || Sample {
        began: Instant::now(),
        available: Ok(u64::MAX),
    };
    assert!(!observation.admit(1, sample(), u128::MAX, 0, 1));
    assert!(!observation.admit(1, sample(), 0, u128::from(u64::MAX), u64::MAX));
}

#[derive(Debug)]
struct GatedProbe {
    enter: std::sync::mpsc::Sender<()>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
}
impl DiskSpaceProbe for GatedProbe {
    fn available_bytes(&self) -> std::io::Result<u64> {
        self.enter.send(()).unwrap();
        self.release.lock().unwrap().recv().unwrap();
        Ok(100)
    }
    fn check_path(&self, _: &Path) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn writes_started_before_and_during_probe_remain_deducted_after_they_finish() {
    let (entered, wait) = std::sync::mpsc::channel();
    let (release, gate) = std::sync::mpsc::channel();
    let probe = Arc::new(GatedProbe {
        enter: entered,
        release: Mutex::new(gate),
    });
    let budget = Arc::new(LogBudget::with_disk_headroom(1000, 20, probe).unwrap());
    let debt = budget.track(Path::new("raft"), 0).unwrap();
    let prior = debt.begin_write(30).unwrap();
    std::thread::scope(|scope| {
        let attempt = scope.spawn(|| budget.reserve(40, RegionId(1)));
        wait.recv_timeout(Duration::from_secs(1)).unwrap();
        // A blocked probe holds no budget lock: another group's write/control
        // and actual apply completion can all advance during the syscall.
        let during = debt.begin_write(20).unwrap();
        debt.retain_position(1, 1, 7).unwrap();
        debt.applied_through(1);
        drop(prior);
        drop(during);
        release.send(()).unwrap();
        assert!(matches!(
            attempt.join().unwrap(),
            Err(Error::WriteBackpressure { .. })
        ));
    });
    let state = budget.snapshot().disk_headroom.unwrap();
    assert_eq!(state.in_flight_write_bytes, 0);
    assert_eq!(state.last_probe_overlap_bytes, 50);
    assert_eq!(state.refusals, 1);
}

#[test]
fn concurrent_groups_cannot_reuse_the_same_free_space() {
    let budget = Arc::new(LogBudget::with_disk_headroom(1000, 20, Probe::new(100)).unwrap());
    let debt = budget.track(Path::new("raft"), 0).unwrap();
    let barrier = std::sync::Barrier::new(32);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..32)
            .map(|i| {
                let (budget, debt, barrier) = (&budget, &debt, &barrier);
                scope.spawn(move || {
                    barrier.wait();
                    budget
                        .reserve(30, RegionId(i))
                        .map(|claim| claim.accept(debt, i + 1, 1))
                })
            })
            .collect();
        assert_eq!(
            handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .filter(Result::is_ok)
                .count(),
            2
        );
    });
    assert_eq!(budget.snapshot().reserved_bytes, 60);
    assert_eq!(budget.snapshot().disk_headroom.unwrap().refusals, 30);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_probe_keeps_its_descriptor_checks_device_and_never_fabricates_zero() {
    let dir = std::env::temp_dir().join(format!(
        "kv9-headroom-{}",
        crate::StoreIncarnation::mint().unwrap()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let probe = OsDiskSpaceProbe::open(&dir).unwrap();
    let _ = probe.available_bytes().unwrap();
    probe
        .check_path(&dir.join("not-yet-created/raft/raft.log"))
        .unwrap();
    assert!(probe.check_path(Path::new("/proc")).is_err());
    let moved = dir.with_extension("moved");
    std::fs::rename(&dir, &moved).unwrap();
    assert!(probe.available_bytes().is_ok());
    assert!(OsDiskSpaceProbe::open(&dir).is_err());
    std::fs::remove_dir_all(moved).unwrap();
}
