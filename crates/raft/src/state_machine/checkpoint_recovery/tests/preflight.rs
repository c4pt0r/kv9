use super::*;
use kv9_engine::checkpoint::{FlushScope, RemoteUploader};
use kv9_engine::{MemoryObjectStore, ObjectKey, ObjectStore};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug, Default)]
struct Objects {
    inner: MemoryObjectStore,
    gets: AtomicUsize,
}
impl ObjectStore for Objects {
    fn put(&self, key: &ObjectKey, bytes: &[u8]) -> Result<()> {
        self.inner.put(key, bytes)
    }
    fn get(&self, key: &ObjectKey) -> Result<Option<Vec<u8>>> {
        self.gets.fetch_add(1, Ordering::SeqCst);
        self.inner.get(key)
    }
    fn delete(&self, key: &ObjectKey) -> Result<()> {
        self.inner.delete(key)
    }
    fn list(&self, prefix: &str) -> Result<Vec<ObjectKey>> {
        self.inner.list(prefix)
    }
}

fn files(path: &std::path::Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(self::files(&path));
        } else {
            files.insert(path.clone(), std::fs::read(path).unwrap());
        }
    }
    files
}

struct Fixture {
    path: PathBuf,
    storage: DiskRaftStorage,
    objects: Arc<Objects>,
    uploader: RemoteUploader,
    winner: CheckpointManifest,
    loser: CheckpointManifest,
    commands: Vec<Command>,
}
impl Fixture {
    fn new(segmented: bool) -> Self {
        let path = directory("preflight");
        let engine = Arc::new(WalEngine::open(path.join("catalog.wal")).unwrap().0);
        if segmented {
            engine.enable_segmentation().unwrap();
        }
        let mut sm = MemStateMachine::with_engine(engine.clone()).unwrap();
        sm.set_fence_adjudicator(Arc::new(Fresh));
        let objects = Arc::new(Objects::default());
        let uploader = RemoteUploader::for_harness(objects.clone());
        let first = Command::Put {
            cf: 0,
            key: b"base".to_vec(),
            value: b"first".to_vec(),
        };
        sm.apply_at(at(1), &first).unwrap();
        let scope = FlushScope {
            cluster: "test".into(),
            region: 9,
            conf_ver: 1,
            version: 1,
        };
        let winner = uploader
            .upload(engine.freeze(scope.clone()).unwrap())
            .unwrap()
            .into_manifest();
        let second = Command::Put {
            cf: 0,
            key: b"base".to_vec(),
            value: b"later".to_vec(),
        };
        sm.apply_at(at(2), &second).unwrap();
        let loser = uploader
            .upload(engine.freeze(scope).unwrap())
            .unwrap()
            .into_manifest();
        let win = command(&winner, 0);
        let lose = command(&loser, 0);
        assert!(matches!(
            sm.apply_at(at(3), &win).unwrap().outcome,
            ApplyOutcome::Manifest(ManifestVerdict::Applied { generation: 1, .. })
        ));
        assert!(!matches!(
            sm.apply_at(at(4), &lose).unwrap().outcome,
            ApplyOutcome::Manifest(ManifestVerdict::Applied { .. })
        ));
        let commands = vec![first, second, win, lose];
        let storage = raft_log(&path, &commands);
        engine.checkpoint_applied(&winner).unwrap();
        drop(sm);
        drop(engine);
        Self {
            path,
            storage,
            objects,
            uploader,
            winner,
            loser,
            commands,
        }
    }
    fn open(
        &mut self,
    ) -> Result<(
        WalEngine,
        kv9_engine::EngineReplay,
        Option<RecoveredCheckpointPublication>,
    )> {
        open_checkpoint_engine(
            &mut self.storage,
            self.path.join("catalog.wal"),
            Some(&self.uploader),
            "test".into(),
            9,
        )
    }
    fn refuse_without_get_or_repair(&mut self) -> String {
        self.objects.gets.store(0, Ordering::SeqCst);
        let before = files(&self.path);
        let error = self.open().unwrap_err().to_string();
        assert_eq!(self.objects.gets.load(Ordering::SeqCst), 0, "{error}");
        assert_eq!(files(&self.path), before);
        error
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).unwrap();
    }
}

#[test]
fn exact_winner_is_inspected_before_get_and_full_recovery_still_validates() {
    for segmented in [false, true] {
        let mut f = Fixture::new(segmented);
        f.objects.gets.store(0, Ordering::SeqCst);
        let before = files(&f.path);
        let plan =
            inspect_checkpoint_read(&mut f.storage, f.path.join("catalog.wal"), "test".into(), 9)
                .unwrap();
        assert_eq!(plan.manifest(), Some(&f.winner));
        assert_eq!(plan.publication(), Some(at(3)));
        assert_eq!(plan.predecessor(), Some(0));
        assert_eq!(f.objects.gets.load(Ordering::SeqCst), 0);
        assert_eq!(files(&f.path), before);
        assert!(plan.check_selection(Some(&f.loser)).is_err());
        let (engine, _, recovered) = f.open().unwrap();
        assert_eq!(recovered.unwrap().publication(), at(3));
        assert_eq!(
            engine.get(ColumnFamily::Default, b"base").unwrap(),
            Some(b"later".to_vec())
        );
        assert!(f.objects.gets.load(Ordering::SeqCst) > 0);
    }
}

#[test]
fn committed_loser_has_zero_object_reads_in_both_layouts() {
    for segmented in [false, true] {
        let mut f = Fixture::new(segmented);
        assert!(f
            .storage
            .has_committed_checkpoint(&f.loser.encode().unwrap())
            .unwrap());
        let engine = f.open().unwrap().0;
        engine.checkpoint_applied(&f.loser).unwrap();
        drop(engine);
        assert!(f
            .refuse_without_get_or_repair()
            .contains("lacks a retained successful publication"));
    }
}

#[test]
fn forged_or_uncommitted_winning_wal_batch_cannot_start_object_reads() {
    for segmented in [false, true] {
        for uncommitted in [false, true] {
            let mut f = Fixture::new(segmented);
            let alternate = directory("alternate-protocol");
            let commands = if uncommitted {
                &f.commands[..2]
            } else {
                f.commands[2] = Command::Noop;
                &f.commands
            };
            f.storage = raft_log(&alternate, commands);
            let error = f.refuse_without_get_or_repair();
            if !uncommitted {
                assert!(
                    error.contains("does not name a manifest command"),
                    "{error}"
                );
            }
            std::fs::remove_dir_all(alternate).unwrap();
        }
    }
}

#[test]
fn erased_or_regressed_publication_is_refused_before_get() {
    for segmented in [false, true] {
        for damage in 0..3 {
            let mut f = Fixture::new(segmented);
            let engine = f.open().unwrap().0;
            let mut batch = WriteBatch::new();
            match damage {
                0 => batch.delete(ColumnFamily::Default, manifest_gen_key(9, 1)),
                1 => batch.delete(ColumnFamily::Default, manifest_pair_key(9)),
                _ => batch.put(
                    ColumnFamily::Default,
                    manifest_pair_key(9),
                    ManifestPair {
                        generation: 0,
                        last_change_id: Vec::new(),
                    }
                    .encode(),
                ),
            };
            engine.write_applied(batch, at(5)).unwrap();
            drop(engine);
            let error = f.refuse_without_get_or_repair();
            assert!(
                error.contains("manifest") || error.contains("publication"),
                "{error}"
            );
        }
    }
}

#[test]
fn read_plan_uses_first_winner_even_when_same_image_is_republished() {
    for next_generation in [0, 1] {
        let path = directory("preflight-republication");
        let selected = manifest();
        let cmds = commands(command(&selected, 0), command(&selected, next_generation));
        let mut storage = raft_log(&path, &cmds);
        apply_log(&path, &cmds);
        let engine = WalEngine::open(path.join("catalog.wal")).unwrap().0;
        engine.checkpoint_applied(&selected).unwrap();
        drop(engine);
        let plan =
            inspect_checkpoint_read(&mut storage, path.join("catalog.wal"), "test".into(), 9)
                .unwrap();
        assert_eq!(plan.publication(), Some(at(2)));
        assert_eq!(plan.predecessor(), Some(0));
        std::fs::remove_dir_all(path).unwrap();
    }
}
