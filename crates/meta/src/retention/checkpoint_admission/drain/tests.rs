use super::*;
use crate::retention::checkpoint_intent::tests::fixture_intent;
use crate::retention::tests::{all, apply, owner, plan};
use kv9_engine::{Engine, MemEngine};

pub(super) fn fixture() -> (MemEngine, RootDescriptor, OwnerBinding, OwnerBinding) {
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
pub(super) fn release(
    engine: &MemEngine,
    root: &RootDescriptor,
    pending: &OwnerBinding,
    executor: &OwnerBinding,
) {
    apply(
        engine,
        root,
        LedgerRequest::QuiesceAfterTransfer {
            from: executor.token(),
            to: pending.token(),
        },
    );
    apply(engine, root, LedgerRequest::Release(executor.token()));
}
fn request(p: &OwnerBinding, e: &OwnerBinding, issued: u64, resolved: u64) -> LedgerRequest {
    LedgerRequest::RecordCheckpointDrain {
        pending: p.token(),
        executor: e.token(),
        issued,
        resolved,
    }
}
pub(super) fn observe(
    engine: &MemEngine,
    root: &RootDescriptor,
    p: &OwnerBinding,
    e: &OwnerBinding,
) -> Result<Option<CheckpointDrain>> {
    checkpoint_drain(
        engine.snapshot().unwrap().as_ref(),
        root,
        p.descriptor.id,
        e.token(),
    )
}

#[test]
fn drain_requires_exact_released_generation_and_is_immutable_across_retries() {
    let (engine, root, p, e) = fixture();
    let command = request(&p, &e, 3, 2);
    assert!(plan(&engine, &root, &command).is_err());
    apply(&engine, &root, LedgerRequest::Publish(e.token()));
    assert!(plan(&engine, &root, &command).is_err());
    release(&engine, &root, &p, &e);
    assert_eq!(
        observe(&engine, &root, &p, &e).unwrap(),
        None,
        "Released alone is no evidence"
    );
    let bytes = encode_request(root.digest(), &command).unwrap();
    assert_eq!(decode_request(&bytes, root.digest()).unwrap(), command);
    for n in 0..bytes.len() {
        assert!(decode_request(&bytes[..n], root.digest()).is_err());
    }
    apply(&engine, &root, command.clone());
    let proof = observe(&engine, &root, &p, &e).unwrap().unwrap();
    assert_eq!((proof.issued, proof.resolved), (3, 2));
    assert_eq!(proof.encode(root.digest()).len(), 176);
    assert!(!plan(&engine, &root, &command).unwrap().changed());
    let before = all(&engine);
    for (issued, resolved) in [(3, 3), (0, 0), (4, 2), (2, 3)] {
        assert!(plan(&engine, &root, &request(&p, &e, issued, resolved)).is_err());
    }
    assert_eq!(all(&engine), before);
    assert_eq!(
        owner(&engine, &root, p.descriptor.id).phase,
        PinPhase::Published
    );
}

#[test]
fn later_generation_success_never_erases_an_earlier_unknown_observation() {
    let (engine, root, p, e) = fixture();
    apply(&engine, &root, LedgerRequest::Publish(e.token()));
    release(&engine, &root, &p, &e);
    apply(&engine, &root, request(&p, &e, 1, 0));
    let original = observe(&engine, &root, &p, &e).unwrap().unwrap();
    let mut next = e.clone();
    next.generation = NonZeroU64::new(2).unwrap();
    apply(
        &engine,
        &root,
        LedgerRequest::Share {
            from: p.token(),
            to: next.clone(),
        },
    );
    apply(&engine, &root, LedgerRequest::Publish(next.token()));
    assert_eq!(
        observe(&engine, &root, &p, &e).unwrap(),
        Some(original.clone())
    );
    assert!(!plan(&engine, &root, &request(&p, &e, 1, 0))
        .unwrap()
        .changed());
    assert_eq!(owner(&engine, &root, next.descriptor.id).binding, next);
    assert_eq!(
        owner(&engine, &root, next.descriptor.id).phase,
        PinPhase::Published
    );
    release(&engine, &root, &p, &next);
    apply(&engine, &root, request(&p, &next, 1, 1));
    assert_eq!(observe(&engine, &root, &p, &e).unwrap(), Some(original));
    assert_eq!(
        observe(&engine, &root, &p, &next)
            .unwrap()
            .unwrap()
            .resolved,
        1
    );
    apply(
        &engine,
        &root,
        LedgerRequest::CloseCheckpointAdmission(p.token()),
    );
    assert_eq!(
        observe(&engine, &root, &p, &e).unwrap().unwrap().resolved,
        0
    );
}

#[test]
fn wrong_tokens_missing_links_and_damaged_observations_refuse() {
    for change in 0..8 {
        let (engine, root, p, e) = fixture();
        release(&engine, &root, &p, &e);
        apply(&engine, &root, request(&p, &e, 0, 0));
        let mut proof = observe(&engine, &root, &p, &e).unwrap().unwrap();
        let mut batch = WriteBatch::new();
        match change {
            0 => proof.resolved = 1,
            1 => proof.reported_at = u64::MAX,
            2 => proof.opened_at += 1,
            3 => proof.binding = RootDigest::from_bytes([9; 32]),
            4 => proof.executor.generation = NonZeroU64::new(2).unwrap(),
            5 => proof.pending = OwnerId::new([8; 16]).unwrap(),
            6 => {
                batch.delete(
                    ColumnFamily::Default,
                    index_key(p.descriptor.id, e.descriptor.id),
                );
            }
            _ => {
                batch.delete(
                    ColumnFamily::Default,
                    owner_registration_key(e.descriptor.id),
                );
            }
        }
        if change < 6 {
            batch.put(
                ColumnFamily::Default,
                record_key(p.descriptor.id, e.token()),
                proof.encode(root.digest()),
            );
        }
        engine.write(batch).unwrap();
        assert!(observe(&engine, &root, &p, &e).is_err());
        assert!(plan(&engine, &root, &request(&p, &e, 0, 0)).is_err());
    }
    let (engine, root, p, e) = fixture();
    release(&engine, &root, &p, &e);
    let mut wrong = e.clone();
    wrong.generation = NonZeroU64::new(2).unwrap();
    assert!(plan(&engine, &root, &request(&p, &wrong, 0, 0)).is_err());
    let mut wrong = p.clone();
    wrong.generation = NonZeroU64::new(2).unwrap();
    assert!(plan(&engine, &root, &request(&wrong, &e, 0, 0)).is_err());
}
