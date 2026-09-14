//! Standalone hot-buffer kernel experiment; never database QPS or Ready-group evidence.

#[path = "../../crates/raft/src/storage/fnv.rs"]
mod fnv;

use std::hint::black_box;
use std::time::{Duration, Instant};

const CASES: [(&str, [usize; 4]); 9] = [
    ("equal-0", [0; 4]),
    ("equal-32", [32; 4]),
    ("equal-205", [205; 4]),
    ("equal-206", [206; 4]),
    ("equal-10600", [10600; 4]),
    ("equal-10601", [10601; 4]),
    ("equal-65536", [65536; 4]),
    ("skew-point-batch", [205, 205, 10600, 205]),
    ("skew-empty-point-batch", [0, 205, 10600, 32]),
];
const REPEATS_PER_ORDER: usize = 3;
const TARGET_BYTES: usize = 64 * 1024 * 1024;
const EMPTY_GROUPS: usize = 1_000_000;
const MIN_GROUPS: usize = 256;
const MAX_GROUPS: usize = 1_000_000;
const WARMUP_GROUPS: usize = 32;

#[inline(never)]
fn serial(inputs: [&[u8]; 4]) -> [u32; 4] {
    [
        fnv::fnv1a(inputs[0]),
        fnv::fnv1a(inputs[1]),
        fnv::fnv1a(inputs[2]),
        fnv::fnv1a(inputs[3]),
    ]
}

#[inline(never)]
fn interleaved(inputs: [&[u8]; 4]) -> [u32; 4] {
    fnv::fnv1a_four(inputs)
}

fn main() {
    assert_eq!(
        std::env::args_os().count(),
        1,
        "fixed protocol has no runtime tuning flags"
    );
    let started = Instant::now();
    let buffers: Vec<[Vec<u8>; 4]> = CASES
        .iter()
        .map(|(_, lengths)| {
            std::array::from_fn(|lane| {
                (0..lengths[lane])
                    .map(|offset| {
                        (offset.wrapping_mul(131).wrapping_add(lane * 17 + 71) & 255) as u8
                    })
                    .collect()
            })
        })
        .collect();
    // Refuse a broken candidate before reporting any timings; correctness qualification
    // is owned separately. This check does not replace its proof or exhaustive controls.
    for data in &buffers {
        let inputs = std::array::from_fn(|lane| data[lane].as_slice());
        assert_eq!(serial(black_box(inputs)), interleaved(black_box(inputs)));
    }
    println!(
        "{{\"kind\":\"protocol\",\"version\":1,\"scope\":\"fnv-four-frame-kernel-only\",\"cases\":9,\"repeats_per_order\":{REPEATS_PER_ORDER},\"target_bytes\":{TARGET_BYTES},\"empty_groups\":{EMPTY_GROUPS},\"min_groups\":{MIN_GROUPS},\"max_groups\":{MAX_GROUPS},\"warmup_groups\":{WARMUP_GROUPS}}}"
    );
    let mut rows = 0usize;
    let mut all_input_bytes = 0u64;
    for repeat in 0..REPEATS_PER_ORDER {
        // Reverse the complete 18-job list, including both case and implementation order.
        for order in ["forward", "reverse"] {
            let mut jobs: Vec<(usize, bool)> = (0..CASES.len())
                .flat_map(|case| [(case, false), (case, true)])
                .collect();
            if order == "reverse" {
                jobs.reverse();
            }
            for (ordinal, (case, parallel)) in jobs.into_iter().enumerate() {
                assert!(
                    started.elapsed() < Duration::from_secs(60),
                    "kernel time budget exceeded"
                );
                let (label, lengths) = CASES[case];
                let bytes_per_group: usize = lengths.iter().sum();
                let groups = if bytes_per_group == 0 {
                    EMPTY_GROUPS
                } else {
                    (TARGET_BYTES / bytes_per_group).clamp(MIN_GROUPS, MAX_GROUPS)
                };
                let inputs = std::array::from_fn(|lane| buffers[case][lane].as_slice());
                let function: fn([&[u8]; 4]) -> [u32; 4] =
                    if parallel { interleaved } else { serial };
                let function = black_box(function);
                for _ in 0..WARMUP_GROUPS {
                    black_box(function(black_box(inputs)));
                }
                let before = Instant::now();
                for _ in 0..groups {
                    black_box(function(black_box(inputs)));
                }
                let elapsed_ns = u64::try_from(before.elapsed().as_nanos()).unwrap();
                assert!(elapsed_ns > 0, "empty clock interval");
                let total_input_bytes =
                    u64::try_from(groups.checked_mul(bytes_per_group).unwrap()).unwrap();
                let ns_per_group = elapsed_ns as f64 / groups as f64;
                let mib_per_second = if bytes_per_group == 0 {
                    "null".to_owned()
                } else {
                    format!(
                        "{:.9}",
                        total_input_bytes as f64 * 1e9 / elapsed_ns as f64 / 1_048_576.0
                    )
                };
                let implementation = if parallel { "interleaved" } else { "serial" };
                println!(
                    "{{\"kind\":\"measurement\",\"repeat\":{repeat},\"order\":\"{order}\",\"ordinal\":{ordinal},\"case\":\"{label}\",\"lengths\":{lengths:?},\"implementation\":\"{implementation}\",\"groups\":{groups},\"bytes_per_group\":{bytes_per_group},\"input_bytes\":{total_input_bytes},\"elapsed_ns\":{elapsed_ns},\"ns_per_group\":{ns_per_group:.9},\"mib_per_second\":{mib_per_second}}}"
                );
                all_input_bytes = all_input_bytes.checked_add(total_input_bytes).unwrap();
                rows += 1;
            }
        }
    }
    assert_eq!(rows, 108);
    println!(
        "{{\"kind\":\"complete\",\"complete\":true,\"rows\":{rows},\"input_bytes\":{all_input_bytes},\"wall_elapsed_ns\":{}}}",
        started.elapsed().as_nanos()
    );
}
