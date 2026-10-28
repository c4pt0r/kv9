use super::*;
use crate::checkpoint::RemoteUploader;
use crate::MemoryObjectStore;
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::Arc;

fn files(directory: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut result = BTreeMap::new();
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            result.extend(files(&path));
        } else {
            result.insert(path.clone(), std::fs::read(path).unwrap());
        }
    }
    result
}

struct Fixture {
    directory: tempfile::TempDir,
    path: PathBuf,
    active: PathBuf,
    manifest: CheckpointManifest,
    uploader: RemoteUploader,
}
impl Fixture {
    fn new(segmented: bool, target: u64) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("data.wal");
        let engine = WalEngine::open(&path).unwrap().0;
        if segmented {
            engine.enable_segmentation_with_target(target).unwrap();
        }
        let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()));
        let mut manifest = None;
        for index in 1u64..=9 {
            let mut batch = WriteBatch::new();
            batch.put(
                ColumnFamily::Default,
                b"key".to_vec(),
                index.to_be_bytes().to_vec(),
            );
            engine
                .write_applied(batch, AppliedPosition { term: 2, index })
                .unwrap();
            if index == 2 {
                manifest = Some(
                    uploader
                        .upload(
                            engine
                                .freeze(FlushScope {
                                    cluster: "inspection".into(),
                                    region: 100,
                                    conf_ver: 1,
                                    version: 1,
                                })
                                .unwrap(),
                        )
                        .unwrap()
                        .into_manifest(),
                );
            }
        }
        let manifest = manifest.unwrap();
        if segmented {
            engine.checkpoint_applied(&manifest).unwrap();
        } else {
            // A visible selector with its old, still covered WAL prefix is a
            // valid publication crash window. Inspection must skip that prefix.
            std::fs::write(
                path.with_extension("checkpoint"),
                manifest.encode().unwrap(),
            )
            .unwrap();
        }
        drop(engine);
        let active = if segmented {
            files(&path.with_extension("segments"))
                .into_keys()
                .max()
                .unwrap()
        } else {
            path.clone()
        };
        Self {
            directory,
            path,
            active,
            manifest,
            uploader,
        }
    }
    fn inspect(&self) -> Result<Option<EngineReplay>> {
        WalEngine::inspect_checkpoint_tail(
            &self.path,
            |m| {
                assert_eq!(m, &self.manifest);
                Ok(())
            },
            |_, at| {
                assert!(at.unwrap().index > 2);
                Ok(())
            },
        )
    }
}

#[test]
fn inspection_matches_recovery_without_repairing_torn_active_tail() {
    for (segmented, target) in [(false, 4096), (true, 4096), (true, 128)] {
        let f = Fixture::new(segmented, target);
        std::fs::OpenOptions::new()
            .append(true)
            .open(&f.active)
            .unwrap()
            .write_all(b"KV9")
            .unwrap();
        let before = files(f.directory.path());
        let mut positions = Vec::new();
        let inspected = WalEngine::inspect_checkpoint_tail(
            &f.path,
            |_| Ok(()),
            |_, at| {
                positions.push(at.unwrap().index);
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(positions, (3..=9).collect::<Vec<_>>());
        assert_eq!(inspected.discarded_tail_bytes, 3);
        assert_eq!(files(f.directory.path()), before);
        let (engine, recovered) = WalEngine::open_with_base_observer(
            &f.path,
            Some(&f.uploader),
            |_| Ok(()),
            |_, _| Ok(()),
            |_, _| Ok(()),
        )
        .unwrap();
        assert_eq!(inspected, recovered);
        assert_eq!(
            engine.get(ColumnFamily::Default, b"key").unwrap(),
            Some(9u64.to_be_bytes().to_vec())
        );
        assert_eq!(
            std::fs::metadata(&f.active).unwrap().len() as usize,
            before[&f.active].len() - 3
        );
    }
}

#[test]
fn corrupt_complete_records_and_visitor_refusal_preserve_every_file() {
    for segmented in [false, true] {
        let f = Fixture::new(segmented, 128);
        let before = files(f.directory.path());
        let error = WalEngine::inspect_checkpoint_tail(
            &f.path,
            |_| Ok(()),
            |_, _| Err(kv9_common::Error::Engine("visitor refusal".into())),
        )
        .unwrap_err();
        assert!(error.to_string().contains("visitor refusal"));
        assert_eq!(files(f.directory.path()), before);
        let mut bytes = std::fs::read(&f.active).unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        std::fs::write(&f.active, &bytes).unwrap();
        let corrupt = files(f.directory.path());
        assert!(f.inspect().unwrap_err().to_string().contains("checksum"));
        assert_eq!(files(f.directory.path()), corrupt);
    }
}

#[test]
fn closed_segment_damage_and_changed_selection_are_refused() {
    let f = Fixture::new(true, 128);
    let closed = files(&f.path.with_extension("segments"))
        .into_keys()
        .min()
        .unwrap();
    assert_ne!(closed, f.active);
    let mut bytes = std::fs::read(&closed).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    std::fs::write(&closed, bytes).unwrap();
    let before = files(f.directory.path());
    assert!(f.inspect().is_err());
    assert_eq!(files(f.directory.path()), before);

    for segmented in [false, true] {
        let f = Fixture::new(segmented, 4096);
        let selector = if segmented {
            f.path.clone()
        } else {
            f.path.with_extension("checkpoint")
        };
        let error = WalEngine::inspect_checkpoint_tail(
            &f.path,
            |_| {
                std::fs::remove_file(&selector).unwrap();
                Ok(())
            },
            |_, _| Ok(()),
        )
        .unwrap_err();
        assert!(error.to_string().contains("changed") || error.to_string().contains("I/O"));
    }
}

#[test]
fn no_selected_checkpoint_does_not_create_or_scan_a_wal() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("data.wal");
    let inspect = || {
        WalEngine::inspect_checkpoint_tail(
            &path,
            |_| panic!("no checkpoint"),
            |_, _| panic!("no tail"),
        )
    };
    assert_eq!(inspect().unwrap(), None);
    assert!(files(directory.path()).is_empty());
    std::fs::write(&path, b"incomplete").unwrap();
    let before = files(directory.path());
    assert_eq!(inspect().unwrap(), None);
    assert_eq!(files(directory.path()), before);
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(path.with_extension("segments")).unwrap();
    assert!(inspect()
        .unwrap_err()
        .to_string()
        .contains("without its WAL topology"));
}
