//! Fixed-work observer microexperiment. No network or server-throughput claim.
use std::hint::black_box;
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use kv9_common::metrics::{bucket_bounds, Latency, NamedLatency, Outcome};

fn recording(threads: usize, iterations: usize, enabled: bool) -> (u128, u64) {
    let metric = Arc::new(Latency::default());
    let barrier = Arc::new(Barrier::new(threads + 1));
    let mut workers = Vec::new();
    for _ in 0..threads {
        let metric = metric.clone();
        let barrier = barrier.clone();
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            for _ in 0..iterations {
                let start = Instant::now();
                let elapsed = black_box(start.elapsed());
                if enabled {
                    metric.record(elapsed, Outcome::Success);
                } else {
                    black_box((elapsed, Outcome::Success));
                }
            }
        }));
    }
    let start = Instant::now();
    barrier.wait();
    for worker in workers {
        worker.join().unwrap();
    }
    let elapsed = start.elapsed().as_nanos();
    let count = metric.snapshot().outcomes[Outcome::Success as usize].count;
    assert_eq!(
        count,
        if enabled {
            (threads * iterations) as u64
        } else {
            0
        }
    );
    (elapsed, count)
}

fn export(iterations: usize, dense: bool) -> (u128, usize) {
    let metrics: [Latency; 26] = std::array::from_fn(|_| Latency::default());
    for metric in &metrics {
        for outcome in Outcome::ALL {
            if dense {
                for bucket in 0..65 {
                    metric.record(
                        Duration::from_nanos(bucket_bounds(bucket).unwrap().lower_ns),
                        outcome,
                    );
                }
            } else {
                metric.record(Duration::from_micros(100), outcome);
            }
        }
    }
    let start = Instant::now();
    let mut bytes = 0;
    for _ in 0..iterations {
        // A fixed 26-metric fixture, comparable in name width to the exporter.
        // Measures snapshot allocation, rank calculation and JSON serialization;
        // filesystem publication and the surrounding envelope are excluded.
        let snapshot: Vec<_> = metrics
            .iter()
            .map(|m| NamedLatency::new("overhead_fixture_latency_metric", m))
            .collect();
        let encoded = serde_json::to_vec(&snapshot).unwrap();
        bytes = encoded.len();
        black_box(encoded);
    }
    (start.elapsed().as_nanos(), bytes)
}

fn main() {
    if cfg!(debug_assertions) {
        eprintln!("observer experiment requires --release");
        std::process::exit(1);
    }
    let iterations = 100_000;
    let trials = 7;
    for threads in [1, 8] {
        for trial in 0..trials {
            // Alternate paired order to reduce systematic warm-up/order bias.
            for enabled in if trial % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                let (elapsed, samples) = recording(threads, iterations, enabled);
                println!(
                    "{}",
                    serde_json::json!({
                        "experiment": "record", "enabled": enabled, "threads": threads,
                        "trial": trial, "operations": threads * iterations,
                        "elapsed_wall_ns": elapsed, "observed_samples": samples,
                        "wall_ns_per_operation": elapsed as f64 / (threads * iterations) as f64,
                        "latency_state_bytes": std::mem::size_of::<Latency>(),
                    })
                );
            }
        }
    }
    for dense in [false, true] {
        for trial in 0..trials {
            let iterations = 100;
            let (elapsed, bytes) = export(iterations, dense);
            println!(
                "{}",
                serde_json::json!({
                    "experiment": "snapshot_json", "dense": dense, "trial": trial,
                    "operations": iterations, "metric_count": 26, "json_bytes": bytes,
                    "elapsed_wall_ns": elapsed,
                    "wall_ns_per_operation": elapsed as f64 / iterations as f64,
                })
            );
        }
    }
}
