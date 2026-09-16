# Inline-key engine write result

Date: 2026-09-16 UTC. Execution base: `940d13e9a74f00834cf223eb72288566f108f4ea`.

**Hold the inline40 candidate.** The declared write gate fails in five of eight
cases. Original-key means improve **3.773% / 1.685%** for overwrite and
**9.759% / 10.831%** for unique insertion, without/with an old snapshot. Three
miss the 10% material threshold. Long-key unique insertion regresses
**5.077% / 4.783%** in mean and **9.149% / 3.734%** in pooled p99. The next
read/snapshot/range timing stage is **not run**, as predeclared. Production is
unchanged; these are engine group timings, not database QPS.

All **52 processes / 32 timing rows / 16 allocation rows** complete. Independent
input, state, terminal, source, raw-statistic and allocation checks pass. A failed
performance gate is retained separately from successful correctness/accounting.
The [full evidence](inline-key-performance-v1/README.md) includes both orders and
every raw sample; there is no retry or threshold relaxation.

## Exact scope

The [qualified candidate](INLINE-KEY-QUALIFICATION.md) is reused unchanged, with
original archery/rpds/triomphe dependencies. Four new ordinary ThinLTO release
executables isolate timing from counting. All pass release Clippy, explicit
first-party artifact invalidation and compiled-input checks. There are no
alignment flags or default runtime changes.

The original 106-group / 100,096-mutation corpus has 27-byte overwrite keys and
35-byte unique keys. The second dataset pads each original key with zeroes to
128 bytes before appending the unique 8-byte ordinal, yielding 128/136-byte
keys. Values stay 128 bytes. The independent decoder reconstructs all four
final maps and eight probe sets. Four preparations verify **3,392 live prefixes /
1,696 old views / 6,784 refused applications**, exact position/revision and
unchanged other-CF sentinels. Refused writes attempt Default and Lock mutations.

Each case uses baseline/candidate/candidate/baseline order, with twelve measured
passes after one excluded pass. Each arm contributes 2,544 pooled group windows.
The inherited `write_applied` loops and boundaries are byte-identical. Input
construction and old-view acquisition/drop are outside; consumed-input
destruction, map updates and position/revision publication are inside. Full
source projection verifies equivalent counted/timed operation and validation
bodies after removing only declared instrumentation and untimed observations.

All 16 count processes pass independent acceptance before any of the 32 timing
processes start. Measurement uses CPU 4, with helpers on 6–15 and 22–31. The
host is shared; two order directions do not establish statistical significance
or a stable production speedup. Whole-group p99 is not client request p99.

## Timing

All mean/p99 values below are **microseconds per engine group**.

| Keys / operation / old view | Baseline mean | Candidate mean | Mean change | Baseline p99 | Candidate p99 | p99 change |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Original / overwrite / no | 112.081 | 107.852 | -3.773% | 303.775 | 301.791 | -0.653% |
| Original / overwrite / yes | 121.696 | 119.645 | -1.685% | 321.809 | 327.760 | +1.849% |
| Original / unique / no | 257.678 | 232.532 | -9.759% | 818.884 | 691.697 | -15.532% |
| Original / unique / yes | 338.460 | 301.802 | -10.831% | 957.252 | 889.415 | -7.087% |
| Long / overwrite / no | 110.888 | 103.970 | -6.239% | 301.431 | 287.365 | -4.666% |
| Long / overwrite / yes | 127.823 | 124.507 | -2.595% | 340.754 | 338.600 | -0.632% |
| Long / unique / no | 311.897 | 327.733 | +5.077% | 1,114.755 | 1,216.745 | +9.149% |
| Long / unique / yes | 390.593 | 409.276 | +4.783% | 1,281.296 | 1,329.144 | +3.734% |

All original-key means improve in both orders. Long unique means regress in
both orders; their per-order p99 direction differs. Both directions and the
pooled raw-ranked p99 are retained in the [analysis](inline-key-performance-v1/analysis.json).
The -9.759% mean remains below the 10% gate; it is not rounded into a pass.

## Allocation and retained-index observations

The counting companion compares **10,176 window pairs**. For every group, the
independent decoder derives mutation count and distinct updated keys. Short
keys remove one buffer allocation per mutation and change requested entry cost
by `24 - key_length`; long entries add 24 bytes. The validator checks all six
request fields, live-byte differences, peak bounds and aggregate samples.
Pinned overwrite frees only intermediate replacements inside the window;
unmodified old entries remain owned by the old snapshot until its excluded drop.

Unpinned short overwrite goes from **3 to 2 requests/mutation**, and short unique
insertion from **4 to 3**. Long writes retain their request counts. Counter output
contains no elapsed-time measurements. The request savings alone do not
establish a sufficient engine gain or identify the cause of long-key regression.

A separate untimed observation constructs a fresh final index, records its
requested heap bytes and verifies full reclamation on drop. The count includes
two unchanged other-CF sentinels and excludes stack bytes and temporary outputs.

| Final map | Keys in Default CF | Baseline requested bytes | Candidate requested bytes | Difference |
| --- | ---: | ---: | ---: | ---: |
| Original overwrite | 4,096 | 1,093,964 | 1,081,674 | -12,290 |
| Original unique | 100,096 | 27,526,732 | 26,425,674 | -1,101,058 |
| Long overwrite | 4,096 | 1,507,660 | 1,605,962 | +98,302 |
| Long unique | 100,096 | 37,636,428 | 40,038,730 | +2,402,302 |

These are instrumented Rust requested bytes, not jemalloc size classes,
fragmentation or RSS. The final maps have no old snapshot retained; their bytes
do not measure long-lived snapshot memory. Per-window net bytes additionally
include destruction of inputs allocated outside the window, so negative window
values are not negative index growth.

## Next changed variable

Stop unchanged inline40, callback and alignment variants. The
[next source/layout qualification](inline-key-performance-v1/next-entry-buffer-plan.json)
investigates one owned buffer containing key and value bytes, with an explicit
key boundary. This may remove one allocation across both short and long keys
without enlarging an entry. It has **not been implemented or measured**.

The source-backed design uses a private map key holding the combined buffer and
unit as the map value. Ordering/borrowing inspect only the key prefix;
`get_key_value` exposes the stored payload for owned/resident value reads.
Pinned rpds source confirms that equal-key insertion replaces the entire Entry,
including its key, which is essential for same-key value replacement. Before
timing, verify actual layout/allocation, checked lengths, split/concatenate
identity, key-only ordering, replacement semantics and old-view stability.
Keep output/range/delete keys independent of value bytes. This changes private
storage representation; it does not consume owned WriteBatch inputs or repeat
the rejected buffer-consumption, borrowed-upsert or packed-index experiments.

No new full-database timing, Redis comparison, external MinIO, ordinary
distributed recovery or actual Chaos coverage is claimed. The latest database
results remain **137,873.776 Put calls/s / 1,022,750.059 BatchPut(64) items/s**,
source `86aa6fc`, c64, 128-byte values, three Raft replicas and the shared-host
volatile tmpfs fixture. Promotion still needs material matched database benefit,
full local correctness, ordinary recovery, actual Chaos Mesh and three-copy
Redis throughput/latency. CI stays local; no industrial checkbox is closed.
