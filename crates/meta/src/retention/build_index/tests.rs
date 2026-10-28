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
        let page = builds(&count, &root, intent.pending_id(), None).unwrap();
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
        let binding = checkpoint_admission::executor_binding(
            &intent,
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
    // These owners predate intent/gate registration and are genuinely V1 owner
    // rows. Reproduce V3 catalog bytes, which had no Build reverse index.
    let mut batch = WriteBatch::new();
    for (k, _) in all(&engine) {
        if k.starts_with(&key(14, &[]))
            || k.starts_with(&key(15, &[]))
            || (k >= key(42, &[]) && k < key(48, &[]))
        {
            batch.delete(ColumnFamily::Default, k);
        }
    }
    let mut old = engine
        .get(ColumnFamily::Default, &key(1, &[]))
        .unwrap()
        .unwrap();
    old.truncate(old.len() - 32);
    old[..8].copy_from_slice(GATED_HEADER_MAGIC);
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
fn collected(engine: &impl Engine, root: &RootDescriptor, pending: OwnerId) -> Vec<LedgerOwner> {
    let view = engine.snapshot().unwrap();
    let mut owners = Vec::new();
    let mut after = None;
    loop {
        let page = builds(view.as_ref(), root, pending, after).unwrap();
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
    assert!(builds(
        engine.snapshot().unwrap().as_ref(),
        &root,
        intent.pending_id(),
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
            checkpoint_admission::executor_binding(
                &intent,
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
        collected(&engine, &root, intent.pending_id())
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
    let page = checkpoint_admission::executors(
        engine.snapshot().unwrap().as_ref(),
        &root,
        intent.pending_id(),
        None,
        16,
    )
    .unwrap();
    assert_eq!(
        page.owners.len(),
        1,
        "census must not turn legacy Held into certified unpublished executors"
    );
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
    if let LedgerRequest::AdvanceBuildIndex { next, .. } = &mut wrong {
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
        collected(&engine, &root, intent.pending_id()).len(),
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
        checked(&header_bytes, GATED_HEADER_MAGIC, 81).is_err(),
        "V3 reader must reject the new format"
    );
    let mut old = header_bytes[..header_bytes.len() - 32].to_vec();
    old[..8].copy_from_slice(GATED_HEADER_MAGIC);
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
    let foreign = OwnerId::new([99; 16]).unwrap();
    batch.put(
        ColumnFamily::Default,
        index_key(foreign, binding.descriptor.id),
        saved,
    );
    engine.write(batch).unwrap();
    assert!(builds(engine.snapshot().unwrap().as_ref(), &root, foreign, None).is_err());
}

#[test]
fn new_build_generation_and_unentered_cancel_keep_reverse_index_binding_stable() {
    let (engine, root, intent, owners) = fixture(1);
    apply(
        &engine,
        &root,
        LedgerRequest::RecordCheckpoint(intent.clone()),
    );
    finish(&engine, &root);
    let binding = &owners[0];
    let k = index_key(intent.pending_id(), binding.descriptor.id);
    let saved = engine.get(ColumnFamily::Default, &k).unwrap().unwrap();
    assert!(plan(&engine, &root, &LedgerRequest::Publish(binding.token())).is_err());
    apply(
        &engine,
        &root,
        LedgerRequest::QuiesceAfterTransfer {
            from: binding.token(),
            to: intent.binding(OwnerKind::Pending).unwrap().token(),
        },
    );
    apply(&engine, &root, LedgerRequest::Release(binding.token()));
    let mut next = binding.clone();
    next.generation = NonZeroU64::new(2).unwrap();
    apply(
        &engine,
        &root,
        LedgerRequest::CancelUnenteredCheckpoint(next.clone()),
    );
    assert_eq!(
        engine.get(ColumnFamily::Default, &k).unwrap().unwrap(),
        saved
    );
    assert_eq!(
        collected(&engine, &root, intent.pending_id())[0].binding,
        next
    );
}

#[test]
fn census_page_and_progress_survive_wal_reopen_as_one_atomic_batch() {
    let (memory, root, intent, _) = fixture(9);
    apply(&memory, &root, LedgerRequest::Initialize);
    let directory = std::env::temp_dir().join(format!(
        "kv9-build-census-{}",
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
