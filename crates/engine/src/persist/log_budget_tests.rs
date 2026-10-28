use super::*;
use std::sync::Arc;

fn directory() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "kv9-wal-debt-{}",
        kv9_common::StoreIncarnation::mint().unwrap()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
fn actual(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                actual(&entry.path())
            } else {
                entry.metadata().unwrap().len()
            }
        })
        .sum()
}
fn write(engine: &WalEngine, index: u64) {
    let mut batch = WriteBatch::new();
    batch.put(
        ColumnFamily::Default,
        index.to_be_bytes().to_vec(),
        vec![index as u8; 4096],
    );
    engine
        .write_applied(batch, AppliedPosition { term: 1, index })
        .unwrap();
}
fn scope() -> FlushScope {
    FlushScope {
        cluster: "log-budget".into(),
        region: 1,
        conf_ver: 1,
        version: 1,
    }
}

#[test]
fn actual_legacy_append_migration_and_segment_rotation_are_charged_above_threshold() {
    let dir = directory();
    let path = dir.join("data.wal");
    let engine = WalEngine::open(&path).unwrap().0;
    let budget = Arc::new(LogBudget::new(1).unwrap());
    engine.attach_log_budget(&budget).unwrap();
    for index in 1..=4 {
        write(&engine, index);
        assert_eq!(budget.snapshot().file_bytes, actual(&dir));
    }
    let legacy_bytes = actual(&dir);
    engine.enable_segmentation_with_target(8192).unwrap();
    assert_eq!(budget.snapshot().file_bytes, actual(&dir));
    assert!(budget.snapshot().peak_accounted_bytes >= legacy_bytes * 2);
    for index in 5..=12 {
        write(&engine, index);
        assert_eq!(
            budget.snapshot().file_bytes,
            actual(&dir),
            "rotation {index}"
        );
    }
    assert!(budget.reserve(1, kv9_common::RegionId(1)).is_err());
    drop(engine);
    assert_eq!(
        budget.snapshot().file_bytes,
        actual(&dir),
        "owner drop does not reclaim files"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn checkpoint_reclaims_actual_files_without_forgiving_orphans_and_reopen_rebuilds_debt() {
    for segmented in [false, true] {
        let dir = directory();
        let path = dir.join("data.wal");
        let engine = WalEngine::open(&path).unwrap().0;
        let budget = Arc::new(LogBudget::new(1).unwrap());
        engine.attach_log_budget(&budget).unwrap();
        if segmented {
            engine.enable_segmentation_with_target(8192).unwrap();
        }
        for index in 1..=8 {
            write(&engine, index);
        }
        let uploader = RemoteUploader::for_harness(Arc::new(crate::MemoryObjectStore::new()));
        let manifest = uploader
            .upload(engine.freeze(scope()).unwrap())
            .unwrap()
            .into_manifest();
        for index in 9..=12 {
            write(&engine, index);
        }
        let before = actual(&dir);
        assert!(engine.checkpoint_applied(&manifest).unwrap());
        assert_eq!(budget.snapshot().file_bytes, actual(&dir));
        assert!(actual(&dir) < before);
        drop(engine);
        // A closed-owner crash residue is inventoried without choosing it for replay.
        std::fs::write(path.with_extension("tail.tmp"), vec![0; 1234]).unwrap();
        if segmented {
            let foreign = path.with_extension("segments").join("foreign");
            std::fs::create_dir_all(&foreign).unwrap();
            std::fs::write(foreign.join("orphan.wal"), vec![0; 5678]).unwrap();
        }
        let reopened = WalEngine::open_with_uploader(&path, Some(&uploader))
            .unwrap()
            .0;
        let new_budget = Arc::new(LogBudget::new(1).unwrap());
        reopened.attach_log_budget(&new_budget).unwrap();
        assert_eq!(new_budget.snapshot().file_bytes, actual(&dir));
        assert_eq!(
            reopened
                .get(ColumnFamily::Default, &12u64.to_be_bytes())
                .unwrap(),
            Some(vec![12; 4096])
        );
        write(&reopened, 13);
        assert_eq!(new_budget.snapshot().file_bytes, actual(&dir));
        drop(reopened);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn failed_topology_publication_never_credits_an_unfinished_rewrite() {
    let dir = directory();
    let path = dir.join("data.wal");
    let engine = WalEngine::open(&path).unwrap().0;
    engine.enable_segmentation_with_target(8192).unwrap();
    let budget = Arc::new(LogBudget::new(1).unwrap());
    engine.attach_log_budget(&budget).unwrap();
    write(&engine, 1);
    let before = budget.snapshot().file_bytes;
    std::fs::create_dir(path.with_extension("topology.tmp")).unwrap();
    let mut batch = WriteBatch::new();
    batch.put(ColumnFamily::Default, b"refused".to_vec(), vec![2; 4096]);
    assert!(engine
        .write_applied(batch, AppliedPosition { term: 1, index: 2 })
        .is_err());
    assert!(budget.snapshot().file_bytes >= actual(&dir));
    assert!(
        budget.snapshot().file_bytes > before,
        "orphan successor header is still debt"
    );
    drop(engine);
    std::fs::remove_dir_all(dir).unwrap();
}
