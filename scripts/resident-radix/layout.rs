//! Allocation requests and complete teardown only; never a timing executable.
mod counting;
pub mod radix;
use radix::RadixMap;
use std::collections::BTreeMap;
use std::hint::black_box;

#[global_allocator]
static ALLOCATOR: counting::Allocator = counting::Allocator;

struct Row {
    case: &'static str,
    phase: &'static str,
    counts: [u64; 6],
    net: i64,
    peak: i64,
}

fn observe<T>(case: &'static str, phase: &'static str, op: impl FnOnce() -> T) -> (T, Row) {
    counting::start();
    let before = counting::live();
    let result = black_box(op());
    let counts = counting::stop();
    (
        result,
        Row {
            case,
            phase,
            counts,
            net: counting::live() - before,
            peak: counting::peak() - before,
        },
    )
}

fn inputs(case: &str) -> Vec<Vec<u8>> {
    match case {
        "empty" => vec![],
        "all_byte_edges" => {
            let mut keys = vec![vec![]];
            for byte in 0..=255 {
                keys.extend([
                    vec![byte],
                    vec![byte, 0],
                    vec![byte, 255],
                    vec![byte, 0, 128],
                ]);
            }
            keys
        }
        "prefix_ladder_1024" => (0..=1024).rev().map(|len| vec![0; len]).collect(),
        "common_prefix_4096" => (0..4096)
            .map(|i| format!("kv9:key:{i:08x}").into_bytes())
            .collect(),
        "long_prefix_4096" => (0..4096)
            .map(|i| {
                let mut key = vec![137; 128];
                key.extend(format!("{i:08x}").bytes());
                key
            })
            .collect(),
        "varied_prefix_4096" => (0u32..4096)
            .map(|i| {
                // Odd multiplication is a permutation of u32; no key collisions.
                let mut key = i.wrapping_mul(2_654_435_761).to_le_bytes().to_vec();
                key.extend(std::iter::repeat_n((i >> 8) as u8, (i % 29) as usize));
                key
            })
            .collect(),
        _ => unreachable!(),
    }
}

fn main() {
    // Warm stdout before any lifetime baseline. Rows have fixed reserved storage.
    println!("{{\"measurement\":\"requested allocations only\",\"elapsed_time_recorded\":false,\"allocator\":\"System\",\"rows\":[");
    let mut rows = Vec::with_capacity(54);
    for case in [
        "empty",
        "all_byte_edges",
        "prefix_ladder_1024",
        "common_prefix_4096",
        "long_prefix_4096",
        "varied_prefix_4096",
    ] {
        let keys = inputs(case);
        let before: BTreeMap<_, _> = keys
            .iter()
            .map(|key| (key.clone(), vec![11; 128]))
            .collect();
        assert_eq!(before.len(), keys.len());
        let mut replaced = before.clone();
        for key in keys.iter().step_by(4) {
            replaced.insert(key.clone(), vec![29; 128]);
        }
        let mut deleted = replaced.clone();
        for key in keys.iter().step_by(3) {
            deleted.remove(key);
        }
        let mut missing = vec![255; 16_385];
        while before.contains_key(&missing) {
            missing.push(255);
        }
        let baseline = counting::live();
        let (mut map, row) = observe(case, "build", || {
            let mut map = RadixMap::new();
            for key in &keys {
                assert!(map.insert_mut(key.clone(), vec![11; 128]));
            }
            map
        });
        rows.push(row);
        assert!(map.iter().eq(before.iter()));
        let (snapshot, row) = observe(case, "snapshot", || map.clone());
        assert_eq!(row.counts, [0; 6]);
        assert_eq!(row.net, 0);
        rows.push(row);
        let (same_root, row) = observe(case, "second_snapshot", || snapshot.clone());
        assert_eq!(row.counts, [0; 6]);
        rows.push(row);
        let (_, row) = observe(case, "shared_overwrite_quarter", || {
            for key in keys.iter().step_by(4) {
                assert!(!map.insert_mut(key.clone(), vec![29; 128]));
            }
        });
        rows.push(row);
        assert!(map.iter().eq(replaced.iter()));
        assert!(snapshot.iter().eq(before.iter()));
        let (_, row) = observe(case, "absent_delete", || assert!(!map.remove_mut(&missing)));
        assert_eq!(row.counts, [0; 6]);
        rows.push(row);
        let (_, row) = observe(case, "delete_third_with_snapshots", || {
            for key in keys.iter().step_by(3) {
                assert!(map.remove_mut(key));
            }
        });
        rows.push(row);
        assert!(map.iter().eq(deleted.iter()));
        assert!(same_root.iter().eq(before.iter()));
        let (_, row) = observe(case, "drop_first_snapshot", || drop(snapshot));
        assert_eq!(row.counts, [0; 6]);
        rows.push(row);
        let (_, row) = observe(case, "drop_last_old_snapshot", || drop(same_root));
        rows.push(row);
        let (_, row) = observe(case, "drop_live_map", || drop(map));
        rows.push(row);
        assert_eq!(
            counting::live(),
            baseline,
            "complete teardown leaked requested bytes: {case}"
        );
    }
    assert_eq!(rows.len(), 54);
    for (index, row) in rows.iter().enumerate() {
        if index != 0 {
            println!(",");
        }
        print!("{{\"case\":\"{}\",\"phase\":\"{}\",\"counts\":{:?},\"net_requested_bytes\":{},\"peak_extra_requested_bytes\":{}}}",
            row.case, row.phase, row.counts, row.net, row.peak);
    }
    println!("\n],\"complete\":true,\"all_six_cases_return_to_requested_byte_baseline\":true}}");
}
