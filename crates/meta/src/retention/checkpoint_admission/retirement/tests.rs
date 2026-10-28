use super::*;
use crate::retention::checkpoint_intent::tests::fixture_intent;
use crate::retention::tests::{all, apply, owner, plan};
use kv9_engine::{Engine, MemEngine};

fn fixture() -> (MemEngine, RootDescriptor, OwnerBinding, OwnerBinding) {
    let (engine, root, intent) = fixture_intent();
    let pending = intent.binding(OwnerKind::Pending).unwrap();
    apply(&engine, &root, LedgerRequest::Acquire(pending.clone()));
    apply(&engine, &root, LedgerRequest::Publish(pending.token()));
    apply(
        &engine,
        &root,
        LedgerRequest::RecordCheckpoint(intent.clone()),
    );
    let executor = executor_binding(
        &intent,
        LocalLifetime {
            node: 1,
            store: [1; 16],
            process: [2; 16],
        },
    )
    .unwrap();
    apply(
        &engine,
        &root,
        LedgerRequest::Share {
            from: pending.token(),
            to: executor.clone(),
        },
    );
    (engine, root, pending, executor)
}
fn close(engine: &MemEngine, root: &RootDescriptor, pending: &OwnerBinding) {
    apply(
        engine,
        root,
        LedgerRequest::CloseCheckpointAdmission(pending.token()),
    );
}
fn request(pending: &OwnerBinding, executor: &OwnerBinding) -> LedgerRequest {
    LedgerRequest::RetireUnpublishedCheckpoint {
        pending: pending.token(),
        executor: executor.token(),
    }
}
fn observe(
    engine: &MemEngine,
    root: &RootDescriptor,
    pending: &OwnerBinding,
    executor: &OwnerBinding,
) -> Result<Option<UnpublishedRetirement>> {
    unpublished_retirement(
        engine.snapshot().unwrap().as_ref(),
        root,
        pending.descriptor.id,
        executor.token(),
    )
}

#[test]
fn closed_held_retires_atomically_with_evidence_and_keeps_global_protection() {
    let (engine, root, pending, executor) = fixture();
    let command = request(&pending, &executor);
    assert!(plan(&engine, &root, &command).is_err());
    close(&engine, &root, &pending);
    let encoded = encode_request(root.digest(), &command).unwrap();
    assert_eq!(decode_request(&encoded, root.digest()).unwrap(), command);
    for end in 0..encoded.len() {
        assert!(decode_request(&encoded[..end], root.digest()).is_err());
    }
    apply(&engine, &root, command.clone());
    let proof = observe(&engine, &root, &pending, &executor)
        .unwrap()
        .unwrap();
    assert_eq!(proof.executor, executor.token());
    assert!(proof.retired_at > proof.closed_at);
    assert_eq!(proof.encode(root.digest()).len(), 160);
    assert!(!plan(&engine, &root, &command).unwrap().changed());
    assert_eq!(
        owner(&engine, &root, executor.descriptor.id).phase,
        PinPhase::Released
    );
    assert_eq!(
        owner(&engine, &root, pending.descriptor.id).phase,
        PinPhase::Published
    );
    assert_eq!(
        executors(
            engine.snapshot().unwrap().as_ref(),
            &root,
            pending.descriptor.id,
            None,
            4
        )
        .unwrap()
        .owners[0]
            .phase,
        PinPhase::Released
    );
    assert!(plan(&engine, &root, &LedgerRequest::Publish(executor.token())).is_err());
    assert!(plan(
        &engine,
        &root,
        &LedgerRequest::Share {
            from: pending.token(),
            to: executor.clone()
        }
    )
    .is_err());
    let mut later = executor.clone();
    later.generation = NonZeroU64::new(2).unwrap();
    apply(
        &engine,
        &root,
        LedgerRequest::CancelUnenteredCheckpoint(later),
    );
    assert_eq!(
        observe(&engine, &root, &pending, &executor).unwrap(),
        Some(proof)
    );
    assert!(
        plan(&engine, &root, &command).is_err(),
        "old-generation mutation must not close a later generation"
    );
}

#[test]
fn published_quiesced_generically_released_and_wrong_tokens_never_create_evidence() {
    for phase in [PinPhase::Published, PinPhase::Quiesced, PinPhase::Released] {
        let (engine, root, pending, executor) = fixture();
        apply(&engine, &root, LedgerRequest::Publish(executor.token()));
        if phase != PinPhase::Published {
            apply(
                &engine,
                &root,
                LedgerRequest::QuiesceAfterTransfer {
                    from: executor.token(),
                    to: pending.token(),
                },
            );
        }
        if phase == PinPhase::Released {
            apply(&engine, &root, LedgerRequest::Release(executor.token()));
        }
        close(&engine, &root, &pending);
        let before = all(&engine);
        assert!(plan(&engine, &root, &request(&pending, &executor)).is_err());
        assert_eq!(observe(&engine, &root, &pending, &executor).unwrap(), None);
        assert_eq!(all(&engine), before);
    }
    let (engine, root, pending, executor) = fixture();
    close(&engine, &root, &pending);
    for change in 0..4 {
        let mut p = pending.clone();
        let mut e = executor.clone();
        match change {
            0 => p.generation = NonZeroU64::new(2).unwrap(),
            1 => e.generation = NonZeroU64::new(2).unwrap(),
            2 => p.descriptor.id = OwnerId::new([7; 16]).unwrap(),
            _ => e.descriptor.id = OwnerId::new([7; 16]).unwrap(),
        }
        assert!(plan(&engine, &root, &request(&p, &e)).is_err());
    }
    assert_eq!(
        owner(&engine, &root, executor.descriptor.id).phase,
        PinPhase::Held
    );
}

#[test]
fn evidence_damage_refuses_and_missing_evidence_is_not_reconstructed_from_released() {
    for change in 0..7 {
        let (engine, root, pending, executor) = fixture();
        close(&engine, &root, &pending);
        apply(&engine, &root, request(&pending, &executor));
        let mut proof = observe(&engine, &root, &pending, &executor)
            .unwrap()
            .unwrap();
        let mut batch = WriteBatch::new();
        let key = record_key(pending.descriptor.id, executor.token());
        if change == 6 {
            batch.delete(ColumnFamily::Default, key);
        } else {
            match change {
                0 => proof.pending = OwnerId::new([9; 16]).unwrap(),
                1 => proof.executor.generation = NonZeroU64::new(2).unwrap(),
                2 => proof.binding = RootDigest::from_bytes([9; 32]),
                3 => proof.closed_at += 1,
                4 => proof.retired_at = u64::MAX,
                _ => proof.retired_at = proof.closed_at,
            }
            batch.put(ColumnFamily::Default, key, proof.encode(root.digest()));
        }
        engine.write(batch).unwrap();
        let before = all(&engine);
        if change == 6 {
            assert_eq!(observe(&engine, &root, &pending, &executor).unwrap(), None);
        } else {
            assert!(observe(&engine, &root, &pending, &executor).is_err());
        }
        assert!(plan(&engine, &root, &request(&pending, &executor)).is_err());
        assert_eq!(all(&engine), before);
    }
}

#[test]
fn untracked_legacy_held_or_broken_crosslinks_do_not_qualify() {
    for legacy in [false, true] {
        let (engine, root, pending, executor) = fixture();
        close(&engine, &root, &pending);
        let mut batch = WriteBatch::new();
        batch.delete(
            ColumnFamily::Default,
            index_key(pending.descriptor.id, executor.descriptor.id),
        );
        if legacy {
            batch.put(
                ColumnFamily::Default,
                owner_key(executor.descriptor.id),
                encode_owner(&owner(&engine, &root, executor.descriptor.id)),
            );
            batch.put(
                ColumnFamily::Default,
                owner_registration_key(executor.descriptor.id),
                owner_registration(&executor),
            );
        }
        engine.write(batch).unwrap();
        let before = all(&engine);
        assert!(plan(&engine, &root, &request(&pending, &executor)).is_err());
        assert_eq!(all(&engine), before);
    }
}
