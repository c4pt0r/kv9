use super::*;
use crate::retention::checkpoint_intent::tests::fixture_intent;
use crate::retention::tests::{all, apply, owner, plan, seed};
use kv9_engine::{Engine, MemEngine, WalEngine};

fn fixture() -> (MemEngine, RootDescriptor, CheckpointIntent) {
    let (e, r, i) = fixture_intent();
    apply(&e, &r, LedgerRequest::BeginCheckpoint(i.clone()));
    (e, r, i)
}
fn source(r: &RootDescriptor) -> NodeEndpoint {
    let voter = r.voter(NodeId(1)).unwrap();
    NodeEndpoint {
        node: voter.node_id,
        incarnation: voter.store_incarnation,
        address: voter.addr,
        generation: 0,
        previous_address: None,
        active: true,
    }
}
fn pending(i: &CheckpointIntent) -> OwnerToken {
    i.binding(OwnerKind::Pending).unwrap().token()
}
fn verified(e: &impl Engine, r: &RootDescriptor, i: &CheckpointIntent) -> LedgerPlan {
    plan_verified(
        e.snapshot().unwrap().as_ref(),
        r,
        pending(i),
        [9; 32],
        source(r),
    )
    .unwrap()
}
fn observed(e: &impl Engine, r: &RootDescriptor, i: &CheckpointIntent) -> Option<Refusal> {
    refusal(e.snapshot().unwrap().as_ref(), r, i.pending_id()).unwrap()
}

#[test]
fn assertion_codec_is_strict_and_ordinary_planner_cannot_create_evidence() {
    let (e, r, i) = fixture();
    let request = LedgerRequest::RecordCheckpointRefusal {
        pending: pending(&i),
        source: NodeId(1),
        winner: [9; 32],
    };
    let bytes = encode_request(r.digest(), &request).unwrap();
    assert_eq!(decode_request(&bytes, r.digest()).unwrap(), request);
    for cut in 0..bytes.len() {
        assert!(decode_request(&bytes[..cut], r.digest()).is_err());
    }
    assert!(decode_request(&bytes, RootDigest::from_bytes([8; 32])).is_err());
    for range in [65..73, 73..105] {
        // source and winner
        let mut b = bytes[..bytes.len() - 32].to_vec();
        b[range].fill(0);
        assert!(decode_request(&seal(b), r.digest()).is_err());
    }
    let before = all(&e);
    assert!(plan(&e, &r, &request).is_err());
    assert!(plan_retry(
        e.snapshot().unwrap().as_ref(),
        &r,
        pending(&i),
        [9; 32],
        NodeId(1)
    )
    .is_err());
    assert_eq!(all(&e), before);
    assert!(observed(&e, &r, &i).is_none());
}

#[test]
fn verified_fact_closes_gate_atomically_and_retries_keep_original_provenance_and_pins() {
    let (e, r, i) = fixture();
    let p = verified(&e, &r, &i);
    let revision = p.revision();
    e.write(p.into_batch()).unwrap();
    let fact = observed(&e, &r, &i).unwrap();
    assert_eq!((fact.closed_at, fact.recorded_at), (revision, revision));
    assert_eq!(fact.source_store, source(&r).incarnation);
    assert_eq!(fact.encode(r.digest()).len(), 200);
    assert!(!verified(&e, &r, &i).changed());
    let before = all(&e);
    assert!(!plan_retry(
        e.snapshot().unwrap().as_ref(),
        &r,
        pending(&i),
        [9; 32],
        NodeId(1)
    )
    .unwrap()
    .changed());
    for (winner, node) in [([8; 32], NodeId(1)), ([9; 32], NodeId(2))] {
        assert!(plan_retry(
            e.snapshot().unwrap().as_ref(),
            &r,
            pending(&i),
            winner,
            node
        )
        .is_err());
    }
    assert_eq!(all(&e), before);
    assert_eq!(owner(&e, &r, i.pending_id()).phase, PinPhase::Published);
    assert!(plan(&e, &r, &LedgerRequest::Release(pending(&i))).is_err());
    assert!(retention_owner(
        e.snapshot().unwrap().as_ref(),
        &r,
        i.binding(OwnerKind::Version).unwrap().descriptor.id
    )
    .unwrap()
    .is_none());
}

#[test]
fn rejects_positive_winner_wrong_generation_and_source_before_changing_gate() {
    let (e, r, i) = fixture();
    let before = all(&e);
    let snapshot = e.snapshot().unwrap();
    let positive = i
        .manifest()
        .unwrap()
        .change_id(i.predecessor())
        .unwrap()
        .try_into()
        .unwrap();
    for winner in [positive, [0; 32]] {
        assert!(plan_verified(snapshot.as_ref(), &r, pending(&i), winner, source(&r)).is_err());
    }
    let mut wrong = pending(&i);
    wrong.generation = NonZeroU64::new(2).unwrap();
    assert!(plan_verified(snapshot.as_ref(), &r, wrong, [9; 32], source(&r)).is_err());
    for damage in 0..3 {
        let mut node = source(&r);
        match damage {
            0 => node.active = false,
            1 => node.incarnation = StoreIncarnation::from_bytes([8; 16]),
            _ => node.node = NodeId(0),
        }
        assert!(plan_verified(snapshot.as_ref(), &r, pending(&i), [9; 32], node).is_err());
    }
    assert_eq!(all(&e), before);
}

#[test]
fn closed_without_fact_is_unknown_and_later_record_does_not_invalidate_generation_census() {
    let (e, r, i) = fixture();
    let closed = apply(&e, &r, LedgerRequest::CloseCheckpointAdmission(pending(&i)));
    assert!(observed(&e, &r, &i).is_none());
    let stamp_key = key(19, i.pending_id().as_bytes());
    let stamp = e.get(ColumnFamily::Default, &stamp_key).unwrap();
    e.write(verified(&e, &r, &i).into_batch()).unwrap();
    let fact = observed(&e, &r, &i).unwrap();
    assert_eq!(fact.closed_at, closed);
    assert!(fact.recorded_at > closed);
    assert_eq!(e.get(ColumnFamily::Default, &stamp_key).unwrap(), stamp);
}

#[test]
fn corrupt_or_orphan_fact_and_old_header_fail_closed() {
    for damage in 0..9 {
        let (e, r, i) = fixture();
        e.write(verified(&e, &r, &i).into_batch()).unwrap();
        let mut b = WriteBatch::new();
        match damage {
            0..=4 => {
                let key = refusal_key(i.pending_id());
                let mut bytes = e.get(ColumnFamily::Default, &key).unwrap().unwrap();
                bytes.truncate(bytes.len() - 32);
                match damage {
                    0 => bytes[8] ^= 1,  // root
                    1 => bytes[56] ^= 1, // intent
                    2 => bytes[88..120].copy_from_slice(
                        &i.manifest().unwrap().change_id(i.predecessor()).unwrap(),
                    ),
                    3 => bytes[152..160].copy_from_slice(&1u64.to_be_bytes()), // gate revision
                    _ => bytes[160..168].copy_from_slice(&u64::MAX.to_be_bytes()),
                }
                b.put(ColumnFamily::Default, key, seal(bytes));
            }
            5 => {
                b.delete(
                    ColumnFamily::Default,
                    checkpoint_intent::intent_key(i.pending_id()),
                );
            }
            6 => {
                b.delete(ColumnFamily::Default, owner_key(i.pending_id()));
            }
            7 => {
                b.delete(ColumnFamily::Default, key(9, i.pending_id().as_bytes()));
            }
            _ => {
                let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
                h.truncate(h.len() - 32);
                h[..8].copy_from_slice(ATOMIC_HEADER_MAGIC);
                b.put(ColumnFamily::Default, key(1, &[]), seal(h));
                // Isolate the new namespace's own downgrade guard.
                b.delete(ColumnFamily::Default, key(19, i.pending_id().as_bytes()));
            }
        }
        e.write(b).unwrap();
        assert!(
            refusal(e.snapshot().unwrap().as_ref(), &r, i.pending_id()).is_err(),
            "damage {damage}"
        );
        if damage == 8 {
            assert!(plan(&e, &r, &LedgerRequest::Initialize).is_err());
        }
        if matches!(damage, 5 | 6) {
            assert!(plan(&e, &r, &LedgerRequest::BeginCheckpoint(i)).is_err());
        }
    }
}

#[test]
fn wal_tail_cuts_never_leave_fact_without_closed_gate_or_drop_pending_protection() {
    let (memory, r, i) = fixture();
    let dir =
        std::env::temp_dir().join(format!("kv9-refusal-{}", StoreIncarnation::mint().unwrap()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("meta.wal");
    let (e, _) = WalEngine::open(&path).unwrap();
    let (_, mut batch) = seed();
    for (k, v) in all(&memory) {
        batch.put(ColumnFamily::Default, k, v);
    }
    e.write(batch).unwrap();
    let before = std::fs::read(&path).unwrap();
    e.write(verified(&e, &r, &i).into_batch()).unwrap();
    let fact = observed(&e, &r, &i).unwrap();
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
        let closed = !matches!(
            checkpoint_admission::sweep::observe(
                e.snapshot().unwrap().as_ref(),
                &r,
                i.pending_id()
            )
            .unwrap(),
            checkpoint_admission::sweep::SweepObservation::Open
        );
        assert_eq!(closed, cut == after.len());
        assert_eq!(
            observed(&e, &r, &i),
            if closed { Some(fact.clone()) } else { None }
        );
        assert_eq!(owner(&e, &r, i.pending_id()).phase, PinPhase::Published);
    }
    std::fs::remove_dir_all(dir).unwrap();
}
