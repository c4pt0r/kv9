//! `MemEngine` (persistent red-black tree) against `VersionedMemEngine` (versioned
//! bbtree) through the `Engine` trait they share: the kv9 read and write paths as the
//! runtime drives them — a snapshot per read, whole batches per write, scans with a limit,
//! snapshots held across writes, and mixed read-mostly load from several threads.
//!
//!     cargo run --release -p kv9-engine --example engine_bench
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use kv9_engine::{ColumnFamily, Engine, MemEngine, VersionedMemEngine, WriteBatch};

const N: u64 = 1_000_000;
const SCANS: usize = 20_000;
const OVERWRITES: u64 = 200_000;
const MIXED_SECS: u64 = 3;

fn key(i: u64) -> Vec<u8> {
    format!("r\x00\x00\x64user/{:012}", i).into_bytes()
}
fn value(i: u64) -> Vec<u8> {
    let mut v = vec![0u8; 128];
    v[..8].copy_from_slice(&i.to_be_bytes());
    v
}
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}
fn ns(t: Instant, ops: u64) -> f64 {
    t.elapsed().as_nanos() as f64 / ops as f64
}
fn put(engine: &dyn Engine, i: u64, v: u64) {
    let mut batch = WriteBatch::new();
    batch.put(ColumnFamily::Default, key(i), value(v));
    engine.write(batch).unwrap();
}

fn single_thread(name: &str, engine: &dyn Engine, order: &[u64], probes: &[u64]) {
    let cf = ColumnFamily::Default;
    let t = Instant::now();
    for &i in order {
        put(engine, i, i);
    }
    let insert = ns(t, N);

    let t = Instant::now();
    let mut hits = 0u64;
    for &i in probes {
        if engine.get(cf, &key(i)).unwrap().is_some() {
            hits += 1;
        }
    }
    let get = ns(t, N);

    let t = Instant::now();
    let mut total = 0usize;
    for s in 0..SCANS as u64 {
        total += engine
            .scan(cf, &key((s * 7919) % (N * 4)), &key(N * 4), 100)
            .unwrap()
            .len();
    }
    let scan = ns(t, SCANS as u64);

    let t = Instant::now();
    let view = engine.snapshot().unwrap();
    let mut hits2 = 0u64;
    for &i in &probes[..100_000] {
        if view.get(cf, &key(i)).unwrap().is_some() {
            hits2 += 1;
        }
    }
    let view_get = ns(t, 100_000);

    let held = engine.snapshot().unwrap();
    let t = Instant::now();
    for &i in &order[..OVERWRITES as usize] {
        put(engine, i, i + 1);
    }
    let overwrite = ns(t, OVERWRITES);
    let before = held.get(cf, &key(order[0])).unwrap();
    drop(held);
    println!(
        "{name:<20} write {insert:>6.0} ns   get {get:>6.0} ns   scan100 {scan:>6.0} ns   \
         view.get {view_get:>5.0} ns   overwrite under held view {overwrite:>5.0} ns   held view stable: {}",
        before == Some(value(order[0]))
    );
    std::hint::black_box((hits, total, hits2));
}

fn mixed<E: Engine + 'static>(name: &str, engine: Arc<E>, threads: usize) {
    let stop = Arc::new(AtomicBool::new(false));
    let ops = Arc::new(AtomicU64::new(0));
    let handles: Vec<_> = (0..threads)
        .map(|t| {
            let (engine, stop, ops) = (engine.clone(), stop.clone(), ops.clone());
            std::thread::spawn(move || {
                let mut r = Lcg(1000 + t as u64);
                let mut n = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    for _ in 0..256 {
                        let x = r.next();
                        let k = x % (N * 4);
                        if x % 100 < 95 {
                            std::hint::black_box(
                                engine.get(ColumnFamily::Default, &key(k)).unwrap(),
                            );
                        } else {
                            put(&*engine, k, x);
                        }
                        n += 1;
                    }
                }
                ops.fetch_add(n, Ordering::Relaxed);
            })
        })
        .collect();
    std::thread::sleep(Duration::from_secs(MIXED_SECS));
    stop.store(true, Ordering::Relaxed);
    for h in handles {
        h.join().unwrap();
    }
    println!(
        "{name:<20} mixed 95/5 on {threads:>2} threads: {:>6.2} M ops/s",
        ops.load(Ordering::Relaxed) as f64 / MIXED_SECS as f64 / 1e6
    );
}

fn main() {
    let order: Vec<u64> = {
        let mut r = Lcg(42);
        (0..N).map(|_| r.next() % (N * 4)).collect()
    };
    let probes: Vec<u64> = {
        let mut r = Lcg(7);
        (0..N).map(|_| r.next() % (N * 4)).collect()
    };
    println!("engine (1M entries, kv9-shaped 30 B keys / 128 B values, 1 thread)");
    single_thread("MemEngine (rpds)", &MemEngine::new(), &order, &probes);
    let versioned = VersionedMemEngine::new();
    single_thread("VersionedMemEngine", &versioned, &order, &probes);
    let t = Instant::now();
    let report = versioned.collect_garbage();
    println!(
        "{:<20} collect_garbage: removed {} versions in {:.0} ms, {} entries stored",
        "VersionedMemEngine",
        report.removed,
        t.elapsed().as_secs_f64() * 1e3,
        versioned.stored_entries(ColumnFamily::Default)
    );
    println!();
    for threads in [1usize, 8, 16] {
        let mem = Arc::new(MemEngine::new());
        let ver = Arc::new(VersionedMemEngine::new());
        for &i in &order {
            put(&*mem, i, i);
            put(&*ver, i, i);
        }
        mixed("MemEngine (rpds)", mem, threads);
        let collector = {
            let (ver, stop) = (ver.clone(), Arc::new(AtomicBool::new(false)));
            let flag = stop.clone();
            (
                std::thread::spawn(move || {
                    while !flag.load(Ordering::Relaxed) {
                        ver.collect_garbage();
                        std::thread::sleep(Duration::from_millis(50));
                    }
                }),
                stop,
            )
        };
        mixed("VersionedMemEngine", ver, threads);
        collector.1.store(true, Ordering::Relaxed);
        collector.0.join().unwrap();
    }
}
