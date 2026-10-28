use super::*;
use crate::retention::tests::{all, apply, fixture, owner, plan};
use kv9_common::{NodeId, StoreIncarnation};
use kv9_engine::checkpoint::{FlushScope, SstReference};
use kv9_engine::Engine;

#[test]
fn only_verified_store_fence_can_retire_bootstrap_and_delayed_entry_cannot_resume() {
    for entered in 0..3 {
        let (e, r, version, b) = setup();
        let identity = StoreIdentity::for_voter(&r, NodeId(1)).unwrap();
        // Derive the canonical Pending from the Version's immutable operation.
        let pending = OwnerToken {
            owner: checkpoint_intent::owner_id(
                r.digest(),
                &version.descriptor.operation,
                b"kv9-checkpoint-pending-v1",
            )
            .unwrap(),
            generation: NonZeroU64::new(1).unwrap(),
        };
        let share = begin_request(&r, &version);
        if entered > 0 {
            apply(&e, &r, share.clone());
        }
        if entered > 1 {
            apply(&e, &r, LedgerRequest::Publish(b.token()));
        }
        let request = LedgerRequest::RetireBootstrapRead {
            pending,
            source: identity.node_id,
            store: identity.store_incarnation,
        };
        assert_eq!(
            decode_request(&encode_request(r.digest(), &request).unwrap(), r.digest()).unwrap(),
            request
        );
        assert!(plan(&e, &r, &request).is_err());
        let before = all(&e);
        assert!(plan_retry(e.snapshot().unwrap().as_ref(), &r, pending, identity).is_err());
        assert_eq!(all(&e), before);
        e.write(
            plan_verified(e.snapshot().unwrap().as_ref(), &r, pending, identity)
                .unwrap()
                .into_batch(),
        )
        .unwrap();
        assert_eq!(owner(&e, &r, b.descriptor.id).phase, PinPhase::Released);
        assert_eq!(
            owner(&e, &r, version.descriptor.id).phase,
            PinPhase::Published
        );
        assert!(
            !plan_retry(e.snapshot().unwrap().as_ref(), &r, pending, identity)
                .unwrap()
                .changed()
        );
        assert!(plan(&e, &r, &share).is_err());
        assert!(plan(&e, &r, &LedgerRequest::Publish(b.token())).is_err());
        // Even an already terminal grant cannot be revived as another generation.
        let mut next = b.clone();
        next.generation = NonZeroU64::new(2).unwrap();
        assert!(plan(&e, &r, &LedgerRequest::Acquire(next)).is_err());
    }
}

pub(in crate::retention) fn setup() -> (
    kv9_engine::MemEngine,
    RootDescriptor,
    OwnerBinding,
    OwnerBinding,
) {
    let (e, r) = fixture();
    let hash = "ab".repeat(32);
    let m = CheckpointManifest {
        scope: FlushScope {
            cluster: r.cluster_id.to_string(),
            region: 1,
            conf_ver: 1,
            version: 1,
        },
        term: 2,
        index: 9,
        files: vec![SstReference {
            key: format!("clusters/{}/regions/1/sst/{hash}", r.cluster_id),
            sha256: hash,
            cf: 0,
            smallest: b"a".to_vec(),
            largest: b"b".to_vec(),
            size: 50,
            count: 2,
        }],
    };
    let i = CheckpointIntent::new(&r, &m, 0).unwrap();
    apply(&e, &r, LedgerRequest::BeginCheckpoint(i.clone()));
    let version = i.binding(OwnerKind::Version).unwrap();
    apply(
        &e,
        &r,
        LedgerRequest::Share {
            from: i.binding(OwnerKind::Pending).unwrap().token(),
            to: version.clone(),
        },
    );
    apply(&e, &r, LedgerRequest::Publish(version.token()));
    let b = binding(&r, &m, 0, StoreIdentity::for_voter(&r, NodeId(1)).unwrap()).unwrap();
    (e, r, version, b)
}

#[test]
fn bootstrap_grant_pins_complete_image_and_refuses_all_generic_retirement_paths() {
    let (e, r, version, b) = setup();
    let request = begin_request(&r, &version);
    assert_eq!(
        decode_request(&encode_request(r.digest(), &request).unwrap(), r.digest()).unwrap(),
        request
    );
    apply(&e, &r, request.clone());
    apply(&e, &r, LedgerRequest::Publish(b.token()));
    apply(&e, &r, request);
    assert_eq!(
        owner(&e, &r, b.descriptor.id).binding.resources,
        version.resources
    );
    let before = all(&e);
    for request in [
        LedgerRequest::QuiesceAfterTransfer {
            from: b.token(),
            to: version.token(),
        },
        LedgerRequest::Release(b.token()),
        LedgerRequest::FinishCheckpointRead {
            successor: version.token(),
            reader: b.clone(),
        },
        LedgerRequest::FinishImageRead {
            successor: version.token(),
            reader: b.clone(),
        },
        LedgerRequest::FinishCheckpointExecution {
            pending: version.token(),
            executor: b.clone(),
            issued: 0,
            resolved: 0,
        },
    ] {
        assert!(plan(&e, &r, &request).is_err(), "{request:?}");
        assert_eq!(all(&e), before);
    }
    assert_eq!(owner(&e, &r, b.descriptor.id).phase, PinPhase::Published);
}

#[test]
fn bootstrap_binding_is_store_specific_and_cannot_become_a_process_or_anchor_owner() {
    let (_, r, _, b) = setup();
    for field in 0..4 {
        let mut bad = b.clone();
        match field {
            0 => {
                bad.descriptor.local = Some(LocalLifetime {
                    node: 1,
                    store: [3; 16],
                    process: [7; 16],
                })
            }
            1 => bad.descriptor.region = 100,
            2 => bad.descriptor.subject_is_anchor = true,
            _ => bad.generation = NonZeroU64::new(2).unwrap(),
        }
        assert!(encode_request(r.digest(), &LedgerRequest::Acquire(bad)).is_err());
    }
    let (_, _, data) = crate::retention::checkpoint_intent::tests::fixture_intent();
    let mut m = data.manifest().unwrap();
    let identity = StoreIdentity::for_voter(&r, NodeId(1)).unwrap();
    assert!(binding(&r, &m, 0, identity).is_err());
    m.scope.region = 1;
    m.files[0].key = m.files[0].key.replace("/regions/100/", "/regions/1/");
    let mut wrong = identity;
    wrong.store_incarnation = StoreIncarnation::from_bytes([8; 16]);
    assert!(binding(&r, &m, 0, wrong).is_err());
    wrong.node_id = NodeId(9);
    assert_ne!(
        binding(&r, &m, 0, identity).unwrap(),
        binding(&r, &m, 0, wrong).unwrap()
    );
}

#[test]
fn terminal_bootstrap_acknowledgement_outlives_intent_and_version_but_keeps_exact_identity_checks()
{
    let (e, r, version, b) = setup();
    let identity = StoreIdentity::for_voter(&r, NodeId(1)).unwrap();
    let pending = OwnerToken {
        owner: checkpoint_intent::owner_id(
            r.digest(),
            &version.descriptor.operation,
            b"kv9-checkpoint-pending-v1",
        )
        .unwrap(),
        generation: NonZeroU64::new(1).unwrap(),
    };
    e.write(
        plan_verified(e.snapshot().unwrap().as_ref(), &r, pending, identity)
            .unwrap()
            .into_batch(),
    )
    .unwrap();
    let mut successor = version.clone();
    successor.descriptor.id = OwnerId::new([83; 16]).unwrap();
    successor.descriptor.kind = OwnerKind::Snapshot;
    apply(
        &e,
        &r,
        LedgerRequest::Share {
            from: version.token(),
            to: successor.clone(),
        },
    );
    apply(&e, &r, LedgerRequest::Publish(successor.token()));
    apply(
        &e,
        &r,
        LedgerRequest::QuiesceAfterTransfer {
            from: version.token(),
            to: successor.token(),
        },
    );
    apply(&e, &r, LedgerRequest::Release(version.token()));
    assert!(
        !plan_retry(e.snapshot().unwrap().as_ref(), &r, pending, identity)
            .unwrap()
            .changed()
    );
    // Test-only archive simulation, not a new production metadata GC path.
    let mut archive = WriteBatch::new();
    archive.delete(
        ColumnFamily::Default,
        checkpoint_intent::intent_key(pending.owner),
    );
    archive.delete(ColumnFamily::Default, owner_key(version.descriptor.id));
    archive.delete(
        ColumnFamily::Default,
        owner_registration_key(version.descriptor.id),
    );
    e.write(archive).unwrap();
    let before = all(&e);
    assert!(
        !plan_retry(e.snapshot().unwrap().as_ref(), &r, pending, identity)
            .unwrap()
            .changed()
    );
    assert!(plan_verified(e.snapshot().unwrap().as_ref(), &r, pending, identity).is_err());
    for which in 0..3 {
        let mut bad_token = pending;
        let mut bad_store = identity;
        match which {
            0 => bad_token.generation = NonZeroU64::new(2).unwrap(),
            1 => bad_token.owner = OwnerId::new([71; 16]).unwrap(),
            _ => bad_store.node_id = NodeId(99),
        }
        assert!(plan_retry(e.snapshot().unwrap().as_ref(), &r, bad_token, bad_store).is_err());
    }
    assert_eq!(all(&e), before);
    // Even a checksum/registration-consistent terminal row cannot substitute
    // another owner kind or operation under the expected hash-derived ID.
    let mut changed = owner(&e, &r, b.descriptor.id);
    for which in 0..2 {
        changed.binding = b.clone();
        if which == 0 {
            changed.binding.descriptor.kind = OwnerKind::Snapshot;
        } else {
            changed.binding.descriptor.operation = [37; 32];
        }
        let mut damage = WriteBatch::new();
        damage.put(
            ColumnFamily::Default,
            owner_key(b.descriptor.id),
            encode_owner(&changed),
        );
        damage.put(
            ColumnFamily::Default,
            owner_registration_key(b.descriptor.id),
            owner_registration(&changed.binding),
        );
        e.write(damage).unwrap();
        assert!(plan_retry(e.snapshot().unwrap().as_ref(), &r, pending, identity).is_err());
    }
}

#[test]
fn process_reader_census_does_not_include_or_retire_persistent_bootstrap_grants() {
    let (e, r, version, b) = setup();
    apply(&e, &r, begin_request(&r, &version));
    apply(&e, &r, LedgerRequest::Publish(b.token()));
    loop {
        match reader_index::observe(e.snapshot().unwrap().as_ref(), &r).unwrap() {
            reader_index::Work::Page { progress, next, .. } => {
                apply(&e, &r, progress.request(next));
            }
            reader_index::Work::Complete => break,
            _ => panic!("expected current ledger"),
        }
    }
    for operation in [version.descriptor.operation, b.descriptor.operation] {
        assert!(
            reader_index::readers(e.snapshot().unwrap().as_ref(), &r, operation, None)
                .unwrap()
                .owners
                .is_empty()
        );
    }
    assert_eq!(owner(&e, &r, b.descriptor.id).phase, PinPhase::Published);
    assert_eq!(
        owner(&e, &r, version.descriptor.id).phase,
        PinPhase::Published
    );
    assert!(plan(&e, &r, &LedgerRequest::Release(b.token())).is_err());
}

pub(in crate::retention) fn begin_request(
    root: &RootDescriptor,
    version: &OwnerBinding,
) -> LedgerRequest {
    let identity = StoreIdentity::for_voter(root, NodeId(1)).unwrap();
    LedgerRequest::BeginBootstrapRead {
        pending: OwnerToken {
            owner: checkpoint_intent::owner_id(
                root.digest(),
                &version.descriptor.operation,
                b"kv9-checkpoint-pending-v1",
            )
            .unwrap(),
            generation: NonZeroU64::new(1).unwrap(),
        },
        source: identity.node_id,
        store: identity.store_incarnation,
    }
}
