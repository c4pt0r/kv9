use super::tests::fixture;
use super::*;
use crate::retention::tests::{all, apply, owner, plan};
use kv9_engine::{Engine, MemEngine};

fn finish(p: &OwnerBinding, b: &OwnerBinding) -> LedgerRequest {
    LedgerRequest::FinishCheckpointExecution {
        pending: p.token(),
        executor: b.clone(),
        issued: 7,
        resolved: 2,
    }
}
fn transfer(e: &MemEngine, r: &RootDescriptor, from: &OwnerBinding, to: &OwnerBinding) {
    apply(
        e,
        r,
        LedgerRequest::Share {
            from: from.token(),
            to: to.clone(),
        },
    );
    apply(e, r, LedgerRequest::Publish(to.token()));
    apply(
        e,
        r,
        LedgerRequest::QuiesceAfterTransfer {
            from: from.token(),
            to: to.token(),
        },
    );
    apply(e, r, LedgerRequest::Release(from.token()));
}
fn retire_sources(e: &MemEngine, r: &RootDescriptor, p: &OwnerBinding) -> OwnerBinding {
    let intent =
        checkpoint_intent::checkpoint_intent(e.snapshot().unwrap().as_ref(), r, p.descriptor.id)
            .unwrap()
            .unwrap();
    let v = intent.binding(OwnerKind::Version).unwrap();
    transfer(e, r, p, &v);
    let mut holder = v.clone();
    holder.descriptor.kind = OwnerKind::Snapshot;
    holder.descriptor.id = OwnerId::new([96; 16]).unwrap();
    // Preserve the exact same image under a real catalog transfer. This is not
    // newer-image recovery authority and does not grant object deletion.
    transfer(e, r, &v, &holder);
    v
}

#[test]
fn terminal_build_ack_survives_source_transfer_and_new_active_generation() {
    let (e, r, p, b) = fixture();
    apply(&e, &r, LedgerRequest::Publish(b.token()));
    apply(&e, &r, finish(&p, &b));
    let report = checkpoint_drain(
        e.snapshot().unwrap().as_ref(),
        &r,
        p.descriptor.id,
        b.token(),
    )
    .unwrap()
    .unwrap();
    let mut next = b.clone();
    next.generation = NonZeroU64::new(2).unwrap();
    apply(
        &e,
        &r,
        LedgerRequest::Share {
            from: p.token(),
            to: next.clone(),
        },
    );
    apply(&e, &r, LedgerRequest::Publish(next.token()));
    let v = retire_sources(&e, &r, &p);
    let before = all(&e);
    assert!(!plan(&e, &r, &finish(&p, &b))
        .expect("terminal Build acknowledgement still needs Published source")
        .changed());
    assert!(!plan(
        &e,
        &r,
        &LedgerRequest::RecordCheckpointDrain {
            pending: p.token(),
            executor: b.token(),
            issued: 7,
            resolved: 2
        }
    )
    .unwrap()
    .changed());
    assert_eq!(all(&e), before);
    assert_eq!(owner(&e, &r, b.descriptor.id).binding, next);
    assert_eq!(owner(&e, &r, b.descriptor.id).phase, PinPhase::Published);
    assert!(
        plan(&e, &r, &finish(&p, &next)).is_err(),
        "first finish still requires exact Published source"
    );
    let mut archived = WriteBatch::new();
    for s in [&p, &v] {
        archived.delete(ColumnFamily::Default, owner_key(s.descriptor.id));
        archived.delete(
            ColumnFamily::Default,
            owner_registration_key(s.descriptor.id),
        );
    }
    // Simulated future source archive only. Keep the Build's own registration,
    // pins, gate, canonical intent and exact report; no production archiver.
    e.write(archived).unwrap();
    assert!(!plan(&e, &r, &finish(&p, &b)).unwrap().changed());
    assert_eq!(
        checkpoint_drain(
            e.snapshot().unwrap().as_ref(),
            &r,
            p.descriptor.id,
            b.token()
        )
        .unwrap(),
        Some(report.clone())
    );
    let history = enrollment::history(
        e.snapshot().unwrap().as_ref(),
        &r,
        p.descriptor.id,
        b.token(),
        None,
    )
    .unwrap();
    assert_eq!(
        history.generations[0].state,
        enrollment::GenerationState::Drained {
            issued: 7,
            resolved: 2
        }
    );
    // The retained report, newer active generation and historical lookup must
    // survive real disk reopen, not just an in-memory source transfer.
    let directory = std::env::temp_dir().join(format!(
        "kv9-terminal-build-{}",
        kv9_common::StoreIncarnation::mint().unwrap()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("meta.wal");
    let (wal, _) = kv9_engine::WalEngine::open(&path).unwrap();
    let (_, mut batch) = crate::retention::tests::seed();
    for (k, v) in all(&e) {
        batch.put(ColumnFamily::Default, k, v);
    }
    wal.write(batch).unwrap();
    drop(wal);
    let bytes = std::fs::read(&path).unwrap();
    let (wal, _) = kv9_engine::WalEngine::open(&path).unwrap();
    assert!(!plan(&wal, &r, &finish(&p, &b)).unwrap().changed());
    assert_eq!(
        checkpoint_drain(
            wal.snapshot().unwrap().as_ref(),
            &r,
            p.descriptor.id,
            b.token()
        )
        .unwrap(),
        Some(report)
    );
    assert_eq!(owner(&wal, &r, b.descriptor.id).binding, next);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    drop(wal);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn terminal_build_ack_rejects_changed_counters_and_damaged_own_evidence() {
    for damage in 0..8 {
        let (e, r, p, b) = fixture();
        apply(&e, &r, LedgerRequest::Publish(b.token()));
        apply(&e, &r, finish(&p, &b));
        retire_sources(&e, &r, &p);
        for which in 0..6 {
            let mut request = finish(&p, &b);
            if let LedgerRequest::FinishCheckpointExecution {
                pending,
                executor,
                issued,
                resolved,
            } = &mut request
            {
                match which {
                    0 => *issued += 1,
                    1 => *resolved += 1,
                    2 => pending.owner = OwnerId::new([97; 16]).unwrap(),
                    3 => pending.generation = NonZeroU64::new(2).unwrap(),
                    4 => executor.descriptor.local.as_mut().unwrap().process[0] ^= 1,
                    _ => executor.generation = NonZeroU64::new(2).unwrap(),
                }
            }
            assert!(plan(&e, &r, &request).is_err());
        }
        let mut batch = WriteBatch::new();
        let key = match damage {
            0 => owner_key(b.descriptor.id),
            1 => owner_registration_key(b.descriptor.id),
            2 => resource_key(&b.resources[0]),
            3 => index_key(p.descriptor.id, b.descriptor.id),
            4 => record_key(p.descriptor.id, b.token()),
            5 => gate_key(p.descriptor.id),
            6 => key(8, p.descriptor.id.as_bytes()),
            _ => {
                let key = record_key(p.descriptor.id, b.token());
                let mut bytes = e.get(ColumnFamily::Default, &key).unwrap().unwrap();
                bytes[72] ^= 1;
                batch.put(ColumnFamily::Default, key.clone(), bytes);
                e.write(batch).unwrap();
                assert!(plan(&e, &r, &finish(&p, &b)).is_err());
                continue;
            }
        };
        batch.delete(ColumnFamily::Default, key);
        e.write(batch).unwrap();
        let before = all(&e);
        assert!(plan(&e, &r, &finish(&p, &b)).is_err(), "damage {damage}");
        assert_eq!(all(&e), before);
    }
}

#[test]
fn terminal_build_ack_does_not_create_missing_reports_or_authorize_new_io() {
    for phase in [PinPhase::Held, PinPhase::Published, PinPhase::Released] {
        let (e, r, p, b) = fixture();
        if phase != PinPhase::Held {
            apply(&e, &r, LedgerRequest::Publish(b.token()));
        }
        if phase == PinPhase::Released {
            super::tests::release(&e, &r, &p, &b);
        }
        retire_sources(&e, &r, &p);
        assert!(checkpoint_drain(
            e.snapshot().unwrap().as_ref(),
            &r,
            p.descriptor.id,
            b.token()
        )
        .unwrap()
        .is_none());
        let before = all(&e);
        assert!(plan(&e, &r, &finish(&p, &b)).is_err());
        assert!(plan(
            &e,
            &r,
            &LedgerRequest::RecordCheckpointDrain {
                pending: p.token(),
                executor: b.token(),
                issued: 0,
                resolved: 0
            }
        )
        .is_err());
        assert!(plan(&e, &r, &LedgerRequest::Publish(b.token())).is_err());
        let mut next = b.clone();
        next.generation = NonZeroU64::new(2).unwrap();
        assert!(plan(&e, &r, &LedgerRequest::Acquire(next)).is_err());
        assert_eq!(all(&e), before);
    }
}
