use super::*;
use crate::retention::checkpoint_intent::tests::fixture_intent;
use crate::retention::tests::{all, apply, owner, plan, seed};
use kv9_engine::{Engine, MemEngine, WalEngine};

fn fixture() -> (MemEngine, RootDescriptor, OwnerBinding, OwnerBinding) {
    let (e, r, i) = fixture_intent();
    apply(&e, &r, LedgerRequest::BeginCheckpoint(i.clone()));
    let mut source = i.binding(OwnerKind::Pending).unwrap();
    source.descriptor.kind = OwnerKind::Migration;
    source.descriptor.id = OwnerId::new([91; 16]).unwrap();
    apply(&e, &r, LedgerRequest::Acquire(source.clone()));
    apply(&e, &r, LedgerRequest::Publish(source.token()));
    let b = binding(
        &source,
        LocalLifetime {
            node: 1,
            store: [3; 16],
            process: [7; 16],
        },
    )
    .unwrap();
    (e, r, source, b)
}
fn finish(source: &OwnerBinding, reader: &OwnerBinding) -> LedgerRequest {
    LedgerRequest::FinishImageRead {
        successor: source.token(),
        reader: reader.clone(),
    }
}

fn enter(e: &MemEngine, r: &RootDescriptor, i: &OwnerBinding, b: &OwnerBinding) {
    apply(
        e,
        r,
        LedgerRequest::Share {
            from: i.token(),
            to: b.clone(),
        },
    );
    apply(e, r, LedgerRequest::Publish(b.token()));
}

#[test]
fn drained_reader_finish_is_atomic_idempotent_and_fences_unconfirmed_entry_generations() {
    let (e, r, i, mut b) = fixture();
    for mode in 0..3 {
        if mode != 1 {
            enter(&e, &r, &i, &b);
        }
        let request = finish(&i, &b);
        let encoded = encode_request(r.digest(), &request).unwrap();
        assert_eq!(decode_request(&encoded, r.digest()).unwrap(), request);
        for cut in 0..encoded.len() {
            assert!(decode_request(&encoded[..cut], r.digest()).is_err());
        }
        apply(&e, &r, request.clone());
        assert!(!plan(&e, &r, &request).unwrap().changed());
        assert_eq!(owner(&e, &r, b.descriptor.id).phase, PinPhase::Released);
        assert!(plan(
            &e,
            &r,
            &LedgerRequest::Share {
                from: i.token(),
                to: b.clone()
            }
        )
        .is_err());
        assert!(plan(&e, &r, &LedgerRequest::Publish(b.token())).is_err());
        assert_eq!(owner(&e, &r, i.descriptor.id).phase, PinPhase::Published);
        let stale = request;
        b.generation = NonZeroU64::new(b.generation.get() + 1).unwrap();
        if mode == 2 {
            enter(&e, &r, &i, &b);
            assert!(plan(&e, &r, &stale).is_err());
            assert_eq!(owner(&e, &r, b.descriptor.id).phase, PinPhase::Published);
        }
    }
    let (e, r, i, b) = fixture();
    apply(&e, &r, finish(&i, &b)); // First Share never committed.
    assert_eq!(owner(&e, &r, b.descriptor.id).phase, PinPhase::Released);
}

#[test]
fn wrong_scope_generation_lifetime_and_unpublished_successors_cannot_finish_readers() {
    let (e, r, i, b) = fixture();
    for kind in [
        OwnerKind::Pending,
        OwnerKind::Version,
        OwnerKind::Build,
        OwnerKind::Publisher,
    ] {
        let mut invalid_source = i.clone();
        invalid_source.descriptor.kind = kind;
        assert!(binding(&invalid_source, b.descriptor.local.unwrap()).is_err());
    }
    enter(&e, &r, &i, &b);
    let before = all(&e);
    for mode in 0..7 {
        let mut wrong = b.clone();
        match mode {
            0 => wrong.descriptor.kind = OwnerKind::Build,
            1 => wrong.descriptor.local.as_mut().unwrap().process = [9; 16],
            2 => wrong.generation = NonZeroU64::new(3).unwrap(),
            3 => wrong.descriptor.subject = [8; 32],
            4 => wrong.descriptor.operation = [8; 32],
            5 => wrong.resources.pop().map(|_| ()).unwrap(),
            _ => wrong.descriptor.local = None,
        }
        assert!(plan(&e, &r, &finish(&i, &wrong)).is_err());
    }
    let mut request = finish(&i, &b);
    if let LedgerRequest::FinishImageRead { successor, .. } = &mut request {
        successor.generation = NonZeroU64::new(2).unwrap();
    }
    assert!(plan(&e, &r, &request).is_err());
    assert_eq!(all(&e), before);
    let mut version = i.clone();
    version.descriptor.kind = OwnerKind::Snapshot;
    version.descriptor.id = OwnerId::new([92; 16]).unwrap();
    apply(
        &e,
        &r,
        LedgerRequest::Share {
            from: i.token(),
            to: version.clone(),
        },
    );
    let reader = binding(&version, b.descriptor.local.unwrap()).unwrap();
    assert!(plan(
        &e,
        &r,
        &LedgerRequest::FinishImageRead {
            successor: version.token(),
            reader
        }
    )
    .is_err());
}

#[test]
fn image_reader_finish_keeps_source_published_and_wal_reopen_keeps_all_pins_atomic() {
    let (memory, r, i, b) = fixture();
    enter(&memory, &r, &i, &b);
    let dir = std::env::temp_dir().join(format!(
        "kv9-reader-{}",
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
    let before = std::fs::read(&path).unwrap();
    e.write(plan(&e, &r, &finish(&i, &b)).unwrap().into_batch())
        .unwrap();
    drop(e);
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
        let (e, _) = WalEngine::open(&path).unwrap();
        assert_eq!(
            owner(&e, &r, b.descriptor.id).phase,
            if cut == after.len() {
                PinPhase::Released
            } else {
                PinPhase::Published
            }
        );
        assert_eq!(owner(&e, &r, i.descriptor.id).phase, PinPhase::Published);
        e.write(plan(&e, &r, &finish(&i, &b)).unwrap().into_batch())
            .unwrap();
        assert!(!plan(&e, &r, &finish(&i, &b)).unwrap().changed());
    }
    std::fs::remove_dir_all(dir).unwrap();
}
