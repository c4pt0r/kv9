use super::super::tests::{begin_request, setup};
use super::*;
use crate::retention::tests::{all, apply, owner, plan, seed};
use kv9_engine::{Engine, WalEngine};

fn pending(request: &LedgerRequest) -> OwnerToken {
    let LedgerRequest::BeginBootstrapRead { pending, .. } = request else {
        panic!()
    };
    *pending
}
fn fact(e: &impl Engine, r: &RootDescriptor, b: &OwnerBinding) -> Option<Admission> {
    observe(e.snapshot().unwrap().as_ref(), r, b.token()).unwrap()
}
fn transfer_version(e: &kv9_engine::MemEngine, r: &RootDescriptor, version: &OwnerBinding) {
    let mut successor = version.clone();
    successor.descriptor.id = OwnerId::new([87; 16]).unwrap();
    successor.descriptor.kind = OwnerKind::Snapshot;
    apply(
        e,
        r,
        LedgerRequest::Share {
            from: version.token(),
            to: successor.clone(),
        },
    );
    apply(e, r, LedgerRequest::Publish(successor.token()));
    apply(
        e,
        r,
        LedgerRequest::QuiesceAfterTransfer {
            from: version.token(),
            to: successor.token(),
        },
    );
    apply(e, r, LedgerRequest::Release(version.token()));
}

#[test]
fn typed_bootstrap_admission_binds_exact_source_and_store_and_blocks_generic_bypass() {
    let (e, r, v, b) = setup();
    let begin = begin_request(&r, &v);
    let before = all(&e);
    for mutate in 0..5 {
        let mut bad = begin.clone();
        let LedgerRequest::BeginBootstrapRead {
            pending,
            source,
            store,
        } = &mut bad
        else {
            panic!()
        };
        match mutate {
            0 => pending.owner = OwnerId::new([55; 16]).unwrap(),
            1 => pending.generation = NonZeroU64::new(2).unwrap(),
            2 => *store = StoreIncarnation::from_bytes([9; 16]),
            3 => *source = NodeId(9), // root-bound data joiner is not a metadata voter
            _ => *pending = v.token(),
        }
        assert!(plan(&e, &r, &bad).is_err());
        assert_eq!(all(&e), before);
    }
    for from in [pending(&begin), v.token()] {
        for forged in 0..3 {
            let mut target = b.clone();
            if forged == 1 {
                target.descriptor.operation = [55; 32];
            }
            if forged == 2 {
                target.descriptor.id = OwnerId::new([56; 16]).unwrap();
            }
            assert!(plan(&e, &r, &LedgerRequest::Acquire(target.clone())).is_err());
            assert!(plan(&e, &r, &LedgerRequest::Share { from, to: target }).is_err());
        }
    }
    assert_eq!(all(&e), before);
    assert_eq!(
        decode_request(&encode_request(r.digest(), &begin).unwrap(), r.digest()).unwrap(),
        begin
    );
    apply(&e, &r, begin.clone());
    let a = fact(&e, &r, &b).unwrap();
    assert_eq!(a.pending, pending(&begin));
    assert_eq!(a.version, v.token());
    assert_eq!(a.node, NodeId(1));
    assert_eq!(
        a.store,
        StoreIdentity::for_voter(&r, NodeId(1))
            .unwrap()
            .store_incarnation
    );
    assert!(a.entered_at > 0);
    assert!(!plan(&e, &r, &begin).unwrap().changed());
    apply(&e, &r, LedgerRequest::Publish(b.token()));
    assert!(!plan(&e, &r, &LedgerRequest::Publish(b.token()))
        .unwrap()
        .changed());
    assert_eq!(fact(&e, &r, &b), Some(a));
}

#[test]
fn source_is_rechecked_before_publish_and_before_acknowledging_entry_retry() {
    for publish in [false, true] {
        let (e, r, v, b) = setup();
        let begin = begin_request(&r, &v);
        apply(&e, &r, begin.clone());
        if publish {
            apply(&e, &r, LedgerRequest::Publish(b.token()));
        }
        transfer_version(&e, &r, &v);
        let before = all(&e);
        assert!(plan(&e, &r, &begin).is_err());
        assert!(plan(&e, &r, &LedgerRequest::Publish(b.token())).is_err());
        assert_eq!(all(&e), before);
        assert_eq!(
            owner(&e, &r, b.descriptor.id).phase,
            if publish {
                PinPhase::Published
            } else {
                PinPhase::Held
            }
        );
    }
}

#[test]
fn old_bootstrap_protection_is_readable_and_retirable_but_never_retroactively_admitted() {
    for publish in [false, true] {
        let (e, r, v, b) = setup();
        let begin = begin_request(&r, &v);
        apply(&e, &r, begin.clone());
        if publish {
            apply(&e, &r, LedgerRequest::Publish(b.token()));
        }
        let old = owner(&e, &r, b.descriptor.id);
        let mut batch = WriteBatch::new();
        batch.delete(ColumnFamily::Default, record_key(b.descriptor.id));
        // Explicit legacy fixture has neither typed origins nor the later index.
        for (k, _) in all(&e) {
            if [29, 30, 33, 34]
                .iter()
                .any(|tag| k.starts_with(&key(*tag, &[])))
            {
                batch.delete(ColumnFamily::Default, k);
            }
        }
        batch.put(
            ColumnFamily::Default,
            owner_key(b.descriptor.id),
            encode_owner(&old),
        );
        batch.put(
            ColumnFamily::Default,
            owner_registration_key(b.descriptor.id),
            owner_registration(&b),
        );
        let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
        h.truncate(h.len() - 32);
        h[..8].copy_from_slice(READER_CLOSURE_HEADER_MAGIC);
        batch.put(ColumnFamily::Default, key(1, &[]), seal(h));
        e.write(batch).unwrap();
        assert_eq!(owner(&e, &r, b.descriptor.id), old);
        assert_eq!(fact(&e, &r, &b), None);
        let before = all(&e);
        assert!(plan(&e, &r, &begin).is_err());
        assert!(plan(&e, &r, &LedgerRequest::Publish(b.token())).is_err());
        assert_eq!(all(&e), before);
        apply(&e, &r, LedgerRequest::Initialize);
        assert_eq!(fact(&e, &r, &b), None);
        assert_eq!(owner(&e, &r, b.descriptor.id), old);
        let identity = StoreIdentity::for_voter(&r, NodeId(1)).unwrap();
        e.write(
            plan_verified(
                e.snapshot().unwrap().as_ref(),
                &r,
                pending(&begin),
                identity,
            )
            .unwrap()
            .into_batch(),
        )
        .unwrap();
        assert_eq!(owner(&e, &r, b.descriptor.id).phase, PinPhase::Released);
        assert_eq!(fact(&e, &r, &b), None);
    }
}

#[test]
fn managed_terminal_acknowledgement_keeps_origin_without_requiring_archived_source() {
    let (e, r, v, b) = setup();
    let begin = begin_request(&r, &v);
    apply(&e, &r, begin.clone());
    apply(&e, &r, LedgerRequest::Publish(b.token()));
    let original = fact(&e, &r, &b).unwrap();
    let identity = StoreIdentity::for_voter(&r, NodeId(1)).unwrap();
    e.write(
        plan_verified(
            e.snapshot().unwrap().as_ref(),
            &r,
            pending(&begin),
            identity,
        )
        .unwrap()
        .into_batch(),
    )
    .unwrap();
    transfer_version(&e, &r, &v);
    // Test-only history removal: no new production GC capability.
    let mut batch = WriteBatch::new();
    batch.delete(
        ColumnFamily::Default,
        checkpoint_intent::intent_key(pending(&begin).owner),
    );
    batch.delete(ColumnFamily::Default, owner_key(v.descriptor.id));
    batch.delete(
        ColumnFamily::Default,
        owner_registration_key(v.descriptor.id),
    );
    e.write(batch).unwrap();
    let before = all(&e);
    assert!(!plan_retry(
        e.snapshot().unwrap().as_ref(),
        &r,
        pending(&begin),
        identity
    )
    .unwrap()
    .changed());
    assert_eq!(fact(&e, &r, &b), Some(original));
    assert!(plan(&e, &r, &begin).is_err());
    assert!(plan(&e, &r, &LedgerRequest::Publish(b.token())).is_err());
    assert_eq!(all(&e), before);
}

#[test]
fn admission_loss_corruption_or_header_downgrade_cannot_fall_back_to_legacy() {
    for damage in 0..11 {
        let (e, r, v, b) = setup();
        let begin = begin_request(&r, &v);
        apply(&e, &r, begin.clone());
        let mut batch = WriteBatch::new();
        let k = record_key(b.descriptor.id);
        match damage {
            0 => {
                batch.delete(ColumnFamily::Default, k);
            }
            1..=7 => {
                let mut bytes = e.get(ColumnFamily::Default, &k).unwrap().unwrap();
                bytes.truncate(bytes.len() - 32);
                match damage {
                    1 => bytes[40] ^= 1,  // Pending
                    2 => bytes[96] ^= 1,  // store
                    3 => bytes[112] ^= 1, // immutable owner digest
                    4 => bytes[144..152].copy_from_slice(&u64::MAX.to_be_bytes()),
                    5 => bytes[64] ^= 1,  // Version
                    6 => bytes[152] ^= 1, // canonical checkpoint operation
                    _ => bytes.push(0),
                }
                batch.put(ColumnFamily::Default, k, seal(bytes));
            }
            8 => {
                let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
                h.truncate(h.len() - 32);
                h[..8].copy_from_slice(READER_CLOSURE_HEADER_MAGIC);
                batch.put(ColumnFamily::Default, key(1, &[]), seal(h));
            }
            9 => {
                batch.put(
                    ColumnFamily::Default,
                    owner_key(b.descriptor.id),
                    encode_owner(&owner(&e, &r, b.descriptor.id)),
                );
            }
            _ => {
                batch.delete(ColumnFamily::Default, owner_key(b.descriptor.id));
                batch.delete(
                    ColumnFamily::Default,
                    owner_registration_key(b.descriptor.id),
                );
            }
        }
        e.write(batch).unwrap();
        let before = all(&e);
        assert!(observe(e.snapshot().unwrap().as_ref(), &r, b.token()).is_err());
        assert!(plan(&e, &r, &begin).is_err());
        assert!(plan(&e, &r, &LedgerRequest::Publish(b.token())).is_err());
        if damage == 8 {
            assert!(plan(&e, &r, &LedgerRequest::Initialize).is_err());
        }
        assert_eq!(all(&e), before);
    }
}

#[test]
fn wal_tail_cuts_keep_bootstrap_origin_owner_and_pins_atomic_through_retirement() {
    for stage in 0..3 {
        let (memory, r, v, b) = setup();
        let begin = begin_request(&r, &v);
        let identity = StoreIdentity::for_voter(&r, NodeId(1)).unwrap();
        if stage > 0 {
            apply(&memory, &r, begin.clone());
        }
        if stage > 1 {
            apply(&memory, &r, LedgerRequest::Publish(b.token()));
        }
        let dir = std::env::temp_dir().join(format!(
            "kv9-bootstrap-admission-{}",
            StoreIncarnation::mint().unwrap()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("meta.wal");
        let (e, _) = WalEngine::open(&path).unwrap();
        let (_, mut batch) = seed();
        for (k, v) in all(&memory) {
            batch.put(ColumnFamily::Default, k, v);
        }
        e.write(batch).unwrap();
        let size = std::fs::read(&path).unwrap().len();
        let publish = LedgerRequest::Publish(b.token());
        let next = |e: &WalEngine| {
            let view = e.snapshot().unwrap();
            if stage == 2 {
                plan_verified(view.as_ref(), &r, pending(&begin), identity)
            } else {
                plan_retention(
                    view.as_ref(),
                    &r,
                    if stage == 0 { &begin } else { &publish },
                )
            }
        };
        e.write(next(&e).unwrap().into_batch()).unwrap();
        drop(e);
        let bytes = std::fs::read(&path).unwrap();
        for cut in [
            size,
            size + 1,
            size + (bytes.len() - size) / 2,
            bytes.len() - 1,
            bytes.len(),
        ] {
            let path = dir.join(format!("cut-{cut}"));
            std::fs::write(&path, &bytes[..cut]).unwrap();
            let (e, _) = WalEngine::open(path).unwrap();
            let plan = next(&e).unwrap();
            assert_eq!(plan.changed(), cut != bytes.len());
            e.write(plan.into_batch()).unwrap();
            let a = fact(&e, &r, &b).unwrap();
            assert_eq!(a.version, v.token());
            assert_eq!(
                owner(&e, &r, b.descriptor.id).phase,
                [PinPhase::Held, PinPhase::Published, PinPhase::Released][stage]
            );
            assert_eq!(owner(&e, &r, v.descriptor.id).phase, PinPhase::Published);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn orphan_origin_cannot_reconstruct_missing_owner_and_resource_protection() {
    let (e, r, v, b) = setup();
    let begin = begin_request(&r, &v);
    apply(&e, &r, begin.clone());
    let (missing, _, _, _) = setup();
    let mut batch = WriteBatch::new();
    // Keep the post-admission revision and origin only, without any bootstrap
    // owner, registration or forward pins. This is damage, never empty history.
    for k in [record_key(b.descriptor.id), key(1, &[])] {
        batch.put(
            ColumnFamily::Default,
            k.clone(),
            e.get(ColumnFamily::Default, &k).unwrap().unwrap(),
        );
    }
    missing.write(batch).unwrap();
    let before = all(&missing);
    assert!(plan(&missing, &r, &begin).is_err());
    assert!(plan(&missing, &r, &LedgerRequest::Publish(b.token())).is_err());
    assert!(plan_verified(
        missing.snapshot().unwrap().as_ref(),
        &r,
        pending(&begin),
        StoreIdentity::for_voter(&r, NodeId(1)).unwrap()
    )
    .is_err());
    assert_eq!(all(&missing), before);
}
