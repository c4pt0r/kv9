use super::*;
use crate::retention::tests::{all, apply, fixture, plan};
use kv9_engine::checkpoint::{FlushScope, SstReference};
use kv9_engine::{Engine, MemEngine};

pub(in crate::retention) fn fixture_intent() -> (MemEngine, RootDescriptor, CheckpointIntent) {
    let (engine, root) = fixture();
    // Only a descriptor is needed: intent recording makes no prepared-object claim.
    let hash = "ab".repeat(32);
    let manifest = CheckpointManifest {
        scope: FlushScope {
            cluster: root.cluster_id.to_string(),
            region: 100,
            conf_ver: 1,
            version: 1,
        },
        term: 2,
        index: 9,
        files: vec![SstReference {
            key: format!("clusters/{}/regions/100/sst/{hash}", root.cluster_id),
            sha256: hash,
            cf: 0,
            smallest: b"a".to_vec(),
            largest: b"b".to_vec(),
            size: 50,
            count: 2,
        }],
    };
    let intent = CheckpointIntent::new(&root, &manifest, 0).unwrap();
    apply(&engine, &root, LedgerRequest::Initialize);
    apply(
        &engine,
        &root,
        LedgerRequest::Register(intent.binding(OwnerKind::Pending).unwrap().resources),
    );
    (engine, root, intent)
}

#[test]
fn exact_intent_requires_full_published_protection_and_survives_positive_transfer() {
    let (engine, root, intent) = fixture_intent();
    let pending = intent.binding(OwnerKind::Pending).unwrap();
    let version = intent.binding(OwnerKind::Version).unwrap();
    let write = LedgerRequest::RecordCheckpoint(intent.clone());
    assert!(plan(&engine, &root, &write).is_err());
    apply(&engine, &root, LedgerRequest::Acquire(pending.clone()));
    assert!(
        plan(&engine, &root, &write).is_err(),
        "Held does not authorize publishing the archive"
    );
    apply(&engine, &root, LedgerRequest::Publish(pending.token()));
    apply(&engine, &root, write.clone());
    let stored = checkpoint_intent(
        engine.snapshot().unwrap().as_ref(),
        &root,
        pending.descriptor.id,
    )
    .unwrap()
    .unwrap();
    assert_eq!(stored, intent);
    assert!(!plan(&engine, &root, &write).unwrap().changed());
    apply(
        &engine,
        &root,
        LedgerRequest::Share {
            from: pending.token(),
            to: version.clone(),
        },
    );
    apply(&engine, &root, LedgerRequest::Publish(version.token()));
    apply(
        &engine,
        &root,
        LedgerRequest::QuiesceAfterTransfer {
            from: pending.token(),
            to: version.token(),
        },
    );
    apply(&engine, &root, LedgerRequest::Release(pending.token()));
    assert_eq!(
        checkpoint_intent(
            engine.snapshot().unwrap().as_ref(),
            &root,
            pending.descriptor.id
        )
        .unwrap()
        .unwrap(),
        intent
    );
    assert!(!plan(&engine, &root, &write).unwrap().changed());
    let mut damage = WriteBatch::new();
    damage.delete(ColumnFamily::Default, owner_key(version.descriptor.id));
    engine.write(damage).unwrap();
    assert!(checkpoint_intent(
        engine.snapshot().unwrap().as_ref(),
        &root,
        pending.descriptor.id
    )
    .is_err());
    assert!(plan(&engine, &root, &write).is_err());
}

#[test]
fn intent_codec_binds_canonical_descriptor_predecessor_root_owner_and_bounds() {
    let (_, root, intent) = fixture_intent();
    let bytes = intent.encode();
    assert_eq!(
        CheckpointIntent::decode(&bytes, &root, intent.pending).unwrap(),
        intent
    );
    let request = encode_request(
        root.digest(),
        &LedgerRequest::RecordCheckpoint(intent.clone()),
    )
    .unwrap();
    assert_eq!(
        decode_request(&request, root.digest()).unwrap(),
        LedgerRequest::RecordCheckpoint(intent.clone())
    );
    for end in 0..bytes.len() {
        assert!(CheckpointIntent::decode(&bytes[..end], &root, intent.pending).is_err());
    }
    for position in [0, 8, 40, 56, 64, 68, bytes.len() - 33, bytes.len() - 1] {
        let mut wrong = bytes.clone();
        wrong[position] ^= 1;
        assert!(CheckpointIntent::decode(&wrong, &root, intent.pending).is_err());
    }
    for position in [40, 56, 64] {
        let mut wrong = bytes[..bytes.len() - 32].to_vec();
        wrong[position] ^= 1;
        assert!(CheckpointIntent::decode(&seal(wrong), &root, intent.pending).is_err());
    }
    assert!(
        CheckpointIntent::decode(&vec![0; MAX_INTENT_BYTES + 1], &root, intent.pending).is_err()
    );
    let mut wrong = root.clone();
    wrong.cluster_id = kv9_common::ClusterId::from_bytes([77; 16]);
    assert!(CheckpointIntent::decode(&bytes, &wrong, intent.pending).is_err());
    assert!(CheckpointIntent::decode(&bytes, &root, OwnerId::new([42; 16]).unwrap()).is_err());
    // Unknown bytes cannot use the larger intent bound to broaden old requests.
    let mut oversized = b"KV9RTX01".to_vec();
    oversized.extend_from_slice(root.digest().as_bytes());
    oversized.push(0);
    oversized.resize(MAX_SMALL_REQUEST_BYTES + 1, 0);
    assert!(decode_request(&seal(oversized), root.digest()).is_err());
    let mut noncanonical = intent.manifest.clone();
    noncanonical.push(b' ');
    assert!(CheckpointManifest::decode(&noncanonical).is_ok());
    assert!(CheckpointIntent::from_bytes(root.digest(), noncanonical, 0).is_err());
}

#[test]
fn large_intent_uses_its_own_bound_and_preserves_descriptor_bytes() {
    let (engine, root, original) = fixture_intent();
    let mut manifest = original.manifest().unwrap();
    manifest.files[0].smallest = vec![0; 20_000];
    manifest.files[0].largest = vec![255; 20_000];
    let intent = CheckpointIntent::new(&root, &manifest, 8).unwrap();
    let pending = intent.binding(OwnerKind::Pending).unwrap();
    apply(&engine, &root, LedgerRequest::Acquire(pending.clone()));
    apply(&engine, &root, LedgerRequest::Publish(pending.token()));
    let request = LedgerRequest::RecordCheckpoint(intent.clone());
    let bytes = encode_request(root.digest(), &request).unwrap();
    assert!(bytes.len() > MAX_SMALL_REQUEST_BYTES);
    assert!(bytes.len() < MAX_LEDGER_REQUEST_BYTES);
    assert_eq!(decode_request(&bytes, root.digest()).unwrap(), request);
    apply(&engine, &root, request);
    assert_eq!(
        checkpoint_intent(
            engine.snapshot().unwrap().as_ref(),
            &root,
            pending.descriptor.id
        )
        .unwrap()
        .unwrap()
        .manifest_bytes(),
        manifest.encode().unwrap()
    );
}

#[test]
fn wrong_binding_or_corrupt_intent_is_not_replaced_by_retry() {
    for wrong_field in 0..5 {
        let (engine, root, intent) = fixture_intent();
        let mut pending = intent.binding(OwnerKind::Pending).unwrap();
        match wrong_field {
            0 => pending.descriptor.operation[0] ^= 1,
            1 => pending.descriptor.subject[0] ^= 1,
            2 => pending.descriptor.version += 1,
            3 => pending.descriptor.subject_is_anchor = true,
            _ => pending.descriptor.kind = OwnerKind::Build,
        }
        apply(&engine, &root, LedgerRequest::Acquire(pending.clone()));
        apply(&engine, &root, LedgerRequest::Publish(pending.token()));
        let before = all(&engine);
        assert!(plan(&engine, &root, &LedgerRequest::RecordCheckpoint(intent)).is_err());
        assert_eq!(all(&engine), before);
    }
    let (engine, root, intent) = fixture_intent();
    let pending = intent.binding(OwnerKind::Pending).unwrap();
    apply(&engine, &root, LedgerRequest::Acquire(pending.clone()));
    apply(&engine, &root, LedgerRequest::Publish(pending.token()));
    apply(
        &engine,
        &root,
        LedgerRequest::RecordCheckpoint(intent.clone()),
    );
    let mut damage = WriteBatch::new();
    damage.put(
        ColumnFamily::Default,
        intent_key(intent.pending),
        b"corrupt".to_vec(),
    );
    engine.write(damage).unwrap();
    let before = all(&engine);
    assert!(checkpoint_intent(engine.snapshot().unwrap().as_ref(), &root, intent.pending).is_err());
    assert!(plan(&engine, &root, &LedgerRequest::RecordCheckpoint(intent)).is_err());
    assert_eq!(all(&engine), before);
}
