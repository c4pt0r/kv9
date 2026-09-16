//! Constructor/map allocation requests only; no elapsed-time measurement.
mod counting;
mod entry_buffer;
use entry_buffer::EntryBuffer;
use rpds::RedBlackTreeMapSync;
use serde_json::json;
use std::hint::black_box;

#[global_allocator]
static ALLOCATOR: counting::Allocator = counting::Allocator;

#[inline(never)]
fn construct(key: &[u8], value: &[u8]) -> EntryBuffer {
    EntryBuffer::from_slices(black_box(key), black_box(value))
}

fn observe<T>(operation: impl FnOnce() -> T) -> (T, [u64; 6], i64, i64) {
    counting::start();
    let before = counting::live();
    let result = black_box(operation());
    let counts = counting::stop();
    (
        result,
        counts,
        counting::live() - before,
        counting::peak() - before,
    )
}

fn main() {
    let mut rows = Vec::new();
    for key_len in [0, 1, 27, 35, 39, 40, 41, 128, 136, 1024, 4096] {
        for value_len in [0, 1, 7, 128, 4096, 65536] {
            let key: Vec<u8> = (0..key_len).map(|i| (i * 113 + key_len) as u8).collect();
            let mut value: Vec<u8> = (0..value_len).map(|i| (i * 71 + value_len) as u8).collect();
            let total = key_len + value_len;
            let (entry, counts, net, peak) = observe(|| construct(&key, &value));
            assert_eq!(entry.key(), key);
            assert_eq!(entry.value(), value);
            assert_eq!(counts, [u64::from(total > 0), total as u64, 0, 0, 0, 0]);
            let (copy, clone_counts, _, _) = observe(|| entry.clone());
            assert_eq!(copy.key(), key);
            assert_eq!(copy.value(), value);
            assert_eq!(clone_counts, counts);
            let (original, baseline_counts, _, _) = observe(|| (key.clone(), value.clone()));
            assert_eq!(original, (key.clone(), value.clone()));
            rows.push(json!({"operation":"construct_and_clone","key_length":key_len,"value_length":value_len,
                "counts":counts,"clone_counts":clone_counts,"baseline_counts":baseline_counts,
                "net_requested_bytes":net,"extra_requested_bytes":peak}));
            let mut baseline = RedBlackTreeMapSync::<Vec<u8>, Vec<u8>>::new_sync();
            let mut candidate = RedBlackTreeMapSync::<EntryBuffer, ()>::new_sync();
            for operation in ["insert", "overwrite"] {
                if operation == "overwrite" {
                    value.fill(29);
                }
                let (_, b, bn, bp) = observe(|| baseline.insert_mut(key.clone(), value.clone()));
                let (_, c, cn, cp) = observe(|| candidate.insert_mut(construct(&key, &value), ()));
                assert_eq!(baseline.get(key.as_slice()), Some(&value));
                let (stored, ()) = candidate.get_key_value(key.as_slice()).unwrap();
                assert_eq!(stored.key(), key);
                assert_eq!(stored.value(), value);
                assert_eq!(b[0] - c[0], u64::from(key_len > 0 && value_len > 0));
                assert_eq!(b[1] - c[1], 24);
                rows.push(json!({"operation":operation,"key_length":key_len,"value_length":value_len,
                    "baseline_counts":b,"candidate_counts":c,"baseline_net_requested_bytes":bn,
                    "candidate_net_requested_bytes":cn,"baseline_extra_requested_bytes":bp,"candidate_extra_requested_bytes":cp}));
            }
        }
    }
    println!(
        "{}",
        json!({"complete":true,"elapsed_time_recorded":false,
        "entry_buffer_bytes":std::mem::size_of::<EntryBuffer>(),"entry_buffer_alignment":std::mem::align_of::<EntryBuffer>(),
        "candidate_key_value_pair_bytes":std::mem::size_of::<(EntryBuffer,())>(),
        "baseline_key_value_pair_bytes":std::mem::size_of::<(Vec<u8>,Vec<u8>)>(),
        "pair_is_not_private_rpds_entry_layout":true,"rows":rows})
    );
}
