//! `Engine::write` must apply a `WriteBatch` atomically (DESIGN §6.2, §13 principle 15).
//!
//! ## Why these tests are shaped this way
//!
//! The obvious test — reader calls `get(a)` then `get(b)` and flags a mismatch — does
//! **not** test atomicity. Even against a perfectly atomic writer, a whole batch can
//! commit *between* the two `get` calls, so the reader legitimately sees a before/after
//! mix. Such a test stays red after the contract is satisfied, which makes it noise
//! rather than a regression test.
//!
//! Each test below therefore observes both keys through a **single** operation that is
//! itself atomic — one `scan()`, or one `ReadView` — so a mismatch cannot be explained by
//! straddling a commit. Every test is paired with a **control** that deliberately writes
//! the keys in separate batches: the control asserts a mismatch *is* observed, which
//! proves the probe is actually sensitive. Without the controls, these assertions could
//! pass vacuously.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use kv9_engine::{ColumnFamily, Engine, MemEngine, VersionedMemEngine, WriteBatch};

// Every test runs against both in-memory engines: the persistent-map `MemEngine` and the
// versioned `VersionedMemEngine`, which must honour the same atomicity contract.

/// Run `writer` and `reader` concurrently for `millis`, returning the reader's count.
///
/// The writer flips state between two values; the reader counts observations that could
/// only come from a half-applied batch.
fn race<E, W, R>(millis: u64, writer: W, reader: R) -> usize
where
    E: Engine + Default + 'static,
    W: Fn(&dyn Engine, u8) + Send + Sync + 'static,
    R: Fn(&dyn Engine) -> bool + Send + Sync + 'static,
{
    let engine = Arc::new(E::default());

    // Warm up so both keys exist before the reader starts.
    writer(&*engine, 1);

    let stop = Arc::new(AtomicBool::new(false));
    let hits = Arc::new(AtomicUsize::new(0));

    let w = {
        let engine = Arc::clone(&engine);
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            let mut v = 1u8;
            while !stop.load(Ordering::Relaxed) {
                v = if v == 1 { 2 } else { 1 };
                writer(&*engine, v);
            }
        })
    };

    let r = {
        let engine = Arc::clone(&engine);
        let stop = Arc::clone(&stop);
        let hits = Arc::clone(&hits);
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if reader(&*engine) {
                    hits.fetch_add(1, Ordering::Relaxed);
                }
            }
        })
    };

    std::thread::sleep(Duration::from_millis(millis));
    stop.store(true, Ordering::Relaxed);
    w.join().unwrap();
    r.join().unwrap();

    hits.load(Ordering::Relaxed)
}

/// Writes `a` and `b` to the same value in ONE batch, so every committed state has a == b.
fn write_pair_one_batch(engine: &dyn Engine, v: u8) {
    let mut b = WriteBatch::new();
    b.put(ColumnFamily::Default, b"a".to_vec(), vec![v]);
    b.put(ColumnFamily::Default, b"b".to_vec(), vec![v]);
    engine.write(b).unwrap();
}

/// Same keys, but two separate batches — an intermediate state is legitimate here.
fn write_pair_two_batches(engine: &dyn Engine, v: u8) {
    let mut b1 = WriteBatch::new();
    b1.put(ColumnFamily::Default, b"a".to_vec(), vec![v]);
    engine.write(b1).unwrap();
    let mut b2 = WriteBatch::new();
    b2.put(ColumnFamily::Default, b"b".to_vec(), vec![v]);
    engine.write(b2).unwrap();
}

/// Writes across two column families in ONE batch (the catalog row + index shape).
fn write_cross_cf_one_batch(engine: &dyn Engine, v: u8) {
    let mut b = WriteBatch::new();
    b.put(ColumnFamily::Default, b"k".to_vec(), vec![v]);
    b.put(ColumnFamily::Lock, b"k".to_vec(), vec![v]);
    engine.write(b).unwrap();
}

fn write_cross_cf_two_batches(engine: &dyn Engine, v: u8) {
    let mut b1 = WriteBatch::new();
    b1.put(ColumnFamily::Default, b"k".to_vec(), vec![v]);
    engine.write(b1).unwrap();
    let mut b2 = WriteBatch::new();
    b2.put(ColumnFamily::Lock, b"k".to_vec(), vec![v]);
    engine.write(b2).unwrap();
}

/// One `scan()` materializes the whole range under a single lock, so it cannot straddle a
/// commit. Seeing `a != b` through it therefore means the batch was half-applied.
fn scan_sees_mismatch(engine: &dyn Engine) -> bool {
    let entries = engine.scan(ColumnFamily::Default, b"a", b"c", 10).unwrap();
    entries.len() == 2 && entries[0].1 != entries[1].1
}

/// A single `ReadView` is one consistent version of the data, so the same argument
/// applies — and unlike `scan`, it spans column families.
fn snapshot_sees_cross_cf_mismatch(engine: &dyn Engine) -> bool {
    let view = engine.snapshot().unwrap();
    let d = view.get(ColumnFamily::Default, b"k").unwrap();
    let l = view.get(ColumnFamily::Lock, b"k").unwrap();
    d != l
}

fn single_scan_never_observes_a_half_applied_batch_on<E: Engine + Default + 'static>() {
    let torn = race::<E, _, _>(400, write_pair_one_batch, scan_sees_mismatch);
    assert_eq!(
        torn, 0,
        "a single scan observed a half-applied WriteBatch {torn} times"
    );
}

#[test]
fn single_scan_never_observes_a_half_applied_batch() {
    single_scan_never_observes_a_half_applied_batch_on::<MemEngine>();
    single_scan_never_observes_a_half_applied_batch_on::<VersionedMemEngine>();
}

fn control_single_cf_separate_batches_do_show_mismatch_on<E: Engine + Default + 'static>() {
    let seen = race::<E, _, _>(200, write_pair_two_batches, scan_sees_mismatch);
    assert!(
        seen > 0,
        "control saw no mismatch — the single-CF probe is not sensitive, so the \
         atomicity assertion above would pass vacuously"
    );
}

#[test]
fn control_single_cf_separate_batches_do_show_mismatch() {
    control_single_cf_separate_batches_do_show_mismatch_on::<MemEngine>();
    control_single_cf_separate_batches_do_show_mismatch_on::<VersionedMemEngine>();
}

fn read_view_never_observes_a_half_applied_cross_cf_batch_on<E: Engine + Default + 'static>() {
    let torn = race::<E, _, _>(
        400,
        write_cross_cf_one_batch,
        snapshot_sees_cross_cf_mismatch,
    );
    assert_eq!(
        torn, 0,
        "a ReadView observed a half-applied cross-CF WriteBatch {torn} times"
    );
}

#[test]
fn read_view_never_observes_a_half_applied_cross_cf_batch() {
    read_view_never_observes_a_half_applied_cross_cf_batch_on::<MemEngine>();
    read_view_never_observes_a_half_applied_cross_cf_batch_on::<VersionedMemEngine>();
}

fn control_cross_cf_separate_batches_do_show_mismatch_on<E: Engine + Default + 'static>() {
    let seen = race::<E, _, _>(
        200,
        write_cross_cf_two_batches,
        snapshot_sees_cross_cf_mismatch,
    );
    assert!(
        seen > 0,
        "control saw no mismatch — the cross-CF probe is not sensitive, so the \
         atomicity assertion above would pass vacuously"
    );
}

#[test]
fn control_cross_cf_separate_batches_do_show_mismatch() {
    control_cross_cf_separate_batches_do_show_mismatch_on::<MemEngine>();
    control_cross_cf_separate_batches_do_show_mismatch_on::<VersionedMemEngine>();
}

/// A `ReadView` taken once must not change under it, no matter how much the engine moves
/// on. This is the property `meta`'s `index_scan` → `get(pk)` and routing's
/// `seek_le` → `end_key` check both rest on.
fn read_view_is_stable_under_concurrent_writes_on<E: Engine + Default + 'static>() {
    let engine = Arc::new(E::default());
    write_pair_one_batch(&*engine, 1);

    let view = engine.snapshot().unwrap();
    let stop = Arc::new(AtomicBool::new(false));

    let w = {
        let engine = Arc::clone(&engine);
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            let mut v = 1u8;
            while !stop.load(Ordering::Relaxed) {
                v = if v == 1 { 2 } else { 1 };
                write_pair_one_batch(&*engine, v);
            }
        })
    };

    // Re-read the same view many times while the engine churns underneath.
    let first = view.get(ColumnFamily::Default, b"a").unwrap();
    for _ in 0..20_000 {
        assert_eq!(
            view.get(ColumnFamily::Default, b"a").unwrap(),
            first,
            "a ReadView changed value under concurrent writes"
        );
        assert_eq!(
            view.scan(ColumnFamily::Default, b"a", b"c", 10)
                .unwrap()
                .len(),
            2
        );
    }

    stop.store(true, Ordering::Relaxed);
    w.join().unwrap();
}

#[test]
fn read_view_is_stable_under_concurrent_writes() {
    read_view_is_stable_under_concurrent_writes_on::<MemEngine>();
    read_view_is_stable_under_concurrent_writes_on::<VersionedMemEngine>();
}
