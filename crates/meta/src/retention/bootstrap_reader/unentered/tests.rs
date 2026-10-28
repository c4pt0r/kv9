use super::*;
use crate::retention::bootstrap_reader::tests::{begin_request, setup};
use crate::retention::tests::{all, apply, owner, plan, seed};
use kv9_engine::{Engine, MemEngine, WalEngine};

fn pending(r: &RootDescriptor, v: &OwnerBinding) -> OwnerToken {
    let LedgerRequest::BeginBootstrapRead { pending, .. } = begin_request(r, v) else {
        panic!()
    };
    pending
}
fn retire(e: &impl Engine, r: &RootDescriptor, v: &OwnerBinding) -> Result<LedgerPlan> {
    plan_verified(
        e.snapshot()?.as_ref(),
        r,
        pending(r, v),
        StoreIdentity::for_voter(r, NodeId(1))?,
    )
}
fn fact(e: &impl Engine, r: &RootDescriptor, b: &OwnerBinding) -> Option<Unentered> {
    observe(e.snapshot().unwrap().as_ref(), r, b.token()).unwrap()
}
fn catalog(e: &MemEngine, r: &RootDescriptor) {
    for _ in 0..1000 {
        match bootstrap_index::observe(e.snapshot().unwrap().as_ref(), r).unwrap() {
            bootstrap_index::Work::Complete => return,
            bootstrap_index::Work::Upgrade => {
                apply(e, r, LedgerRequest::Initialize);
            }
            bootstrap_index::Work::Page { progress, next, .. } => {
                apply(e, r, progress.request(next));
            }
            _ => panic!(),
        }
    }
    panic!("catalog did not finish")
}

#[test]
fn first_independent_unentered_fence_has_exact_origin_but_never_authorizes_read_entry() {
    let (e, r, v, b) = setup();
    let p = pending(&r, &v);
    assert!(plan(
        &e,
        &r,
        &LedgerRequest::RetireBootstrapRead {
            pending: p,
            source: NodeId(1),
            store: StoreIdentity::for_voter(&r, NodeId(1))
                .unwrap()
                .store_incarnation,
        }
    )
    .is_err());
    e.write(retire(&e, &r, &v).unwrap().into_batch()).unwrap();
    let u = fact(&e, &r, &b).unwrap();
    assert_eq!((u.pending, u.version, u.node), (p, v.token(), NodeId(1)));
    assert_eq!(u.checkpoint_operation, v.descriptor.operation);
    assert!(u.fenced_at > 0);
    assert_eq!(owner(&e, &r, b.descriptor.id).phase, PinPhase::Released);
    assert!(
        admission::observe(e.snapshot().unwrap().as_ref(), &r, b.token())
            .unwrap()
            .is_none()
    );
    assert!(!retire(&e, &r, &v).unwrap().changed());
    assert!(plan(&e, &r, &begin_request(&r, &v)).is_err());
    assert!(plan(&e, &r, &LedgerRequest::Publish(b.token())).is_err());
    catalog(&e, &r);
    let page = bootstrap_index::grants(
        e.snapshot().unwrap().as_ref(),
        &r,
        b.descriptor.subject,
        None,
    )
    .unwrap();
    assert_eq!(page.grants[0].unentered, Some(u));
    assert!(page.grants[0].admission.is_none());
    assert_eq!(owner(&e, &r, v.descriptor.id).phase, PinPhase::Published);
    let mut unknown = StoreIdentity::for_voter(&r, NodeId(1)).unwrap();
    unknown.node_id = NodeId(99);
    assert!(plan_verified(e.snapshot().unwrap().as_ref(), &r, p, unknown).is_err());
}

#[test]
fn legacy_held_published_and_released_grants_never_acquire_unentered_provenance() {
    for phase in [PinPhase::Held, PinPhase::Published, PinPhase::Released] {
        let (e, r, v, b) = setup();
        apply(&e, &r, begin_request(&r, &v));
        if phase != PinPhase::Held {
            apply(&e, &r, LedgerRequest::Publish(b.token()));
        }
        if phase == PinPhase::Released {
            e.write(retire(&e, &r, &v).unwrap().into_batch()).unwrap();
        }
        let terminal = owner(&e, &r, b.descriptor.id);
        let mut historical = WriteBatch::new();
        historical.delete(ColumnFamily::Default, key(28, b.descriptor.id.as_bytes()));
        historical.put(
            ColumnFamily::Default,
            owner_key(b.descriptor.id),
            encode_owner(&terminal),
        );
        historical.put(
            ColumnFamily::Default,
            owner_registration_key(b.descriptor.id),
            owner_registration(&b),
        );
        e.write(historical).unwrap();
        e.write(retire(&e, &r, &v).unwrap().into_batch()).unwrap();
        assert!(fact(&e, &r, &b).is_none());
        assert!(
            admission::observe(e.snapshot().unwrap().as_ref(), &r, b.token())
                .unwrap()
                .is_none()
        );
        assert_eq!(owner(&e, &r, b.descriptor.id).phase, PinPhase::Released);
    }
}

#[test]
fn surviving_registration_admission_index_pin_or_terminal_fact_cannot_be_repaired_as_unentered() {
    for fragment in 0..5 {
        let (e, r, v, b) = setup();
        let before = all(&e);
        if fragment == 4 {
            e.write(retire(&e, &r, &v).unwrap().into_batch()).unwrap();
        } else {
            apply(&e, &r, begin_request(&r, &v));
        }
        let mut suffix = b.descriptor.subject.to_vec();
        suffix.extend_from_slice(b.descriptor.id.as_bytes());
        let after = all(&e);
        let wanted = match fragment {
            0 => owner_registration_key(b.descriptor.id),
            1 => key(28, b.descriptor.id.as_bytes()),
            2 => key(29, &suffix),
            3 => after
                .iter()
                .find(|(k, _)| {
                    k.starts_with(&key(6, &[])) && k.ends_with(b.descriptor.id.as_bytes())
                })
                .unwrap()
                .0
                .clone(),
            _ => record_key(b.descriptor.id),
        };
        let value = after.iter().find(|(k, _)| *k == wanted).unwrap().1.clone();
        let mut restore = WriteBatch::new();
        for (k, _) in &after {
            restore.delete(ColumnFamily::Default, k.clone());
        }
        for (k, v) in before {
            restore.put(ColumnFamily::Default, k, v);
        }
        restore.put(ColumnFamily::Default, wanted, value);
        e.write(restore).unwrap();
        let damaged = all(&e);
        assert!(retire(&e, &r, &v).is_err(), "fragment {fragment}");
        assert!(
            plan(&e, &r, &begin_request(&r, &v)).is_err(),
            "entry repaired fragment {fragment}"
        );
        assert_eq!(all(&e), damaged);
    }
}

#[test]
fn unentered_record_damage_missing_origin_and_header_downgrade_fail_without_repair() {
    let (e, r, v, b) = setup();
    e.write(retire(&e, &r, &v).unwrap().into_batch()).unwrap();
    let bytes = e
        .get(ColumnFamily::Default, &record_key(b.descriptor.id))
        .unwrap()
        .unwrap();
    assert_eq!(bytes.len(), 216);
    for cut in 0..bytes.len() {
        assert!(Unentered::decode(&bytes[..cut], r.digest()).is_err());
    }
    let saved = all(&e);
    for damage in 0..6 {
        let mut batch = WriteBatch::new();
        for (k, v) in &saved {
            batch.put(ColumnFamily::Default, k.clone(), v.clone());
        }
        match damage {
            0 => {
                batch.delete(ColumnFamily::Default, record_key(b.descriptor.id));
            }
            1..=3 => {
                let mut changed = bytes[..bytes.len() - 32].to_vec();
                match damage {
                    1 => changed[144..152].copy_from_slice(&u64::MAX.to_be_bytes()),
                    2 => changed[40] ^= 1,
                    _ => changed[152] ^= 1,
                }
                batch.put(
                    ColumnFamily::Default,
                    record_key(b.descriptor.id),
                    seal(changed),
                );
            }
            4 => {
                let owner = LedgerOwner {
                    binding: b.clone(),
                    phase: PinPhase::Released,
                };
                batch.put(
                    ColumnFamily::Default,
                    owner_key(b.descriptor.id),
                    encode_owner(&owner),
                );
                batch.put(
                    ColumnFamily::Default,
                    owner_registration_key(b.descriptor.id),
                    owner_registration(&b),
                );
            }
            _ => {
                let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
                h.truncate(h.len() - 32);
                h[..8].copy_from_slice(BOOTSTRAP_CLOSURE_HEADER_MAGIC);
                batch.put(ColumnFamily::Default, key(1, &[]), seal(h));
            }
        }
        e.write(batch).unwrap();
        let before = all(&e);
        assert!(observe(e.snapshot().unwrap().as_ref(), &r, b.token()).is_err());
        assert!(plan_retry(
            e.snapshot().unwrap().as_ref(),
            &r,
            pending(&r, &v),
            StoreIdentity::for_voter(&r, NodeId(1)).unwrap()
        )
        .is_err());
        assert!(retire(&e, &r, &v).is_err());
        if damage == 5 {
            assert!(plan(&e, &r, &LedgerRequest::Initialize).is_err());
        }
        assert_eq!(all(&e), before);
    }
}

#[test]
fn v16_complete_census_resumes_fourth_pass_and_detects_orphan_unentered_fact() {
    for orphan in [false, true] {
        let (e, r, v, b) = setup();
        catalog(&e, &r);
        let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
        h.truncate(h.len() - 32);
        h[..8].copy_from_slice(BOOTSTRAP_CLOSURE_HEADER_MAGIC);
        let mut progress = e
            .get(ColumnFamily::Default, &key(30, &[]))
            .unwrap()
            .unwrap();
        progress.truncate(progress.len() - 32);
        progress[..8].copy_from_slice(b"KV9BRP01");
        progress[48] = 3;
        progress[49] = 2;
        let mut old = WriteBatch::new();
        old.put(ColumnFamily::Default, key(1, &[]), seal(h));
        old.put(ColumnFamily::Default, key(30, &[]), seal(progress));
        e.write(old).unwrap();
        assert!(matches!(
            bootstrap_index::observe(e.snapshot().unwrap().as_ref(), &r).unwrap(),
            bootstrap_index::Work::Upgrade
        ));
        apply(&e, &r, LedgerRequest::Initialize);
        let bootstrap_index::Work::Page { progress, .. } =
            bootstrap_index::observe(e.snapshot().unwrap().as_ref(), &r).unwrap()
        else {
            panic!()
        };
        assert_eq!(progress.stage, 3);
        e.write(retire(&e, &r, &v).unwrap().into_batch()).unwrap();
        if orphan {
            let mut corrupt = WriteBatch::new();
            corrupt.delete(ColumnFamily::Default, owner_key(b.descriptor.id));
            corrupt.delete(
                ColumnFamily::Default,
                owner_registration_key(b.descriptor.id),
            );
            e.write(corrupt).unwrap();
        }
        let bootstrap_index::Work::Page {
            progress,
            next,
            examined,
        } = bootstrap_index::observe(e.snapshot().unwrap().as_ref(), &r).unwrap()
        else {
            panic!()
        };
        assert_eq!(examined, 1);
        let request = progress.request(next);
        assert_eq!(
            decode_request(&encode_request(r.digest(), &request).unwrap(), r.digest()).unwrap(),
            request
        );
        let next = plan(&e, &r, &request);
        if orphan {
            assert!(next.is_err());
        } else {
            e.write(next.unwrap().into_batch()).unwrap();
            assert!(matches!(
                bootstrap_index::observe(e.snapshot().unwrap().as_ref(), &r).unwrap(),
                bootstrap_index::Work::Complete
            ));
            assert!(!plan(&e, &r, &request).unwrap().changed());
        }
    }
}

#[test]
fn late_unentered_fence_after_read_gate_restarts_empty_census_without_inventing_admission() {
    use crate::retention::metadata_read_gate::tests::{fixture, select, step};
    let (e, r, old, new) = fixture(1);
    select(&e, &r, &new, 1);
    step(&e, &r, &old);
    catalog(&e, &r);
    let observe = || {
        bootstrap_closure::observe(e.snapshot().unwrap().as_ref(), &r, old.pending_id()).unwrap()
    };
    let bootstrap_closure::Work::Advance { cursor, .. } = observe() else {
        panic!()
    };
    apply(&e, &r, cursor.request(old.pending_id()));
    assert!(observe().registered_grants_retired());
    let v = old.binding(OwnerKind::Version).unwrap();
    e.write(retire(&e, &r, &v).unwrap().into_batch()).unwrap();
    let bootstrap_closure::Work::Advance {
        cursor,
        restart: true,
    } = observe()
    else {
        panic!()
    };
    apply(&e, &r, cursor.request(old.pending_id()));
    let bootstrap_closure::Work::Complete(c) = observe() else {
        panic!()
    };
    assert_eq!(
        (c.counts.grants, c.counts.retired, c.counts.legacy),
        (1, 1, 0)
    );
    assert_eq!(owner(&e, &r, v.descriptor.id).phase, PinPhase::Published);
}

#[test]
fn wal_tail_cuts_preserve_unentered_fact_owner_pins_index_and_invalidation_atomically() {
    let (memory, r, v, b) = setup();
    catalog(&memory, &r);
    let dir = std::env::temp_dir().join(format!(
        "kv9-bootstrap-unentered-{}",
        StoreIncarnation::mint().unwrap()
    ));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("base");
    let (e, _) = WalEngine::open(&path).unwrap();
    let (_, mut batch) = seed();
    for (k, v) in all(&memory) {
        batch.put(ColumnFamily::Default, k, v);
    }
    e.write(batch).unwrap();
    let size = std::fs::read(&path).unwrap().len();
    let stamp_key = key(33, &b.descriptor.subject);
    let old_stamp = e.get(ColumnFamily::Default, &stamp_key).unwrap();
    e.write(retire(&e, &r, &v).unwrap().into_batch()).unwrap();
    let new_stamp = e.get(ColumnFamily::Default, &stamp_key).unwrap();
    assert_ne!(old_stamp, new_stamp);
    drop(e);
    let bytes = std::fs::read(&path).unwrap();
    for cut in [
        size,
        size + 1,
        size + (bytes.len() - size) / 2,
        bytes.len() - 1,
        bytes.len(),
    ] {
        let p = dir.join(format!("cut-{cut}"));
        std::fs::write(&p, &bytes[..cut]).unwrap();
        let (e, _) = WalEngine::open(p).unwrap();
        assert_eq!(
            e.get(ColumnFamily::Default, &stamp_key).unwrap(),
            if cut == bytes.len() {
                new_stamp.clone()
            } else {
                old_stamp.clone()
            }
        );
        let page = bootstrap_index::grants(
            e.snapshot().unwrap().as_ref(),
            &r,
            b.descriptor.subject,
            None,
        )
        .unwrap();
        assert_eq!(page.grants.len(), usize::from(cut == bytes.len()));
        let next = retire(&e, &r, &v).unwrap();
        assert_eq!(next.changed(), cut != bytes.len());
        e.write(next.into_batch()).unwrap();
        assert!(fact(&e, &r, &b).is_some());
        assert!(!retire(&e, &r, &v).unwrap().changed());
        assert_eq!(owner(&e, &r, v.descriptor.id).phase, PinPhase::Published);
    }
    std::fs::remove_dir_all(dir).unwrap();
}
