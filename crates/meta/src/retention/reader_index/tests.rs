use super::*;
use crate::retention::checkpoint_intent::tests::fixture_intent;
use crate::retention::tests::{all, apply, owner, plan};
use kv9_engine::{Engine, MemEngine, WalEngine};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Count<'a> {
    view: &'a dyn ReadView,
    rows: AtomicUsize,
    gets: AtomicUsize,
}
impl ReadView for Count<'_> {
    fn get(&self, cf: ColumnFamily, k: &[u8]) -> Result<Option<Vec<u8>>> {
        if k.starts_with(PREFIX) {
            self.gets.fetch_add(1, Ordering::Relaxed);
        }
        self.view.get(cf, k)
    }
    fn scan(
        &self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<kv9_engine::ScanEntry>> {
        assert!(
            !start.starts_with(PREFIX),
            "census must consume a bounded iterator"
        );
        self.view.scan(cf, start, end, limit)
    }
    fn seek_le(&self, cf: ColumnFamily, k: &[u8]) -> Result<Option<kv9_engine::ScanEntry>> {
        assert!(!k.starts_with(PREFIX));
        self.view.seek_le(cf, k)
    }
    fn iter<'a>(
        &'a self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
    ) -> Result<Box<dyn Iterator<Item = Result<kv9_engine::ScanEntry>> + 'a>> {
        let retention = start.starts_with(PREFIX);
        Ok(Box::new(self.view.iter(cf, start, end)?.inspect(
            move |_| {
                if retention {
                    self.rows.fetch_add(1, Ordering::Relaxed);
                }
            },
        )))
    }
    fn iter_rev<'a>(
        &'a self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
    ) -> Result<Box<dyn Iterator<Item = Result<kv9_engine::ScanEntry>> + 'a>> {
        assert!(!start.starts_with(PREFIX));
        self.view.iter_rev(cf, start, end)
    }
}

#[test]
fn census_and_attempt_queries_consume_bounded_rows_independent_of_catalog_size() {
    let mut measurements = Vec::new();
    for size in [13, 113] {
        let (engine, root, intent, _) = fixture(size);
        apply(&engine, &root, LedgerRequest::Initialize);
        let view = engine.snapshot().unwrap();
        let count = Count {
            view: view.as_ref(),
            rows: AtomicUsize::new(0),
            gets: AtomicUsize::new(0),
        };
        let Work::Page {
            progress,
            next,
            examined,
        } = observe(&count, &root).unwrap()
        else {
            panic!()
        };
        assert_eq!(examined, PAGE);
        assert_eq!(count.rows.swap(0, Ordering::Relaxed), PAGE + 1);
        plan_retention(&count, &root, &progress.request(next)).unwrap();
        assert_eq!(count.rows.load(Ordering::Relaxed), PAGE + 1);
        drop(view);
        finish(&engine, &root);
        let view = engine.snapshot().unwrap();
        let count = Count {
            view: view.as_ref(),
            rows: AtomicUsize::new(0),
            gets: AtomicUsize::new(0),
        };
        let page = readers(
            &count,
            &root,
            intent
                .binding(OwnerKind::Pending)
                .unwrap()
                .descriptor
                .operation,
            None,
        )
        .unwrap();
        assert_eq!(page.owners.len(), PAGE);
        assert!(page.next.is_some());
        assert_eq!(count.rows.load(Ordering::Relaxed), PAGE + 1);
        measurements.push(count.gets.load(Ordering::Relaxed));
    }
    assert_eq!(
        measurements[0], measurements[1],
        "query work grew with catalog history"
    );
}

fn fixture(
    count: u8,
) -> (
    MemEngine,
    RootDescriptor,
    CheckpointIntent,
    Vec<OwnerBinding>,
) {
    let (engine, root, intent) = fixture_intent();
    let pending = intent.binding(OwnerKind::Pending).unwrap();
    apply(&engine, &root, LedgerRequest::Acquire(pending.clone()));
    apply(&engine, &root, LedgerRequest::Publish(pending.token()));
    let mut owners = Vec::new();
    for n in 1..=count {
        let binding = checkpoint_reader::binding(
            &pending,
            LocalLifetime {
                node: 1,
                store: [1; 16],
                process: [n; 16],
            },
        )
        .unwrap();
        apply(
            &engine,
            &root,
            LedgerRequest::Share {
                from: pending.token(),
                to: binding.clone(),
            },
        );
        owners.push(binding);
    }
    // Explicit legacy V8 fixture: per-process Reader owners predate this
    // reverse index or generation records. Encode explicit legacy owner bytes;
    // this is a fixture conversion, not a production downgrade operation.
    let mut batch = WriteBatch::new();
    for (k, _) in all(&engine) {
        if (k >= key(38, &[]) && k < key(42, &[]))
            || k.starts_with(&key(23, &[]))
            || k.starts_with(&key(24, &[]))
            || k.starts_with(&key(25, &[]))
        {
            batch.delete(ColumnFamily::Default, k);
        }
    }
    for binding in &owners {
        batch.put(
            ColumnFamily::Default,
            owner_key(binding.descriptor.id),
            encode_owner(&LedgerOwner {
                binding: binding.clone(),
                phase: PinPhase::Held,
            }),
        );
        batch.put(
            ColumnFamily::Default,
            owner_registration_key(binding.descriptor.id),
            owner_registration(binding),
        );
    }
    let mut old = engine
        .get(ColumnFamily::Default, &key(1, &[]))
        .unwrap()
        .unwrap();
    old.truncate(old.len() - 32);
    old[..8].copy_from_slice(MIGRATION_HEADER_MAGIC);
    batch.put(ColumnFamily::Default, key(1, &[]), seal(old));
    engine.write(batch).unwrap();
    owners.sort_by_key(|o| o.descriptor.id);
    (engine, root, intent, owners)
}
fn work(engine: &impl Engine, root: &RootDescriptor) -> Work {
    observe(engine.snapshot().unwrap().as_ref(), root).unwrap()
}
fn request(engine: &impl Engine, root: &RootDescriptor) -> LedgerRequest {
    match work(engine, root) {
        Work::Upgrade => LedgerRequest::Initialize,
        Work::Page {
            progress,
            next,
            examined,
        } => {
            assert!(examined <= PAGE);
            progress.request(next)
        }
        _ => panic!("expected pending census work"),
    }
}
fn finish(engine: &MemEngine, root: &RootDescriptor) {
    for _ in 0..100 {
        if matches!(work(engine, root), Work::Complete) {
            return;
        }
        apply(engine, root, request(engine, root));
    }
    panic!("census did not finish");
}
fn collected(engine: &impl Engine, root: &RootDescriptor, operation: [u8; 32]) -> Vec<LedgerOwner> {
    let view = engine.snapshot().unwrap();
    let mut owners = Vec::new();
    let mut after = None;
    loop {
        let page = readers(view.as_ref(), root, operation, after).unwrap();
        assert!(page.owners.len() <= PAGE);
        owners.extend(page.owners);
        match page.next {
            Some(next) => after = Some(next),
            None => break,
        }
    }
    owners
}

#[test]
fn census_includes_legacy_owners_and_concurrent_insertions_behind_cursor_without_promoting_them() {
    let (engine, root, intent, mut old) = fixture(13);
    assert!(matches!(work(&engine, &root), Work::Upgrade));
    assert!(readers(
        engine.snapshot().unwrap().as_ref(),
        &root,
        intent
            .binding(OwnerKind::Pending)
            .unwrap()
            .descriptor
            .operation,
        None
    )
    .is_err());
    apply(
        &engine,
        &root,
        LedgerRequest::RecordCheckpoint(intent.clone()),
    );
    let originals: Vec<_> = old
        .iter()
        .map(|b| {
            (
                b.descriptor.id,
                engine
                    .get(ColumnFamily::Default, &owner_key(b.descriptor.id))
                    .unwrap()
                    .unwrap(),
            )
        })
        .collect();
    apply(&engine, &root, request(&engine, &root));
    let after = match work(&engine, &root) {
        Work::Page { progress, .. } => progress.after.unwrap(),
        _ => panic!(),
    };
    let fresh = (14..=255)
        .map(|n| {
            checkpoint_reader::binding(
                &intent.binding(OwnerKind::Pending).unwrap(),
                LocalLifetime {
                    node: 1,
                    store: [1; 16],
                    process: [n; 16],
                },
            )
            .unwrap()
        })
        .find(|b| b.descriptor.id < after)
        .unwrap();
    apply(
        &engine,
        &root,
        LedgerRequest::Share {
            from: intent.binding(OwnerKind::Pending).unwrap().token(),
            to: fresh.clone(),
        },
    );
    finish(&engine, &root);
    old.push(fresh);
    old.sort_by_key(|b| b.descriptor.id);
    assert_eq!(
        collected(
            &engine,
            &root,
            intent
                .binding(OwnerKind::Pending)
                .unwrap()
                .descriptor
                .operation
        )
        .into_iter()
        .map(|o| o.binding)
        .collect::<Vec<_>>(),
        old
    );
    for (id, bytes) in originals {
        assert!(bytes.starts_with(OWNER_MAGIC));
        assert_eq!(
            engine
                .get(ColumnFamily::Default, &owner_key(id))
                .unwrap()
                .unwrap(),
            bytes
        );
    }
    assert!(collected(
        &engine,
        &root,
        intent
            .binding(OwnerKind::Pending)
            .unwrap()
            .descriptor
            .operation
    )
    .iter()
    .all(|owner| owner.phase == PinPhase::Held));
    assert_eq!(
        owner(&engine, &root, intent.pending_id()).phase,
        PinPhase::Published
    );
}

#[test]
fn census_progress_requires_exact_page_and_immediate_retry_is_idempotent() {
    let (engine, root, _, _) = fixture(9);
    apply(&engine, &root, LedgerRequest::Initialize);
    let first = request(&engine, &root);
    let mut wrong = first.clone();
    if let LedgerRequest::AdvanceReaderIndex { next, .. } = &mut wrong {
        *next = None;
    }
    let before = all(&engine);
    assert!(plan(&engine, &root, &wrong).is_err());
    assert_eq!(all(&engine), before);
    let bytes = encode_request(root.digest(), &first).unwrap();
    assert_eq!(decode_request(&bytes, root.digest()).unwrap(), first);
    for n in 0..bytes.len() {
        assert!(decode_request(&bytes[..n], root.digest()).is_err());
    }
    let revision = apply(&engine, &root, first.clone());
    assert!(!plan(&engine, &root, &first).unwrap().changed());
    assert_eq!(apply(&engine, &root, first.clone()), revision);
    apply(&engine, &root, request(&engine, &root));
    assert!(plan(&engine, &root, &first).is_err());
    let mut last = first;
    while !matches!(work(&engine, &root), Work::Complete) {
        last = request(&engine, &root);
        apply(&engine, &root, last.clone());
    }
    assert!(!plan(&engine, &root, &last).unwrap().changed());
}

#[test]
fn immutable_registration_pass_detects_missing_state_and_resumes_after_repair() {
    let (engine, root, intent, owners) = fixture(6);
    let id = owners[0].descriptor.id;
    let saved = engine
        .get(ColumnFamily::Default, &owner_key(id))
        .unwrap()
        .unwrap();
    let mut batch = WriteBatch::new();
    batch.delete(ColumnFamily::Default, owner_key(id));
    engine.write(batch).unwrap();
    apply(&engine, &root, LedgerRequest::Initialize);
    while matches!(
        work(&engine, &root),
        Work::Page {
            progress: Progress { stage: 0, .. },
            ..
        }
    ) {
        apply(&engine, &root, request(&engine, &root));
    }
    let mut caught = false;
    for _ in 0..10 {
        let command = request(&engine, &root);
        let before = all(&engine);
        match plan(&engine, &root, &command) {
            Ok(_) => {
                apply(&engine, &root, command);
            }
            Err(_) => {
                assert_eq!(all(&engine), before);
                caught = true;
                break;
            }
        }
    }
    assert!(caught);
    let mut batch = WriteBatch::new();
    batch.put(ColumnFamily::Default, owner_key(id), saved);
    engine.write(batch).unwrap();
    finish(&engine, &root);
    assert_eq!(
        collected(
            &engine,
            &root,
            intent
                .binding(OwnerKind::Pending)
                .unwrap()
                .descriptor
                .operation
        )
        .len(),
        owners.len()
    );
}

#[test]
fn complete_index_keeps_legacy_classification_and_rejects_missing_links_or_header_downgrade() {
    let (engine, root, intent, owners) = fixture(2);
    finish(&engine, &root);
    let header_bytes = engine
        .get(ColumnFamily::Default, &key(1, &[]))
        .unwrap()
        .unwrap();
    assert!(
        checked(&header_bytes, MIGRATION_HEADER_MAGIC, 81).is_err(),
        "V8 planner must reject the new format"
    );
    let mut old = header_bytes[..header_bytes.len() - 32].to_vec();
    old[..8].copy_from_slice(MIGRATION_HEADER_MAGIC);
    let mut batch = WriteBatch::new();
    batch.put(ColumnFamily::Default, key(1, &[]), seal(old));
    engine.write(batch).unwrap();
    assert!(observe(engine.snapshot().unwrap().as_ref(), &root).is_err());
    assert!(plan(&engine, &root, &LedgerRequest::Initialize).is_err());
    let mut batch = WriteBatch::new();
    batch.put(ColumnFamily::Default, key(1, &[]), header_bytes);
    engine.write(batch).unwrap();
    let binding = &owners[0];
    let k = index_key(intent.pending_id(), binding.descriptor.id);
    let saved = engine.get(ColumnFamily::Default, &k).unwrap().unwrap();
    let mut batch = WriteBatch::new();
    batch.delete(ColumnFamily::Default, k.clone());
    engine.write(batch).unwrap();
    assert!(retention_owner(
        engine.snapshot().unwrap().as_ref(),
        &root,
        binding.descriptor.id
    )
    .is_err());
    assert!(plan(&engine, &root, &LedgerRequest::Acquire(binding.clone())).is_err());
    let mut batch = WriteBatch::new();
    batch.put(ColumnFamily::Default, k, saved.clone());
    let foreign_operation = [99; 32];
    let foreign = checkpoint_intent::owner_id(
        root.digest(),
        &foreign_operation,
        b"kv9-checkpoint-pending-v1",
    )
    .unwrap();
    batch.put(
        ColumnFamily::Default,
        index_key(foreign, binding.descriptor.id),
        saved,
    );
    engine.write(batch).unwrap();
    assert!(readers(
        engine.snapshot().unwrap().as_ref(),
        &root,
        foreign_operation,
        None
    )
    .is_err());
}

#[test]
fn new_reader_generations_and_unentered_finish_keep_index_binding_stable() {
    let (engine, root, intent, owners) = fixture(1);
    finish(&engine, &root);
    let source = intent.binding(OwnerKind::Pending).unwrap();
    let binding = &owners[0];
    let k = index_key(intent.pending_id(), binding.descriptor.id);
    let saved = engine.get(ColumnFamily::Default, &k).unwrap().unwrap();
    assert!(plan(&engine, &root, &LedgerRequest::Publish(binding.token())).is_err());
    apply(
        &engine,
        &root,
        LedgerRequest::QuiesceAfterTransfer {
            from: binding.token(),
            to: source.token(),
        },
    );
    apply(&engine, &root, LedgerRequest::Release(binding.token()));
    let mut next = binding.clone();
    next.generation = NonZeroU64::new(2).unwrap();
    apply(
        &engine,
        &root,
        LedgerRequest::FinishCheckpointRead {
            successor: source.token(),
            reader: next.clone(),
        },
    );
    assert_eq!(
        engine.get(ColumnFamily::Default, &k).unwrap().unwrap(),
        saved
    );
    assert_eq!(
        collected(&engine, &root, source.descriptor.operation)[0].binding,
        next
    );
    let fresh = checkpoint_reader::binding(
        &source,
        LocalLifetime {
            node: 1,
            store: [1; 16],
            process: [250; 16],
        },
    )
    .unwrap();
    apply(
        &engine,
        &root,
        LedgerRequest::FinishCheckpointRead {
            successor: source.token(),
            reader: fresh.clone(),
        },
    );
    let page = collected(&engine, &root, source.descriptor.operation);
    assert_eq!(page.len(), 2);
    assert!(page.iter().all(|owner| owner.phase == PinPhase::Released));
    assert!(page.iter().any(|owner| owner.binding == fresh));
}

#[test]
fn census_page_and_progress_survive_wal_reopen_as_one_atomic_batch() {
    let (memory, root, intent, _) = fixture(9);
    apply(&memory, &root, LedgerRequest::Initialize);
    let directory = std::env::temp_dir().join(format!(
        "kv9-reader-census-{}",
        kv9_common::StoreIncarnation::mint().unwrap()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("meta.wal");
    let (engine, _) = WalEngine::open(&path).unwrap();
    let (seed_root, mut seed) = crate::retention::tests::seed();
    assert_eq!(seed_root, root);
    for (k, v) in all(&memory) {
        seed.put(ColumnFamily::Default, k, v);
    }
    engine.write(seed).unwrap();
    let command = request(&engine, &root);
    let view = engine.snapshot().unwrap();
    let plan = plan_retention(view.as_ref(), &root, &command).unwrap();
    drop(view);
    let original = std::fs::read(&path).unwrap();
    engine.write(plan.into_batch()).unwrap();
    drop(engine);
    let complete = std::fs::read(&path).unwrap();
    for cut in [
        original.len(),
        original.len() + 1,
        original.len() + (complete.len() - original.len()) / 2,
        complete.len() - 1,
        complete.len(),
    ] {
        let variant = directory.join(format!("cut-{cut}.wal"));
        std::fs::write(&variant, &complete[..cut]).unwrap();
        let (engine, _) = WalEngine::open(&variant).unwrap();
        let plan = plan_retention(engine.snapshot().unwrap().as_ref(), &root, &command).unwrap();
        assert_eq!(plan.changed(), cut != complete.len());
        assert_eq!(
            retention_owner(
                engine.snapshot().unwrap().as_ref(),
                &root,
                intent.pending_id()
            )
            .unwrap()
            .unwrap()
            .phase,
            PinPhase::Published
        );
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn pending_and_version_readers_share_operation_but_image_operations_remain_separate() {
    let (e, r, intent, _) = fixture(1);
    finish(&e, &r);
    let pending = intent.binding(OwnerKind::Pending).unwrap();
    let version = intent.binding(OwnerKind::Version).unwrap();
    apply(
        &e,
        &r,
        LedgerRequest::Share {
            from: pending.token(),
            to: version.clone(),
        },
    );
    apply(&e, &r, LedgerRequest::Publish(version.token()));
    let lifetime = LocalLifetime {
        node: 1,
        store: [1; 16],
        process: [1; 16],
    };
    let version_reader = checkpoint_reader::binding(&version, lifetime).unwrap();
    apply(
        &e,
        &r,
        LedgerRequest::Share {
            from: version.token(),
            to: version_reader.clone(),
        },
    );
    apply(&e, &r, LedgerRequest::Publish(version_reader.token()));
    let mut image = pending.clone();
    image.descriptor.id = OwnerId::new([71; 16]).unwrap();
    image.descriptor.kind = OwnerKind::Migration;
    image.descriptor.operation = [71; 32];
    apply(&e, &r, LedgerRequest::Acquire(image.clone()));
    apply(&e, &r, LedgerRequest::Publish(image.token()));
    let image_reader = image_reader::binding(&image, lifetime).unwrap();
    // The first Share never committed; finish must still atomically index the
    // terminal reader, even though global coverage has already completed.
    apply(
        &e,
        &r,
        LedgerRequest::FinishImageRead {
            successor: image.token(),
            reader: image_reader.clone(),
        },
    );
    let page = collected(&e, &r, pending.descriptor.operation);
    assert_eq!(page.len(), 2);
    assert!(page
        .iter()
        .any(|owner| owner.binding == version_reader && owner.phase == PinPhase::Published));
    let page = collected(&e, &r, image.descriptor.operation);
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].binding, image_reader);
    assert_eq!(page[0].phase, PinPhase::Released);
    let projection = group_id(&image_reader).unwrap();
    assert!(
        retention_owner(e.snapshot().unwrap().as_ref(), &r, projection)
            .unwrap()
            .is_none(),
        "a reverse-index group is not a source Pending"
    );
    assert_eq!(
        owner(&e, &r, pending.descriptor.id).phase,
        PinPhase::Published
    );
    assert_eq!(
        owner(&e, &r, version.descriptor.id).phase,
        PinPhase::Published
    );
}

#[test]
fn damaged_registration_or_cursor_stops_census_without_changing_pins() {
    for case in 0..3 {
        let (e, r, _, owners) = fixture(9);
        apply(&e, &r, LedgerRequest::Initialize);
        apply(&e, &r, request(&e, &r));
        let mut batch = WriteBatch::new();
        if case == 0 {
            // The first scan may already have passed this ID; the second must
            // still fail instead of accepting a stale incomplete registration.
            let id = owners[0].descriptor.id;
            let mut bytes = e
                .get(ColumnFamily::Default, &owner_registration_key(id))
                .unwrap()
                .unwrap();
            bytes[45] ^= 1;
            batch.put(ColumnFamily::Default, owner_registration_key(id), bytes);
        } else if case == 1 {
            let mut bytes = e
                .get(ColumnFamily::Default, &key(24, &[]))
                .unwrap()
                .unwrap();
            bytes[45] ^= 1;
            batch.put(ColumnFamily::Default, key(24, &[]), bytes);
        } else {
            let mut k = key(5, &[255; 16]);
            k.push(0); // malformed registration beyond the current owner cursor
            batch.put(ColumnFamily::Default, k, vec![1]);
        }
        e.write(batch).unwrap();
        let mut caught = false;
        for _ in 0..12 {
            let before = all(&e);
            let observed = observe(e.snapshot().unwrap().as_ref(), &r);
            match observed {
                Ok(Work::Page { progress, next, .. }) => {
                    let planned = plan(&e, &r, &progress.request(next));
                    match planned {
                        Ok(planned) => e.write(planned.into_batch()).unwrap(),
                        Err(_) => {
                            assert_eq!(all(&e), before);
                            caught = true;
                            break;
                        }
                    }
                }
                Err(_) => {
                    assert_eq!(all(&e), before);
                    caught = true;
                    break;
                }
                _ => panic!("damaged catalog accepted as complete"),
            }
        }
        assert!(caught);
    }
}
