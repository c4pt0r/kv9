use super::*;
use crate::retention::tests::{all, apply, owner, plan, seed_with_voters};
use kv9_common::{NodeId, RootVoter, StoreIdentity, StoreIncarnation};
use kv9_engine::checkpoint::{CheckpointManifest, FlushScope, SstReference};
use kv9_engine::{Engine, MemEngine, WalEngine};

pub(in crate::retention) fn token(i: &CheckpointIntent) -> OwnerToken {
    i.binding(OwnerKind::Pending).unwrap().token()
}
pub(in crate::retention) fn publish(e: &MemEngine, r: &RootDescriptor, i: &CheckpointIntent) {
    apply(e, r, LedgerRequest::BeginCheckpoint(i.clone()));
    let version = i.binding(OwnerKind::Version).unwrap();
    apply(
        e,
        r,
        LedgerRequest::Share {
            from: token(i),
            to: version.clone(),
        },
    );
    apply(e, r, LedgerRequest::Publish(version.token()));
    apply(
        e,
        r,
        LedgerRequest::QuiesceAfterTransfer {
            from: token(i),
            to: version.token(),
        },
    );
    apply(e, r, LedgerRequest::Release(token(i)));
}
pub(in crate::retention) fn fixture(
    n: u64,
) -> (
    MemEngine,
    RootDescriptor,
    CheckpointIntent,
    CheckpointIntent,
) {
    let (r, b) = seed_with_voters(
        (1..=n)
            .map(|n| {
                let mut store = [3; 16];
                store[..8].copy_from_slice(&n.to_be_bytes());
                RootVoter {
                    node_id: NodeId(n),
                    addr: format!("127.0.0.1:{}", 21000 + n).parse().unwrap(),
                    store_incarnation: StoreIncarnation::from_bytes(store),
                }
            })
            .collect(),
    );
    let e = MemEngine::new();
    e.write(b).unwrap();
    let hash = "ab".repeat(32);
    let mut m = CheckpointManifest {
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
    let first = CheckpointIntent::new(&r, &m, 0).unwrap();
    publish(&e, &r, &first);
    m.index += 1;
    let next = CheckpointIntent::new(&r, &m, 1).unwrap();
    publish(&e, &r, &next);
    (e, r, first, next)
}
pub(in crate::retention) fn grant(
    e: &MemEngine,
    r: &RootDescriptor,
    i: &CheckpointIntent,
    n: u64,
) -> OwnerBinding {
    let id = StoreIdentity::for_voter(r, NodeId(n)).unwrap();
    let b = bootstrap_reader::binding(r, &i.manifest().unwrap(), i.predecessor(), id).unwrap();
    apply(
        e,
        r,
        LedgerRequest::BeginBootstrapRead {
            pending: token(i),
            source: id.node_id,
            store: id.store_incarnation,
        },
    );
    apply(e, r, LedgerRequest::Publish(b.token()));
    b
}
pub(in crate::retention) fn select(
    e: &MemEngine,
    r: &RootDescriptor,
    i: &CheckpointIntent,
    n: u64,
) {
    grant(e, r, i, n);
    let v = r.voter(NodeId(n)).unwrap();
    let source = crate::endpoint::NodeEndpoint {
        node: v.node_id,
        incarnation: v.store_incarnation,
        address: v.addr,
        generation: 0,
        previous_address: None,
        active: true,
    };
    e.write(
        checkpoint_selection::plan_verified(e.snapshot().unwrap().as_ref(), r, token(i), source)
            .unwrap()
            .into_batch(),
    )
    .unwrap();
}
fn observed(e: &impl Engine, r: &RootDescriptor, i: &CheckpointIntent) -> Work {
    observe(e.snapshot().unwrap().as_ref(), r, i.pending_id()).unwrap()
}
pub(in crate::retention) fn step(
    e: &MemEngine,
    r: &RootDescriptor,
    i: &CheckpointIntent,
) -> LedgerRequest {
    let Work::Advance(g) = observed(e, r, i) else {
        panic!("gate not advanceable")
    };
    let req = g.request(i.pending_id());
    apply(e, r, req.clone());
    req
}
fn reader(i: &CheckpointIntent, r: &RootDescriptor, process: u8) -> OwnerBinding {
    checkpoint_reader::binding(
        &i.binding(OwnerKind::Version).unwrap(),
        LocalLifetime {
            node: 1,
            store: *r.voters[0].store_incarnation.as_bytes(),
            process: [process; 16],
        },
    )
    .unwrap()
}
fn share(i: &CheckpointIntent, b: &OwnerBinding) -> LedgerRequest {
    LedgerRequest::Share {
        from: i.binding(OwnerKind::Version).unwrap().token(),
        to: b.clone(),
    }
}
fn finish(i: &CheckpointIntent, b: &OwnerBinding) -> LedgerRequest {
    LedgerRequest::FinishCheckpointRead {
        successor: i.binding(OwnerKind::Version).unwrap().token(),
        reader: b.clone(),
    }
}

#[test]
fn all_root_stores_must_advance_and_only_final_page_blocks_new_readers_and_bootstrap() {
    let (e, r, old, new) = fixture(5);
    assert!(matches!(
        observed(&e, &r, &old),
        Work::WaitingForSelection(_)
    ));
    for n in 1..=4 {
        select(&e, &r, &new, n);
    }
    let held = reader(&old, &r, 1);
    apply(&e, &r, share(&old, &held));
    let live = reader(&old, &r, 2);
    apply(&e, &r, share(&old, &live));
    apply(&e, &r, LedgerRequest::Publish(live.token()));
    let persistent = grant(&e, &r, &old, 5);
    let req = step(&e, &r, &old);
    assert!(matches!(
        observed(&e, &r, &old),
        Work::WaitingForSelection(Progress {
            checked_stores: 4,
            ..
        })
    ));
    assert!(!plan(&e, &r, &req).unwrap().changed());
    apply(&e, &r, share(&old, &held));
    select(&e, &r, &old, 5); // Selection at the target is insufficient.
    assert!(matches!(
        observed(&e, &r, &old),
        Work::WaitingForSelection(_)
    ));
    select(&e, &r, &new, 5);
    let before = e.snapshot().unwrap();
    let last = step(&e, &r, &old);
    assert!(matches!(
        observe(before.as_ref(), &r, old.pending_id()).unwrap(),
        Work::Advance(_)
    ));
    assert!(matches!(observed(&e, &r, &old), Work::Closed(_)));
    assert!(!plan(&e, &r, &last).unwrap().changed());
    for request in [
        share(&old, &held),
        LedgerRequest::Publish(held.token()),
        share(&old, &reader(&old, &r, 3)),
        LedgerRequest::Publish(live.token()),
        LedgerRequest::Publish(persistent.token()),
        LedgerRequest::BeginBootstrapRead {
            pending: token(&old),
            source: NodeId(1),
            store: r.voters[0].store_incarnation,
        },
    ] {
        assert!(plan(&e, &r, &request).is_err(), "{request:?}");
    }
    // Existing grants and owner pins are not released by this fence.
    for b in [
        &live,
        &persistent,
        &old.binding(OwnerKind::Version).unwrap(),
    ] {
        assert_eq!(owner(&e, &r, b.descriptor.id).phase, PinPhase::Published);
    }
    apply(&e, &r, finish(&old, &held));
    apply(&e, &r, finish(&old, &live));
    apply(&e, &r, share(&new, &reader(&new, &r, 4)));
    assert!(
        checkpoint_refusal::refusal(e.snapshot().unwrap().as_ref(), &r, old.pending_id())
            .unwrap()
            .is_none()
    );
}

#[test]
fn gate_request_and_record_reject_skips_corruption_and_header_downgrade_without_repair() {
    let (e, r, old, new) = fixture(1);
    select(&e, &r, &new, 1);
    let request = Progress::default().request(old.pending_id());
    let bytes = encode_request(r.digest(), &request).unwrap();
    assert_eq!(decode_request(&bytes, r.digest()).unwrap(), request);
    for cut in 0..bytes.len() {
        assert!(decode_request(&bytes[..cut], r.digest()).is_err());
    }
    let before = all(&e);
    assert!(plan(
        &e,
        &r,
        &LedgerRequest::AdvanceMetadataReadGate {
            pending: token(&old),
            expected: 1
        }
    )
    .is_err());
    assert_eq!(all(&e), before);
    step(&e, &r, &old);
    let original = e
        .get(ColumnFamily::Default, &gate_key(old.pending_id()))
        .unwrap()
        .unwrap();
    assert_eq!(original.len(), 152);
    for offset in [40, 56, 88, 96, 104, 112] {
        let mut corrupt = original[..original.len() - 32].to_vec();
        corrupt[offset] ^= 1;
        let mut b = WriteBatch::new();
        b.put(
            ColumnFamily::Default,
            gate_key(old.pending_id()),
            seal(corrupt),
        );
        e.write(b).unwrap();
        let saved = all(&e);
        assert!(observe(e.snapshot().unwrap().as_ref(), &r, old.pending_id()).is_err());
        assert!(plan(&e, &r, &share(&old, &reader(&old, &r, 9))).is_err());
        assert_eq!(all(&e), saved);
    }
    let mut b = WriteBatch::new();
    b.put(ColumnFamily::Default, gate_key(old.pending_id()), original);
    let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
    h.truncate(h.len() - 32);
    h[..8].copy_from_slice(METADATA_SELECTION_HEADER_MAGIC);
    b.put(ColumnFamily::Default, key(1, &[]), seal(h));
    e.write(b).unwrap();
    assert!(plan(&e, &r, &LedgerRequest::Initialize).is_err());
    assert!(plan(&e, &r, &share(&old, &reader(&old, &r, 9))).is_err());
}

fn complete_reader_index(e: &MemEngine, r: &RootDescriptor) {
    loop {
        match reader_index::observe(e.snapshot().unwrap().as_ref(), r).unwrap() {
            reader_index::Work::Complete => break,
            reader_index::Work::Page { progress, next, .. } => apply(e, r, progress.request(next)),
            reader_index::Work::Upgrade => apply(e, r, LedgerRequest::Initialize),
            _ => panic!(),
        };
    }
}
fn close_readers(
    e: &MemEngine,
    r: &RootDescriptor,
    i: &CheckpointIntent,
) -> reader_closure::Cursor {
    loop {
        match reader_closure::observe(e.snapshot().unwrap().as_ref(), r, i.pending_id()).unwrap() {
            reader_closure::Work::Advance { cursor, .. } => {
                apply(e, r, cursor.request(i.pending_id()));
            }
            reader_closure::Work::Complete(c) => return c,
            _ => panic!(),
        }
    }
}
#[test]
fn positive_gate_drives_dense_reader_census_and_late_drain_invalidates_the_same_fence() {
    let (e, r, old, new) = fixture(1);
    let b = reader(&old, &r, 1);
    apply(&e, &r, share(&old, &b));
    apply(&e, &r, LedgerRequest::Publish(b.token()));
    complete_reader_index(&e, &r);
    assert!(matches!(
        reader_closure::observe(e.snapshot().unwrap().as_ref(), &r, old.pending_id()).unwrap(),
        reader_closure::Work::Open
    ));
    select(&e, &r, &new, 1);
    step(&e, &r, &old);
    let first = close_readers(&e, &r, &old);
    assert_eq!((first.counts.generations, first.counts.active), (1, 1));
    apply(&e, &r, finish(&old, &b));
    assert!(matches!(
        reader_closure::observe(e.snapshot().unwrap().as_ref(), &r, old.pending_id()).unwrap(),
        reader_closure::Work::Advance { restart: true, .. }
    ));
    let drained = close_readers(&e, &r, &old);
    assert_eq!((drained.counts.drained, drained.counts.active), (1, 0));
    assert_eq!(drained.refusal_revision, first.refusal_revision);
    let mut next = b.clone();
    next.generation = NonZeroU64::new(2).unwrap();
    assert!(plan(&e, &r, &share(&old, &next)).is_err());
    apply(&e, &r, finish(&old, &next));
    let final_state = close_readers(&e, &r, &old);
    assert_eq!(
        (final_state.counts.generations, final_state.counts.unentered),
        (2, 1)
    );
    assert_eq!(
        owner(
            &e,
            &r,
            old.binding(OwnerKind::Version).unwrap().descriptor.id
        )
        .phase,
        PinPhase::Published
    );
}

#[test]
fn v14_upgrade_retains_existing_selection_and_unclosed_gate_never_claims_drain() {
    let (e, r, old, new) = fixture(5);
    for n in 1..=5 {
        select(&e, &r, &new, n);
    }
    let selections: Vec<_> = all(&e)
        .into_iter()
        .filter(|(k, _)| k.starts_with(&key(31, &[])))
        .collect();
    let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
    h.truncate(h.len() - 32);
    h[..8].copy_from_slice(METADATA_SELECTION_HEADER_MAGIC);
    let mut b = WriteBatch::new();
    // Explicit v14 fixture predates the image census and its mutation stamps.
    for (k, _) in all(&e) {
        if k >= key(33, &[]) && k < key(35, &[]) {
            b.delete(ColumnFamily::Default, k);
        }
    }
    b.put(ColumnFamily::Default, key(1, &[]), seal(h));
    e.write(b).unwrap();
    assert!(matches!(observed(&e, &r, &old), Work::Advance(_)));
    step(&e, &r, &old);
    assert!(fence(e.snapshot().unwrap().as_ref(), &r, old.pending_id())
        .unwrap()
        .is_none());
    assert!(matches!(
        reader_closure::observe(e.snapshot().unwrap().as_ref(), &r, old.pending_id()).unwrap(),
        reader_closure::Work::Open
    ));
    for (k, v) in selections {
        assert_eq!(e.get(ColumnFamily::Default, &k).unwrap(), Some(v));
    }
    step(&e, &r, &old);
    assert!(matches!(
        reader_closure::observe(e.snapshot().unwrap().as_ref(), &r, old.pending_id()).unwrap(),
        reader_closure::Work::WaitingForCatalog
    ));
}

struct Count<'a> {
    view: &'a dyn ReadView,
    selection_gets: std::sync::atomic::AtomicUsize,
}
impl ReadView for Count<'_> {
    fn get(&self, cf: ColumnFamily, k: &[u8]) -> Result<Option<Vec<u8>>> {
        if k.starts_with(&key(31, &[])) {
            self.selection_gets
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        self.view.get(cf, k)
    }
    fn scan(
        &self,
        cf: ColumnFamily,
        a: &[u8],
        b: &[u8],
        n: usize,
    ) -> Result<Vec<kv9_engine::ScanEntry>> {
        assert!(!a.starts_with(PREFIX));
        self.view.scan(cf, a, b, n)
    }
    fn seek_le(&self, cf: ColumnFamily, k: &[u8]) -> Result<Option<kv9_engine::ScanEntry>> {
        assert!(!k.starts_with(PREFIX));
        self.view.seek_le(cf, k)
    }
    fn iter<'a>(
        &'a self,
        cf: ColumnFamily,
        a: &[u8],
        b: &[u8],
    ) -> Result<Box<dyn Iterator<Item = Result<kv9_engine::ScanEntry>> + 'a>> {
        assert!(!a.starts_with(PREFIX), "gate may only use point reads");
        self.view.iter(cf, a, b)
    }
    fn iter_rev<'a>(
        &'a self,
        cf: ColumnFamily,
        a: &[u8],
        b: &[u8],
    ) -> Result<Box<dyn Iterator<Item = Result<kv9_engine::ScanEntry>> + 'a>> {
        assert!(!a.starts_with(PREFIX));
        self.view.iter_rev(cf, a, b)
    }
}
#[test]
fn every_page_queries_only_four_store_selections_for_13_and_113_provisioned_stores() {
    for n in [13, 113] {
        let (e, r, old, new) = fixture(n);
        for node in 1..=4 {
            select(&e, &r, &new, node);
        }
        let view = e.snapshot().unwrap();
        let count = Count {
            view: view.as_ref(),
            selection_gets: std::sync::atomic::AtomicUsize::new(0),
        };
        let Work::Advance(g) = observe(&count, &r, old.pending_id()).unwrap() else {
            panic!()
        };
        assert_eq!(
            count
                .selection_gets
                .swap(0, std::sync::atomic::Ordering::Relaxed),
            PAGE
        );
        let p = plan_retention(&count, &r, &g.request(old.pending_id())).unwrap();
        assert_eq!(
            count
                .selection_gets
                .load(std::sync::atomic::Ordering::Relaxed),
            PAGE
        );
        e.write(p.into_batch()).unwrap();
        // Missing next-store facts must not invalidate the durable first page.
        assert!(matches!(
            observed(&e, &r, &old),
            Work::WaitingForSelection(_)
        ));
        assert!(!plan(&e, &r, &g.request(old.pending_id()))
            .unwrap()
            .changed());
    }
}

#[test]
fn wal_tail_cuts_preserve_partial_pages_and_final_read_fence_atomically() {
    for final_page in [false, true] {
        let (memory, r, old, new) = fixture(5);
        for n in 1..=5 {
            select(&memory, &r, &new, n);
        }
        if final_page {
            step(&memory, &r, &old);
        }
        let Work::Advance(before) = observed(&memory, &r, &old) else {
            panic!()
        };
        let req = before.request(old.pending_id());
        let dir = std::env::temp_dir().join(format!(
            "kv9-metadata-read-gate-{}",
            StoreIncarnation::mint().unwrap()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("base");
        let (e, _) = WalEngine::open(&path).unwrap();
        let (_, mut batch) = seed_with_voters(r.voters.clone());
        for (k, v) in all(&memory) {
            batch.put(ColumnFamily::Default, k, v);
        }
        e.write(batch).unwrap();
        let size = std::fs::read(&path).unwrap().len();
        e.write(plan(&e, &r, &req).unwrap().into_batch()).unwrap();
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
            assert_eq!(
                matches!(observed(&e, &r, &old), Work::Closed(_)),
                final_page && cut == bytes.len()
            );
            let p = plan(&e, &r, &req).unwrap();
            assert_eq!(p.changed(), cut != bytes.len());
            e.write(p.into_batch()).unwrap();
            assert!(!plan(&e, &r, &req).unwrap().changed());
            assert_eq!(
                plan(&e, &r, &share(&old, &reader(&old, &r, 3))).is_err(),
                final_page
            );
            assert_eq!(
                owner(
                    &e,
                    &r,
                    old.binding(OwnerKind::Version).unwrap().descriptor.id
                )
                .phase,
                PinPhase::Published
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn later_negative_fact_cannot_move_an_existing_positive_reader_census_boundary() {
    let (e, r, old, new) = fixture(1);
    let reader = reader(&old, &r, 1);
    apply(&e, &r, share(&old, &reader));
    apply(&e, &r, finish(&old, &reader));
    complete_reader_index(&e, &r);
    select(&e, &r, &new, 1);
    step(&e, &r, &old);
    let before = close_readers(&e, &r, &old);
    // Version existence alone is not independent positive settlement proof.
    // Even a later independently recorded negative result must preserve the
    // first effective read boundary while keeping its own negative meaning.
    let voter = &r.voters[0];
    e.write(
        checkpoint_refusal::plan_verified(
            e.snapshot().unwrap().as_ref(),
            &r,
            token(&old),
            [8; 32],
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
        .into_batch(),
    )
    .unwrap();
    let negative =
        checkpoint_refusal::refusal(e.snapshot().unwrap().as_ref(), &r, old.pending_id())
            .unwrap()
            .unwrap();
    assert!(negative.recorded_at > before.refusal_revision);
    let after = close_readers(&e, &r, &old);
    assert_eq!(after.refusal_revision, before.refusal_revision);
    assert_eq!(after.counts, before.counts);
    assert!(plan(&e, &r, &share(&old, &reader)).is_err());
}
