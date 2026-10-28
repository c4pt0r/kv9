use super::*;
use crate::retention::checkpoint_intent::tests::fixture_intent;
use crate::retention::tests::{all, apply, owner, plan};
use kv9_engine::{Engine, MemEngine};

fn fixture() -> (MemEngine, RootDescriptor, CheckpointIntent, OwnerBinding) {
    let (engine, root, intent) = fixture_intent();
    let pending = intent.binding(OwnerKind::Pending).unwrap();
    apply(&engine, &root, LedgerRequest::Acquire(pending.clone()));
    apply(&engine, &root, LedgerRequest::Publish(pending.token()));
    apply(
        &engine,
        &root,
        LedgerRequest::RecordCheckpoint(intent.clone()),
    );
    (engine, root, intent, pending)
}
fn execution(intent: &CheckpointIntent, process: u8) -> OwnerBinding {
    executor_binding(
        intent,
        LocalLifetime {
            node: 1,
            store: [1; 16],
            process: [process; 16],
        },
    )
    .unwrap()
}
fn page(
    engine: &impl Engine,
    root: &RootDescriptor,
    id: OwnerId,
    after: Option<OwnerId>,
    limit: usize,
) -> ExecutorPage {
    executors(engine.snapshot().unwrap().as_ref(), root, id, after, limit).unwrap()
}

#[test]
fn close_is_monotone_blocks_new_and_repeated_entry_but_allows_covered_drain() {
    let (engine, root, intent, pending) = fixture();
    let held = execution(&intent, 1);
    let published = execution(&intent, 2);
    for binding in [&held, &published] {
        apply(
            &engine,
            &root,
            LedgerRequest::Share {
                from: pending.token(),
                to: binding.clone(),
            },
        );
    }
    apply(&engine, &root, LedgerRequest::Publish(published.token()));
    let close = LedgerRequest::CloseCheckpointAdmission(pending.token());
    apply(&engine, &root, close.clone());
    let observed = page(&engine, &root, pending.descriptor.id, None, 16);
    assert!(observed.admission.is_closed());
    assert_eq!(observed.owners.len(), 2);
    assert!(!plan(&engine, &root, &close).unwrap().changed());
    apply(
        &engine,
        &root,
        LedgerRequest::RecordCheckpoint(intent.clone()),
    );
    assert_eq!(
        page(&engine, &root, pending.descriptor.id, None, 16).admission,
        observed.admission
    );
    let before = all(&engine);
    for binding in [held.clone(), published.clone(), execution(&intent, 3)] {
        assert!(plan(&engine, &root, &LedgerRequest::Acquire(binding.clone())).is_err());
        assert!(plan(
            &engine,
            &root,
            &LedgerRequest::Share {
                from: pending.token(),
                to: binding.clone()
            }
        )
        .is_err());
        assert!(plan(&engine, &root, &LedgerRequest::Publish(binding.token())).is_err());
    }
    assert_eq!(all(&engine), before);
    for binding in [&held, &published] {
        assert!(plan(
            &engine,
            &root,
            &LedgerRequest::CancelUnenteredCheckpoint(binding.clone())
        )
        .is_err());
        apply(
            &engine,
            &root,
            LedgerRequest::QuiesceAfterTransfer {
                from: binding.token(),
                to: pending.token(),
            },
        );
        apply(&engine, &root, LedgerRequest::Release(binding.token()));
        assert_eq!(
            owner(&engine, &root, binding.descriptor.id).phase,
            PinPhase::Released
        );
    }
    assert_eq!(
        owner(&engine, &root, pending.descriptor.id).phase,
        PinPhase::Published
    );
    let mut next = published;
    next.generation = NonZeroU64::new(2).unwrap();
    assert!(plan(&engine, &root, &LedgerRequest::Acquire(next)).is_err());
}

#[test]
fn unknown_entry_cancellation_fences_exact_generation_with_open_or_closed_gate() {
    for closed in [false, true] {
        let (engine, root, intent, pending) = fixture();
        if closed {
            apply(
                &engine,
                &root,
                LedgerRequest::CloseCheckpointAdmission(pending.token()),
            );
        }
        let first = execution(&intent, 1);
        apply(
            &engine,
            &root,
            LedgerRequest::CancelUnenteredCheckpoint(first.clone()),
        );
        assert!(!plan(
            &engine,
            &root,
            &LedgerRequest::CancelUnenteredCheckpoint(first.clone())
        )
        .unwrap()
        .changed());
        assert!(plan(
            &engine,
            &root,
            &LedgerRequest::Share {
                from: pending.token(),
                to: first.clone()
            }
        )
        .is_err());
        let mut second = first.clone();
        second.generation = NonZeroU64::new(2).unwrap();
        apply(
            &engine,
            &root,
            LedgerRequest::CancelUnenteredCheckpoint(second.clone()),
        );
        assert!(plan(
            &engine,
            &root,
            &LedgerRequest::CancelUnenteredCheckpoint(first)
        )
        .is_err());
        let rows = page(&engine, &root, pending.descriptor.id, None, 16);
        assert_eq!(
            rows.owners,
            [LedgerOwner {
                binding: second,
                phase: PinPhase::Released
            }]
        );
        assert_eq!(rows.admission.is_closed(), closed);
    }
}

#[test]
fn bounded_executor_pages_preserve_all_generations_and_validate_both_links() {
    let (engine, root, intent, pending) = fixture();
    for n in 1..=35 {
        apply(
            &engine,
            &root,
            LedgerRequest::CancelUnenteredCheckpoint(execution(&intent, n)),
        );
    }
    let mut cursor = None;
    let mut ids = Vec::new();
    loop {
        let page = page(&engine, &root, pending.descriptor.id, cursor, 4);
        let encoded = page.encode(root.digest()).unwrap();
        assert_eq!(
            ExecutorPage::decode(&encoded, root.digest(), pending.descriptor.id, cursor, 4)
                .unwrap(),
            page
        );
        assert!(
            ExecutorPage::decode(&encoded, root.digest(), pending.descriptor.id, cursor, 1)
                .is_err()
        );
        assert!(page.owners.len() <= 4);
        for owner in page.owners {
            assert_eq!(owner.phase, PinPhase::Released);
            ids.push(owner.binding.descriptor.id);
        }
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(ids.len(), 35);
    assert!(ids.windows(2).all(|p| p[0] < p[1]));
    assert!(executors(
        engine.snapshot().unwrap().as_ref(),
        &root,
        pending.descriptor.id,
        None,
        17
    )
    .is_err());
    let mut damage = WriteBatch::new();
    damage.delete(
        ColumnFamily::Default,
        index_key(pending.descriptor.id, ids[0]),
    );
    engine.write(damage).unwrap();
    assert!(retention_owner(engine.snapshot().unwrap().as_ref(), &root, ids[0]).is_err());
    let damaged = (1..=35)
        .map(|n| execution(&intent, n))
        .find(|b| b.descriptor.id == ids[0])
        .unwrap();
    assert!(plan(
        &engine,
        &root,
        &LedgerRequest::CancelUnenteredCheckpoint(damaged)
    )
    .is_err());
}

#[test]
fn executor_page_codec_rejects_cross_query_unordered_and_truncated_observations() {
    let (engine, root, intent, pending) = fixture();
    let binding = execution(&intent, 1);
    for request in [
        LedgerRequest::CloseCheckpointAdmission(pending.token()),
        LedgerRequest::CancelUnenteredCheckpoint(binding.clone()),
    ] {
        assert_eq!(
            decode_request(
                &encode_request(root.digest(), &request).unwrap(),
                root.digest()
            )
            .unwrap(),
            request
        );
        apply(&engine, &root, request);
    }
    let page = page(&engine, &root, pending.descriptor.id, None, 1);
    let bytes = page.encode(root.digest()).unwrap();
    for (r, id, cursor, limit) in [
        (
            RootDigest::from_bytes([9; 32]),
            pending.descriptor.id,
            None,
            1,
        ),
        (root.digest(), binding.descriptor.id, None, 1),
        (
            root.digest(),
            pending.descriptor.id,
            Some(binding.descriptor.id),
            1,
        ),
        (root.digest(), pending.descriptor.id, None, 0),
        (root.digest(), pending.descriptor.id, None, 17),
    ] {
        assert!(ExecutorPage::decode(&bytes, r, id, cursor, limit).is_err());
    }
    for len in 0..bytes.len() {
        assert!(
            ExecutorPage::decode(&bytes[..len], root.digest(), pending.descriptor.id, None, 1)
                .is_err()
        );
    }
    for change in 0..5 {
        let mut damaged = page.clone();
        match change {
            0 => damaged.owners.push(damaged.owners[0].clone()),
            1 => damaged.next = Some(pending.descriptor.id),
            2 => damaged.admission.opened_at = 0,
            3 => damaged.admission.closed_at = damaged.admission.opened_at - 1,
            _ => damaged.owners[0].binding.descriptor.operation = [9; 32],
        }
        assert!(ExecutorPage::decode(
            &damaged.encode(root.digest()).unwrap(),
            root.digest(),
            pending.descriptor.id,
            None,
            2
        )
        .is_err());
    }
}

#[test]
fn lost_gate_state_or_index_cannot_be_reconstructed_and_legacy_archive_upgrades() {
    for damage in 0..5 {
        let (engine, root, intent, pending) = fixture();
        let binding = execution(&intent, 1);
        apply(
            &engine,
            &root,
            LedgerRequest::Share {
                from: pending.token(),
                to: binding.clone(),
            },
        );
        let mut batch = WriteBatch::new();
        match damage {
            0 => {
                batch.delete(ColumnFamily::Default, gate_key(pending.descriptor.id));
            }
            1 => {
                batch.delete(
                    ColumnFamily::Default,
                    checkpoint_intent::intent_key(pending.descriptor.id),
                );
            }
            2 => {
                batch.delete(
                    ColumnFamily::Default,
                    index_key(pending.descriptor.id, binding.descriptor.id),
                );
            }
            3 => {
                let observed = owner(&engine, &root, binding.descriptor.id);
                batch.put(
                    ColumnFamily::Default,
                    owner_key(binding.descriptor.id),
                    encode_owner(&observed),
                );
            }
            _ => {
                let mut h = header(root.digest(), 100);
                h.truncate(h.len() - 32);
                h[..8].copy_from_slice(ROWS_HEADER_MAGIC);
                batch.put(ColumnFamily::Default, key(1, &[]), seal(h));
            }
        }
        engine.write(batch).unwrap();
        let before = all(&engine);
        assert!(
            retention_owner(
                engine.snapshot().unwrap().as_ref(),
                &root,
                binding.descriptor.id
            )
            .is_err(),
            "damage {damage}"
        );
        assert!(plan(&engine, &root, &LedgerRequest::Publish(binding.token())).is_err());
        if damage <= 1 || damage == 4 {
            assert!(plan(&engine, &root, &LedgerRequest::RecordCheckpoint(intent)).is_err());
        }
        assert_eq!(all(&engine), before);
    }
    let (engine, root, intent) = fixture_intent();
    let pending = intent.binding(OwnerKind::Pending).unwrap();
    apply(&engine, &root, LedgerRequest::Acquire(pending.clone()));
    apply(&engine, &root, LedgerRequest::Publish(pending.token()));
    let mut old = intent.encode();
    old.truncate(old.len() - 32);
    old[..8].copy_from_slice(b"KV9CPI01");
    let mut batch = WriteBatch::new();
    batch.put(
        ColumnFamily::Default,
        checkpoint_intent::intent_key(pending.descriptor.id),
        seal(old),
    );
    engine.write(batch).unwrap();
    apply(&engine, &root, LedgerRequest::RecordCheckpoint(intent));
    assert!(!page(&engine, &root, pending.descriptor.id, None, 1)
        .admission
        .is_closed());
}
