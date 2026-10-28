use super::*;
use crate::retention::checkpoint_intent::tests::fixture_intent;
use crate::retention::tests::{all, apply, owner, plan};
use kv9_engine::{Engine, MemEngine};

fn fixture(count: u8) -> (MemEngine, RootDescriptor, OwnerBinding, Vec<OwnerBinding>) {
    let (engine, root, intent) = fixture_intent();
    let pending = intent.binding(OwnerKind::Pending).unwrap();
    apply(&engine, &root, LedgerRequest::Acquire(pending.clone()));
    apply(&engine, &root, LedgerRequest::Publish(pending.token()));
    apply(
        &engine,
        &root,
        LedgerRequest::RecordCheckpoint(intent.clone()),
    );
    let mut owners = Vec::new();
    for n in 1..=count {
        let binding = executor_binding(
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
    owners.sort_by_key(|owner| owner.descriptor.id);
    (engine, root, pending, owners)
}
fn page(
    engine: &MemEngine,
    root: &RootDescriptor,
    pending: OwnerId,
) -> (SweepCursor, ExecutorPage) {
    match observe(engine.snapshot().unwrap().as_ref(), root, pending).unwrap() {
        SweepObservation::Page { cursor, page } => (cursor, page),
        _ => panic!("expected active page"),
    }
}
fn close(engine: &MemEngine, root: &RootDescriptor, pending: &OwnerBinding) {
    apply(
        engine,
        root,
        LedgerRequest::CloseCheckpointAdmission(pending.token()),
    );
}
fn retire(
    engine: &MemEngine,
    root: &RootDescriptor,
    pending: &OwnerBinding,
    binding: &OwnerBinding,
) {
    apply(
        engine,
        root,
        LedgerRequest::RetireUnpublishedCheckpoint {
            pending: pending.token(),
            executor: binding.token(),
        },
    );
}

#[test]
fn progress_requires_closed_exact_nonheld_page_and_survives_lost_ack() {
    let (engine, root, pending, owners) = fixture(9);
    assert!(matches!(
        observe(
            engine.snapshot().unwrap().as_ref(),
            &root,
            pending.descriptor.id
        )
        .unwrap(),
        SweepObservation::Open
    ));
    assert!(plan(
        &engine,
        &root,
        &SweepCursor::default().advance_request(pending.descriptor.id, None)
    )
    .is_err());
    close(&engine, &root, &pending);
    let (cursor, first) = page(&engine, &root, pending.descriptor.id);
    assert_eq!(first.owners.len(), 4);
    assert_eq!(first.next, Some(owners[3].descriptor.id));
    let advance = cursor.advance_request(pending.descriptor.id, first.next);
    let bytes = encode_request(root.digest(), &advance).unwrap();
    assert_eq!(decode_request(&bytes, root.digest()).unwrap(), advance);
    for end in 0..bytes.len() {
        assert!(decode_request(&bytes[..end], root.digest()).is_err());
    }
    for owner in &owners[..3] {
        retire(&engine, &root, &pending, owner);
    }
    let before = all(&engine);
    assert!(
        plan(&engine, &root, &advance).is_err(),
        "cannot skip the last Held"
    );
    assert_eq!(all(&engine), before);
    retire(&engine, &root, &pending, &owners[3]);
    for next in [
        None,
        Some(owners[2].descriptor.id),
        Some(owners[4].descriptor.id),
    ] {
        assert!(plan(
            &engine,
            &root,
            &cursor.advance_request(pending.descriptor.id, next)
        )
        .is_err());
    }
    apply(&engine, &root, advance.clone());
    assert!(
        !plan(&engine, &root, &advance).unwrap().changed(),
        "lost acknowledgement retries exact transition"
    );
    let (second_cursor, second) = page(&engine, &root, pending.descriptor.id);
    assert_eq!(second_cursor.sequence, 1);
    assert_eq!(second.owners[0].binding, owners[4]);
    assert_eq!(second.next, Some(owners[7].descriptor.id));
    for owner in &owners[4..8] {
        retire(&engine, &root, &pending, owner);
    }
    apply(
        &engine,
        &root,
        second_cursor.advance_request(pending.descriptor.id, second.next),
    );
    assert!(
        plan(&engine, &root, &advance).is_err(),
        "stale requests cannot overwrite a later cursor"
    );
    let (last_cursor, last) = page(&engine, &root, pending.descriptor.id);
    assert_eq!(last_cursor.sequence, 2);
    assert_eq!(last.owners.len(), 1);
    assert_eq!(last.next, None);
    retire(&engine, &root, &pending, &owners[8]);
    apply(
        &engine,
        &root,
        last_cursor.advance_request(pending.descriptor.id, last.next),
    );
    assert!(matches!(
        observe(
            engine.snapshot().unwrap().as_ref(),
            &root,
            pending.descriptor.id
        )
        .unwrap(),
        SweepObservation::Done
    ));
    assert_eq!(
        owner(&engine, &root, pending.descriptor.id).phase,
        PinPhase::Published
    );
}

#[test]
fn done_is_only_a_held_sweep_and_later_cancel_cannot_add_held_behind_cursor() {
    let (engine, root, pending, owners) = fixture(3);
    for binding in &owners {
        apply(&engine, &root, LedgerRequest::Publish(binding.token()));
    }
    apply(
        &engine,
        &root,
        LedgerRequest::QuiesceAfterTransfer {
            from: owners[1].token(),
            to: pending.token(),
        },
    );
    apply(
        &engine,
        &root,
        LedgerRequest::QuiesceAfterTransfer {
            from: owners[2].token(),
            to: pending.token(),
        },
    );
    apply(&engine, &root, LedgerRequest::Release(owners[2].token()));
    close(&engine, &root, &pending);
    let (cursor, page) = page(&engine, &root, pending.descriptor.id);
    let done = cursor.advance_request(pending.descriptor.id, page.next);
    apply(&engine, &root, done.clone());
    let before = all(&engine);
    assert!(!plan(&engine, &root, &done).unwrap().changed());
    assert!(matches!(
        observe(
            engine.snapshot().unwrap().as_ref(),
            &root,
            pending.descriptor.id
        )
        .unwrap(),
        SweepObservation::Done
    ));
    assert_eq!(all(&engine), before);
    let mut later = owners[2].clone();
    later.generation = NonZeroU64::new(2).unwrap();
    apply(
        &engine,
        &root,
        LedgerRequest::CancelUnenteredCheckpoint(later.clone()),
    );
    assert!(plan(
        &engine,
        &root,
        &LedgerRequest::Share {
            from: pending.token(),
            to: later
        }
    )
    .is_err());
    assert_eq!(
        owner(&engine, &root, owners[0].descriptor.id).phase,
        PinPhase::Published
    );
    assert_eq!(
        owner(&engine, &root, owners[1].descriptor.id).phase,
        PinPhase::Quiesced
    );
    assert_eq!(
        owner(&engine, &root, pending.descriptor.id).phase,
        PinPhase::Published
    );
}

#[test]
fn corrupt_cursor_refuses_and_missing_progress_safely_repeats_work() {
    for damage in 0..7 {
        let (engine, root, pending, owners) = fixture(5);
        close(&engine, &root, &pending);
        let (cursor, page) = page(&engine, &root, pending.descriptor.id);
        for binding in &owners[..4] {
            retire(&engine, &root, &pending, binding);
        }
        apply(
            &engine,
            &root,
            cursor.advance_request(pending.descriptor.id, page.next),
        );
        let view = engine.snapshot().unwrap();
        let key = cursor_key(pending.descriptor.id);
        let mut bytes = view.get(ColumnFamily::Default, &key).unwrap().unwrap();
        let mut batch = WriteBatch::new();
        if damage == 6 {
            batch.delete(ColumnFamily::Default, key);
        } else {
            bytes.truncate(bytes.len() - 32);
            match damage {
                0 => bytes[8] ^= 1,  // root
                1 => bytes[40] ^= 1, // pending
                2 => bytes[56..64].copy_from_slice(&u64::MAX.to_be_bytes()),
                3 => bytes[64] ^= 1, // closure revision
                4 => bytes[72] = 1,  // done with a nonempty cursor
                _ => bytes[73] = 2,  // noncanonical optional field
            }
            batch.put(ColumnFamily::Default, key, seal(bytes));
        }
        engine.write(batch).unwrap();
        if damage == 6 {
            let (cursor, repeated) = self::page(&engine, &root, pending.descriptor.id);
            assert_eq!(cursor.sequence, 0);
            assert_eq!(repeated.owners.len(), 4);
            assert!(repeated
                .owners
                .iter()
                .all(|owner| owner.phase == PinPhase::Released));
            apply(
                &engine,
                &root,
                cursor.advance_request(pending.descriptor.id, repeated.next),
            );
        } else {
            assert!(observe(
                engine.snapshot().unwrap().as_ref(),
                &root,
                pending.descriptor.id
            )
            .is_err());
        }
    }
}

#[test]
fn discovery_is_exclusive_and_damaged_values_do_not_hide_later_keys() {
    let (engine, root, original) = fixture_intent();
    let mut ids = Vec::new();
    for predecessor in 0..7 {
        let intent =
            CheckpointIntent::new(&root, &original.manifest().unwrap(), predecessor).unwrap();
        let pending = intent.binding(OwnerKind::Pending).unwrap();
        apply(&engine, &root, LedgerRequest::Acquire(pending.clone()));
        apply(&engine, &root, LedgerRequest::Publish(pending.token()));
        apply(&engine, &root, LedgerRequest::RecordCheckpoint(intent));
        ids.push(pending.descriptor.id);
    }
    ids.sort();
    let mut damage = WriteBatch::new();
    damage.put(ColumnFamily::Default, gate_key(ids[0]), vec![0; 136]);
    engine.write(damage).unwrap();
    let view = engine.snapshot().unwrap();
    let mut after = None;
    for id in &ids {
        assert_eq!(
            next_attempt(view.as_ref(), &root, after).unwrap(),
            Some(*id)
        );
        after = Some(*id);
    }
    assert_eq!(next_attempt(view.as_ref(), &root, after).unwrap(), None);
    assert!(observe(view.as_ref(), &root, ids[0]).is_err());
    assert!(matches!(
        observe(view.as_ref(), &root, ids[1]).unwrap(),
        SweepObservation::Open
    ));
    let (empty, empty_root) = crate::retention::tests::fixture();
    assert_eq!(
        next_attempt(empty.snapshot().unwrap().as_ref(), &empty_root, None).unwrap(),
        None
    );
    let mut orphan = WriteBatch::new();
    orphan.put(ColumnFamily::Default, gate_key(ids[0]), vec![]);
    empty.write(orphan).unwrap();
    assert!(next_attempt(empty.snapshot().unwrap().as_ref(), &empty_root, None).is_err());
}
