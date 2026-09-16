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
fn main() {
    let input = std::env::args().nth(1).expect("sorted original key input");
    let output = std::env::args().nth(2).expect("fresh output");
    let keys: Vec<Vec<u8>> = serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
    assert_eq!(keys.len(), 4096);
    assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
    let mut map = RedBlackTreeMapSync::new_sync();
    for key in &keys {
        map.insert_mut(Key(key.clone()), ());
    }
    let rows: Vec<_> = [false, true].into_iter().map(|miss| {
        let counts: Vec<_> = keys.iter().map(|key| {
            let mut probe = key.clone();
            if miss { probe.push(255); assert!(!keys.contains(&probe)); }
            COMPARISONS.store(0, AtomicOrdering::Relaxed);
            assert_eq!(map.get(&Key(probe)).is_some(), !miss);
            COMPARISONS.load(AtomicOrdering::Relaxed)
        }).collect();
        let selected: Vec<_> = (0..512).map(|i| counts[(i * keys.len() / 512 + 71) % keys.len()]).collect();
        json!({"miss":miss,"all_key_comparisons":counts,
            "all_key_mean":counts.iter().sum::<u64>() as f64/counts.len() as f64,
            "selected_comparisons":selected,"selected_mean":selected.iter().sum::<u64>() as f64/selected.len() as f64})
    }).collect();
    let result = json!({"complete":true,"keys":keys.len(),"rows":rows,
        "scope":"Comparison counts in an order-equivalent key wrapper on unchanged rpds, same sorted insertion order. No timing or allocation result. A fixed-stride probe set may favor internal red-black nodes."});
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .unwrap();
    serde_json::to_writer_pretty(file, &result).unwrap();
    println!("{}", json!({"complete":true,"keys":keys.len()}));
}
