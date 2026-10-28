use super::*;
use crate::retention::checkpoint_intent::tests::fixture_intent;
use crate::retention::tests::{all, apply, owner, plan, seed};
use kv9_engine::{Engine, MemEngine, WalEngine};

fn fixture(empty: bool) -> (MemEngine, RootDescriptor, CheckpointIntent) {
    let (engine, root, intent) = fixture_intent();
    if empty {
        let mut batch = WriteBatch::new();
        for (k, _) in all(&engine) {
            batch.delete(ColumnFamily::Default, k);
        }
        engine.write(batch).unwrap();
    }
    (engine, root, intent)
}
fn birth(
    engine: &impl Engine,
    root: &RootDescriptor,
    intent: &CheckpointIntent,
) -> Option<AtomicBirth> {
    atomic_birth(
        engine.snapshot().unwrap().as_ref(),
        root,
        intent.pending_id(),
    )
    .unwrap()
}

#[test]
fn first_begin_atomically_creates_the_entire_attempt_without_earlier_initialization() {
    let (engine, root, intent) = fixture(true);
    let command = LedgerRequest::BeginCheckpoint(intent.clone());
    let encoded = encode_request(root.digest(), &command).unwrap();
    assert_eq!(decode_request(&encoded, root.digest()).unwrap(), command);
    for cut in 0..encoded.len() {
        assert!(decode_request(&encoded[..cut], root.digest()).is_err());
    }
    let uncommitted = plan(&engine, &root, &command).unwrap();
    assert!(all(&engine).is_empty());
    let revision = uncommitted.revision();
    engine.write(uncommitted.into_batch()).unwrap();
    assert_eq!(revision, 1);
    let origin = birth(&engine, &root, &intent).unwrap();
    assert_eq!(origin.created_at, revision);
    assert_eq!(origin.encode(root.digest()).len(), 128);
    assert_eq!(
        owner(&engine, &root, intent.pending_id()).phase,
        PinPhase::Published
    );
    let page = checkpoint_admission::executors(
        engine.snapshot().unwrap().as_ref(),
        &root,
        intent.pending_id(),
        None,
        4,
    )
    .unwrap();
    assert_eq!(page.admission.opened_at, revision);
    assert!(!page.admission.is_closed());
    assert!(page.owners.is_empty());
    assert!(!plan(&engine, &root, &command).unwrap().changed());
    let before = all(&engine);
    assert!(plan(
        &engine,
        &root,
        &LedgerRequest::RecordCheckpoint(intent.clone())
    )
    .is_err());
    assert_eq!(
        all(&engine),
        before,
        "legacy final authorization must not mutate an atomic attempt"
    );
}

#[test]
fn every_legacy_pending_phase_resumes_without_manufacturing_birth() {
    for phase in [
        PinPhase::Held,
        PinPhase::Published,
        PinPhase::Quiesced,
        PinPhase::Released,
    ] {
        let (engine, root, intent) = fixture(false);
        let pending = intent.binding(OwnerKind::Pending).unwrap();
        let version = intent.binding(OwnerKind::Version).unwrap();
        apply(&engine, &root, LedgerRequest::Acquire(pending.clone()));
        if phase != PinPhase::Held {
            apply(&engine, &root, LedgerRequest::Publish(pending.token()));
        }
        if matches!(phase, PinPhase::Quiesced | PinPhase::Released) {
            apply(
                &engine,
                &root,
                LedgerRequest::Share {
                    from: pending.token(),
                    to: version.clone(),
                },
            );
            apply(&engine, &root, LedgerRequest::Publish(version.token()));
            apply(
                &engine,
                &root,
                LedgerRequest::QuiesceAfterTransfer {
                    from: pending.token(),
                    to: version.token(),
                },
            );
            if phase == PinPhase::Released {
                apply(&engine, &root, LedgerRequest::Release(pending.token()));
            }
        }
        let command = LedgerRequest::BeginCheckpoint(intent.clone());
        apply(&engine, &root, command.clone());
        assert_eq!(birth(&engine, &root, &intent), None);
        assert_eq!(
            owner(&engine, &root, intent.pending_id()).phase,
            if phase == PinPhase::Held {
                PinPhase::Published
            } else {
                phase
            }
        );
        assert!(!plan(&engine, &root, &command).unwrap().changed());
        apply(&engine, &root, LedgerRequest::RecordCheckpoint(intent));
    }
}

#[test]
fn original_birth_survives_positive_handoff_and_cannot_reopen_closed_admission() {
    let (engine, root, intent) = fixture(true);
    let command = LedgerRequest::BeginCheckpoint(intent.clone());
    apply(&engine, &root, command.clone());
    let original = birth(&engine, &root, &intent);
    let p = intent.binding(OwnerKind::Pending).unwrap();
    let v = intent.binding(OwnerKind::Version).unwrap();
    for request in [
        LedgerRequest::CloseCheckpointAdmission(p.token()),
        LedgerRequest::Share {
            from: p.token(),
            to: v.clone(),
        },
        LedgerRequest::Publish(v.token()),
        LedgerRequest::QuiesceAfterTransfer {
            from: p.token(),
            to: v.token(),
        },
        LedgerRequest::Release(p.token()),
    ] {
        apply(&engine, &root, request);
    }
    assert!(!plan(&engine, &root, &command).unwrap().changed());
    assert_eq!(birth(&engine, &root, &intent), original);
    assert!(checkpoint_admission::executors(
        engine.snapshot().unwrap().as_ref(),
        &root,
        intent.pending_id(),
        None,
        4
    )
    .unwrap()
    .admission
    .is_closed());
    assert_eq!(
        owner(&engine, &root, p.descriptor.id).phase,
        PinPhase::Released
    );
}

#[test]
fn missing_birth_remains_a_blocker_and_orphan_or_corrupt_birth_refuses_repair() {
    for damage in 0..7 {
        let (engine, root, intent) = fixture(true);
        let command = LedgerRequest::BeginCheckpoint(intent.clone());
        apply(&engine, &root, command.clone());
        let mut origin = birth(&engine, &root, &intent).unwrap();
        let mut batch = WriteBatch::new();
        match damage {
            0 => {
                batch.delete(ColumnFamily::Default, origin_key(intent.pending_id()));
            }
            1 => {
                batch.delete(ColumnFamily::Default, owner_key(intent.pending_id()));
            }
            2 => {
                origin.created_at += 1;
                batch.put(
                    ColumnFamily::Default,
                    origin_key(intent.pending_id()),
                    origin.encode(root.digest()),
                );
            }
            3 => {
                origin.intent = RootDigest::from_bytes([99; 32]);
                batch.put(
                    ColumnFamily::Default,
                    origin_key(intent.pending_id()),
                    origin.encode(root.digest()),
                );
            }
            4 => {
                batch.delete(
                    ColumnFamily::Default,
                    checkpoint_intent::intent_key(intent.pending_id()),
                );
            }
            5 => {
                batch.delete(
                    ColumnFamily::Default,
                    key(9, intent.pending_id().as_bytes()),
                );
            }
            _ => {
                let mut bytes = engine
                    .get(ColumnFamily::Default, &key(1, &[]))
                    .unwrap()
                    .unwrap();
                bytes.truncate(bytes.len() - 32);
                bytes[..8].copy_from_slice(ENROLLMENT_HEADER_MAGIC);
                batch.put(ColumnFamily::Default, key(1, &[]), seal(bytes));
            }
        }
        engine.write(batch).unwrap();
        let before = all(&engine);
        if damage == 0 {
            assert_eq!(birth(&engine, &root, &intent), None);
            assert!(!plan(&engine, &root, &command).unwrap().changed());
        } else {
            assert!(atomic_birth(
                engine.snapshot().unwrap().as_ref(),
                &root,
                intent.pending_id()
            )
            .is_err());
            assert!(plan(&engine, &root, &command).is_err());
        }
        assert_eq!(all(&engine), before);
    }
}

#[test]
fn atomic_birth_does_not_relabel_preexisting_builds_or_prove_their_drain() {
    let (engine, root, intent) = fixture(false);
    let mut generic = checkpoint_admission::executor_binding(
        &intent,
        LocalLifetime {
            node: 1,
            store: [1; 16],
            process: [2; 16],
        },
    )
    .unwrap();
    apply(&engine, &root, LedgerRequest::Acquire(generic.clone()));
    let original = engine
        .get(ColumnFamily::Default, &owner_key(generic.descriptor.id))
        .unwrap()
        .unwrap();
    apply(
        &engine,
        &root,
        LedgerRequest::BeginCheckpoint(intent.clone()),
    );
    assert!(birth(&engine, &root, &intent).is_some());
    assert_eq!(
        engine
            .get(ColumnFamily::Default, &owner_key(generic.descriptor.id))
            .unwrap()
            .unwrap(),
        original
    );
    assert_eq!(
        checkpoint_admission::enrollment::history(
            engine.snapshot().unwrap().as_ref(),
            &root,
            intent.pending_id(),
            generic.token(),
            None
        )
        .unwrap()
        .generations[0]
            .state,
        checkpoint_admission::enrollment::GenerationState::MissingOrigin
    );
    generic.generation = NonZeroU64::new(2).unwrap();
    assert!(plan(&engine, &root, &LedgerRequest::Acquire(generic)).is_err());
}

#[test]
fn actual_wal_cuts_restore_all_birth_protection_or_no_attempt() {
    let (memory, root, intent) = fixture(true);
    let dir = std::env::temp_dir().join(format!(
        "kv9-begin-{}",
        kv9_common::StoreIncarnation::mint().unwrap()
    ));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("meta.wal");
    let (engine, _) = WalEngine::open(&path).unwrap();
    let (seed_root, batch) = seed();
    assert_eq!(root, seed_root);
    engine.write(batch).unwrap();
    let before = std::fs::read(&path).unwrap();
    let command = LedgerRequest::BeginCheckpoint(intent.clone());
    engine
        .write(plan(&memory, &root, &command).unwrap().into_batch())
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
        let path = dir.join(format!("cut-{cut}.wal"));
        std::fs::write(&path, &after[..cut]).unwrap();
        let (engine, _) = WalEngine::open(&path).unwrap();
        let view = engine.snapshot().unwrap();
        let complete = cut == after.len();
        for k in [
            key(1, &[]),
            owner_key(intent.pending_id()),
            owner_registration_key(intent.pending_id()),
            checkpoint_intent::intent_key(intent.pending_id()),
            key(9, intent.pending_id().as_bytes()),
            origin_key(intent.pending_id()),
        ] {
            assert_eq!(
                view.get(ColumnFamily::Default, &k).unwrap().is_some(),
                complete
            );
        }
        assert_eq!(
            plan_retention(view.as_ref(), &root, &command)
                .unwrap()
                .changed(),
            !complete
        );
        if complete {
            assert!(birth(&engine, &root, &intent).is_some());
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn existing_version_cannot_be_reinterpreted_as_a_fresh_attempt() {
    let (engine, root, intent) = fixture(false);
    let version = intent.binding(OwnerKind::Version).unwrap();
    apply(&engine, &root, LedgerRequest::Acquire(version.clone()));
    apply(&engine, &root, LedgerRequest::Publish(version.token()));
    let before = all(&engine);
    assert!(plan(&engine, &root, &LedgerRequest::BeginCheckpoint(intent)).is_err());
    assert_eq!(all(&engine), before);
}
