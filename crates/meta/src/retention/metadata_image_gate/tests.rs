use super::*;
use crate::retention::metadata_read_gate::tests::{fixture, grant, publish, select, step, token};
use crate::retention::tests::{all, apply, owner, plan, seed_with_voters};
use kv9_common::{NodeId, StoreIdentity, StoreIncarnation};
use kv9_engine::{Engine, MemEngine, WalEngine};
use std::sync::atomic::{AtomicUsize, Ordering};

fn alias(
    e: &MemEngine,
    r: &RootDescriptor,
    i: &CheckpointIntent,
    predecessor: u64,
) -> CheckpointIntent {
    let a = CheckpointIntent::new(r, &i.manifest().unwrap(), predecessor).unwrap();
    publish(e, r, &a);
    a
}
fn observed(e: &impl Engine, r: &RootDescriptor, i: &CheckpointIntent) -> Work {
    observe(e.snapshot().unwrap().as_ref(), r, i.pending_id()).unwrap()
}
fn close_attempt(e: &MemEngine, r: &RootDescriptor, i: &CheckpointIntent, new: &CheckpointIntent) {
    for voter in &r.voters {
        select(e, r, new, voter.node_id.0);
    }
    while !matches!(
        metadata_read_gate::observe(e.snapshot().unwrap().as_ref(), r, i.pending_id()).unwrap(),
        metadata_read_gate::Work::Closed(_)
    ) {
        step(e, r, i);
    }
}
fn close(e: &MemEngine, r: &RootDescriptor, i: &CheckpointIntent) {
    apply(e, r, LedgerRequest::CloseMetadataImageReads(token(i)));
}
fn version(i: &CheckpointIntent) -> OwnerBinding {
    i.binding(OwnerKind::Version).unwrap()
}
fn reader(source: &OwnerBinding, process: u8) -> OwnerBinding {
    let lifetime = LocalLifetime {
        node: 1,
        store: [1; 16],
        process: [process; 16],
    };
    if source.descriptor.kind == OwnerKind::Version {
        checkpoint_reader::binding(source, lifetime).unwrap()
    } else {
        image_reader::binding(source, lifetime).unwrap()
    }
}
fn begin(r: &RootDescriptor, i: &CheckpointIntent) -> LedgerRequest {
    let id = StoreIdentity::for_voter(r, NodeId(1)).unwrap();
    LedgerRequest::BeginBootstrapRead {
        pending: token(i),
        source: id.node_id,
        store: id.store_incarnation,
    }
}
fn share(source: &OwnerBinding, b: &OwnerBinding) -> LedgerRequest {
    LedgerRequest::Share {
        from: source.token(),
        to: b.clone(),
    }
}

#[test]
fn complete_all_store_gate_closes_existing_and_future_aliases_and_image_reader_sources() {
    let (e, r, old, new) = fixture(5);
    let a = alias(&e, &r, &old, 2);
    let v = version(&a);
    let held = reader(&v, 1);
    apply(&e, &r, share(&v, &held));
    apply(&e, &r, begin(&r, &a));
    let bootstrap = bootstrap_reader::binding(
        &r,
        &a.manifest().unwrap(),
        a.predecessor(),
        StoreIdentity::for_voter(&r, NodeId(1)).unwrap(),
    )
    .unwrap();
    let mut snapshot = v.clone();
    snapshot.descriptor.id = OwnerId::new([71; 16]).unwrap();
    snapshot.descriptor.kind = OwnerKind::Snapshot;
    apply(&e, &r, share(&v, &snapshot));
    apply(&e, &r, LedgerRequest::Publish(snapshot.token()));
    let image_read = reader(&snapshot, 2);
    apply(&e, &r, share(&snapshot, &image_read));
    let request = LedgerRequest::CloseMetadataImageReads(token(&old));
    assert!(plan(&e, &r, &request).is_err());
    for n in 1..=4 {
        select(&e, &r, &new, n);
    }
    step(&e, &r, &old);
    assert!(plan(&e, &r, &request).is_err());
    select(&e, &r, &new, 5);
    step(&e, &r, &old);
    assert_eq!(observed(&e, &r, &old), Work::Ready);
    let old_view = e.snapshot().unwrap();
    close(&e, &r, &old);
    assert!(plan_retention(old_view.as_ref(), &r, &share(&v, &reader(&v, 9))).is_ok());
    let later = alias(&e, &r, &old, 3);
    for source in [&v, &snapshot, &version(&later)] {
        assert!(plan(&e, &r, &share(source, &reader(source, 9))).is_err());
    }
    for b in [&held, &image_read, &bootstrap] {
        assert!(plan(&e, &r, &LedgerRequest::Publish(b.token())).is_err());
        assert_eq!(owner(&e, &r, b.descriptor.id).phase, PinPhase::Held);
    }
    for i in [&a, &later] {
        assert!(plan(&e, &r, &begin(&r, i)).is_err());
    }
    let before = all(&e);
    close(&e, &r, &later);
    assert_eq!(all(&e), before);
    // Same SST set, different exact manifest/index: the newer image stays open.
    assert_eq!(version(&old).resources, version(&new).resources);
    assert!(plan(&e, &r, &share(&version(&new), &reader(&version(&new), 10))).is_ok());
    assert_eq!(
        owner(&e, &r, version(&old).descriptor.id).phase,
        PinPhase::Published
    );
}

fn catalogs(e: &MemEngine, r: &RootDescriptor) {
    loop {
        match reader_index::observe(e.snapshot().unwrap().as_ref(), r).unwrap() {
            reader_index::Work::Complete => break,
            reader_index::Work::Page { progress, next, .. } => {
                apply(e, r, progress.request(next));
            }
            _ => panic!(),
        }
    }
    loop {
        match bootstrap_index::observe(e.snapshot().unwrap().as_ref(), r).unwrap() {
            bootstrap_index::Work::Complete => break,
            bootstrap_index::Work::Page { progress, next, .. } => {
                apply(e, r, progress.request(next));
            }
            _ => panic!(),
        }
    }
}
fn census(e: &MemEngine, r: &RootDescriptor, i: &CheckpointIntent) -> reader_closure::Cursor {
    loop {
        match reader_closure::observe(e.snapshot().unwrap().as_ref(), r, i.pending_id()).unwrap() {
            reader_closure::Work::Complete(c) => return c,
            reader_closure::Work::Advance { cursor, .. } => {
                apply(e, r, cursor.request(i.pending_id()));
            }
            _ => panic!(),
        }
    }
}
#[test]
fn inherited_alias_fence_keeps_active_readers_and_bootstrap_until_independent_finish() {
    let (e, r, old, new) = fixture(1);
    let a = alias(&e, &r, &old, 2);
    let v = version(&a);
    let b = reader(&v, 1);
    apply(&e, &r, share(&v, &b));
    apply(&e, &r, LedgerRequest::Publish(b.token()));
    let g = grant(&e, &r, &a, 1);
    close_attempt(&e, &r, &old, &new);
    let first = metadata_read_gate::fence(e.snapshot().unwrap().as_ref(), &r, old.pending_id())
        .unwrap()
        .unwrap()
        .recorded_at;
    let old_view = e.snapshot().unwrap();
    close(&e, &r, &old);
    assert!(matches!(
        reader_closure::observe(old_view.as_ref(), &r, a.pending_id()).unwrap(),
        reader_closure::Work::Open
    ));
    let Work::Closed(f) = observed(&e, &r, &a) else {
        panic!()
    };
    assert!(f.closed_at > first);
    assert_eq!(
        metadata_read_gate::fence(e.snapshot().unwrap().as_ref(), &r, old.pending_id())
            .unwrap()
            .unwrap()
            .recorded_at,
        first
    );
    catalogs(&e, &r);
    let c = census(&e, &r, &a);
    assert_eq!((c.counts.active, c.refusal_revision), (1, f.closed_at));
    assert_eq!(owner(&e, &r, g.descriptor.id).phase, PinPhase::Published);
    apply(
        &e,
        &r,
        LedgerRequest::FinishCheckpointRead {
            successor: v.token(),
            reader: b,
        },
    );
    let unentered = reader(&v, 2);
    assert!(plan(&e, &r, &share(&v, &unentered)).is_err());
    apply(
        &e,
        &r,
        LedgerRequest::FinishCheckpointRead {
            successor: v.token(),
            reader: unentered,
        },
    );
    e.write(
        bootstrap_reader::plan_verified(
            e.snapshot().unwrap().as_ref(),
            &r,
            token(&a),
            StoreIdentity::for_voter(&r, NodeId(1)).unwrap(),
        )
        .unwrap()
        .into_batch(),
    )
    .unwrap();
    let c = census(&e, &r, &a);
    assert_eq!(
        (c.counts.drained, c.counts.unentered, c.counts.active),
        (1, 1, 0)
    );
    loop {
        match bootstrap_closure::observe(e.snapshot().unwrap().as_ref(), &r, a.pending_id())
            .unwrap()
        {
            bootstrap_closure::Work::Complete(c) => {
                assert_eq!((c.counts.grants, c.counts.retired), (1, 1));
                break;
            }
            bootstrap_closure::Work::Advance { cursor, .. } => {
                apply(&e, &r, cursor.request(a.pending_id()));
            }
            _ => panic!(),
        }
    }
    assert_eq!(owner(&e, &r, v.descriptor.id).phase, PinPhase::Published);
}

#[test]
fn historical_image_gate_survives_source_version_transfer_and_blocks_alias_reentry() {
    let (e, r, old, new) = fixture(1);
    let a = alias(&e, &r, &old, 2);
    close_attempt(&e, &r, &old, &new);
    close(&e, &r, &old);
    let original = observed(&e, &r, &old);
    apply(
        &e,
        &r,
        LedgerRequest::QuiesceAfterTransfer {
            from: version(&old).token(),
            to: version(&a).token(),
        },
    );
    apply(&e, &r, LedgerRequest::Release(version(&old).token()));
    assert_eq!(observed(&e, &r, &old), original);
    let before = all(&e);
    close(&e, &r, &old);
    close(&e, &r, &a);
    assert_eq!(all(&e), before);
    assert!(plan(&e, &r, &begin(&r, &a)).is_err());
    assert!(plan(&e, &r, &share(&version(&a), &reader(&version(&a), 1))).is_err());
}

#[test]
fn negative_attempt_refusal_alone_cannot_close_every_alias_of_the_image() {
    let (e, r, old, _) = fixture(1);
    let a = alias(&e, &r, &old, 2);
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
    assert_eq!(observed(&e, &r, &old), Work::Ineligible);
    assert!(plan(&e, &r, &LedgerRequest::CloseMetadataImageReads(token(&old))).is_err());
    assert!(plan(&e, &r, &begin(&r, &a)).is_ok());
}

#[test]
fn v17_upgrade_preserves_prior_facts_and_missing_or_damaged_image_gate_never_reopens() {
    let (e, r, old, new) = fixture(1);
    let a = alias(&e, &r, &old, 2);
    close_attempt(&e, &r, &old, &new);
    e.write(
        bootstrap_reader::plan_verified(
            e.snapshot().unwrap().as_ref(),
            &r,
            token(&old),
            StoreIdentity::for_voter(&r, NodeId(1)).unwrap(),
        )
        .unwrap()
        .into_batch(),
    )
    .unwrap();
    catalogs(&e, &r);
    let retained: Vec<_> = all(&e)
        .into_iter()
        .filter(|(k, _)| *k >= key(28, &[]) && *k < key(36, &[]))
        .collect();
    let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
    h.truncate(h.len() - 32);
    h[..8].copy_from_slice(BOOTSTRAP_UNENTERED_HEADER_MAGIC);
    let mut b = WriteBatch::new();
    b.put(ColumnFamily::Default, key(1, &[]), seal(h));
    e.write(b).unwrap();
    assert_eq!(observed(&e, &r, &old), Work::Ready);
    close(&e, &r, &old);
    for (k, v) in retained {
        assert_eq!(e.get(ColumnFamily::Default, &k).unwrap(), Some(v));
    }
    let request = LedgerRequest::CloseMetadataImageReads(token(&old));
    let bytes = encode_request(r.digest(), &request).unwrap();
    assert_eq!(decode_request(&bytes, r.digest()).unwrap(), request);
    for cut in 0..bytes.len() {
        assert!(decode_request(&bytes[..cut], r.digest()).is_err());
    }
    let mut wrong = token(&old);
    wrong.generation = NonZeroU64::new(2).unwrap();
    assert!(encode_request(r.digest(), &LedgerRequest::CloseMetadataImageReads(wrong)).is_err());
    let saved = all(&e);
    let image = subject(&old);
    let gate = e
        .get(ColumnFamily::Default, &gate_key(image))
        .unwrap()
        .unwrap();
    assert_eq!(gate.len(), 168);
    assert_eq!(
        e.get(ColumnFamily::Default, &marker_key(image))
            .unwrap()
            .unwrap()
            .len(),
        136
    );
    for damage in 0..7 {
        let mut batch = WriteBatch::new();
        for (k, v) in &saved {
            batch.put(ColumnFamily::Default, k.clone(), v.clone());
        }
        match damage {
            0 => {
                batch.delete(ColumnFamily::Default, gate_key(image));
            }
            1 => {
                batch.delete(ColumnFamily::Default, marker_key(image));
            }
            2..=4 => {
                let mut bytes = gate[..gate.len() - 32].to_vec();
                match damage {
                    2 => bytes[72] ^= 1,
                    3 => bytes[128..136].copy_from_slice(&u64::MAX.to_be_bytes()),
                    _ => bytes[120..128].copy_from_slice(&1u64.to_be_bytes()),
                }
                let bytes = seal(bytes);
                batch.put(
                    ColumnFamily::Default,
                    marker_key(image),
                    marker(r.digest(), image, &bytes),
                );
                batch.put(ColumnFamily::Default, gate_key(image), bytes);
            }
            5 => {
                batch.delete(ColumnFamily::Default, key(32, old.pending_id().as_bytes()));
            }
            _ => {
                let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
                h.truncate(h.len() - 32);
                h[..8].copy_from_slice(BOOTSTRAP_UNENTERED_HEADER_MAGIC);
                batch.put(ColumnFamily::Default, key(1, &[]), seal(h));
            }
        }
        e.write(batch).unwrap();
        let before = all(&e);
        assert!(observe(e.snapshot().unwrap().as_ref(), &r, a.pending_id()).is_err());
        assert!(plan(&e, &r, &request).is_err());
        assert!(plan(&e, &r, &begin(&r, &a)).is_err());
        if damage == 6 {
            assert!(plan(&e, &r, &LedgerRequest::Initialize).is_err());
        }
        assert_eq!(all(&e), before);
    }
}

struct Count<'a> {
    view: &'a dyn ReadView,
    gets: AtomicUsize,
}
impl ReadView for Count<'_> {
    fn get(&self, cf: ColumnFamily, k: &[u8]) -> Result<Option<Vec<u8>>> {
        if k.starts_with(PREFIX) {
            self.gets.fetch_add(1, Ordering::Relaxed);
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
        assert!(!a.starts_with(PREFIX));
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
fn image_closure_uses_constant_point_work_for_13_and_113_aliases_without_scanning_them() {
    let mut costs = Vec::new();
    for n in [13, 113] {
        let (e, r, old, new) = fixture(1);
        for p in 2..n + 2 {
            alias(&e, &r, &old, p);
        }
        close_attempt(&e, &r, &old, &new);
        let view = e.snapshot().unwrap();
        let counted = Count {
            view: view.as_ref(),
            gets: AtomicUsize::new(0),
        };
        let plan = plan_retention(
            &counted,
            &r,
            &LedgerRequest::CloseMetadataImageReads(token(&old)),
        )
        .unwrap();
        costs.push(counted.gets.load(Ordering::Relaxed));
        e.write(plan.into_batch()).unwrap();
        assert!(matches!(observed(&e, &r, &old), Work::Closed(_)));
    }
    assert_eq!(costs[0], costs[1]);
}

#[test]
fn wal_tail_cuts_keep_image_gate_marker_and_cross_alias_read_refusal_atomic() {
    let (memory, r, old, new) = fixture(1);
    let a = alias(&memory, &r, &old, 2);
    close_attempt(&memory, &r, &old, &new);
    let dir = std::env::temp_dir().join(format!(
        "kv9-image-read-gate-{}",
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
    let request = LedgerRequest::CloseMetadataImageReads(token(&old));
    e.write(plan(&e, &r, &request).unwrap().into_batch())
        .unwrap();
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
        assert_eq!(plan(&e, &r, &begin(&r, &a)).is_err(), cut == bytes.len());
        let next = plan(&e, &r, &request).unwrap();
        assert_eq!(next.changed(), cut != bytes.len());
        e.write(next.into_batch()).unwrap();
        assert!(!plan(&e, &r, &request).unwrap().changed());
        assert!(plan(&e, &r, &begin(&r, &a)).is_err());
        assert_eq!(
            owner(&e, &r, version(&old).descriptor.id).phase,
            PinPhase::Published
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}
