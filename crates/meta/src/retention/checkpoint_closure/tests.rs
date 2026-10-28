use super::*;
use crate::retention::checkpoint_intent::tests::fixture_intent;
use crate::retention::tests::{all, apply, owner, plan, seed};
use kv9_engine::{Engine, MemEngine, WalEngine};

fn fixture() -> (MemEngine, RootDescriptor, CheckpointIntent) {
    let (e, r, i) = fixture_intent();
    apply(&e, &r, LedgerRequest::BeginCheckpoint(i.clone()));
    (e, r, i)
}
fn binding(i: &CheckpointIntent, n: u8) -> OwnerBinding {
    checkpoint_admission::executor_binding(
        i,
        LocalLifetime {
            node: 1,
            store: [1; 16],
            process: [n; 16],
        },
    )
    .unwrap()
}
fn share(i: &CheckpointIntent, e: &OwnerBinding) -> LedgerRequest {
    LedgerRequest::Share {
        from: i.binding(OwnerKind::Pending).unwrap().token(),
        to: e.clone(),
    }
}
fn finish(i: &CheckpointIntent, e: &OwnerBinding, issued: u64, resolved: u64) -> LedgerRequest {
    LedgerRequest::FinishCheckpointExecution {
        pending: i.binding(OwnerKind::Pending).unwrap().token(),
        executor: e.clone(),
        issued,
        resolved,
    }
}
fn close(e: &MemEngine, r: &RootDescriptor, i: &CheckpointIntent) {
    apply(
        e,
        r,
        LedgerRequest::CloseCheckpointAdmission(i.binding(OwnerKind::Pending).unwrap().token()),
    );
}
fn census(e: &MemEngine, r: &RootDescriptor) {
    loop {
        let work = build_index::observe(e.snapshot().unwrap().as_ref(), r).unwrap();
        match work {
            build_index::Work::Complete => break,
            build_index::Work::Page { progress, next, .. } => {
                apply(e, r, progress.request(next));
            }
            build_index::Work::Upgrade => {
                apply(e, r, LedgerRequest::Initialize);
            }
            _ => panic!("ledger must exist"),
        }
    }
}
fn observed(e: &impl Engine, r: &RootDescriptor, i: &CheckpointIntent) -> Work {
    observe(e.snapshot().unwrap().as_ref(), r, i.pending_id()).unwrap()
}
fn advance_once(e: &MemEngine, r: &RootDescriptor, i: &CheckpointIntent) -> LedgerRequest {
    let Work::Advance { cursor, .. } = observed(e, r, i) else {
        panic!("work expected")
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
        advance_once(e, r, i);
    }
    panic!("closure did not complete")
}
fn zero_generations(
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
        apply(e, r, finish(i, &b, 0, 0));
    }
    b
}

#[test]
fn aggregates_every_owner_and_dense_generation_without_releasing_pending() {
    let (e, r, i) = fixture();
    for n in 1..=5 {
        zero_generations(&e, &r, &i, n, 13);
    }
    assert_eq!(observed(&e, &r, &i), Work::Open);
    close(&e, &r, &i);
    assert_eq!(observed(&e, &r, &i), Work::WaitingForCatalog);
    census(&e, &r);
    let first = advance_once(&e, &r, &i);
    assert!(
        !plan(&e, &r, &first).unwrap().changed(),
        "lost receipt is an exact no-op retry"
    );
    let encoded = encode_request(r.digest(), &first).unwrap();
    assert_eq!(decode_request(&encoded, r.digest()).unwrap(), first);
    for n in 0..encoded.len() {
        assert!(decode_request(&encoded[..n], r.digest()).is_err());
    }
    let c = complete(&e, &r, &i);
    assert_eq!(
        c.counts,
        Counts {
            owners: 5,
            generations: 65,
            zero_put: 65,
            ..Counts::default()
        }
    );
    assert!(observed(&e, &r, &i).registered_zero_put_closed());
    assert!(!plan(&e, &r, &c.request(i.pending_id())).unwrap().changed());
    assert!(
        plan(&e, &r, &first).is_err(),
        "older prefixes cannot skip a page"
    );
    assert_eq!(owner(&e, &r, i.pending_id()).phase, PinPhase::Published);
    assert!(e
        .get(
            ColumnFamily::Default,
            &owner_key(i.binding(OwnerKind::Version).unwrap().descriptor.id)
        )
        .unwrap()
        .is_none());
}

#[test]
fn legacy_origin_missing_terminal_active_and_nonzero_puts_survive_later_success() {
    let (e, r, i) = fixture_intent();
    let legacy = binding(&i, 1);
    apply(&e, &r, LedgerRequest::Acquire(legacy.clone()));
    apply(&e, &r, LedgerRequest::BeginCheckpoint(i.clone()));
    let mut b = binding(&i, 2);
    apply(&e, &r, share(&i, &b));
    apply(&e, &r, LedgerRequest::Publish(b.token()));
    apply(
        &e,
        &r,
        LedgerRequest::QuiesceAfterTransfer {
            from: b.token(),
            to: i.binding(OwnerKind::Pending).unwrap().token(),
        },
    );
    apply(&e, &r, LedgerRequest::Release(b.token())); // no generation 1 report
    for (g, issued, resolved) in [(2, 2, 1), (3, 1, 1)] {
        b.generation = NonZeroU64::new(g).unwrap();
        apply(&e, &r, share(&i, &b));
        apply(&e, &r, LedgerRequest::Publish(b.token()));
        apply(&e, &r, finish(&i, &b, issued, resolved));
    }
    b.generation = NonZeroU64::new(4).unwrap();
    apply(&e, &r, LedgerRequest::CancelUnenteredCheckpoint(b));
    let active = binding(&i, 3);
    apply(&e, &r, share(&i, &active));
    close(&e, &r, &i);
    census(&e, &r);
    let c = complete(&e, &r, &i);
    assert_eq!(
        c.counts,
        Counts {
            owners: 3,
            generations: 6,
            zero_put: 1,
            missing_origin: 1,
            active: 1,
            missing_terminal: 1,
            nonzero_put: 2,
            unresolved_puts: 1
        }
    );
    assert!(!observed(&e, &r, &i).registered_zero_put_closed());
    assert_eq!(owner(&e, &r, i.pending_id()).phase, PinPhase::Published);
}

#[test]
fn late_facts_and_insertion_behind_cursor_restart_but_other_attempts_do_not() {
    let (e, r, i) = fixture();
    let mut sorted: Vec<_> = (1..=3).map(|n| binding(&i, n)).collect();
    sorted.sort_by_key(|b| b.descriptor.id);
    for b in &sorted[1..] {
        apply(&e, &r, share(&i, b));
        apply(&e, &r, finish(&i, b, 0, 0));
    }
    close(&e, &r, &i);
    census(&e, &r);
    advance_once(&e, &r, &i); // passed the first existing owner
    let other = CheckpointIntent::new(&r, &i.manifest().unwrap(), 1).unwrap();
    apply(&e, &r, LedgerRequest::BeginCheckpoint(other));
    assert!(matches!(
        observed(&e, &r, &i),
        Work::Advance { restart: false, .. }
    ));
    apply(
        &e,
        &r,
        LedgerRequest::CancelUnenteredCheckpoint(sorted[0].clone()),
    );
    assert!(matches!(
        observed(&e, &r, &i),
        Work::Advance { restart: true, .. }
    ));
    assert_eq!(complete(&e, &r, &i).counts.generations, 3);
    let old = e.snapshot().unwrap();
    let mut next = sorted[1].clone();
    next.generation = NonZeroU64::new(2).unwrap();
    apply(&e, &r, LedgerRequest::CancelUnenteredCheckpoint(next));
    assert!(observe(old.as_ref(), &r, i.pending_id())
        .unwrap()
        .registered_zero_put_closed());
    assert!(
        !observed(&e, &r, &i).registered_zero_put_closed(),
        "stale complete must not qualify"
    );
    assert_eq!(complete(&e, &r, &i).counts.generations, 4);
    let before = all(&e);
    assert!(!plan(&e, &r, &finish(&i, &sorted[2], 0, 0))
        .unwrap()
        .changed());
    assert_eq!(all(&e), before);

    let (e, r, i) = fixture();
    let b = binding(&i, 7);
    apply(&e, &r, share(&i, &b));
    close(&e, &r, &i);
    census(&e, &r);
    assert_eq!(complete(&e, &r, &i).counts.active, 1);
    apply(&e, &r, finish(&i, &b, 0, 0));
    assert!(matches!(
        observed(&e, &r, &i),
        Work::Advance { restart: true, .. }
    ));
    assert_eq!(complete(&e, &r, &i).counts.zero_put, 1);
}

#[test]
fn legacy_attempt_never_gains_atomic_origin_from_complete_zero_put_census() {
    let (e, r, i) = fixture_intent();
    let p = i.binding(OwnerKind::Pending).unwrap();
    apply(&e, &r, LedgerRequest::Acquire(p.clone()));
    apply(&e, &r, LedgerRequest::Publish(p.token()));
    apply(&e, &r, LedgerRequest::RecordCheckpoint(i.clone()));
    zero_generations(&e, &r, &i, 1, 2);
    close(&e, &r, &i);
    census(&e, &r);
    let c = complete(&e, &r, &i);
    assert_eq!(c.birth_revision, 0);
    assert_eq!(c.counts.zero_put, 2);
    assert!(!observed(&e, &r, &i).registered_zero_put_closed());
}

#[test]
fn malformed_progress_missing_stamp_and_header_downgrade_refuse() {
    for damage in 0..6 {
        let (e, r, i) = fixture();
        close(&e, &r, &i);
        census(&e, &r);
        complete(&e, &r, &i);
        let mut batch = WriteBatch::new();
        match damage {
            0 => {
                batch.delete(ColumnFamily::Default, stamp_key(i.pending_id()));
            }
            1 => {
                batch.put(
                    ColumnFamily::Default,
                    progress_key(i.pending_id()),
                    vec![0; 200],
                );
            }
            2 => {
                let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
                h.truncate(h.len() - 32);
                h[..8].copy_from_slice(ATOMIC_HEADER_MAGIC);
                batch.put(ColumnFamily::Default, key(1, &[]), seal(h));
            }
            3 => {
                batch.delete(ColumnFamily::Default, key(17, i.pending_id().as_bytes()));
            }
            _ => {
                let k = stamp_key(i.pending_id());
                let mut b = e.get(ColumnFamily::Default, &k).unwrap().unwrap();
                b.truncate(b.len() - 32);
                let n = if damage == 4 { u64::MAX } else { 1 };
                b[56..64].copy_from_slice(&n.to_be_bytes());
                batch.put(ColumnFamily::Default, k, seal(b));
            }
        }
        e.write(batch).unwrap();
        assert!(
            observe(e.snapshot().unwrap().as_ref(), &r, i.pending_id()).is_err(),
            "damage {damage}"
        );
    }
}

#[test]
fn version_six_upgrade_preserves_birth_and_builds_then_bootstraps_change_stamp() {
    let (e, r, i) = fixture();
    zero_generations(&e, &r, &i, 1, 2);
    close(&e, &r, &i);
    census(&e, &r);
    let original = e
        .get(ColumnFamily::Default, &key(17, i.pending_id().as_bytes()))
        .unwrap();
    let mut batch = WriteBatch::new();
    batch.delete(ColumnFamily::Default, stamp_key(i.pending_id()));
    let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
    h.truncate(h.len() - 32);
    h[..8].copy_from_slice(ATOMIC_HEADER_MAGIC);
    batch.put(ColumnFamily::Default, key(1, &[]), seal(h));
    // A stopped older-ledger fixture has no V20 image Publisher records.
    for (k, _) in all(&e) {
        if k >= key(42, &[]) && k < key(48, &[]) {
            batch.delete(ColumnFamily::Default, k);
        }
    }
    e.write(batch).unwrap();
    assert_eq!(observed(&e, &r, &i), Work::WaitingForCatalog);
    apply(&e, &r, LedgerRequest::Initialize);
    assert_eq!(
        e.get(ColumnFamily::Default, &key(17, i.pending_id().as_bytes()))
            .unwrap(),
        original
    );
    assert_eq!(complete(&e, &r, &i).counts.zero_put, 2);
    assert!(observed(&e, &r, &i).registered_zero_put_closed());
}

#[test]
fn wal_tail_cuts_keep_drain_and_stamp_atomic_and_resume_old_cursor() {
    let (memory, r, i) = fixture();
    let b = binding(&i, 1);
    apply(&memory, &r, share(&i, &b));
    close(&memory, &r, &i);
    census(&memory, &r);
    let before_cursor = complete(&memory, &r, &i);
    assert_eq!(before_cursor.counts.active, 1);
    let dir = std::env::temp_dir().join(format!(
        "kv9-closure-{}",
        kv9_common::StoreIncarnation::mint().unwrap()
    ));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("meta.wal");
    let (engine, _) = WalEngine::open(&path).unwrap();
    let (_, mut batch) = seed();
    for (k, v) in all(&memory) {
        batch.put(ColumnFamily::Default, k, v);
    }
    engine.write(batch).unwrap();
    let before = std::fs::read(&path).unwrap();
    engine
        .write(
            plan(&engine, &r, &finish(&i, &b, 0, 0))
                .unwrap()
                .into_batch(),
        )
        .unwrap();
    drop(engine);
    let after = std::fs::read(&path).unwrap();
    for cut in [
        before.len(),
        before.len() + 1,
        before.len() + (after.len() - before.len()) / 2,
        after.len() - 1,
        after.len(),
    ] {
        let p = dir.join(format!("cut-{cut}.wal"));
        std::fs::write(&p, &after[..cut]).unwrap();
        let (engine, _) = WalEngine::open(&p).unwrap();
        let w = observed(&engine, &r, &i);
        if cut == after.len() {
            assert!(matches!(w, Work::Advance { restart: true, .. }));
            loop {
                match observed(&engine, &r, &i) {
                    Work::Advance { cursor, .. } => {
                        let request = cursor.request(i.pending_id());
                        engine
                            .write(plan(&engine, &r, &request).unwrap().into_batch())
                            .unwrap();
                        assert!(!plan(&engine, &r, &request).unwrap().changed());
                    }
                    Work::Complete(c) => {
                        assert_eq!(c.counts.zero_put, 1);
                        break;
                    }
                    _ => panic!("closed census expected"),
                }
            }
            assert!(observed(&engine, &r, &i).registered_zero_put_closed());
        } else {
            assert_eq!(w, Work::Complete(before_cursor.clone()));
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

struct Count<'a> {
    view: &'a dyn ReadView,
    reads: std::sync::atomic::AtomicUsize,
    rows: std::sync::atomic::AtomicUsize,
}
impl ReadView for Count<'_> {
    fn get(&self, cf: ColumnFamily, k: &[u8]) -> Result<Option<Vec<u8>>> {
        self.reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.view.get(cf, k)
    }
    fn scan(
        &self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<kv9_engine::ScanEntry>> {
        assert!(!start.starts_with(PREFIX));
        self.view.scan(cf, start, end, limit)
    }
    fn seek_le(&self, cf: ColumnFamily, target: &[u8]) -> Result<Option<kv9_engine::ScanEntry>> {
        assert!(!target.starts_with(PREFIX));
        self.view.seek_le(cf, target)
    }
    fn iter<'a>(
        &'a self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
    ) -> Result<Box<dyn Iterator<Item = Result<kv9_engine::ScanEntry>> + 'a>> {
        let tracked = start.starts_with(PREFIX);
        if tracked {
            assert!(
                start.starts_with(&key(14, &[])),
                "only the bounded Build index may be scanned"
            );
        }
        Ok(Box::new(self.view.iter(cf, start, end)?.inspect(
            move |_| {
                if tracked {
                    self.rows.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
#[test]
fn each_committed_page_has_fixed_work_for_13_and_113_owners_or_generations() {
    let mut measured = Vec::new();
    for (owners, generations) in [(1, 13), (1, 113), (13, 1), (113, 1)] {
        let (e, r, i) = fixture();
        for n in 1..=owners {
            zero_generations(&e, &r, &i, n, generations);
        }
        close(&e, &r, &i);
        census(&e, &r);
        let Work::Advance { cursor, .. } = observed(&e, &r, &i) else {
            panic!("page")
        };
        let snapshot = e.snapshot().unwrap();
        let count = Count {
            view: snapshot.as_ref(),
            reads: 0.into(),
            rows: 0.into(),
        };
        let planned = plan_retention(&count, &r, &cursor.request(i.pending_id())).unwrap();
        let reads = count.reads.load(std::sync::atomic::Ordering::Relaxed);
        let rows = count.rows.load(std::sync::atomic::Ordering::Relaxed);
        assert!(rows <= build_index::PAGE + 1);
        e.write(planned.into_batch()).unwrap();
        let Work::Advance { cursor, .. } = observed(&e, &r, &i) else {
            panic!("next page")
        };
        assert!(cursor.counts.generations <= enrollment::HISTORY_PAGE as u64);
        eprintln!("closure_page owners={owners} generations={generations} point_reads={reads} index_rows={rows}");
        measured.push((reads, rows));
    }
    assert_eq!(measured[0], measured[1]);
    assert_eq!(measured[2], measured[3]);
}
