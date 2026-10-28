use super::*;
use crate::checkpoint_memory::ImageBudget;
use crate::{Engine, MemoryObjectStore, ReplicatedEngine, WalEngine, WriteBatch};
use std::path::PathBuf;

#[test]
fn concurrent_transfer_spools_keep_distinct_images_and_leave_checkpoint_scratch_alone() {
    let f = Fixture::new(3, 1024, 1024 * 1024);
    let scratch = f.directory.join("data.checkpoint-image.tmp");
    std::fs::write(&scratch, b"exclusive checkpoint worker scratch").unwrap();
    let mut snapshots = Vec::new();
    for n in 0..8u8 {
        let mut batch = WriteBatch::new();
        batch.put(ColumnFamily::Default, b"unique".to_vec(), vec![n; 4096]);
        f.engine
            .write_applied(
                batch,
                AppliedPosition {
                    term: 2,
                    index: 10 + u64::from(n),
                },
            )
            .unwrap();
        snapshots.push((n, f.freeze()));
    }
    let barrier = std::sync::Barrier::new(8);
    std::thread::scope(|scope| {
        for (n, frozen) in snapshots {
            let barrier = &barrier;
            let f = &f;
            scope.spawn(move || {
                let plan = f.uploader.plan_spooled_in(frozen, &f.directory).unwrap();
                barrier.wait();
                assert_eq!(f.uploader.spool_disk_snapshot().active_spools, 8);
                barrier.wait();
                let manifest = plan.manifest().clone();
                let prepared = f.uploader.upload_spooled(plan).unwrap();
                assert_eq!(prepared.into_manifest(), manifest);
                assert_eq!(
                    f.uploader
                        .restore(&manifest)
                        .unwrap()
                        .get(ColumnFamily::Default, b"unique")
                        .unwrap(),
                    Some(vec![n; 4096])
                );
            });
        }
    });
    assert_eq!(
        std::fs::read(scratch).unwrap(),
        b"exclusive checkpoint worker scratch"
    );
    assert_eq!(f.uploader.spool_disk_snapshot().reserved_bytes, 0);
    assert_eq!(f.memory.snapshot().retained_bytes, 0);
    assert_eq!(std::fs::read_dir(&f.directory).unwrap().count(), 2);
}

#[derive(Debug)]
struct RefuseTransferPut {
    disk: Arc<crate::checkpoint_spool_budget::SpoolBudget>,
}
impl ObjectStore for RefuseTransferPut {
    fn put(&self, _: &ObjectKey, _: &[u8]) -> Result<()> {
        assert_eq!(self.disk.snapshot().active_spools, 1);
        assert!(self.disk.snapshot().reserved_bytes > 0);
        Err(Error::Engine("injected transfer PUT failure".into()))
    }
    fn get(&self, _: &ObjectKey) -> Result<Option<Vec<u8>>> {
        panic!("failed PUT")
    }
    fn delete(&self, _: &ObjectKey) -> Result<()> {
        panic!("no deletion")
    }
    fn list(&self, _: &str) -> Result<Vec<ObjectKey>> {
        panic!("no listing")
    }
}

#[test]
fn transfer_spool_failure_returns_admission_only_after_file_and_upload_finish() {
    let f = Fixture::new(3, 1024, 8192);
    let disk = Arc::new(crate::checkpoint_spool_budget::SpoolBudget::new(1024 * 1024).unwrap());
    let uploader = RemoteUploader::for_harness(Arc::new(RefuseTransferPut { disk: disk.clone() }))
        .with_spool_budget(disk.clone())
        .with_image_budget(f.memory.clone());
    let plan = uploader.plan_spooled_in(f.freeze(), &f.directory).unwrap();
    assert!(uploader
        .upload_spooled(plan)
        .unwrap_err()
        .to_string()
        .contains("injected transfer PUT"));
    assert_eq!(disk.snapshot().reserved_bytes, 0);
    assert_eq!(f.memory.snapshot().retained_bytes, 0);
    let unavailable = f.directory.join("missing");
    assert!(uploader.plan_spooled_in(f.freeze(), &unavailable).is_err());
    assert_eq!(disk.snapshot().active_spools, 0);
}

fn scope() -> FlushScope {
    FlushScope {
        cluster: "spool-test".into(),
        region: 103,
        conf_ver: 1,
        version: 1,
    }
}

#[test]
fn large_image_round_trips_v2_while_legacy_decoder_and_resident_load_refuse() {
    let f = Fixture::new(60, 1024 * 1024, (SST_TARGET + 1024) as u64);
    assert!(RemoteUploader::plan(f.freeze()).is_err());
    let mut journal = f.journal();
    let mut spool = f.uploader.plan_spooled(f.freeze(), &mut journal).unwrap();
    let manifest = spool.manifest().clone();
    assert!(manifest.requires_large_format().unwrap());
    let encoded = manifest.encode().unwrap();
    assert!(encoded.starts_with(LARGE_MAGIC));
    assert_eq!(CheckpointManifest::decode(&encoded).unwrap(), manifest);
    assert!(CheckpointManifest::decode_legacy(&encoded).is_err());
    let mut forged_legacy = encoded.clone();
    forged_legacy[MAGIC.len() - 1] = 1;
    assert!(CheckpointManifest::decode(&forged_legacy).is_err());
    journal.stage_spooled(&mut spool, 3).unwrap();
    let bytes = manifest.files.iter().map(|f| f.size).sum::<u64>();
    assert_eq!(f.uploader.spool_disk_snapshot().reserved_bytes, bytes);
    drop(spool);
    assert_eq!(f.uploader.spool_disk_snapshot().reserved_bytes, 0);
    assert!(FlushJournal::new(f.directory.join("pending"))
        .load()
        .is_err());
    let pending = journal.load().unwrap().unwrap();
    assert_eq!(pending.manifest(), &manifest);
    let (prepared, generation) = pending.recover(&f.uploader).unwrap();
    journal.stage(&prepared, generation).unwrap();
    assert!(!journal.load().unwrap().unwrap().requires_upload());
    let restored = f.uploader.restore(&manifest).unwrap();
    for i in 0..60 {
        assert_eq!(
            restored.get(ColumnFamily::Default, &[b'k', i]).unwrap(),
            Some(vec![i; 1024 * 1024])
        );
        assert_eq!(
            restored.get(ColumnFamily::Write, &[b'k', i]).unwrap(),
            Some(vec![i; 64])
        );
    }
    assert_eq!(f.memory.snapshot().retained_bytes, 0);
    assert!(f.memory.snapshot().peak_bytes <= (SST_TARGET + 1024) as u64);
    eprintln!(
        "large_checkpoint image_bytes={bytes} peak_buffer_bytes={} spool_reserved_after=0",
        f.memory.snapshot().peak_bytes
    );
}

#[test]
fn live_spools_share_disk_admission_across_scoped_uploaders_and_release_on_drop() {
    use crate::checkpoint_spool_budget::SpoolBudget;
    let f = Fixture::new(3, 1024, 8192);
    let bytes = RemoteUploader::plan(f.freeze())
        .unwrap()
        .manifest
        .files
        .iter()
        .map(|r| r.size)
        .sum();
    let budget = Arc::new(SpoolBudget::new(bytes).unwrap());
    let first = RemoteUploader::for_harness(f.objects.clone()).with_spool_budget(budget.clone());
    let second = RemoteUploader::for_harness(f.objects.clone()).with_spool_budget(budget.clone());
    let mut a = first.journal(f.directory.join("a"));
    let mut b = second.journal(f.directory.join("b"));
    let held = first.plan_spooled(f.freeze(), &mut a).unwrap();
    assert_eq!(budget.snapshot().active_spools, 1);
    second.with_local_execution(|scoped| {
        assert!(scoped.plan_spooled(f.freeze(), &mut b).is_err());
    });
    assert_eq!(budget.snapshot().reserved_bytes, bytes);
    assert!(!f.directory.join("b.checkpoint-image.tmp").exists());
    drop(held);
    assert_eq!(budget.snapshot().reserved_bytes, 0);
    drop(second.plan_spooled(f.freeze(), &mut b).unwrap());
    assert_eq!(budget.snapshot().active_spools, 0);
    assert_eq!(budget.snapshot().peak_bytes, bytes);
    assert!(SpoolBudget::new(0).is_err());
}

#[test]
fn failed_spool_open_or_buffer_admission_returns_disk_reservation() {
    let f = Fixture::new(3, 1024, 8192);
    let occupied = f.directory.join("occupied");
    std::fs::write(&occupied, b"not a directory").unwrap();
    let mut journal = f.uploader.journal(occupied.join("pending"));
    assert!(f.uploader.plan_spooled(f.freeze(), &mut journal).is_err());
    assert_eq!(f.uploader.spool_disk_snapshot().reserved_bytes, 0);
    assert_eq!(f.uploader.spool_disk_snapshot().active_spools, 0);
    let uploader = RemoteUploader::for_harness(f.objects.clone())
        .with_image_budget(Arc::new(ImageBudget::new(1).unwrap()));
    let mut journal = uploader.journal(f.directory.join("pending"));
    assert!(uploader.plan_spooled(f.freeze(), &mut journal).is_err());
    assert_eq!(uploader.spool_disk_snapshot().reserved_bytes, 0);
    assert_eq!(uploader.spool_disk_snapshot().active_spools, 0);
    assert_eq!(uploader.image_memory_snapshot().retained_bytes, 0);
    assert!(!f.directory.join("pending.checkpoint-image.tmp").exists());
}

#[test]
fn resident_journal_refuses_large_manifest_even_inside_legacy_file_length_envelope() {
    let f = Fixture::new(3, 1024, 8192);
    let mut manifest = RemoteUploader::plan(f.freeze()).unwrap().manifest;
    assert_eq!(manifest.files.len(), 2);
    manifest.files[0].size = MAX_CHECKPOINT_BYTES as u64 / 2;
    manifest.files[1].size = MAX_CHECKPOINT_BYTES as u64 / 2 + 1;
    let manifest = manifest.encode().unwrap();
    let mut header = b"KV9PENDING\x02".to_vec();
    header.extend(0u64.to_le_bytes());
    header.extend((manifest.len() as u32).to_le_bytes());
    header.extend(manifest);
    let path = f.directory.join("sparse-large-pending");
    std::fs::write(&path, &header).unwrap();
    // A sparse invalid payload exercises refusal before any SST allocation;
    // the overall file still fits the old image-plus-manifest length cap.
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(header.len() as u64 + MAX_CHECKPOINT_BYTES as u64 + 1 + 32)
        .unwrap();
    let error = FlushJournal::new(path).load().unwrap_err();
    assert!(
        error.to_string().contains("requires streaming load"),
        "{error}"
    );
}

struct Fixture {
    directory: PathBuf,
    engine: WalEngine,
    objects: Arc<MemoryObjectStore>,
    memory: Arc<ImageBudget>,
    uploader: RemoteUploader,
}
impl Fixture {
    fn new(rows: u8, value_bytes: usize, limit: u64) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "kv9-spool-{}",
            kv9_common::StoreIncarnation::mint().unwrap()
        ));
        std::fs::create_dir(&directory).unwrap();
        let engine = WalEngine::open(directory.join("data.wal")).unwrap().0;
        let mut batch = WriteBatch::new();
        for i in 0..rows {
            batch.put(ColumnFamily::Default, vec![b'k', i], vec![i; value_bytes]);
            batch.put(ColumnFamily::Write, vec![b'k', i], vec![i; 64]);
        }
        engine
            .write_applied(batch, AppliedPosition { term: 2, index: 9 })
            .unwrap();
        let objects = Arc::new(MemoryObjectStore::new());
        let memory = Arc::new(ImageBudget::new(limit).unwrap());
        let uploader =
            RemoteUploader::for_harness(objects.clone()).with_image_budget(memory.clone());
        Self {
            directory,
            engine,
            objects,
            memory,
            uploader,
        }
    }
    fn freeze(&self) -> FrozenFlush {
        self.engine.freeze(scope()).unwrap()
    }
    fn journal(&self) -> FlushJournal {
        self.uploader.journal(self.directory.join("pending"))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}

fn file_digest(path: &std::path::Path) -> Vec<u8> {
    let mut input = File::open(path).unwrap();
    let mut hash = Sha256::new();
    let mut buf = [0; 8192];
    loop {
        let n = input.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    hash.finalize().to_vec()
}

#[test]
fn spooled_multi_sst_plan_stage_reopen_upload_and_restore_use_one_object_budget() {
    let f = Fixture::new(12, 1024 * 1024, (SST_TARGET + 1024) as u64);
    assert!(f.uploader.plan_checkpoint(f.freeze()).is_err());
    let mut journal = f.journal();
    let mut spool = f.uploader.plan_spooled(f.freeze(), &mut journal).unwrap();
    let manifest = spool.manifest().clone();
    let total = manifest.files.iter().map(|s| s.size).sum::<u64>();
    let largest = manifest.files.iter().map(|s| s.size).max().unwrap();
    assert!(total > f.memory.snapshot().limit_bytes);
    assert_eq!(f.memory.snapshot().retained_bytes, 0);
    assert_eq!(f.memory.snapshot().peak_bytes, largest);
    assert!(f.objects.is_empty());
    assert!(!f.directory.join("pending").exists());
    assert!(!f
        .directory
        .join("pending")
        .with_extension("checkpoint-image.tmp")
        .exists());
    journal.stage_spooled(&mut spool, 7).unwrap();
    journal.stage_spooled(&mut spool, 7).unwrap();
    assert!(journal.stage_spooled(&mut spool, 8).is_err());
    // The on-disk V2 format and manifest identity are byte-for-byte identical
    // to the resident planner. This legacy allocation is test comparison only.
    let resident = RemoteUploader::plan(f.freeze()).unwrap();
    assert_eq!(resident.manifest(), &manifest);
    let legacy_path = f.directory.join("legacy-pending");
    FlushJournal::new(&legacy_path)
        .stage_planned(&resident, 7)
        .unwrap();
    assert_eq!(
        file_digest(&legacy_path),
        file_digest(&f.directory.join("pending"))
    );
    drop((resident, spool, journal));
    let mut journal = f.journal();
    let pending = journal.load().unwrap().unwrap();
    assert!(pending.requires_upload());
    assert_eq!(pending.manifest(), &manifest);
    assert_eq!(f.memory.snapshot().retained_bytes, 0);
    let (prepared, generation) = pending.recover(&f.uploader).unwrap();
    assert_eq!(generation, 7);
    journal.stage(&prepared, generation).unwrap();
    let prepared_pending = journal.load().unwrap().unwrap();
    assert!(!prepared_pending.requires_upload());
    assert_eq!(prepared_pending.manifest(), &manifest);
    let restored = f.uploader.restore(&manifest).unwrap();
    for i in 0..12 {
        assert_eq!(
            restored.get(ColumnFamily::Default, &[b'k', i]).unwrap(),
            Some(vec![i; 1024 * 1024])
        );
        assert_eq!(
            restored.get(ColumnFamily::Write, &[b'k', i]).unwrap(),
            Some(vec![i; 64])
        );
    }
    assert_eq!(f.memory.snapshot().retained_bytes, 0);
    assert_eq!(f.memory.snapshot().active_images, 0);
    assert_eq!(f.memory.snapshot().peak_bytes, largest);
    eprintln!(
        "spooled_checkpoint total_bytes={total} largest_sst={largest} peak_reserved_bytes={}",
        f.memory.snapshot().peak_bytes
    );
}

#[test]
fn loaded_pending_keeps_the_verified_inode_across_path_replacement() {
    let f = Fixture::new(3, 512, 4096);
    let mut journal = f.journal();
    let mut spool = f.uploader.plan_spooled(f.freeze(), &mut journal).unwrap();
    journal.stage_spooled(&mut spool, 2).unwrap();
    let pending = journal.load().unwrap().unwrap();
    let original = pending.manifest().clone();
    let path = f.directory.join("pending");
    std::fs::rename(&path, f.directory.join("old-inode")).unwrap();
    std::fs::write(&path, b"replacement is not the loaded journal").unwrap();
    let (prepared, generation) = pending.recover(&f.uploader).unwrap();
    assert_eq!(prepared.into_manifest(), original);
    assert_eq!(generation, 2);
    assert_eq!(f.memory.snapshot().retained_bytes, 0);
    assert!(journal.load().is_err());
}

#[test]
fn streamed_load_checks_whole_checksum_and_upload_rechecks_in_place_changes() {
    let f = Fixture::new(3, 512, 4096);
    let mut journal = f.journal();
    let mut spool = f.uploader.plan_spooled(f.freeze(), &mut journal).unwrap();
    journal.stage_spooled(&mut spool, 2).unwrap();
    let path = f.directory.join("pending");
    let original = std::fs::read(&path).unwrap();
    let mut corrupt = original.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    std::fs::write(&path, &corrupt).unwrap();
    assert!(journal.load().is_err());
    assert!(f.objects.is_empty());
    assert_eq!(f.memory.snapshot().retained_bytes, 0);
    std::fs::write(&path, &original).unwrap();
    let pending = journal.load().unwrap().unwrap();
    let start =
        (b"KV9PENDING\x02".len() + 8 + 4 + pending.manifest().encode().unwrap().len()) as u64;
    let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.seek(SeekFrom::Start(start)).unwrap();
    file.write_all(b"bad!").unwrap();
    file.sync_all().unwrap();
    assert!(pending.recover(&f.uploader).is_err());
    assert!(f.objects.is_empty());
    assert_eq!(f.memory.snapshot().retained_bytes, 0);
}

#[test]
fn spool_stage_failure_cancels_only_an_unpublished_candidate_and_can_retry() {
    let f = Fixture::new(3, 512, 4096);
    let mut journal = f.journal();
    let (frozen, mut candidate) = f
        .engine
        .freeze_checkpoint_candidate(|_| Ok(scope()))
        .unwrap();
    let mut spool = f.uploader.plan_spooled(frozen, &mut journal).unwrap();
    let tmp = f.directory.join("pending").with_extension("pending.tmp");
    std::fs::create_dir(&tmp).unwrap();
    assert!(candidate
        .stage_spooled(&mut journal, &mut spool, 0)
        .is_err());
    assert!(!candidate.is_durable());
    assert!(journal.load().unwrap().is_none());
    assert!(f
        .engine
        .checkpoint_history()
        .unwrap()
        .compaction_ceiling()
        .unwrap()
        .is_none());
    assert!(f.objects.is_empty());
    assert_eq!(f.memory.snapshot().retained_bytes, 0);
    drop((candidate, spool));
    std::fs::remove_dir(&tmp).unwrap();
    let (frozen, mut candidate) = f
        .engine
        .freeze_checkpoint_candidate(|_| Ok(scope()))
        .unwrap();
    let mut spool = f.uploader.plan_spooled(frozen, &mut journal).unwrap();
    candidate
        .stage_spooled(&mut journal, &mut spool, 0)
        .unwrap();
    assert!(candidate.is_durable());
    let cut = candidate.position();
    drop((candidate, spool));
    assert_eq!(
        f.engine
            .checkpoint_history()
            .unwrap()
            .compaction_ceiling()
            .unwrap(),
        Some(cut)
    );
    assert!(journal.load().unwrap().unwrap().requires_upload());
}
