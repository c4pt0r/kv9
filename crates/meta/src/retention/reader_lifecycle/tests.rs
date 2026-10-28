use super::*;
use crate::retention::checkpoint_intent::tests::fixture_intent;
use crate::retention::tests::{all, apply, owner, plan, seed};
use kv9_engine::{Engine, MemEngine, WalEngine};

fn fixture() -> (MemEngine, RootDescriptor, CheckpointIntent, OwnerBinding) {
    let (e, r, i) = fixture_intent();
    apply(&e, &r, LedgerRequest::BeginCheckpoint(i.clone()));
    let reader = binding(&i.binding(OwnerKind::Pending).unwrap(), 7);
    (e, r, i, reader)
}
fn binding(source: &OwnerBinding, process: u8) -> OwnerBinding {
    checkpoint_reader::binding(
        source,
        LocalLifetime {
            node: 1,
            store: [3; 16],
            process: [process; 16],
        },
    )
    .unwrap()
}
fn share(i: &CheckpointIntent, b: &OwnerBinding) -> LedgerRequest {
    LedgerRequest::Share {
        from: i.binding(OwnerKind::Pending).unwrap().token(),
        to: b.clone(),
    }
}
fn finish(i: &CheckpointIntent, b: &OwnerBinding) -> LedgerRequest {
    LedgerRequest::FinishCheckpointRead {
        successor: i.binding(OwnerKind::Pending).unwrap().token(),
        reader: b.clone(),
    }
}
fn fact(e: &impl Engine, r: &RootDescriptor, token: OwnerToken) -> Option<ReaderGeneration> {
    generation(e.snapshot().unwrap().as_ref(), r, token).unwrap()
}
fn refuse(e: &impl Engine, r: &RootDescriptor, i: &CheckpointIntent) -> LedgerPlan {
    let voter = &r.voters[0];
    checkpoint_refusal::plan_verified(
        e.snapshot().unwrap().as_ref(),
        r,
        i.binding(OwnerKind::Pending).unwrap().token(),
        [9; 32],
        crate::endpoint::NodeEndpoint {
            node: voter.node_id,
            incarnation: voter.store_incarnation,
            address: voter.addr,
            generation: 0,
            previous_address: None,
            active: true,
        },
    )
    .unwrap()
}

#[test]
fn exact_share_and_publish_record_each_generation_and_forbid_generic_drain() {
    let (e, r, i, b) = fixture();
    assert!(plan(&e, &r, &LedgerRequest::Acquire(b.clone())).is_err());
    for mode in 0..6 {
        let mut wrong = b.clone();
        match mode {
            0 => wrong.descriptor.operation = [8; 32],
            1 => wrong.descriptor.id = OwnerId::new([8; 16]).unwrap(),
            2 => wrong.descriptor.region += 1,
            3 => wrong.descriptor.local.as_mut().unwrap().process = [8; 16],
            4 => wrong.resources.pop().map(|_| ()).unwrap(),
            _ => wrong.generation = NonZeroU64::new(2).unwrap(),
        }
        assert!(plan(&e, &r, &share(&i, &wrong)).is_err());
    }
    let entered = apply(&e, &r, share(&i, &b));
    let held = fact(&e, &r, b.token()).unwrap();
    assert_eq!(
        (
            held.origin,
            held.entered_at,
            held.published_at,
            held.finished_at
        ),
        (Origin::Entered, entered, 0, 0)
    );
    assert!(!plan(&e, &r, &share(&i, &b)).unwrap().changed());
    let published = apply(&e, &r, LedgerRequest::Publish(b.token()));
    assert_eq!(fact(&e, &r, b.token()).unwrap().published_at, published);
    let before = all(&e);
    for request in [
        LedgerRequest::Release(b.token()),
        LedgerRequest::QuiesceAfterTransfer {
            from: b.token(),
            to: i.binding(OwnerKind::Pending).unwrap().token(),
        },
    ] {
        assert!(plan(&e, &r, &request).is_err());
    }
    assert_eq!(all(&e), before);
    let finished = apply(&e, &r, finish(&i, &b));
    let old = fact(&e, &r, b.token()).unwrap();
    assert_eq!(
        (old.entered_at, old.published_at, old.finished_at),
        (entered, published, finished)
    );
    let mut next = b.clone();
    next.generation = NonZeroU64::new(2).unwrap();
    apply(&e, &r, finish(&i, &next));
    assert_eq!(
        fact(&e, &r, next.token()).unwrap().origin,
        Origin::Unentered
    );
    assert!(plan(&e, &r, &share(&i, &next)).is_err());
    next.generation = NonZeroU64::new(3).unwrap();
    apply(&e, &r, share(&i, &next));
    apply(&e, &r, LedgerRequest::Publish(next.token()));
    assert_eq!(fact(&e, &r, b.token()), Some(old));
    assert!(plan(&e, &r, &finish(&i, &b)).is_err());
    assert_eq!(owner(&e, &r, i.pending_id()).phase, PinPhase::Published);
}

#[test]
fn refusal_blocks_fresh_held_published_retries_and_new_generations_but_preserves_finish() {
    for state in 0..4 {
        let (e, r, i, mut b) = fixture();
        if state > 0 {
            apply(&e, &r, share(&i, &b));
        }
        if state > 1 {
            apply(&e, &r, LedgerRequest::Publish(b.token()));
        }
        if state == 3 {
            apply(&e, &r, finish(&i, &b));
            b.generation = NonZeroU64::new(2).unwrap();
        }
        e.write(refuse(&e, &r, &i).into_batch()).unwrap();
        let before = all(&e);
        for request in [
            share(&i, &b),
            LedgerRequest::Acquire(b.clone()),
            LedgerRequest::Publish(b.token()),
        ] {
            assert!(plan(&e, &r, &request).is_err());
        }
        let mut wrong = b.clone();
        wrong.descriptor.operation = [8; 32];
        assert!(plan(&e, &r, &share(&i, &wrong)).is_err());
        assert_eq!(all(&e), before);
        apply(&e, &r, finish(&i, &b));
        assert!(!plan(&e, &r, &finish(&i, &b)).unwrap().changed());
        assert_eq!(owner(&e, &r, b.descriptor.id).phase, PinPhase::Released);
        let record = fact(&e, &r, b.token()).unwrap();
        assert_eq!(
            record.origin,
            if state == 0 || state == 3 {
                Origin::Unentered
            } else {
                Origin::Entered
            }
        );
        assert_eq!(owner(&e, &r, i.pending_id()).phase, PinPhase::Published);
    }
}

#[test]
fn build_gate_closed_without_refusal_keeps_positive_version_readers_available() {
    let (e, r, i, b) = fixture();
    let pending = i.binding(OwnerKind::Pending).unwrap();
    apply(
        &e,
        &r,
        LedgerRequest::CloseCheckpointAdmission(pending.token()),
    );
    apply(&e, &r, share(&i, &b));
    apply(&e, &r, LedgerRequest::Publish(b.token()));
    let version = i.binding(OwnerKind::Version).unwrap();
    apply(
        &e,
        &r,
        LedgerRequest::Share {
            from: pending.token(),
            to: version.clone(),
        },
    );
    apply(&e, &r, LedgerRequest::Publish(version.token()));
    apply(
        &e,
        &r,
        LedgerRequest::QuiesceAfterTransfer {
            from: pending.token(),
            to: version.token(),
        },
    );
    apply(&e, &r, LedgerRequest::Release(pending.token()));
    apply(&e, &r, finish(&i, &b));
    let b = binding(&version, 8);
    apply(
        &e,
        &r,
        LedgerRequest::Share {
            from: version.token(),
            to: b.clone(),
        },
    );
    apply(&e, &r, LedgerRequest::Publish(b.token()));
    assert_eq!(
        fact(&e, &r, b.token()).unwrap().source_kind,
        OwnerKind::Version
    );
}

// Construct actual pre-generation bytes. No production downgrade or inferred
// admission is exercised by this fixture conversion.
fn legacy(e: &MemEngine, r: &RootDescriptor, b: &OwnerBinding) {
    let old = owner(e, r, b.descriptor.id);
    let mut batch = WriteBatch::new();
    batch.put(
        ColumnFamily::Default,
        owner_key(b.descriptor.id),
        encode_owner(&old),
    );
    batch.put(
        ColumnFamily::Default,
        owner_registration_key(b.descriptor.id),
        owner_registration(b),
    );
    batch.delete(ColumnFamily::Default, record_key(b.token()));
    // Reader closure stamps did not exist in this explicit V9 fixture.
    for (k, _) in all(e) {
        if (k >= key(38, &[]) && k < key(42, &[]))
            || k.starts_with(&key(26, &[]))
            || k.starts_with(&key(27, &[]))
        {
            batch.delete(ColumnFamily::Default, k);
        }
    }
    let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
    h.truncate(h.len() - 32);
    h[..8].copy_from_slice(READER_INDEX_HEADER_MAGIC);
    batch.put(ColumnFamily::Default, key(1, &[]), seal(h));
    e.write(batch).unwrap();
}

#[test]
fn legacy_admission_stays_unknown_and_upgrade_preserves_indexes_without_fabricating_history() {
    let (e, r, i, b) = fixture();
    apply(&e, &r, share(&i, &b));
    legacy(&e, &r, &b);
    let indexes: Vec<_> = all(&e)
        .into_iter()
        .filter(|(k, _)| k.starts_with(&key(23, &[])))
        .collect();
    assert!(fact(&e, &r, b.token()).is_none());
    assert!(plan(&e, &r, &share(&i, &b)).is_err());
    assert!(plan(&e, &r, &LedgerRequest::Publish(b.token())).is_err());
    apply(&e, &r, finish(&i, &b));
    let first = fact(&e, &r, b.token()).unwrap();
    assert_eq!(
        (first.origin, first.entered_at, first.published_at),
        (Origin::Legacy, 0, 0)
    );
    assert_eq!(
        indexes,
        all(&e)
            .into_iter()
            .filter(|(k, _)| k.starts_with(&key(23, &[])))
            .collect::<Vec<_>>()
    );
    let mut next = b.clone();
    next.generation = NonZeroU64::new(2).unwrap();
    apply(&e, &r, share(&i, &next));
    assert_eq!(fact(&e, &r, next.token()).unwrap().origin, Origin::Entered);
    assert_eq!(fact(&e, &r, b.token()), Some(first));
    // Explicit lost historical evidence remains a hole, never a zero-work fact.
    let mut batch = WriteBatch::new();
    batch.delete(ColumnFamily::Default, record_key(b.token()));
    e.write(batch).unwrap();
    assert!(fact(&e, &r, b.token()).is_none());
}

#[test]
fn malformed_refusal_cannot_reopen_read_admission_or_promote_held_reader() {
    for damage in 0..3 {
        let (e, r, i, b) = fixture();
        apply(&e, &r, share(&i, &b));
        e.write(refuse(&e, &r, &i).into_batch()).unwrap();
        let mut batch = WriteBatch::new();
        match damage {
            0 => {
                let k = key(20, i.pending_id().as_bytes());
                let mut bytes = e.get(ColumnFamily::Default, &k).unwrap().unwrap();
                bytes[50] ^= 1;
                batch.put(ColumnFamily::Default, k, bytes);
            }
            1 => {
                batch.delete(
                    ColumnFamily::Default,
                    checkpoint_intent::intent_key(i.pending_id()),
                );
            }
            _ => {
                batch.delete(ColumnFamily::Default, key(9, i.pending_id().as_bytes()));
            }
        }
        e.write(batch).unwrap();
        let before = all(&e);
        assert!(plan(&e, &r, &share(&i, &b)).is_err());
        assert!(plan(&e, &r, &LedgerRequest::Publish(b.token())).is_err());
        assert_eq!(all(&e), before);
    }
}

#[test]
fn missing_generation_wrong_source_noncanonical_record_and_downgrade_refuse() {
    for damage in 0..7 {
        let (e, r, i, b) = fixture();
        apply(&e, &r, share(&i, &b));
        let mut batch = WriteBatch::new();
        let k = record_key(b.token());
        match damage {
            0 => {
                batch.delete(ColumnFamily::Default, k);
            }
            1..=4 => {
                let mut bytes = e.get(ColumnFamily::Default, &k).unwrap().unwrap();
                bytes.truncate(bytes.len() - 32);
                match damage {
                    1 => bytes[64] ^= 1,
                    2 => bytes[89] = 2,
                    3 => bytes[130..138].copy_from_slice(&u64::MAX.to_be_bytes()),
                    _ => bytes.push(0),
                }
                batch.put(ColumnFamily::Default, k, seal(bytes));
            }
            5 => {
                let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
                h.truncate(h.len() - 32);
                h[..8].copy_from_slice(READER_INDEX_HEADER_MAGIC);
                batch.put(ColumnFamily::Default, key(1, &[]), seal(h));
            }
            _ => {
                batch.put(
                    ColumnFamily::Default,
                    owner_key(b.descriptor.id),
                    encode_owner(&owner(&e, &r, b.descriptor.id)),
                );
            }
        }
        e.write(batch).unwrap();
        assert!(retention_owner(e.snapshot().unwrap().as_ref(), &r, b.descriptor.id).is_err());
        assert!(plan(&e, &r, &finish(&i, &b)).is_err());
        if damage == 5 {
            assert!(plan(&e, &r, &LedgerRequest::Initialize).is_err());
        }
    }
}

#[test]
fn wal_tail_cuts_keep_reader_state_generation_index_and_pins_atomic() {
    for stage in 0..3 {
        let (memory, r, i, b) = fixture();
        if stage > 0 {
            apply(&memory, &r, share(&i, &b));
        }
        if stage > 1 {
            apply(&memory, &r, LedgerRequest::Publish(b.token()));
        }
        let request = match stage {
            0 => share(&i, &b),
            1 => LedgerRequest::Publish(b.token()),
            _ => finish(&i, &b),
        };
        let dir = std::env::temp_dir().join(format!(
            "kv9-reader-generation-{}",
            kv9_common::StoreIncarnation::mint().unwrap()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("meta.wal");
        let (e, _) = WalEngine::open(&path).unwrap();
        let (_, mut batch) = seed();
        for (k, v) in all(&memory) {
            batch.put(ColumnFamily::Default, k, v);
        }
        e.write(batch).unwrap();
        let before = std::fs::read(&path).unwrap().len();
        let next = plan_retention(e.snapshot().unwrap().as_ref(), &r, &request).unwrap();
        e.write(next.into_batch()).unwrap();
        drop(e);
        let complete = std::fs::read(&path).unwrap();
        for cut in [
            before,
            before + 1,
            before + (complete.len() - before) / 2,
            complete.len() - 1,
            complete.len(),
        ] {
            let path = dir.join(format!("cut-{cut}"));
            std::fs::write(&path, &complete[..cut]).unwrap();
            let (e, _) = WalEngine::open(path).unwrap();
            let next = plan_retention(e.snapshot().unwrap().as_ref(), &r, &request).unwrap();
            assert_eq!(next.changed(), cut != complete.len());
            e.write(next.into_batch()).unwrap();
            let record = fact(&e, &r, b.token()).unwrap();
            assert_eq!(
                record.phase(),
                retention_owner(e.snapshot().unwrap().as_ref(), &r, b.descriptor.id)
                    .unwrap()
                    .unwrap()
                    .phase
            );
            assert_eq!(
                retention_owner(e.snapshot().unwrap().as_ref(), &r, i.pending_id())
                    .unwrap()
                    .unwrap()
                    .phase,
                PinPhase::Published
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
