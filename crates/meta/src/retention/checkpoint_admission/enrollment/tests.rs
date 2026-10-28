use super::*;
use crate::retention::checkpoint_intent::tests::fixture_intent;
use crate::retention::tests::{all, apply, owner, plan, seed};
use kv9_engine::{Engine, MemEngine, WalEngine};

fn fixture(gated: bool) -> (MemEngine, RootDescriptor, CheckpointIntent, OwnerBinding) {
    let (engine, root, intent) = fixture_intent();
    let pending = intent.binding(OwnerKind::Pending).unwrap();
    apply(&engine, &root, LedgerRequest::Acquire(pending.clone()));
    apply(&engine, &root, LedgerRequest::Publish(pending.token()));
    if gated {
        apply(
            &engine,
            &root,
            LedgerRequest::RecordCheckpoint(intent.clone()),
        );
    }
    let executor = executor_binding(
        &intent,
        LocalLifetime {
            node: 1,
            store: [1; 16],
            process: [2; 16],
        },
    )
    .unwrap();
    (engine, root, intent, executor)
}
fn share(intent: &CheckpointIntent, binding: &OwnerBinding) -> LedgerRequest {
    LedgerRequest::Share {
        from: intent.binding(OwnerKind::Pending).unwrap().token(),
        to: binding.clone(),
    }
}
fn release(
    engine: &MemEngine,
    root: &RootDescriptor,
    intent: &CheckpointIntent,
    binding: &OwnerBinding,
) {
    apply(
        engine,
        root,
        LedgerRequest::QuiesceAfterTransfer {
            from: binding.token(),
            to: intent.binding(OwnerKind::Pending).unwrap().token(),
        },
    );
    apply(engine, root, LedgerRequest::Release(binding.token()));
}
fn history_all(
    engine: &impl Engine,
    root: &RootDescriptor,
    pending: OwnerId,
    through: OwnerToken,
) -> Vec<GenerationObservation> {
    let view = engine.snapshot().unwrap();
    let mut observations = Vec::new();
    let mut after = None;
    loop {
        let page = history(view.as_ref(), root, pending, through, after).unwrap();
        assert!(page.generations.len() <= HISTORY_PAGE);
        observations.extend(page.generations);
        match page.next {
            Some(n) => after = Some(n),
            None => break,
        }
    }
    observations
}
fn finish(
    intent: &CheckpointIntent,
    binding: &OwnerBinding,
    issued: u64,
    resolved: u64,
) -> LedgerRequest {
    LedgerRequest::FinishCheckpointExecution {
        pending: intent.binding(OwnerKind::Pending).unwrap().token(),
        executor: binding.clone(),
        issued,
        resolved,
    }
}
fn set_old_header(engine: &MemEngine, magic: &[u8]) {
    let mut bytes = engine
        .get(ColumnFamily::Default, &key(1, &[]))
        .unwrap()
        .unwrap();
    bytes.truncate(bytes.len() - 32);
    bytes[..8].copy_from_slice(magic);
    let mut batch = WriteBatch::new();
    batch.put(ColumnFamily::Default, key(1, &[]), seal(bytes));
    engine.write(batch).unwrap();
}

#[test]
fn legacy_held_cannot_be_retroactively_enrolled_by_retry_publish_handoff_or_cancel() {
    let (engine, root, intent, old) = fixture(false);
    apply(&engine, &root, share(&intent, &old));
    apply(
        &engine,
        &root,
        LedgerRequest::RecordCheckpoint(intent.clone()),
    );
    let before = all(&engine);
    for r in [
        share(&intent, &old),
        LedgerRequest::Acquire(old.clone()),
        LedgerRequest::Publish(old.token()),
    ] {
        assert!(plan(&engine, &root, &r).is_err());
        assert_eq!(all(&engine), before);
    }
    release(&engine, &root, &intent, &old);
    apply(
        &engine,
        &root,
        LedgerRequest::CancelUnenteredCheckpoint(old.clone()),
    );
    assert!(engine
        .get(ColumnFamily::Default, &owner_key(old.descriptor.id))
        .unwrap()
        .unwrap()
        .starts_with(OWNER_MAGIC));
    assert!(engine
        .get(
            ColumnFamily::Default,
            &record_key(intent.pending_id(), old.token())
        )
        .unwrap()
        .is_none());
    let mut next = old.clone();
    next.generation = NonZeroU64::new(2).unwrap();
    apply(&engine, &root, share(&intent, &next));
    apply(&engine, &root, LedgerRequest::Publish(next.token()));
    apply(&engine, &root, finish(&intent, &next, 0, 0));
    assert_eq!(
        history_all(&engine, &root, intent.pending_id(), next.token())
            .into_iter()
            .map(|o| o.state)
            .collect::<Vec<_>>(),
        vec![
            GenerationState::MissingOrigin,
            GenerationState::Drained {
                issued: 0,
                resolved: 0
            }
        ]
    );
    assert_eq!(
        owner(&engine, &root, intent.pending_id()).phase,
        PinPhase::Published
    );
}

#[test]
fn old_v2_held_without_origin_cannot_publish_or_retire_and_upgrade_keeps_build_index() {
    let (engine, root, intent, e) = fixture(true);
    apply(&engine, &root, share(&intent, &e));
    let mut batch = WriteBatch::new();
    batch.delete(
        ColumnFamily::Default,
        record_key(intent.pending_id(), e.token()),
    );
    // This fixture represents V4, before closure change stamps existed.
    batch.delete(
        ColumnFamily::Default,
        key(19, intent.pending_id().as_bytes()),
    );
    // A stopped older-ledger fixture has no V20 image Publisher records.
    for (k, _) in all(&engine) {
        if k >= key(42, &[]) && k < key(48, &[]) {
            batch.delete(ColumnFamily::Default, k);
        }
    }
    engine.write(batch).unwrap();
    set_old_header(&engine, BUILD_INDEX_HEADER_MAGIC);
    let original = all(&engine);
    assert!(plan(&engine, &root, &share(&intent, &e)).is_err());
    assert!(plan(&engine, &root, &LedgerRequest::Publish(e.token())).is_err());
    assert_eq!(all(&engine), original);
    apply(&engine, &root, LedgerRequest::Initialize);
    for (k, v) in original
        .iter()
        .filter(|(k, _)| k.starts_with(&key(14, &[])))
    {
        assert_eq!(
            engine.get(ColumnFamily::Default, k).unwrap().as_ref(),
            Some(v)
        );
    }
    apply(
        &engine,
        &root,
        LedgerRequest::CloseCheckpointAdmission(
            intent.binding(OwnerKind::Pending).unwrap().token(),
        ),
    );
    assert!(plan(
        &engine,
        &root,
        &LedgerRequest::RetireUnpublishedCheckpoint {
            pending: intent.binding(OwnerKind::Pending).unwrap().token(),
            executor: e.token()
        }
    )
    .is_err());
    assert_eq!(owner(&engine, &root, e.descriptor.id).phase, PinPhase::Held);
    assert_eq!(
        history_all(&engine, &root, intent.pending_id(), e.token())[0].state,
        GenerationState::MissingOrigin
    );
}

#[test]
fn cancellation_records_only_new_unentered_generations_and_survives_later_entry() {
    let (engine, root, intent, mut e) = fixture(true);
    let command = LedgerRequest::CancelUnenteredCheckpoint(e.clone());
    apply(&engine, &root, command.clone());
    let bytes = engine
        .get(
            ColumnFamily::Default,
            &record_key(intent.pending_id(), e.token()),
        )
        .unwrap()
        .unwrap();
    assert_eq!(bytes.len(), MAX_BYTES);
    assert!(!plan(&engine, &root, &command).unwrap().changed());
    let first = e.token();
    e.generation = NonZeroU64::new(2).unwrap();
    apply(&engine, &root, share(&intent, &e));
    apply(&engine, &root, LedgerRequest::Publish(e.token()));
    assert_eq!(
        history_all(&engine, &root, intent.pending_id(), e.token())
            .into_iter()
            .map(|o| o.state)
            .collect::<Vec<_>>(),
        vec![
            GenerationState::Cancelled,
            GenerationState::Active(PinPhase::Published)
        ]
    );
    assert_eq!(
        engine
            .get(
                ColumnFamily::Default,
                &record_key(intent.pending_id(), first)
            )
            .unwrap()
            .unwrap(),
        bytes
    );
    assert!(plan(&engine, &root, &command).is_err());
}

#[test]
fn dense_history_retains_missing_terminal_and_unknown_puts_across_successful_later_generations() {
    let (engine, root, intent, mut e) = fixture(true);
    let mut expected = Vec::new();
    for generation in 1..=10 {
        e.generation = NonZeroU64::new(generation).unwrap();
        if generation == 6 || generation == 10 {
            apply(
                &engine,
                &root,
                LedgerRequest::CancelUnenteredCheckpoint(e.clone()),
            );
            expected.push(GenerationState::Cancelled);
            continue;
        }
        apply(&engine, &root, share(&intent, &e));
        if generation == 9 {
            apply(
                &engine,
                &root,
                LedgerRequest::CloseCheckpointAdmission(
                    intent.binding(OwnerKind::Pending).unwrap().token(),
                ),
            );
            apply(
                &engine,
                &root,
                LedgerRequest::RetireUnpublishedCheckpoint {
                    pending: intent.binding(OwnerKind::Pending).unwrap().token(),
                    executor: e.token(),
                },
            );
            expected.push(GenerationState::RetiredUnpublished);
        } else if generation <= 2 {
            release(&engine, &root, &intent, &e);
            if generation == 1 {
                let mut batch = WriteBatch::new();
                batch.delete(
                    ColumnFamily::Default,
                    record_key(intent.pending_id(), e.token()),
                );
                engine.write(batch).unwrap();
                expected.push(GenerationState::MissingOrigin);
            } else {
                expected.push(GenerationState::MissingTerminal);
            }
        } else {
            apply(&engine, &root, LedgerRequest::Publish(e.token()));
            let (issued, resolved) = match generation {
                3 => (2, 1),
                4 => (1, 1),
                _ => (0, 0),
            };
            apply(&engine, &root, finish(&intent, &e, issued, resolved));
            expected.push(GenerationState::Drained { issued, resolved });
        }
    }
    let observations = history_all(&engine, &root, intent.pending_id(), e.token());
    assert_eq!(
        observations
            .iter()
            .map(|o| o.executor.generation.get())
            .collect::<Vec<_>>(),
        (1..=10).collect::<Vec<_>>()
    );
    assert_eq!(
        observations
            .into_iter()
            .map(|o| o.state)
            .collect::<Vec<_>>(),
        expected
    );
    assert!(history(
        engine.snapshot().unwrap().as_ref(),
        &root,
        intent.pending_id(),
        e.token(),
        Some(e.generation)
    )
    .is_err());
    let mut ahead = e.token();
    ahead.generation = NonZeroU64::new(11).unwrap();
    assert!(history(
        engine.snapshot().unwrap().as_ref(),
        &root,
        intent.pending_id(),
        ahead,
        None
    )
    .is_err());
    assert_eq!(
        owner(&engine, &root, intent.pending_id()).phase,
        PinPhase::Published
    );
}

#[test]
fn corrupt_origin_or_header_downgrade_refuses_instead_of_certifying_absence() {
    for change in 0..8 {
        let (engine, root, intent, e) = fixture(true);
        apply(&engine, &root, share(&intent, &e));
        let k = record_key(intent.pending_id(), e.token());
        let bytes = engine.get(ColumnFamily::Default, &k).unwrap().unwrap();
        let mut record = Enrollment::decode(&bytes, root.digest()).unwrap();
        match change {
            0 => record.pending = OwnerId::new([99; 16]).unwrap(),
            1 => record.executor.generation = NonZeroU64::new(2).unwrap(),
            2 => record.binding = RootDigest::from_bytes([99; 32]),
            3 => record.opened_at += 1,
            4 => record.recorded_at = u64::MAX,
            5 => record.origin = Origin::Cancelled,
            6 => set_old_header(&engine, BUILD_INDEX_HEADER_MAGIC),
            _ => {}
        }
        if change != 6 {
            let mut bytes = record.encode(root.digest());
            if change == 7 {
                bytes[0] ^= 1;
            }
            let mut batch = WriteBatch::new();
            batch.put(ColumnFamily::Default, k, bytes);
            engine.write(batch).unwrap();
        }
        let before = all(&engine);
        assert!(history(
            engine.snapshot().unwrap().as_ref(),
            &root,
            intent.pending_id(),
            e.token(),
            None
        )
        .is_err());
        assert!(plan(&engine, &root, &LedgerRequest::Publish(e.token())).is_err());
        assert_eq!(all(&engine), before);
    }
}

#[test]
fn original_entry_and_owner_share_are_atomic_across_actual_wal_tail_cuts() {
    let (memory, root, intent, e) = fixture(true);
    let directory = std::env::temp_dir().join(format!(
        "kv9-enrollment-{}",
        kv9_common::StoreIncarnation::mint().unwrap()
    ));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("meta.wal");
    let (engine, _) = WalEngine::open(&path).unwrap();
    let (seed_root, mut batch) = seed();
    assert_eq!(seed_root, root);
    for (k, v) in all(&memory) {
        batch.put(ColumnFamily::Default, k, v);
    }
    engine.write(batch).unwrap();
    let before = std::fs::read(&path).unwrap();
    let request = share(&intent, &e);
    engine
        .write(
            plan_retention(engine.snapshot().unwrap().as_ref(), &root, &request)
                .unwrap()
                .into_batch(),
        )
        .unwrap();
    drop(engine);
    let after = std::fs::read(&path).unwrap();
    for cut in [
        before.len(),
        before.len() + 1,
        before.len() + (after.len() - before.len()) / 2,
        after.len() - 1,
        after.len(),
    ] {
        let variant = directory.join(format!("cut-{cut}.wal"));
        std::fs::write(&variant, &after[..cut]).unwrap();
        let (engine, _) = WalEngine::open(&variant).unwrap();
        let view = engine.snapshot().unwrap();
        let present = cut == after.len();
        assert_eq!(
            retention_owner(view.as_ref(), &root, e.descriptor.id)
                .unwrap()
                .is_some(),
            present
        );
        assert_eq!(
            view.get(
                ColumnFamily::Default,
                &record_key(intent.pending_id(), e.token())
            )
            .unwrap()
            .is_some(),
            present
        );
        assert_eq!(
            plan_retention(view.as_ref(), &root, &request)
                .unwrap()
                .changed(),
            !present
        );
        if present {
            assert_eq!(
                history_all(&engine, &root, intent.pending_id(), e.token())[0].state,
                GenerationState::Active(PinPhase::Held)
            );
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn unentered_and_retired_generations_refuse_contradictory_reports_without_partial_writes() {
    for retired in [false, true] {
        let (engine, root, intent, e) = fixture(true);
        if retired {
            apply(&engine, &root, share(&intent, &e));
            apply(
                &engine,
                &root,
                LedgerRequest::CloseCheckpointAdmission(
                    intent.binding(OwnerKind::Pending).unwrap().token(),
                ),
            );
            apply(
                &engine,
                &root,
                LedgerRequest::RetireUnpublishedCheckpoint {
                    pending: intent.binding(OwnerKind::Pending).unwrap().token(),
                    executor: e.token(),
                },
            );
        } else {
            apply(
                &engine,
                &root,
                LedgerRequest::CancelUnenteredCheckpoint(e.clone()),
            );
        }
        let before = all(&engine);
        for command in [
            finish(&intent, &e, 1, 1),
            LedgerRequest::RecordCheckpointDrain {
                pending: intent.binding(OwnerKind::Pending).unwrap().token(),
                executor: e.token(),
                issued: 1,
                resolved: 1,
            },
        ] {
            assert!(plan(&engine, &root, &command).is_err());
            assert_eq!(all(&engine), before);
        }
        apply(&engine, &root, finish(&intent, &e, 0, 0));
        assert_eq!(
            history_all(&engine, &root, intent.pending_id(), e.token())[0].state,
            if retired {
                GenerationState::RetiredUnpublished
            } else {
                GenerationState::Cancelled
            }
        );
    }
}

struct Count<'a> {
    view: &'a dyn ReadView,
    reads: std::sync::atomic::AtomicUsize,
}
impl ReadView for Count<'_> {
    fn get(&self, cf: ColumnFamily, k: &[u8]) -> Result<Option<Vec<u8>>> {
        self.reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.view.get(cf, k)
    }
    fn scan(
        &self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<kv9_engine::ScanEntry>> {
        assert!(!start.starts_with(PREFIX));
        self.view.scan(cf, start, end, limit)
    }
    fn seek_le(&self, cf: ColumnFamily, target: &[u8]) -> Result<Option<kv9_engine::ScanEntry>> {
        assert!(!target.starts_with(PREFIX));
        self.view.seek_le(cf, target)
    }
    fn iter<'a>(
        &'a self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
    ) -> Result<Box<dyn Iterator<Item = Result<kv9_engine::ScanEntry>> + 'a>> {
        assert!(!start.starts_with(PREFIX));
        self.view.iter(cf, start, end)
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
fn generation_pages_have_fixed_point_work_and_never_scan_only_present_reports() {
    let mut counts = Vec::new();
    for size in [13, 113] {
        let (engine, root, intent, mut e) = fixture(true);
        for n in 1..=size {
            e.generation = NonZeroU64::new(n).unwrap();
            apply(
                &engine,
                &root,
                LedgerRequest::CancelUnenteredCheckpoint(e.clone()),
            );
        }
        let view = engine.snapshot().unwrap();
        let count = Count {
            view: view.as_ref(),
            reads: std::sync::atomic::AtomicUsize::new(0),
        };
        let page = history(&count, &root, intent.pending_id(), e.token(), None).unwrap();
        assert_eq!(page.generations.len(), HISTORY_PAGE);
        assert_eq!(page.next.unwrap().get(), HISTORY_PAGE as u64);
        assert!(page
            .generations
            .iter()
            .all(|g| g.state == GenerationState::Cancelled));
        counts.push(count.reads.load(std::sync::atomic::Ordering::Relaxed));
    }
    assert_eq!(counts[0], counts[1]);
}
