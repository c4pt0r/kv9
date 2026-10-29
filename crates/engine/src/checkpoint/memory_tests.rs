use super::*;

fn frozen(rows: &[(ColumnFamily, &[u8], &[u8])]) -> FrozenFlush {
    let engine = VersionedMemEngine::new();
    let mut batch = WriteBatch::new();
    for (cf, key, value) in rows {
        batch.put(*cf, key.to_vec(), value.to_vec());
    }
    engine
        .write_applied(batch, AppliedPosition { term: 2, index: 3 })
        .unwrap();
    let (view, position, data_revision) = engine.freeze_parts();
    FrozenFlush {
        view,
        position: position.unwrap(),
        data_revision,
        scope: FlushScope {
            cluster: "bounded-plan".into(),
            region: 1,
            conf_ver: 1,
            version: 1,
        },
    }
}

#[test]
fn planner_counts_sst_headers_and_footer_keys_in_the_shared_image_limit() {
    let rows = ColumnFamily::ALL.map(|cf| (cf, b"a".as_slice(), b"b".as_slice()));
    // Each one-row SST is 22 + (8 + 1 + 1) + (1 + 1) = 34 bytes.
    let plan = plan_with_limits(frozen(&rows), 100, 102).unwrap();
    assert_eq!(
        plan.objects.iter().map(Vec::len).collect::<Vec<_>>(),
        [34; 3]
    );
    assert!(plan_with_limits(frozen(&rows), 100, 101).is_err());
    let long_key = [7; 128];
    let rows = [(ColumnFamily::Default, long_key.as_slice(), b"v".as_slice())];
    assert!(plan_with_limits(frozen(&rows), 100, 414).is_err());
    assert_eq!(
        plan_with_limits(frozen(&rows), 100, 415).unwrap().objects[0].len(),
        415
    );
}

#[test]
fn bounded_planner_keeps_existing_sst_partition_and_exact_bytes() {
    let rows = [
        (
            ColumnFamily::Default,
            b"a".as_slice(),
            b"1111111111".as_slice(),
        ),
        (
            ColumnFamily::Default,
            b"b".as_slice(),
            b"2222222222".as_slice(),
        ),
        (
            ColumnFamily::Write,
            b"a".as_slice(),
            b"3333333333".as_slice(),
        ),
    ];
    let plan = plan_with_limits(frozen(&rows), 19, 129).unwrap();
    assert_eq!(plan.objects.len(), 3);
    for (object, (cf, key, value)) in plan.objects.iter().zip(rows) {
        let mut original_writer = SstWriter::new(cf);
        original_writer.add(key.to_vec(), value.to_vec()).unwrap();
        assert_eq!(*object, original_writer.finish().unwrap());
    }
    assert!(plan_with_limits(frozen(&rows), 19, 128).is_err());
}

#[test]
fn borrowed_entry_limit_rejection_preserves_the_writer_and_can_resume() {
    let mut writer = SstWriter::new(ColumnFamily::Default);
    writer.add_bounded(b"a", b"b", 34).unwrap();
    let before = writer.len();
    // The limit includes the enlarged footer; rejection happens before cloning
    // this value or changing the writer's entries/size accounting.
    assert!(writer.add_bounded(b"b", &[0; 1024], 100).is_err());
    assert_eq!(writer.len(), before);
    writer.add_bounded(b"c", b"d", 44).unwrap();
    assert!(writer.add_bounded(b"b", b"d", 100).is_err());
    let bytes = writer.finish().unwrap();
    assert_eq!(bytes.len(), 44);
    let sst = Sst::parse(&bytes).unwrap();
    assert_eq!(
        sst.iter().collect::<Vec<_>>(),
        [
            (b"a".as_slice(), b"b".as_slice()),
            (b"c".as_slice(), b"d".as_slice())
        ]
    );
}

#[test]
fn shared_image_budget_counts_exact_plans_across_uploaders_and_refuses_without_leaks() {
    use crate::checkpoint_memory::ImageBudget;
    let rows = [(ColumnFamily::Default, b"a".as_slice(), b"b".as_slice())];
    let budget = Arc::new(ImageBudget::new(68).unwrap());
    let a = RemoteUploader::for_harness(Arc::new(crate::MemoryObjectStore::new()))
        .with_image_budget(budget.clone());
    let b = RemoteUploader::for_harness(Arc::new(crate::MemoryObjectStore::new()))
        .with_image_budget(budget.clone());
    let first = a.plan_checkpoint(frozen(&rows)).unwrap();
    let second = b.plan_checkpoint(frozen(&rows)).unwrap();
    assert_eq!(budget.snapshot().retained_bytes, 68);
    assert_eq!(budget.snapshot().active_images, 2);
    assert!(a.plan_checkpoint(frozen(&rows)).is_err());
    assert_eq!(budget.snapshot().retained_bytes, 68);
    drop(first);
    assert_eq!(budget.snapshot().retained_bytes, 34);
    let third = a.plan_checkpoint(frozen(&rows)).unwrap();
    drop((second, third));
    assert_eq!(budget.snapshot().retained_bytes, 0);
    assert_eq!(budget.snapshot().active_images, 0);
    assert_eq!(budget.snapshot().peak_bytes, 68);
    assert_eq!(budget.snapshot().refused_images, 1);
    assert!(ImageBudget::new(0).is_err());
}

#[test]
fn borrowed_preflight_matches_real_partition_across_large_rows_and_column_families() {
    let value = vec![7; SST_TARGET];
    let rows = [
        (ColumnFamily::Default, b"a".as_slice(), value.as_slice()),
        (ColumnFamily::Default, b"b".as_slice(), b"small".as_slice()),
        (ColumnFamily::Write, b"x".as_slice(), b"expiry".as_slice()),
    ];
    let source = frozen(&rows);
    let measured = image_bytes(&source).unwrap();
    let uploader = RemoteUploader::for_harness(Arc::new(crate::MemoryObjectStore::new()));
    let planned = uploader.plan_checkpoint(source).unwrap();
    assert_eq!(planned.objects.len(), 3);
    assert_eq!(
        measured,
        planned.objects.iter().map(Vec::len).sum::<usize>()
    );
    assert_eq!(
        uploader.image_memory_snapshot().retained_bytes,
        measured as u64
    );
    drop(planned);
    assert_eq!(uploader.image_memory_snapshot().retained_bytes, 0);
}

#[test]
fn pending_pressure_keeps_disk_identity_and_resumes_after_releasing_the_plan() {
    use crate::checkpoint_memory::ImageBudget;
    let path =
        std::env::temp_dir().join(format!("kv9-image-budget-pending-{}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    let rows = [(ColumnFamily::Default, b"a".as_slice(), b"b".as_slice())];
    let budget = Arc::new(ImageBudget::new(34).unwrap());
    let uploader = RemoteUploader::for_harness(Arc::new(crate::MemoryObjectStore::new()))
        .with_image_budget(budget.clone());
    let plan = uploader.plan_checkpoint(frozen(&rows)).unwrap();
    let original_manifest = plan.manifest().clone();
    let mut journal = uploader.journal(path.join("pending"));
    journal.stage_planned(&plan, 7).unwrap();
    let original = std::fs::read(path.join("pending")).unwrap();
    assert!(journal.load().is_err());
    assert_eq!(std::fs::read(path.join("pending")).unwrap(), original);
    assert_eq!(budget.snapshot().retained_bytes, 34);
    drop((plan, journal));
    let mut reopened = uploader.journal(path.join("pending"));
    let pending = reopened.load().unwrap().unwrap();
    assert_eq!(pending.manifest(), &original_manifest);
    assert_eq!(pending.expected_generation(), 7);
    // The production journal pins the validated inode, then admits one SST
    // during recovery; an idle pending descriptor retains no serialized image.
    assert_eq!(budget.snapshot().retained_bytes, 0);
    let (prepared, generation) = pending.recover(&uploader).unwrap();
    assert_eq!(budget.snapshot().retained_bytes, 0);
    reopened.stage(&prepared, generation).unwrap();
    let prepared_pending = reopened.load().unwrap().unwrap();
    assert!(!prepared_pending.requires_upload());
    assert_eq!(budget.snapshot().active_images, 0);
    assert_eq!(prepared_pending.manifest(), &original_manifest);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn corrupt_pending_and_ambiguous_upload_release_images_without_changing_the_journal() {
    use crate::checkpoint_memory::ImageBudget;
    use crate::{MemoryObjectStore, ObjectKey};
    #[derive(Debug)]
    struct Unknown(MemoryObjectStore);
    impl ObjectStore for Unknown {
        fn put(&self, key: &ObjectKey, value: &[u8]) -> Result<()> {
            self.0.put(key, value)?;
            Err(Error::Engine("injected lost PUT receipt".into()))
        }
        fn get(&self, key: &ObjectKey) -> Result<Option<Vec<u8>>> {
            self.0.get(key)
        }
        fn delete(&self, key: &ObjectKey) -> Result<()> {
            self.0.delete(key)
        }
        fn list(&self, prefix: &str) -> Result<Vec<ObjectKey>> {
            self.0.list(prefix)
        }
    }
    let path =
        std::env::temp_dir().join(format!("kv9-image-budget-failure-{}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    let budget = Arc::new(ImageBudget::new(34).unwrap());
    let object = Arc::new(Unknown(MemoryObjectStore::new()));
    let uploader = RemoteUploader::for_harness(object.clone()).with_image_budget(budget.clone());
    let rows = [(ColumnFamily::Default, b"a".as_slice(), b"b".as_slice())];
    let plan = uploader.plan_checkpoint(frozen(&rows)).unwrap();
    let mut journal = uploader.journal(path.join("pending"));
    journal.stage_planned(&plan, 7).unwrap();
    drop(plan);
    let original = std::fs::read(path.join("pending")).unwrap();
    let mut corrupt = original.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    std::fs::write(path.join("pending"), corrupt).unwrap();
    assert!(journal.load().is_err());
    assert_eq!(budget.snapshot().retained_bytes, 0);
    std::fs::write(path.join("pending"), &original).unwrap();
    assert!(journal.load().unwrap().unwrap().recover(&uploader).is_err());
    assert!(!object.list("").unwrap().is_empty());
    assert_eq!(budget.snapshot().retained_bytes, 0);
    assert_eq!(budget.snapshot().active_images, 0);
    assert_eq!(std::fs::read(path.join("pending")).unwrap(), original);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn compatibility_plan_upload_admits_before_its_first_object_operation() {
    let rows = [(ColumnFamily::Default, b"a".as_slice(), b"b".as_slice())];
    let budget = Arc::new(crate::checkpoint_memory::ImageBudget::new(34).unwrap());
    let objects = Arc::new(crate::MemoryObjectStore::new());
    let uploader = RemoteUploader::for_harness(objects.clone()).with_image_budget(budget.clone());
    let held = uploader.plan_checkpoint(frozen(&rows)).unwrap();
    let legacy = RemoteUploader::plan(frozen(&rows)).unwrap();
    assert!(uploader.upload_planned(legacy).is_err());
    assert!(objects.is_empty());
    assert_eq!(budget.snapshot().retained_bytes, 34);
    drop(held);
    uploader
        .upload_planned(RemoteUploader::plan(frozen(&rows)).unwrap())
        .unwrap();
    assert!(!objects.is_empty());
    assert_eq!(budget.snapshot().retained_bytes, 0);
}

#[test]
fn checkpoint_upload_recovery_and_restore_keep_the_shared_read_interface() {
    use crate::{MemoryObjectStore, ObjectKey};
    #[derive(Debug)]
    struct SharedOnly {
        objects: MemoryObjectStore,
        memory: Arc<crate::object_memory::BufferBudget>,
    }
    impl ObjectStore for SharedOnly {
        fn put(&self, key: &ObjectKey, value: &[u8]) -> Result<()> {
            self.objects.put(key, value)
        }
        fn get(&self, _: &ObjectKey) -> Result<Option<Vec<u8>>> {
            panic!("checkpoint must not discard the shared buffer owner")
        }
        fn get_shared_bounded(
            &self,
            key: &ObjectKey,
            limit: usize,
        ) -> Result<Option<bytes::Bytes>> {
            self.objects
                .get_bounded(key, limit)?
                .map(|value| self.memory.copy(&value))
                .transpose()
        }
        fn delete(&self, key: &ObjectKey) -> Result<()> {
            self.objects.delete(key)
        }
        fn list(&self, prefix: &str) -> Result<Vec<ObjectKey>> {
            self.objects.list(prefix)
        }
    }
    let memory = Arc::new(crate::object_memory::BufferBudget::new(34).unwrap());
    let store = Arc::new(SharedOnly {
        objects: MemoryObjectStore::new(),
        memory: memory.clone(),
    });
    let uploader = RemoteUploader::for_harness(store);
    let rows = ColumnFamily::ALL.map(|cf| (cf, b"a".as_slice(), b"b".as_slice()));
    let manifest = uploader.upload(frozen(&rows)).unwrap().into_manifest();
    let recovered = uploader.recover_prepared(&manifest).unwrap();
    assert_eq!(recovered.into_manifest(), manifest);
    let restored = uploader.restore(&manifest).unwrap();
    for cf in ColumnFamily::ALL {
        assert_eq!(
            crate::Engine::get(&restored, cf, b"a").unwrap(),
            Some(b"b".to_vec())
        );
    }
    assert_eq!(memory.snapshot().retained_bytes, 0);
    assert_eq!(memory.snapshot().active_buffers, 0);
    assert_eq!(memory.snapshot().admitted_buffers, 9);
    assert_eq!(memory.snapshot().peak_bytes, 34);
}

#[test]
fn direct_restore_preserves_one_batch_revision_and_the_exact_applied_position() {
    let uploader = RemoteUploader::for_harness(Arc::new(crate::MemoryObjectStore::new()));
    for (cf, key, revision) in [
        (ColumnFamily::Default, b"\0kv9\0manifest_pair".as_slice(), 0),
        (
            ColumnFamily::Default,
            b"\0kv9\0retention_v1\0owner".as_slice(),
            0,
        ),
        (
            ColumnFamily::Default,
            kv9_common::data_range::COMPACTION_CONFIRMED_KEY,
            0,
        ),
        (ColumnFamily::Write, b"\0kv9\0manifest_pair".as_slice(), 1),
        (ColumnFamily::Default, b"user".as_slice(), 1),
    ] {
        let rows = [(cf, key, b"raw\0\xff".as_slice())];
        let manifest = uploader.upload(frozen(&rows)).unwrap().into_manifest();
        let restored = uploader.restore(&manifest).unwrap();
        assert_eq!(
            restored.volatile_applied_position(),
            Some(manifest.position())
        );
        assert_eq!(restored.data_revision(), revision);
        assert_eq!(
            crate::Engine::get(&restored, cf, key).unwrap(),
            Some(b"raw\0\xff".to_vec())
        );
        let mut next = WriteBatch::new();
        next.put(ColumnFamily::Default, b"next".to_vec(), b"value".to_vec());
        restored
            .write_applied(next, AppliedPosition { term: 2, index: 4 })
            .unwrap();
        assert_eq!(restored.data_revision(), revision + 1);
    }
}

#[test]
fn later_sst_failure_discards_unpublished_restore_and_releases_remote_buffer() {
    use crate::{MemoryObjectStore, ObjectKey};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    #[derive(Debug)]
    struct CorruptSecond {
        objects: MemoryObjectStore,
        memory: Arc<crate::object_memory::BufferBudget>,
        corrupt: AtomicBool,
        reads: AtomicUsize,
    }
    impl ObjectStore for CorruptSecond {
        fn put(&self, key: &ObjectKey, value: &[u8]) -> Result<()> {
            self.objects.put(key, value)
        }
        fn get(&self, _: &ObjectKey) -> Result<Option<Vec<u8>>> {
            panic!("shared GET required")
        }
        fn get_shared_bounded(
            &self,
            key: &ObjectKey,
            limit: usize,
        ) -> Result<Option<bytes::Bytes>> {
            let n = self.reads.fetch_add(1, Ordering::SeqCst);
            self.objects
                .get_bounded(key, limit)?
                .map(|mut value| {
                    if n == 1 && self.corrupt.load(Ordering::SeqCst) {
                        value[0] ^= 1;
                    }
                    self.memory.copy(&value)
                })
                .transpose()
        }
        fn delete(&self, key: &ObjectKey) -> Result<()> {
            self.objects.delete(key)
        }
        fn list(&self, prefix: &str) -> Result<Vec<ObjectKey>> {
            self.objects.list(prefix)
        }
    }
    let memory = Arc::new(crate::object_memory::BufferBudget::new(34).unwrap());
    let store = Arc::new(CorruptSecond {
        objects: MemoryObjectStore::new(),
        memory: memory.clone(),
        corrupt: AtomicBool::new(false),
        reads: AtomicUsize::new(0),
    });
    let uploader = RemoteUploader::for_harness(store.clone());
    let rows = ColumnFamily::ALL.map(|cf| (cf, b"a".as_slice(), b"b".as_slice()));
    let manifest = uploader.upload(frozen(&rows)).unwrap().into_manifest();
    store.corrupt.store(true, Ordering::SeqCst);
    store.reads.store(0, Ordering::SeqCst);
    assert!(uploader.restore(&manifest).is_err());
    assert_eq!(store.reads.load(Ordering::SeqCst), 2);
    assert_eq!(memory.snapshot().retained_bytes, 0);
    store.corrupt.store(false, Ordering::SeqCst);
    store.reads.store(0, Ordering::SeqCst);
    let complete = uploader.restore(&manifest).unwrap();
    assert_eq!(
        complete.volatile_applied_position(),
        Some(manifest.position())
    );
    assert_eq!(complete.data_revision(), 1);
    for cf in ColumnFamily::ALL {
        assert_eq!(
            crate::Engine::get(&complete, cf, b"a").unwrap(),
            Some(b"b".to_vec())
        );
    }
    assert_eq!(memory.snapshot().retained_bytes, 0);
}

#[test]
fn oversized_manifest_releases_image_admission_before_any_object_upload() {
    let memory =
        Arc::new(crate::checkpoint_memory::ImageBudget::new(MAX_MANIFEST_BYTES as u64).unwrap());
    let store = Arc::new(crate::MemoryObjectStore::new());
    let uploader = RemoteUploader::for_harness(store.clone()).with_image_budget(memory.clone());
    // One SST fits the image budget, but its two footer keys expand into JSON
    // arrays that exceed the separate manifest bound.
    let key = vec![255; MAX_MANIFEST_BYTES / 8 + 1];
    let rows = [(ColumnFamily::Default, key.as_slice(), b"v".as_slice())];
    let error = uploader.upload(frozen(&rows)).unwrap_err();
    assert!(error.to_string().contains("checkpoint manifest too large"));
    assert!(store.is_empty());
    assert_eq!(memory.snapshot().admitted_images, 1);
    assert_eq!(memory.snapshot().retained_bytes, 0);
    assert_eq!(memory.snapshot().active_images, 0);
    let small = [(ColumnFamily::Default, b"k".as_slice(), b"v".as_slice())];
    let complete = uploader.upload(frozen(&small)).unwrap().into_manifest();
    assert_eq!(complete.files.len(), 1);
    assert!(!store.is_empty());
    assert_eq!(memory.snapshot().retained_bytes, 0);
}

#[test]
fn borrowed_planner_matches_legacy_partition_and_limit_acceptance_matrix() {
    // Keep the former owned-entry planner as an independent partition oracle.
    // Both codecs additionally face a fixed v1 frame in sst::encode tests.
    fn legacy(source: &FrozenFlush, target: usize, limit: usize) -> Result<Vec<Vec<u8>>> {
        let mut objects = Vec::new();
        let mut total = 0;
        for cf in ColumnFamily::ALL {
            let mut writer = SstWriter::new(cf);
            let mut size = 0;
            for (key, value) in source.view.iter_all(cf) {
                let row_size = 8 + key.len() + value.len();
                size += row_size;
                if size > target && !writer.is_empty() {
                    let bytes = writer.finish()?;
                    total += bytes.len();
                    objects.push(bytes);
                    writer = SstWriter::new(cf);
                    size = row_size;
                }
                writer.add_bounded(&key, &value, limit - total)?;
            }
            if !writer.is_empty() {
                let bytes = writer.finish()?;
                total += bytes.len();
                objects.push(bytes);
            }
        }
        if objects.is_empty() {
            return Err(Error::Engine("empty checkpoint".into()));
        }
        Ok(objects)
    }
    for case in 0..4 {
        let mut owned = Vec::new();
        for (cf_id, cf) in ColumnFamily::ALL.into_iter().enumerate() {
            if case == 0 || (case == 1 && cf_id != 0) || (case == 2 && cf_id == 1) {
                continue;
            }
            for i in 0..16 {
                let mut key = if i == 0 { Vec::new() } else { vec![i as u8] };
                if i != 0 {
                    key.extend(vec![255; (i * 17 + case * 31) % 70]);
                }
                owned.push((cf, key, vec![cf_id as u8; (i * 11 + case) % 33]));
            }
        }
        let rows: Vec<_> = owned
            .iter()
            .map(|(cf, k, v)| (*cf, k.as_slice(), v.as_slice()))
            .collect();
        for target in [0, 9, 32, 100, 1024, usize::MAX] {
            for limit in [0, 34, 128, 512, 1024, 4096, 16384] {
                let expected = legacy(&frozen(&rows), target, limit);
                let actual = plan_with_limits(frozen(&rows), target, limit);
                match (actual, expected) {
                    (Ok(plan), Ok(objects)) => {
                        assert_eq!(
                            plan.objects, objects,
                            "case {case}, target {target}, limit {limit}"
                        );
                        for (file, bytes) in plan.manifest.files.iter().zip(&plan.objects) {
                            assert_eq!(bytes.capacity(), bytes.len());
                            assert_eq!(file.sha256, digest(bytes));
                            assert_eq!(file.key, object_name(&plan.manifest.scope, &file.sha256));
                        }
                    }
                    (Err(_), Err(_)) => {}
                    _ => panic!("acceptance differs: case {case}, target {target}, limit {limit}"),
                }
            }
        }
    }
}

#[test]
fn shrinking_footer_reserves_only_final_image_but_preserves_prefix_limit_policy() {
    let long = vec![b'a'; 256];
    let rows = [
        (ColumnFamily::Default, long.as_slice(), b"v".as_slice()),
        (ColumnFamily::Default, b"b", b"v"),
    ];
    let exact = image_bytes(&frozen(&rows)).unwrap();
    let prefix = SstWriter::table_bytes(8 + 256 + 1, 256, 256).unwrap();
    assert!(exact < prefix);
    assert!(plan_with_limits(frozen(&rows), SST_TARGET, prefix - 1).is_err());
    let memory = Arc::new(crate::checkpoint_memory::ImageBudget::new(exact as u64).unwrap());
    let uploader = RemoteUploader::for_harness(Arc::new(crate::MemoryObjectStore::new()))
        .with_image_budget(memory.clone());
    let plan = uploader.plan_checkpoint(frozen(&rows)).unwrap();
    assert_eq!(plan.objects.len(), 1);
    assert_eq!(plan.objects[0].len(), exact);
    assert_eq!(plan.objects[0].capacity(), exact);
    assert_eq!(memory.snapshot().retained_bytes, exact as u64);
    drop(plan);
    assert_eq!(memory.snapshot().retained_bytes, 0);
}

#[test]
fn later_table_encoding_failure_discards_partial_plan_and_returns_image_lease() {
    let rows = ColumnFamily::ALL.map(|cf| (cf, b"a".as_slice(), b"b".as_slice()));
    let source = frozen(&rows);
    let mut layout = image_layout(&source, SST_TARGET, MAX_CHECKPOINT_BYTES).unwrap();
    let memory = Arc::new(crate::checkpoint_memory::ImageBudget::new(layout.total as u64).unwrap());
    let reservation = memory.reserve(layout.total as u64).unwrap();
    // Exercise a defensive failure after the first SST buffer was encoded.
    // Immutable snapshots cannot actually change between layout and encoding.
    layout.tables[1][0].bytes -= 1;
    assert!(encode_plan(source, layout, Some(reservation)).is_err());
    assert_eq!(memory.snapshot().retained_bytes, 0);
    assert_eq!(memory.snapshot().active_images, 0);
}
