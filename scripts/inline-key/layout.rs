//! Allocation requests and actual Rust layout only; no elapsed-time measurement.
mod counting;
#[path = "mem_key.rs"]
mod mem_key;
use mem_key::MapKey;
use rpds::RedBlackTreeMapSync;
use serde_json::json;
use std::hint::black_box;

#[global_allocator]
static ALLOCATOR: counting::Allocator = counting::Allocator;

// Retain one inspectable compiled constructor boundary with ordinary release flags.
#[inline(never)]
fn construct(key: &[u8]) -> MapKey {
    MapKey::from_slice(black_box(key))
}

fn observe<T>(operation: impl FnOnce() -> T) -> (T, [u64; 6], i64, i64) {
    counting::start();
    let initial = counting::live();
    let result = black_box(operation());
    let counts = counting::stop();
    let net = counting::live() - initial;
    let extra = counting::peak() - initial;
    (result, counts, net, extra)
}

fn main() {
    let mut rows = Vec::new();
    for len in [0, 1, 27, 35, 39, 40, 41, 42, 64, 128, 1024] {
        let input: Vec<u8> = (0..len).map(|i| (i * 113 + len) as u8).collect();
        let (key, counts, net, extra) = observe(|| construct(&input));
        assert_eq!(key.as_slice(), input);
        assert_eq!(
            counts,
            if len <= 40 {
                [0; 6]
            } else {
                [1, len as u64, 0, 0, 0, 0]
            }
        );
        let (copy, clone_counts, clone_net, clone_extra) = observe(|| key.clone());
        assert_eq!(copy.as_slice(), input);
        assert_eq!(clone_counts, counts);
        let (owned, baseline_counts, _, _) = observe(|| input.clone());
        assert_eq!(owned, input);
        rows.push(json!({"length":len,"operation":"construct_and_clone",
            "counts":counts,"net_requested_bytes":net,"extra_requested_bytes":extra,
            "clone_counts":clone_counts,"clone_net_requested_bytes":clone_net,
            "clone_extra_requested_bytes":clone_extra,"baseline_counts":baseline_counts}));
        // Input/value construction is outside the insertion window; both paths
        // clone the input in the window, as MemEngine currently does.
        let value = vec![17; 128];
        let mut baseline = RedBlackTreeMapSync::<Vec<u8>, Vec<u8>>::new_sync();
        let mut candidate = RedBlackTreeMapSync::<MapKey, Vec<u8>>::new_sync();
        for operation in ["insert", "overwrite"] {
            let (_, b, bn, bp) = observe(|| baseline.insert_mut(input.clone(), value.clone()));
            let (_, c, cn, cp) = observe(|| candidate.insert_mut(construct(&input), value.clone()));
            assert_eq!(baseline.get(input.as_slice()), Some(&value));
            assert_eq!(candidate.get(input.as_slice()), Some(&value));
            assert_eq!(b[0] - c[0], u64::from(len > 0 && len <= 40));
            rows.push(json!({"length":len,"operation":operation,"baseline_counts":b,
                "candidate_counts":c,"baseline_net_requested_bytes":bn,"candidate_net_requested_bytes":cn,
                "baseline_extra_requested_bytes":bp,"candidate_extra_requested_bytes":cp}));
        }
    }
    println!(
        "{}",
        json!({"complete":true,"elapsed_time_recorded":false,
        "key_bytes":std::mem::size_of::<MapKey>(),"key_alignment":std::mem::align_of::<MapKey>(),
        "baseline_key_bytes":std::mem::size_of::<Vec<u8>>(),
        "key_value_pair_bytes":std::mem::size_of::<(MapKey,Vec<u8>)>(),
        "baseline_key_value_pair_bytes":std::mem::size_of::<(Vec<u8>,Vec<u8>)>(),
        "pair_is_not_private_rpds_entry_layout":true,"rows":rows})
    );
}
