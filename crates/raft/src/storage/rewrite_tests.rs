use super::*;
use kv9_common::log_budget::headroom::DiskSpaceProbe;
use raft::Storage as _;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

fn path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "kv9-rewrite-{}",
        kv9_common::StoreIncarnation::mint().unwrap()
    ))
}
fn entry(index: u64, bytes: usize) -> Entry {
    Entry {
        index,
        term: 1,
        data: vec![index as u8; bytes].into(),
        ..Default::default()
    }
}
fn populated(path: &Path, entries: u64, bytes: usize) -> DiskRaftStorage {
    let storage = DiskRaftStorage::open(path, &[1, 2, 3]).unwrap().0;
    let rows: Vec<_> = (1..=entries).map(|n| entry(n, bytes)).collect();
    storage.append(&rows).unwrap();
    storage
        .set_hardstate(&HardState {
            term: 2,
            vote: 3,
            commit: entries - 1,
            ..Default::default()
        })
        .unwrap();
    storage
}
fn output_bytes(storage: &DiskRaftStorage) -> u64 {
    let mut bytes = 0;
    storage
        .visit_reclamation_records(|_, value| {
            bytes += 9 + value.len() as u64;
            Ok(())
        })
        .unwrap();
    bytes
}

#[test]
fn rewrite_and_retained_byte_scan_use_bounded_scratch_for_a_large_tail() {
    let dir = path();
    let storage = populated(&dir, 2048, 16 * 1024);
    // No old resident entries are destroyed in either measured operation.
    // This distinguishes temporary heap allocation from the existing mirror.
    let (bytes, scan_peak) =
        crate::test_alloc::peak(|| storage.retained_committed_bytes(2047).unwrap());
    assert_eq!(bytes, 2047 * 16 * 1024);
    let before = storage.initial_state().unwrap();
    let budget = Arc::new(
        LogBudget::new(1)
            .unwrap()
            .with_rewrite_limit(40 * 1024 * 1024)
            .unwrap(),
    );
    storage.attach_log_budget(&budget).unwrap();
    let expected = output_bytes(&storage);
    let (rewritten, rewrite_peak) = crate::test_alloc::peak(|| storage.rewrite_log().unwrap());
    assert!(rewritten);
    assert!(scan_peak < 512 * 1024, "scan scratch {scan_peak}");
    assert!(rewrite_peak < 512 * 1024, "rewrite scratch {rewrite_peak}");
    println!("raft_rewrite_memory tail_bytes={bytes} output_bytes={expected} scan_peak={scan_peak} rewrite_peak={rewrite_peak}");
    assert_eq!(storage.log_file_bytes(), expected);
    assert_eq!(budget.snapshot().file_bytes, expected);
    assert_eq!(budget.snapshot().rewrite_workspace.peak_bytes, expected);
    assert_eq!(budget.snapshot().rewrite_workspace.reserved_bytes, 0);
    drop(storage);
    let recovered = DiskRaftStorage::recover(&dir).unwrap();
    let after = recovered.initial_state().unwrap();
    assert_eq!(after.hard_state, before.hard_state);
    assert_eq!(after.conf_state, before.conf_state);
    assert_eq!(recovered.last_index().unwrap(), 2048);
    recovered
        .visit_retained_entries(1, 2048, |value| {
            assert_eq!(*value, entry(value.index, 16 * 1024));
            Ok(())
        })
        .unwrap();
    drop(recovered);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn insufficient_rewrite_workspace_defers_before_mutating_live_or_orphan_files() {
    let dir = path();
    let storage = populated(&dir, 20, 1024);
    let required = output_bytes(&storage);
    let tmp = dir.join("raft.log.tmp");
    std::fs::write(&tmp, b"previous interrupted rewrite").unwrap();
    let live = std::fs::read(dir.join("raft.log")).unwrap();
    let budget = Arc::new(
        LogBudget::new(1)
            .unwrap()
            .with_rewrite_limit(required - 1)
            .unwrap(),
    );
    storage.attach_log_budget(&budget).unwrap();
    assert!(!storage.rewrite_log().unwrap());
    assert_eq!(std::fs::read(dir.join("raft.log")).unwrap(), live);
    assert_eq!(
        std::fs::read(&tmp).unwrap(),
        b"previous interrupted rewrite"
    );
    assert_eq!(budget.snapshot().rewrite_workspace.deferred, 1);
    assert_eq!(budget.snapshot().rewrite_workspace.active, 0);
    storage.append(&[entry(21, 10)]).unwrap(); // deferral did not poison the writer
    drop(storage);
    let recovered = DiskRaftStorage::recover(&dir).unwrap();
    let budget = Arc::new(
        LogBudget::new(1)
            .unwrap()
            .with_rewrite_limit(output_bytes(&recovered))
            .unwrap(),
    );
    recovered.attach_log_budget(&budget).unwrap();
    assert!(recovered.rewrite_log().unwrap());
    assert!(!tmp.exists());
    assert_eq!(budget.snapshot().file_bytes, recovered.log_file_bytes());
    drop(recovered);
    std::fs::remove_dir_all(dir).unwrap();
}

#[derive(Debug)]
struct Probe {
    free: AtomicU64,
    failed: AtomicBool,
}
impl DiskSpaceProbe for Probe {
    fn available_bytes(&self) -> std::io::Result<u64> {
        if self.failed.load(Ordering::SeqCst) {
            Err(std::io::Error::other("probe failed"))
        } else {
            Ok(self.free.load(Ordering::SeqCst))
        }
    }
    fn check_path(&self, _: &Path) -> std::io::Result<()> {
        Ok(())
    }
}
#[test]
fn rewrite_disk_failure_defers_and_recovery_above_data_limit_releases_only_actual_file_debt() {
    let dir = path();
    let storage = populated(&dir, 30, 1024);
    storage
        .compact_retained_prefix(
            kv9_common::AppliedPosition { term: 1, index: 20 },
            &ConfState::from((vec![1, 2, 3], vec![])),
            b"authorized-test-floor",
        )
        .unwrap();
    let required = output_bytes(&storage);
    let probe = Arc::new(Probe {
        free: AtomicU64::new(required + 19),
        failed: AtomicBool::new(false),
    });
    let budget = Arc::new(
        LogBudget::with_disk_headroom(1, 20, probe.clone())
            .unwrap()
            .with_rewrite_limit(required)
            .unwrap(),
    );
    let debt = storage.attach_log_budget(&budget).unwrap();
    let before = storage.log_file_bytes();
    assert!(!storage.rewrite_log().unwrap());
    probe.free.store(required + 20, Ordering::SeqCst);
    probe.failed.store(true, Ordering::SeqCst);
    assert!(!storage.rewrite_log().unwrap());
    assert_eq!(budget.snapshot().file_bytes, before);
    assert!(!dir.join("raft.log.tmp").exists());
    probe.failed.store(false, Ordering::SeqCst);
    assert!(storage.rewrite_log().unwrap());
    assert!(storage.log_file_bytes() < before);
    assert_eq!(budget.snapshot().file_bytes, storage.log_file_bytes());
    assert_eq!(budget.snapshot().rewrite_workspace.active, 0);
    assert!(debt.reserve(1, kv9_common::RegionId(1)).is_err());
    drop(storage);
    assert_eq!(
        DiskRaftStorage::recover(&dir)
            .unwrap()
            .last_index()
            .unwrap(),
        30
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn failed_temp_creation_refunds_workspace_without_poisoning_the_original_writer() {
    let dir = path();
    let storage = populated(&dir, 10, 100);
    let budget = Arc::new(LogBudget::new(10000).unwrap());
    storage.attach_log_budget(&budget).unwrap();
    let before = storage.log_file_bytes();
    std::fs::create_dir(dir.join("raft.log.tmp")).unwrap();
    assert!(storage.rewrite_log().is_err());
    assert_eq!(storage.log_file_bytes(), before);
    assert_eq!(budget.snapshot().rewrite_workspace.reserved_bytes, 0);
    assert_eq!(budget.snapshot().rewrite_workspace.active, 0);
    storage.append(&[entry(11, 100)]).unwrap();
    std::fs::remove_dir(dir.join("raft.log.tmp")).unwrap();
    assert!(storage.rewrite_log().unwrap());
    drop(storage);
    assert_eq!(
        DiskRaftStorage::recover(&dir)
            .unwrap()
            .last_index()
            .unwrap(),
        11
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rewrite_keeps_durable_lease_fence_and_does_not_reserve_workspace_for_excluded_layout() {
    let dir = path();
    let storage = populated(&dir, 10, 100);
    let policy = LeasePolicy {
        node: 1,
        group: 9,
        configuration: 1,
        voters: vec![1, 2, 3],
        promise_ns: 1000,
        drift_ppb: 1,
        margin_ns: 1,
    };
    let epoch = storage.begin_lease_incarnation(&policy).unwrap();
    let budget = Arc::new(LogBudget::new(1).unwrap());
    storage.attach_log_budget(&budget).unwrap();
    let before = std::fs::read(dir.join("raft.log")).unwrap();
    assert!(!storage.rewrite_log().unwrap());
    assert_eq!(std::fs::read(dir.join("raft.log")).unwrap(), before);
    assert_eq!(budget.snapshot().rewrite_workspace.admitted, 0);
    drop(storage);
    assert_eq!(
        DiskRaftStorage::recover(&dir)
            .unwrap()
            .recovered_lease_epoch(),
        Some(epoch)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn record_framing_matches_the_existing_length_kind_payload_and_fnv_contract() {
    let mut file = std::io::Cursor::new(Vec::new());
    // A real DurableFile wrapper avoids depending on a second encoder.
    struct File(std::io::Cursor<Vec<u8>>);
    impl Read for File {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            self.0.read(out)
        }
    }
    impl Write for File {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.write(b)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl Seek for File {
        fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
            self.0.seek(pos)
        }
    }
    impl DurableFile for File {
        fn sync_data(&self) -> std::io::Result<()> {
            Ok(())
        }
        fn set_len(&self, _: u64) -> std::io::Result<()> {
            unreachable!()
        }
    }
    for (kind, payload) in [
        (REC_ENTRY, &b"payload\0\xff"[..]),
        (REC_CONF_STATE, &b""[..]),
    ] {
        let mut framed = File(std::io::Cursor::new(Vec::new()));
        DiskRaftStorage::<OsFileSystem>::write_record_unsynced(
            &WalIoMetrics::default(),
            &mut framed,
            kind,
            payload,
        )
        .unwrap();
        let body: Vec<_> = std::iter::once(kind)
            .chain(payload.iter().copied())
            .collect();
        file.write_all(&(body.len() as u32).to_be_bytes()).unwrap();
        file.write_all(&fnv1a(&body).to_be_bytes()).unwrap();
        file.write_all(&body).unwrap();
        assert_eq!(framed.0.get_ref(), file.get_ref());
        assert_eq!(
            next_record(file.get_ref(), 0),
            Some((kind, payload, file.get_ref().len()))
        );
        file = std::io::Cursor::new(Vec::new());
    }
}
