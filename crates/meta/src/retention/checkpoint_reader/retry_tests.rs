use super::*;
use crate::retention::checkpoint_intent::tests::fixture_intent;
use crate::retention::tests::{all, apply, owner, plan};
use kv9_engine::{Engine, MemEngine};

fn fixture(mode: u8) -> (MemEngine, RootDescriptor, OwnerBinding, OwnerBinding) {
    let (e, r, intent) = fixture_intent();
    apply(&e, &r, LedgerRequest::BeginCheckpoint(intent.clone()));
    let pending = intent.binding(OwnerKind::Pending).unwrap();
    let mut source = match mode {
        0 => pending.clone(),
        _ => intent.binding(OwnerKind::Version).unwrap(),
    };
    if mode >= 2 {
        source.descriptor.kind = OwnerKind::Snapshot;
        source.descriptor.id = OwnerId::new([91; 16]).unwrap();
    }
    if mode > 0 {
        apply(
            &e,
            &r,
            LedgerRequest::Share {
                from: pending.token(),
                to: source.clone(),
            },
        );
        apply(&e, &r, LedgerRequest::Publish(source.token()));
    }
    let lifetime = LocalLifetime {
        node: 1,
        store: [3; 16],
        process: [7; 16],
    };
    let b = if mode < 2 {
        binding(&source, lifetime)
    } else {
        image_reader::binding(&source, lifetime)
    }
    .unwrap();
    (e, r, source, b)
}
fn request(source: &OwnerBinding, reader: &OwnerBinding) -> LedgerRequest {
    if matches!(
        source.descriptor.kind,
        OwnerKind::Pending | OwnerKind::Version
    ) {
        LedgerRequest::FinishCheckpointRead {
            successor: source.token(),
            reader: reader.clone(),
        }
    } else {
        LedgerRequest::FinishImageRead {
            successor: source.token(),
            reader: reader.clone(),
        }
    }
}
fn enter(e: &MemEngine, r: &RootDescriptor, source: &OwnerBinding, b: &OwnerBinding) {
    apply(
        e,
        r,
        LedgerRequest::Share {
            from: source.token(),
            to: b.clone(),
        },
    );
    apply(e, r, LedgerRequest::Publish(b.token()));
}
fn retire_source(e: &MemEngine, r: &RootDescriptor, source: &OwnerBinding) {
    // Exact same-image coverage; this is a test transfer, not a claim that a
    // newer checkpoint qualifies for Version retirement or object deletion.
    let mut holder = source.clone();
    holder.descriptor.id = OwnerId::new([93; 16]).unwrap();
    holder.descriptor.kind = OwnerKind::Snapshot;
    apply(
        e,
        r,
        LedgerRequest::Share {
            from: source.token(),
            to: holder.clone(),
        },
    );
    apply(e, r, LedgerRequest::Publish(holder.token()));
    apply(
        e,
        r,
        LedgerRequest::QuiesceAfterTransfer {
            from: source.token(),
            to: holder.token(),
        },
    );
    apply(e, r, LedgerRequest::Release(source.token()));
}

#[test]
fn terminal_reader_acknowledgements_survive_retired_and_archived_sources_without_new_writes() {
    for mode in 0..2 {
        let (e, r, source, b) = fixture(mode);
        enter(&e, &r, &source, &b);
        let finish = request(&source, &b);
        apply(&e, &r, finish.clone());
        retire_source(&e, &r, &source);
        assert_eq!(
            owner(&e, &r, source.descriptor.id).phase,
            PinPhase::Released
        );
        let before = all(&e);
        assert!(!plan(&e, &r, &finish).unwrap().changed());
        assert_eq!(all(&e), before);
        // Simulate a future source-history archiver. The released Reader's own
        // immutable record and full resource cross-links remain authoritative.
        let mut archived = WriteBatch::new();
        archived.delete(ColumnFamily::Default, owner_key(source.descriptor.id));
        archived.delete(
            ColumnFamily::Default,
            owner_registration_key(source.descriptor.id),
        );
        e.write(archived).unwrap();
        assert!(!plan(&e, &r, &finish).unwrap().changed());
        let opposite = if mode < 2 {
            LedgerRequest::FinishImageRead {
                successor: source.token(),
                reader: b.clone(),
            }
        } else {
            LedgerRequest::FinishCheckpointRead {
                successor: source.token(),
                reader: b.clone(),
            }
        };
        assert!(plan(&e, &r, &opposite).is_err());
        for which in 0..5 {
            let mut wrong_source = source.clone();
            let mut wrong = b.clone();
            match which {
                0 => wrong_source.descriptor.id = OwnerId::new([92; 16]).unwrap(),
                1 => wrong_source.generation = NonZeroU64::new(2).unwrap(),
                2 => wrong.descriptor.local.as_mut().unwrap().process = [9; 16],
                3 => wrong.descriptor.subject = [17; 32],
                _ => wrong.generation = NonZeroU64::new(2).unwrap(),
            }
            assert!(plan(&e, &r, &request(&wrong_source, &wrong)).is_err());
        }
        let mut corrupt = WriteBatch::new();
        corrupt.delete(
            ColumnFamily::Default,
            resource_registration_key(&b.resources[0]),
        );
        e.write(corrupt).unwrap();
        assert!(plan(&e, &r, &finish).is_err());
    }
}

#[test]
fn terminal_retry_cannot_finish_absent_held_published_or_newer_reader_generations() {
    for mode in 0..2 {
        for state in 0..4 {
            let (e, r, source, mut b) = fixture(mode);
            let old = b.clone();
            if state == 3 {
                enter(&e, &r, &source, &b);
                apply(&e, &r, request(&source, &b));
                b.generation = NonZeroU64::new(2).unwrap();
            }
            if state > 0 {
                apply(
                    &e,
                    &r,
                    LedgerRequest::Share {
                        from: source.token(),
                        to: b.clone(),
                    },
                );
            }
            if state >= 2 {
                apply(&e, &r, LedgerRequest::Publish(b.token()));
            }
            retire_source(&e, &r, &source);
            let before = all(&e);
            assert!(plan(&e, &r, &request(&source, &b)).is_err());
            assert!(plan(&e, &r, &request(&source, &old)).is_err());
            assert_eq!(all(&e), before);
            if state > 0 {
                assert_eq!(
                    owner(&e, &r, b.descriptor.id).phase,
                    if state == 1 {
                        PinPhase::Held
                    } else {
                        PinPhase::Published
                    }
                );
            }
        }
    }
}

#[test]
fn image_reader_only_acknowledges_terminal_generation_when_source_metadata_is_unavailable() {
    for state in 0..4 {
        let (e, r, source, b) = fixture(2);
        if state > 0 {
            apply(
                &e,
                &r,
                LedgerRequest::Share {
                    from: source.token(),
                    to: b.clone(),
                },
            );
        }
        if state > 1 {
            apply(&e, &r, LedgerRequest::Publish(b.token()));
        }
        if state == 3 {
            apply(&e, &r, request(&source, &b));
        }
        // Deliberately unavailable source rows, not a valid Snapshot retirement
        // procedure: real Snapshot/Migration quiescence still requires install
        // evidence. The reader's retained fact and its own links stay intact.
        let mut unavailable = WriteBatch::new();
        unavailable.delete(ColumnFamily::Default, owner_key(source.descriptor.id));
        unavailable.delete(
            ColumnFamily::Default,
            owner_registration_key(source.descriptor.id),
        );
        e.write(unavailable).unwrap();
        let before = all(&e);
        let result = plan(&e, &r, &request(&source, &b));
        if state == 3 {
            assert!(!result.unwrap().changed());
        } else {
            assert!(result.is_err());
        }
        assert!(plan(
            &e,
            &r,
            &LedgerRequest::FinishCheckpointRead {
                successor: source.token(),
                reader: b.clone(),
            }
        )
        .is_err());
        assert_eq!(all(&e), before);
    }
}
