use super::*;
use crate::retention::checkpoint_intent::tests::fixture_intent;
use crate::retention::tests::{all, apply, owner, plan, seed};
use kv9_engine::{Engine, MemEngine, WalEngine};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Count<'a> {
    view: &'a dyn ReadView,
    rows: AtomicUsize,
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
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<kv9_engine::ScanEntry>> {
        assert!(
            !start.starts_with(PREFIX),
            "census must consume a bounded iterator"
        );
        self.view.scan(cf, start, end, limit)
    }
    fn seek_le(&self, cf: ColumnFamily, k: &[u8]) -> Result<Option<kv9_engine::ScanEntry>> {
        assert!(!k.starts_with(PREFIX));
        self.view.seek_le(cf, k)
    }
    fn iter<'a>(
        &'a self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
    ) -> Result<Box<dyn Iterator<Item = Result<kv9_engine::ScanEntry>> + 'a>> {
        let retention = start.starts_with(PREFIX);
        Ok(Box::new(self.view.iter(cf, start, end)?.inspect(
            move |_| {
                if retention {
                    self.rows.fetch_add(1, Ordering::Relaxed);
                }
            },
        )))
    }
    fn iter_rev<'a>(
        &'a self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
    ) -> Result<Box<dyn Iterator<Item = Result<kv9_engine::ScanEntry>> + 'a>> {
        assert!(!start.starts_with(PREFIX));
        self.view.iter_rev(cf, start, end)
    }
}

fn fixture() -> (MemEngine, RootDescriptor, CheckpointIntent) {
    let (e, r, i) = fixture_intent();
    apply(&e, &r, LedgerRequest::BeginCheckpoint(i.clone()));
    (e, r, i)
}
fn binding(i: &CheckpointIntent, process: u8) -> OwnerBinding {
    checkpoint_reader::binding(
        &i.binding(OwnerKind::Pending).unwrap(),
        LocalLifetime {
            node: 1,
            store: [1; 16],
            process: [process; 16],
        },
    )
    .unwrap()
}
fn share(i: &CheckpointIntent, b: &OwnerBinding) -> LedgerRequest {
    LedgerRequest::Share {
        from: i.binding(OwnerKind::Pending).unwrap().token(),
        to: b.clone(),
    }
}
fn finish(i: &CheckpointIntent, b: &OwnerBinding) -> LedgerRequest {
    LedgerRequest::FinishCheckpointRead {
        successor: i.binding(OwnerKind::Pending).unwrap().token(),
        reader: b.clone(),
    }
}
fn rounds(
    e: &MemEngine,
    r: &RootDescriptor,
    i: &CheckpointIntent,
    n: u8,
    count: u64,
) -> OwnerBinding {
    let mut b = binding(i, n);
    for g in 1..=count {
        b.generation = NonZeroU64::new(g).unwrap();
        apply(e, r, share(i, &b));
        apply(e, r, LedgerRequest::Publish(b.token()));
        apply(e, r, finish(i, &b));
    }
    b
}
fn refusal(e: &MemEngine, r: &RootDescriptor, i: &CheckpointIntent) {
    let voter = &r.voters[0];
    let plan = checkpoint_refusal::plan_verified(
        e.snapshot().unwrap().as_ref(),
        r,
        i.binding(OwnerKind::Pending).unwrap().token(),
        [9; 32],
        crate::endpoint::NodeEndpoint {
            node: voter.node_id,
            incarnation: voter.store_incarnation,
            address: voter.addr,
            generation: 0,
            previous_address: None,
            active: true,
        },
    )
    .unwrap();
    e.write(plan.into_batch()).unwrap();
}
fn census(e: &MemEngine, r: &RootDescriptor) {
    loop {
        match reader_index::observe(e.snapshot().unwrap().as_ref(), r).unwrap() {
            reader_index::Work::Complete => break,
            reader_index::Work::Page { progress, next, .. } => {
                apply(e, r, progress.request(next));
            }
            reader_index::Work::Upgrade => {
                apply(e, r, LedgerRequest::Initialize);
            }
            _ => panic!("ledger exists"),
        }
    }
}
fn observed(e: &impl Engine, r: &RootDescriptor, i: &CheckpointIntent) -> Work {
    observe(e.snapshot().unwrap().as_ref(), r, i.pending_id()).unwrap()
}
fn advance(e: &MemEngine, r: &RootDescriptor, i: &CheckpointIntent) -> LedgerRequest {
    let Work::Advance { cursor, .. } = observed(e, r, i) else {
        panic!("expected work")
    };
    let request = cursor.request(i.pending_id());
    apply(e, r, request.clone());
    request
}
fn complete(e: &MemEngine, r: &RootDescriptor, i: &CheckpointIntent) -> Cursor {
    for _ in 0..1000 {
        if let Work::Complete(c) = observed(e, r, i) {
            return c;
        }
        advance(e, r, i);
    }
    panic!("closure did not finish")
}
fn generation_key(token: OwnerToken) -> Vec<u8> {
    let mut id = token.owner.as_bytes().to_vec();
    uint(&mut id, token.generation.get());
    key(25, &id)
}

#[test]
fn dense_history_and_aggregate_require_refusal_and_cover_every_registered_generation() {
    let (e, r, i) = fixture();
    for n in 1..=5 {
        rounds(&e, &r, &i, n, 13);
    }
    assert_eq!(observed(&e, &r, &i), Work::Open);
    apply(
        &e,
        &r,
        LedgerRequest::CloseCheckpointAdmission(i.binding(OwnerKind::Pending).unwrap().token()),
    );
    assert_eq!(
        observed(&e, &r, &i),
        Work::Open,
        "Build closure is not a Reader refusal"
    );
    refusal(&e, &r, &i);
    assert_eq!(observed(&e, &r, &i), Work::WaitingForCatalog);
    census(&e, &r);
    let first = advance(&e, &r, &i);
    let encoded = encode_request(r.digest(), &first).unwrap();
    assert_eq!(decode_request(&encoded, r.digest()).unwrap(), first);
    for cut in 0..encoded.len() {
        assert!(decode_request(&encoded[..cut], r.digest()).is_err());
    }
    assert!(!plan(&e, &r, &first).unwrap().changed());
    let c = complete(&e, &r, &i);
    assert_eq!(
        c.counts,
        Counts {
            owners: 5,
            generations: 65,
            drained: 65,
            ..Counts::default()
        }
    );
    assert!(observed(&e, &r, &i).registered_generations_closed());
    assert!(!plan(&e, &r, &c.request(i.pending_id())).unwrap().changed());
    assert!(plan(&e, &r, &first).is_err());
    assert_eq!(owner(&e, &r, i.pending_id()).phase, PinPhase::Published);
    assert!(plan(
        &e,
        &r,
        &LedgerRequest::Release(i.binding(OwnerKind::Pending).unwrap().token())
    )
    .is_err());
}

// Explicit legacy bytes, without inventing origin or changing resource slots.
fn legacy(e: &MemEngine, r: &RootDescriptor, b: &OwnerBinding) {
    let o = owner(e, r, b.descriptor.id);
    let mut batch = WriteBatch::new();
    batch.put(
        ColumnFamily::Default,
        owner_key(b.descriptor.id),
        encode_owner(&o),
    );
    batch.put(
        ColumnFamily::Default,
        owner_registration_key(b.descriptor.id),
        owner_registration(b),
    );
    batch.delete(ColumnFamily::Default, generation_key(b.token()));
    e.write(batch).unwrap();
}
#[test]
fn missing_legacy_and_active_generations_remain_blockers_despite_later_success() {
    let (e, r, i) = fixture();
    let mut b = rounds(&e, &r, &i, 1, 3);
    let mut batch = WriteBatch::new();
    b.generation = NonZeroU64::new(1).unwrap();
    batch.delete(ColumnFamily::Default, generation_key(b.token()));
    e.write(batch).unwrap();
    let active = binding(&i, 2);
    apply(&e, &r, share(&i, &active));
    let old = binding(&i, 3);
    apply(&e, &r, share(&i, &old));
    legacy(&e, &r, &old);
    apply(&e, &r, finish(&i, &old));
    let unentered = binding(&i, 4);
    apply(&e, &r, finish(&i, &unentered));
    let released = rounds(&e, &r, &i, 5, 1);
    legacy(&e, &r, &released);
    refusal(&e, &r, &i);
    census(&e, &r);
    let c = complete(&e, &r, &i);
    assert_eq!(
        c.counts,
        Counts {
            owners: 5,
            generations: 7,
            drained: 2,
            unentered: 1,
            legacy: 1,
            missing: 2,
            active: 1
        }
    );
    assert!(!observed(&e, &r, &i).registered_generations_closed());
    b.generation = NonZeroU64::new(3).unwrap();
    let operation = b.descriptor.operation;
    let page = reader_lifecycle::history(
        e.snapshot().unwrap().as_ref(),
        &r,
        operation,
        b.token(),
        None,
    )
    .unwrap();
    assert_eq!(page.generations.len(), 3);
    assert!(page.generations[0].record.is_none());
    assert_eq!(page.generations[2].reader, b.token());
    assert!(reader_lifecycle::history(
        e.snapshot().unwrap().as_ref(),
        &r,
        [8; 32],
        b.token(),
        None
    )
    .is_err());
    assert!(reader_lifecycle::history(
        e.snapshot().unwrap().as_ref(),
        &r,
        operation,
        b.token(),
        Some(b.generation)
    )
    .is_err());
}

#[test]
fn late_finish_and_unentered_rows_restart_progress_but_other_attempts_and_builds_do_not() {
    let (e, r, i) = fixture();
    rounds(&e, &r, &i, 1, 13);
    let active = binding(&i, 2);
    apply(&e, &r, share(&i, &active));
    refusal(&e, &r, &i);
    census(&e, &r);
    advance(&e, &r, &i);
    apply(&e, &r, finish(&i, &active));
    assert!(matches!(
        observed(&e, &r, &i),
        Work::Advance { restart: true, .. }
    ));
    let c = complete(&e, &r, &i);
    assert_eq!(c.counts.drained, 14);
    let late = binding(&i, 3);
    apply(&e, &r, finish(&i, &late));
    assert!(matches!(
        observed(&e, &r, &i),
        Work::Advance { restart: true, .. }
    ));
    let c = complete(&e, &r, &i);
    assert_eq!(c.counts.unentered, 1);
    let build = checkpoint_admission::executor_binding(
        &i,
        LocalLifetime {
            node: 1,
            store: [1; 16],
            process: [4; 16],
        },
    )
    .unwrap();
    apply(&e, &r, LedgerRequest::CancelUnenteredCheckpoint(build));
    let other = CheckpointIntent::new(&r, &i.manifest().unwrap(), 1).unwrap();
    apply(&e, &r, LedgerRequest::BeginCheckpoint(other.clone()));
    rounds(&e, &r, &other, 5, 1);
    assert_eq!(observed(&e, &r, &i), Work::Complete(c));
    assert_eq!(owner(&e, &r, i.pending_id()).phase, PinPhase::Published);
}

#[test]
fn every_page_has_fixed_cost_for_13_and_113_owners_or_generations() {
    for owners in [false, true] {
        let mut measured = Vec::new();
        for size in [13, 113] {
            let (e, r, i) = fixture();
            if owners {
                for n in 1..=size {
                    rounds(&e, &r, &i, n, 1);
                }
            } else {
                rounds(&e, &r, &i, 1, u64::from(size));
            }
            refusal(&e, &r, &i);
            census(&e, &r);
            let Work::Advance { cursor, .. } = observed(&e, &r, &i) else {
                panic!()
            };
            let view = e.snapshot().unwrap();
            let counted = Count {
                view: view.as_ref(),
                rows: AtomicUsize::new(0),
                gets: AtomicUsize::new(0),
            };
            plan_retention(&counted, &r, &cursor.request(i.pending_id())).unwrap();
            assert!(counted.rows.load(Ordering::Relaxed) <= reader_index::PAGE + 1);
            measured.push((
                counted.rows.load(Ordering::Relaxed),
                counted.gets.load(Ordering::Relaxed),
            ));
        }
        assert_eq!(measured[0], measured[1], "page work grew with history");
    }
}

#[test]
fn corrupt_progress_missing_or_regressed_stamp_and_header_downgrade_refuse() {
    for damage in 0..5 {
        let (e, r, i) = fixture();
        rounds(&e, &r, &i, 1, 2);
        refusal(&e, &r, &i);
        census(&e, &r);
        let mut c = complete(&e, &r, &i);
        let mut batch = WriteBatch::new();
        match damage {
            0 => {
                batch.delete(ColumnFamily::Default, stamp_key(i.pending_id()));
            }
            1 => {
                let mut b = e
                    .get(ColumnFamily::Default, &stamp_key(i.pending_id()))
                    .unwrap()
                    .unwrap();
                b.truncate(b.len() - 32);
                let end = b.len();
                b[end - 8..].copy_from_slice(&1u64.to_be_bytes());
                batch.put(ColumnFamily::Default, stamp_key(i.pending_id()), seal(b));
            }
            2 | 3 => {
                if damage == 2 {
                    c.counts.drained += 1;
                } else {
                    c.sequence = u64::MAX;
                }
                let fact =
                    checkpoint_refusal::refusal(e.snapshot().unwrap().as_ref(), &r, i.pending_id())
                        .unwrap()
                        .unwrap();
                batch.put(
                    ColumnFamily::Default,
                    progress_key(i.pending_id()),
                    c.encode(
                        r.digest(),
                        &ReadFence {
                            shared_image: false,
                            pending: fact.pending,
                            closed_at: fact.closed_at,
                            recorded_at: fact.recorded_at,
                        },
                    ),
                );
            }
            _ => {
                let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
                h.truncate(h.len() - 32);
                h[..8].copy_from_slice(READER_GENERATION_HEADER_MAGIC);
                batch.put(ColumnFamily::Default, key(1, &[]), seal(h));
            }
        }
        e.write(batch).unwrap();
        let before = all(&e);
        assert!(observe(e.snapshot().unwrap().as_ref(), &r, i.pending_id()).is_err());
        assert!(plan(&e, &r, &c.request(i.pending_id())).is_err());
        if damage == 4 {
            assert!(plan(&e, &r, &LedgerRequest::Initialize).is_err());
        }
        assert_eq!(all(&e), before);
    }
}

#[test]
fn version_10_upgrade_preserves_reader_generations_and_can_census_empty_registered_set() {
    for readers in [false, true] {
        let (e, r, i) = fixture();
        if readers {
            rounds(&e, &r, &i, 1, 2);
        }
        refusal(&e, &r, &i);
        census(&e, &r);
        let records: Vec<_> = all(&e)
            .into_iter()
            .filter(|(k, _)| k.starts_with(&key(25, &[])))
            .collect();
        let mut batch = WriteBatch::new();
        // Explicit V10 fixture contains all Reader generations and indexes,
        // but no aggregate/stamp namespace introduced by this increment.
        for (k, _) in all(&e) {
            if (k >= key(38, &[]) && k < key(42, &[]))
                || k.starts_with(&key(26, &[]))
                || k.starts_with(&key(27, &[]))
            {
                batch.delete(ColumnFamily::Default, k);
            }
        }
        let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
        h.truncate(h.len() - 32);
        h[..8].copy_from_slice(READER_GENERATION_HEADER_MAGIC);
        batch.put(ColumnFamily::Default, key(1, &[]), seal(h));
        e.write(batch).unwrap();
        assert_eq!(observed(&e, &r, &i), Work::WaitingForCatalog);
        apply(&e, &r, LedgerRequest::Initialize);
        let c = complete(&e, &r, &i);
        assert_eq!(c.counts.generations, if readers { 2 } else { 0 });
        assert!(observed(&e, &r, &i).registered_generations_closed());
        assert_eq!(
            records,
            all(&e)
                .into_iter()
                .filter(|(k, _)| k.starts_with(&key(25, &[])))
                .collect::<Vec<_>>()
        );
        assert_eq!(owner(&e, &r, i.pending_id()).phase, PinPhase::Published);
    }
}

#[test]
fn wal_cuts_keep_aggregate_pages_and_late_reader_invalidation_atomic() {
    for mutation in [false, true] {
        let (memory, r, i) = fixture();
        let mut b = rounds(&memory, &r, &i, 1, 9);
        refusal(&memory, &r, &i);
        census(&memory, &r);
        let request = if mutation {
            complete(&memory, &r, &i);
            b.generation = NonZeroU64::new(10).unwrap();
            finish(&i, &b)
        } else {
            let Work::Advance { cursor, .. } = observed(&memory, &r, &i) else {
                panic!()
            };
            cursor.request(i.pending_id())
        };
        let dir = std::env::temp_dir().join(format!(
            "kv9-reader-closure-{}",
            kv9_common::StoreIncarnation::mint().unwrap()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("meta.wal");
        let (e, _) = WalEngine::open(&path).unwrap();
        let (_, mut batch) = seed();
        for (k, v) in all(&memory) {
            batch.put(ColumnFamily::Default, k, v);
        }
        e.write(batch).unwrap();
        let before = std::fs::read(&path).unwrap().len();
        let next = plan_retention(e.snapshot().unwrap().as_ref(), &r, &request).unwrap();
        e.write(next.into_batch()).unwrap();
        drop(e);
        let bytes = std::fs::read(&path).unwrap();
        for cut in [
            before,
            before + 1,
            before + (bytes.len() - before) / 2,
            bytes.len() - 1,
            bytes.len(),
        ] {
            let p = dir.join(format!("cut-{cut}"));
            std::fs::write(&p, &bytes[..cut]).unwrap();
            let (e, _) = WalEngine::open(p).unwrap();
            let next = plan_retention(e.snapshot().unwrap().as_ref(), &r, &request).unwrap();
            assert_eq!(next.changed(), cut != bytes.len());
            if mutation {
                assert_eq!(
                    matches!(observed(&e, &r, &i), Work::Advance { restart: true, .. }),
                    cut == bytes.len()
                );
            }
            e.write(next.into_batch()).unwrap();
            assert!(
                !plan_retention(e.snapshot().unwrap().as_ref(), &r, &request)
                    .unwrap()
                    .changed()
            );
            assert_eq!(
                retention_owner(e.snapshot().unwrap().as_ref(), &r, i.pending_id())
                    .unwrap()
                    .unwrap()
                    .phase,
                PinPhase::Published
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn version_11_upgrade_keeps_completed_reader_closure_and_all_source_history_unchanged() {
    let (e, r, i) = fixture();
    rounds(&e, &r, &i, 1, 9);
    refusal(&e, &r, &i);
    census(&e, &r);
    let completed = complete(&e, &r, &i);
    // Image directory and stamp did not exist under the explicit V11 header.
    let mut batch = WriteBatch::new();
    for (k, _) in all(&e) {
        if k >= key(38, &[]) && k < key(42, &[]) {
            batch.delete(ColumnFamily::Default, k);
        }
    }
    e.write(batch).unwrap();
    let rows = || {
        all(&e)
            .into_iter()
            .filter(|(k, _)| *k != key(1, &[]))
            .collect::<Vec<_>>()
    };
    let before = rows();
    let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
    h.truncate(h.len() - 32);
    h[..8].copy_from_slice(READER_CLOSURE_HEADER_MAGIC);
    let mut batch = WriteBatch::new();
    batch.put(ColumnFamily::Default, key(1, &[]), seal(h));
    e.write(batch).unwrap();
    assert_eq!(observed(&e, &r, &i), Work::WaitingForCatalog);
    apply(&e, &r, LedgerRequest::Initialize);
    assert_eq!(observed(&e, &r, &i), Work::Complete(completed));
    assert_eq!(rows(), before);
    assert_eq!(owner(&e, &r, i.pending_id()).phase, PinPhase::Published);
}
