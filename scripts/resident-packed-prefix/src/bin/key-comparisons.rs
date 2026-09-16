//! Read-only comparison-count diagnostic; not a latency benchmark.
use std::cmp::Ordering;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use rpds::RedBlackTreeMapSync;
use serde_json::json;

static COMPARISONS: AtomicU64 = AtomicU64::new(0);
#[derive(Clone, Eq, PartialEq)]
struct Key(Vec<u8>);
impl Ord for Key {
    fn cmp(&self, other: &Self) -> Ordering {
        COMPARISONS.fetch_add(1, AtomicOrdering::Relaxed);
        self.0.cmp(&other.0)
    }
}
impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
fn unhex(text: &str) -> Vec<u8> {
    assert!(text.len().is_multiple_of(2));
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}
fn distribution(counts: &[u64]) -> serde_json::Value {
    let mut hist = std::collections::BTreeMap::<u64, usize>::new();
    for count in counts {
        *hist.entry(*count).or_default() += 1;
    }
    json!({"count":counts.len(),"sum":counts.iter().sum::<u64>(),"mean":counts.iter().sum::<u64>() as f64 / counts.len() as f64,"histogram":hist})
}
fn main() {
    let input = std::env::args().nth(1).expect("prepared inputs");
    let output = std::env::args().nth(2).expect("fresh output");
    let bytes = std::fs::read(input).unwrap();
    let prepared: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(prepared["complete"], true);
    let mut rows = Vec::new();
    for work in prepared["read_inputs"].as_array().unwrap() {
        let keys: Vec<_> = work["keys_hex"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| unhex(v.as_str().unwrap()))
            .collect();
        assert!(keys.windows(2).all(|w| w[0] < w[1]));
        let mut map = RedBlackTreeMapSync::new_sync();
        for key in &keys {
            map.insert_mut(Key(key.clone()), ());
        }
        for miss in [false, true] {
            let operation = if miss { "get_miss" } else { "get_hit" };
            let all: Vec<_> = keys
                .iter()
                .map(|k| {
                    let mut probe = k.clone();
                    if miss {
                        probe.push(255);
                    }
                    COMPARISONS.store(0, AtomicOrdering::Relaxed);
                    assert_eq!(map.get(&Key(probe)).is_some(), !miss);
                    COMPARISONS.load(AtomicOrdering::Relaxed)
                })
                .collect();
            let probes = work["queries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["operation"] == operation)
                .unwrap();
            let selected: Vec<_> = probes["queries_hex"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| {
                    let probe = Key(unhex(v.as_str().unwrap()));
                    COMPARISONS.store(0, AtomicOrdering::Relaxed);
                    assert_eq!(map.get(&probe).is_some(), !miss);
                    COMPARISONS.load(AtomicOrdering::Relaxed)
                })
                .collect();
            rows.push(json!({"dataset":work["dataset"],"workload":work["workload"],"operation":operation,"query_sha256":probes["query_sha256"],"all":distribution(&all),"selected":distribution(&selected)}));
        }
    }
    use sha2::{Digest, Sha256};
    let result = json!({"complete":true,"prepared_sha256":format!("{:x}",Sha256::digest(&bytes)),"rows":rows,
        "scope":"Count-only rpds order-equivalent key wrapper, same sorted population and exact prepared queries. All-key versus seeded sample comparison depth, not latency."});
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .unwrap();
    serde_json::to_writer_pretty(file, &result).unwrap();
    println!("{}", json!({"complete":true,"rows":12}));
}
