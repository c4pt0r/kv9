use super::tests::{fixture, observe, release};
use super::*;
use crate::retention::tests::{all, apply, owner, plan, seed};
use kv9_engine::{Engine, WalEngine};

fn request(p: &OwnerBinding, e: &OwnerBinding, issued: u64, resolved: u64) -> LedgerRequest {
    LedgerRequest::FinishCheckpointExecution {
        pending: p.token(),
        executor: e.clone(),
        issued,
        resolved,
    }
}

#[test]
fn atomic_finish_covers_every_entered_phase_and_keeps_unknown_puts() {
    for phase in [
        PinPhase::Held,
        PinPhase::Published,
        PinPhase::Quiesced,
        PinPhase::Released,
    ] {
        let (engine, root, p, e) = fixture();
        if phase != PinPhase::Held {
            apply(&engine, &root, LedgerRequest::Publish(e.token()));
        }
        if matches!(phase, PinPhase::Quiesced | PinPhase::Released) {
            apply(
                &engine,
                &root,
                LedgerRequest::QuiesceAfterTransfer {
                    from: e.token(),
                    to: p.token(),
                },
            );
        }
        if phase == PinPhase::Released {
            apply(&engine, &root, LedgerRequest::Release(e.token()));
        }
        apply(
            &engine,
            &root,
            LedgerRequest::CloseCheckpointAdmission(p.token()),
        );
        let issued = if phase == PinPhase::Held { 0 } else { 3 };
        let command = request(&p, &e, issued, 0);
        let before = engine.snapshot().unwrap();
        let revision = apply(&engine, &root, command.clone());
        let report = observe(&engine, &root, &p, &e).unwrap().unwrap();
        assert_eq!(
            (report.issued, report.resolved, report.reported_at),
            (issued, 0, revision)
        );
        assert_eq!(
            owner(&engine, &root, e.descriptor.id).phase,
            PinPhase::Released
        );
        assert_eq!(
            owner(&engine, &root, p.descriptor.id).phase,
            PinPhase::Published
        );
        assert!(!plan(&engine, &root, &command).unwrap().changed());
        assert_eq!(
            retention_owner(before.as_ref(), &root, e.descriptor.id)
                .unwrap()
                .unwrap()
                .phase,
            phase
        );
        assert_eq!(
            checkpoint_drain(before.as_ref(), &root, p.descriptor.id, e.token()).unwrap(),
            None
        );
        assert!(plan(&engine, &root, &request(&p, &e, issued + 1, 0)).is_err());
        assert_eq!(observe(&engine, &root, &p, &e).unwrap(), Some(report));
    }
}

#[test]
fn atomic_finish_cancels_unentered_generations_without_erasing_older_debt() {
    let (engine, root, p, first) = fixture();
    apply(&engine, &root, LedgerRequest::Publish(first.token()));
    apply(&engine, &root, request(&p, &first, 2, 1));
    let unknown = observe(&engine, &root, &p, &first).unwrap().unwrap();
    let mut next = first.clone();
    next.generation = NonZeroU64::new(2).unwrap();
    let intent = checkpoint_intent::checkpoint_intent(
        engine.snapshot().unwrap().as_ref(),
        &root,
        p.descriptor.id,
    )
    .unwrap()
    .unwrap();
    let absent = executor_binding(
        &intent,
        LocalLifetime {
            node: 2,
            store: [3; 16],
            process: [4; 16],
        },
    )
    .unwrap();
    apply(
        &engine,
        &root,
        LedgerRequest::CloseCheckpointAdmission(p.token()),
    );
    for e in [next, absent] {
        assert!(plan(&engine, &root, &request(&p, &e, 1, 0)).is_err());
        apply(&engine, &root, request(&p, &e, 0, 0));
        assert_eq!(
            owner(&engine, &root, e.descriptor.id).phase,
            PinPhase::Released
        );
        assert_eq!(observe(&engine, &root, &p, &e).unwrap().unwrap().issued, 0);
        assert!(plan(
            &engine,
            &root,
            &LedgerRequest::Share {
                from: p.token(),
                to: e.clone()
            }
        )
        .is_err());
        assert!(plan(&engine, &root, &LedgerRequest::Publish(e.token())).is_err());
    }
    assert_eq!(observe(&engine, &root, &p, &first).unwrap(), Some(unknown));
    let before = all(&engine);
    assert!(!plan(&engine, &root, &request(&p, &first, 2, 1))
        .unwrap()
        .changed());
    assert_eq!(
        all(&engine),
        before,
        "old acknowledgement changed the newer generation"
    );
}

#[test]
fn atomic_finish_requires_complete_coverage_exact_binding_and_consistent_counters() {
    let (engine, root, p, e) = fixture();
    let before = all(&engine);
    assert!(
        plan(&engine, &root, &request(&p, &e, 1, 0)).is_err(),
        "Held never authorized I/O"
    );
    assert!(plan(&engine, &root, &request(&p, &e, 0, 1)).is_err());
    for edit in 0..5 {
        let mut wrong = e.clone();
        match edit {
            0 => wrong.generation = NonZeroU64::new(2).unwrap(),
            1 => wrong.descriptor.subject[0] ^= 1,
            2 => wrong.descriptor.kind = OwnerKind::Version,
            3 => wrong.resources.pop().map(|_| ()).unwrap(),
            _ => wrong.descriptor.local.as_mut().unwrap().process[0] ^= 1,
        }
        assert!(plan(&engine, &root, &request(&p, &wrong, 0, 0)).is_err());
    }
    assert_eq!(all(&engine), before);
    apply(&engine, &root, LedgerRequest::Publish(e.token()));
    // Missing global coverage invalidates the entire plan, not just its report.
    let mut batch = WriteBatch::new();
    batch.delete(ColumnFamily::Default, owner_key(p.descriptor.id));
    engine.write(batch).unwrap();
    let before = all(&engine);
    assert!(plan(&engine, &root, &request(&p, &e, 1, 1)).is_err());
    assert_eq!(all(&engine), before);
}

#[test]
fn atomic_finish_accepts_version_handoff_and_keeps_legacy_report_absence_unknown() {
    let (engine, root, p, e) = fixture();
    let intent = checkpoint_intent::checkpoint_intent(
        engine.snapshot().unwrap().as_ref(),
        &root,
        p.descriptor.id,
    )
    .unwrap()
    .unwrap();
    let version = intent.binding(OwnerKind::Version).unwrap();
    apply(&engine, &root, LedgerRequest::Publish(e.token()));
    apply(
        &engine,
        &root,
        LedgerRequest::Share {
            from: p.token(),
            to: version.clone(),
        },
    );
    apply(&engine, &root, LedgerRequest::Publish(version.token()));
    apply(
        &engine,
        &root,
        LedgerRequest::QuiesceAfterTransfer {
            from: p.token(),
            to: version.token(),
        },
    );
    apply(&engine, &root, LedgerRequest::Release(p.token()));
    apply(&engine, &root, request(&p, &e, 1, 1));
    assert_eq!(
        observe(&engine, &root, &p, &e).unwrap().unwrap().resolved,
        1
    );
    assert_eq!(
        owner(&engine, &root, version.descriptor.id).phase,
        PinPhase::Published
    );

    let (engine, root, p, e) = fixture();
    release(&engine, &root, &p, &e);
    assert!(observe(&engine, &root, &p, &e).unwrap().is_none());
    let mut next = e.clone();
    next.generation = NonZeroU64::new(2).unwrap();
    apply(&engine, &root, request(&p, &next, 0, 0));
    assert!(
        observe(&engine, &root, &p, &e).unwrap().is_none(),
        "new report cannot reconstruct the old scope"
    );
}

#[test]
fn atomic_finish_codec_and_wal_reopen_preserve_both_sides_of_commit() {
    let (source, root, p, e) = fixture();
    apply(&source, &root, LedgerRequest::Publish(e.token()));
    let command = request(&p, &e, 7, 5);
    let bytes = encode_request(root.digest(), &command).unwrap();
    assert_eq!(decode_request(&bytes, root.digest()).unwrap(), command);
    for n in 0..bytes.len() {
        assert!(decode_request(&bytes[..n], root.digest()).is_err());
    }
    let mut extra = bytes[..bytes.len() - 32].to_vec();
    extra.push(0);
    assert!(decode_request(&seal(extra), root.digest()).is_err());
    assert!(decode_request(&bytes, RootDigest::from_bytes([9; 32])).is_err());
    let dir = std::env::temp_dir().join(format!(
        "kv9-atomic-drain-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("meta.wal");
    let (wal, _) = WalEngine::open(&path).unwrap();
    let (_, mut initial) = seed();
    for (key, value) in all(&source) {
        initial.put(ColumnFamily::Default, key, value);
    }
    wal.write(initial).unwrap();
    // Planning and abandoning the batch cannot change either durable fact.
    drop(plan(&wal, &root, &command).unwrap());
    drop(wal);
    let (wal, _) = WalEngine::open(&path).unwrap();
    assert_eq!(
        owner(&wal, &root, e.descriptor.id).phase,
        PinPhase::Published
    );
    assert!(checkpoint_drain(
        wal.snapshot().unwrap().as_ref(),
        &root,
        p.descriptor.id,
        e.token()
    )
    .unwrap()
    .is_none());
    let prefix = std::fs::read(&path).unwrap().len();
    wal.write(plan(&wal, &root, &command).unwrap().into_batch())
        .unwrap();
    drop(wal); // No caller receipt or volatile report is retained.
    let complete = std::fs::read(&path).unwrap();
    let appended = complete.len() - prefix;
    assert!(appended > 16);
    for cut in [0, 1, 8, appended / 2, appended - 1, appended] {
        let torn_path = dir.join(format!("cut-{cut}.wal"));
        std::fs::write(&torn_path, &complete[..prefix + cut]).unwrap();
        let (recovered, _) = WalEngine::open(&torn_path).unwrap();
        let view = recovered.snapshot().unwrap();
        let phase = retention_owner(view.as_ref(), &root, e.descriptor.id)
            .unwrap()
            .unwrap()
            .phase;
        let report = checkpoint_drain(view.as_ref(), &root, p.descriptor.id, e.token()).unwrap();
        assert_eq!(
            phase,
            if cut == appended {
                PinPhase::Released
            } else {
                PinPhase::Published
            },
            "cut {cut}"
        );
        assert_eq!(report.is_some(), cut == appended, "cut {cut}");
    }
    let (wal, _) = WalEngine::open(&path).unwrap();
    assert_eq!(
        owner(&wal, &root, e.descriptor.id).phase,
        PinPhase::Released
    );
    let report = checkpoint_drain(
        wal.snapshot().unwrap().as_ref(),
        &root,
        p.descriptor.id,
        e.token(),
    )
    .unwrap()
    .unwrap();
    assert_eq!((report.issued, report.resolved), (7, 5));
    assert!(!plan(&wal, &root, &command).unwrap().changed());
    drop(wal);
    std::fs::remove_dir_all(dir).unwrap();
}
