use super::*;
use std::sync::{mpsc, Arc};
use std::time::Duration;

struct Fixture {
    path: PathBuf,
    uploader: RemoteUploader,
    engine: Arc<WalEngine>,
}
impl Fixture {
    fn new(segmented: bool) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "kv9-selection-{}",
            kv9_common::StoreIncarnation::mint().unwrap()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("data.wal");
        let engine = Arc::new(WalEngine::open(&path).unwrap().0);
        if segmented {
            engine.enable_segmentation_with_target(256).unwrap();
        }
        Self {
            path,
            uploader: RemoteUploader::for_harness(Arc::new(crate::MemoryObjectStore::new())),
            engine,
        }
    }
    fn write(&self, term: u64, index: u64) {
        let mut batch = WriteBatch::new();
        batch.put(
            ColumnFamily::Default,
            vec![index as u8],
            vec![index as u8; 128],
        );
        self.engine
            .write_applied(batch, AppliedPosition { term, index })
            .unwrap();
    }
    fn manifest(&self) -> CheckpointManifest {
        self.uploader
            .upload(
                self.engine
                    .freeze(FlushScope {
                        cluster: "selection-test".into(),
                        region: 100,
                        conf_ver: 2,
                        version: 3,
                    })
                    .unwrap(),
            )
            .unwrap()
            .into_manifest()
    }
    fn reopen(&self) -> WalEngine {
        WalEngine::open_with_uploader(&self.path, Some(&self.uploader))
            .unwrap()
            .0
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(self.path.parent().unwrap()).unwrap();
    }
}

#[test]
fn selection_rejects_conflicting_successors_before_either_layout_changes() {
    for segmented in [false, true] {
        for case in 0..10 {
            let f = Fixture::new(segmented);
            assert_eq!(f.engine.selected_checkpoint().unwrap(), None);
            f.write(1, 1);
            f.write(2, 2);
            let old = f.manifest();
            assert!(f.engine.checkpoint_applied(&old).unwrap());
            f.write(3, 3);
            let mut bad = f.manifest();
            let expected = match case {
                0 => {
                    bad = old.clone();
                    bad.scope.version += 1;
                    "conflicting checkpoint"
                }
                1 => {
                    bad.term = 2;
                    "conflicting term"
                }
                2 | 3 => {
                    if case == 2 {
                        bad.scope.cluster = "foreign".into();
                        for file in &mut bad.files {
                            file.key = file.key.replace("selection-test", "foreign");
                        }
                    } else {
                        bad.scope.region = 101;
                        for file in &mut bad.files {
                            file.key = file.key.replace("regions/100/", "regions/101/");
                        }
                    }
                    "scope changed"
                }
                4 | 5 => {
                    if case == 4 {
                        bad.scope.conf_ver = 1;
                    } else {
                        bad.scope.version = 2;
                    }
                    "epoch regresses"
                }
                6 => {
                    f.write(4, 4);
                    bad.term = 1;
                    "term regresses"
                }
                7 => {
                    bad.index = 4;
                    "ahead of applied"
                }
                8 => {
                    bad.files.clear();
                    "invalid empty checkpoint"
                }
                9 => {
                    bad = old.clone();
                    bad.files[0].count += 1;
                    "conflicting checkpoint"
                }
                _ => unreachable!(),
            };
            let before = std::fs::read(&f.path).unwrap();
            let error = f.engine.checkpoint_applied(&bad).unwrap_err().to_string();
            assert!(
                error.contains(expected),
                "layout={segmented} case={case}: {error}"
            );
            assert_eq!(std::fs::read(&f.path).unwrap(), before);
            assert_eq!(
                WalEngine::checkpoint_reference(&f.path).unwrap(),
                Some(old.clone())
            );
            assert!(
                f.engine.selected_checkpoint().is_err(),
                "failed selection requires recovery"
            );
            let recovered = f.reopen();
            assert_eq!(recovered.selected_checkpoint().unwrap(), Some(old));
            assert_eq!(
                recovered.get(ColumnFamily::Default, &[3]).unwrap(),
                Some(vec![3; 128])
            );
        }
    }
}

#[test]
fn legacy_selection_checks_retained_terms_before_rewriting_its_replay_source() {
    let f = Fixture::new(false);
    f.write(3, 1);
    let mut bad = f.manifest();
    f.write(5, 3);
    bad.index = 2;
    bad.term = 2;
    let before = std::fs::read(&f.path).unwrap();
    assert!(f
        .engine
        .checkpoint_applied(&bad)
        .unwrap_err()
        .to_string()
        .contains("disagrees"));
    assert_eq!(std::fs::read(&f.path).unwrap(), before);
    assert_eq!(WalEngine::checkpoint_reference(&f.path).unwrap(), None);
    let reopened = f.reopen();
    assert_eq!(
        reopened.get(ColumnFamily::Default, &[1]).unwrap(),
        Some(vec![1; 128])
    );
    assert_eq!(
        reopened.get(ColumnFamily::Default, &[3]).unwrap(),
        Some(vec![3; 128])
    );
}

#[test]
fn selection_retries_and_epoch_advances_survive_reopen_in_both_layouts() {
    for segmented in [false, true] {
        let f = Fixture::new(segmented);
        f.write(1, 1);
        let old = f.manifest();
        assert!(f.engine.checkpoint_applied(&old).unwrap());
        assert!(f.engine.checkpoint_applied(&old).unwrap());
        f.write(2, 2);
        let mut next = f.manifest();
        next.scope.conf_ver += 1;
        next.scope.version += 1;
        assert!(f.engine.checkpoint_applied(&next).unwrap());
        assert!(!f.engine.checkpoint_applied(&old).unwrap());
        assert_eq!(f.engine.selected_checkpoint().unwrap(), Some(next.clone()));
        f.write(2, 3);
        let recovered = f.reopen();
        assert_eq!(recovered.selected_checkpoint().unwrap(), Some(next));
        assert_eq!(
            recovered.get(ColumnFamily::Default, &[3]).unwrap(),
            Some(vec![3; 128])
        );
    }
}

#[test]
fn visible_reference_after_failed_checkpoint_is_not_a_live_selection_observation() {
    for segmented in [false, true] {
        let f = Fixture::new(segmented);
        f.write(1, 1);
        f.write(2, 2);
        let manifest = f.manifest();
        f.write(2, 3);
        let obstruction = if segmented {
            let log = f.engine.wal.lock().unwrap();
            let Some(WalBacking::Segmented(stream)) = &log.backing else {
                unreachable!()
            };
            let first = stream.closed_segments().first().unwrap().header();
            let directory = std::fs::read_dir(f.path.with_extension("segments"))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            let path = directory.join(format!("{:020}.wal", first.sequence));
            // Only a segment wholly covered by the actual uploaded image is
            // replaced. The publication succeeds; post-publication unlink fails.
            std::fs::remove_file(&path).unwrap();
            path
        } else {
            // The durable reference is published before legacy tail rewriting.
            f.path.with_extension("tail.tmp")
        };
        std::fs::create_dir(&obstruction).unwrap();
        assert!(f.engine.checkpoint_applied(&manifest).is_err());
        assert_eq!(
            WalEngine::checkpoint_reference(&f.path).unwrap(),
            Some(manifest.clone())
        );
        assert!(f
            .engine
            .selected_checkpoint()
            .unwrap_err()
            .to_string()
            .contains("requires recovery"));
        std::fs::remove_dir(obstruction).unwrap();
        let recovered = f.reopen();
        assert_eq!(recovered.selected_checkpoint().unwrap(), Some(manifest));
        assert_eq!(
            recovered.get(ColumnFamily::Default, &[3]).unwrap(),
            Some(vec![3; 128])
        );
    }
}

#[test]
fn selection_observation_waits_for_compaction_and_rejects_its_ambiguous_failure() {
    let f = Fixture::new(true);
    f.write(1, 1);
    let manifest = f.manifest();
    f.engine.checkpoint_applied(&manifest).unwrap();
    let history = f.engine.history.clone();
    let (entered, on_enter) = mpsc::channel();
    let (release, on_release) = mpsc::channel();
    let compaction = std::thread::spawn(move || {
        history.with_compaction(manifest.position(), || {
            entered.send(()).unwrap();
            on_release.recv_timeout(Duration::from_secs(5)).unwrap();
            Err::<(), _>(kv9_common::Error::Engine(
                "injected compaction failure".into(),
            ))
        })
    });
    on_enter.recv_timeout(Duration::from_secs(5)).unwrap();
    let engine = f.engine.clone();
    let (send, receive) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        send.send(engine.selected_checkpoint()).unwrap();
    });
    let early = receive.recv_timeout(Duration::from_millis(50));
    release.send(()).unwrap();
    assert!(compaction.join().unwrap().is_err());
    reader.join().unwrap();
    assert!(matches!(early, Err(mpsc::RecvTimeoutError::Timeout)));
    assert!(receive
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .is_err());
}

#[test]
fn failed_layout_or_segment_append_fences_online_selection_even_without_a_new_checkpoint() {
    for segmented in [false, true] {
        let f = Fixture::new(segmented);
        f.write(1, 1);
        let manifest = f.manifest();
        f.engine.checkpoint_applied(&manifest).unwrap();
        let obstruction = f.path.with_extension(if segmented {
            "topology.tmp"
        } else {
            "migration"
        });
        std::fs::create_dir(&obstruction).unwrap();
        if segmented {
            let mut batch = WriteBatch::new();
            batch.put(ColumnFamily::Default, b"failed".to_vec(), vec![0; 512]);
            assert!(f
                .engine
                .write_applied(batch, AppliedPosition { term: 1, index: 2 })
                .is_err());
        } else {
            assert!(f.engine.enable_segmentation().is_err());
        }
        assert_eq!(
            WalEngine::checkpoint_reference(&f.path).unwrap(),
            Some(manifest.clone())
        );
        assert!(f.engine.selected_checkpoint().is_err());
        assert!(f.engine.applied_position().is_err());
        std::fs::remove_dir(obstruction).unwrap();
        assert_eq!(f.reopen().selected_checkpoint().unwrap(), Some(manifest));
    }
}
