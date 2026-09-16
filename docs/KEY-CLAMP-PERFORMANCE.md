# Safe key-accessor qualification and three-arm write screen

Date: 2026-09-16 UTC. Execution base: `b23506dc5ee43d52b3a82a0c7f2b3d5abe0003ad`.

**Reject the clamped accessor for production. Stop this single-buffer family.**
The new candidate passes source/model/proof/allocation qualification but fails
all four original-key write gates. Against the selected baseline, their mean
latencies regress 2.462%–6.486% and pooled p99 regresses 3.143%–8.430%.
Against the concurrently built failed single-buffer control, the accessor has
no consistent original-key gain. Read/range timing stops by the declared gate.
No production or database-QPS result changes.

## Qualification and exact change

Only `key()` changes from `bytes[..key_len]` to the safe expression
`bytes[..key_len.min(bytes.len())]`. Constructor, value suffix accessor,
whole-batch length preflight, engine integration and original dependencies are
unchanged. The private constructor establishes the length invariant and derived
Clone preserves both fields. No unsafe code or mutable field API is added.

The [Lean proof](../proofs/lean/key-clamp/README.md) rechecks the 23 representation
statements under the changed key definition and adds three invariant/accessor
lemmas: **26 theorems / 15 rejecting controls** pass. This is conditional
refinement with source mapping, not verified Rust extraction. Safe Rust/compiler
and pinned rpds map/order/persistence remain explicit premises. No new Raft,
allocation-failure or lock proof is claimed.

The changed candidate passes **206 local engine/common tests**, with Clippy
warnings denied and 25 existing ignored entries. The latter are 23 external
MinIO-related tests and two WAL microbenchmarks; they are not executed here.
The same independent models check 240 live states, 1,080 old views and two
payload-replacement/boundary scenarios. Exact-source-verified baseline/control
results (202 / 206 passed, 25 ignored each) are reused without rerunning unchanged
tests. All 198 new standalone allocation observations equal the control's
observations exactly. Selected upstream files match 50 source/manifest members
of their Cargo.lock-pinned archives.

## Actual release preparation and allocation

Six fresh ordinary ThinLTO screen builds identify separate timing/counting
executables for baseline, single-buffer control and clamped candidate. All deny
Clippy warnings. First-party artifacts are invalidated and actual dependency
origins are checked. Timing/counting operation bodies match after removing only
declared instrumentation and explicitly untimed observations. Exact timing-ELF
insertion excerpts retain **0 / 2 / 0 unsigned boundary-failure branches** before
memcmp; the new accessor uses conditional moves. This establishes the intended
code-generation difference, not a performance benefit.

The first harness attempt fails its first preparation before workload
construction: inherited Rust configuration checks still required the old
four-position/two-arm plan. The correction changes only the fixed order array
and `order < 6` guard in both harnesses. That failed process, six initial builds
and original tools are retained separately. No timing sample came from them.
The corrected attempt keeps operation loops byte-identical and runs all
**78 accepted workload processes: six preparations, 24 allocation and 48 timing**.

Independent byte reconstruction checks **5,088 live prefixes / 2,544 old views /
10,176 refused applications**, final maps, read-probe identities and both other-CF
sentinels. All process identities, terminal records, hashes and raw statistics
pass. There are **20,352 allocation comparisons**, including 10,176 complete
window-record equalities between the two single-buffer arms: requests,
deallocations, live deltas and peak requested bytes all match. Final-index
footprints match too. Their reduction against baseline stays exactly
`24 * (default_keys + 2)`, or 2,402,352 requested bytes for 100,096 default keys.
This is allocator accounting, not RSS or jemalloc physical footprint.

## Selection against the concurrent baseline

The fixed sequence is baseline/control/new/new/control/baseline for eight cases.
Each timed process runs twelve 106-group / 100,096-mutation passes after one
excluded pass. Original keys are 27/35 bytes; long keys are 128/136 bytes;
values are 128 bytes. Timed `write_applied` includes preflight, input destruction
and position/revision. Batch creation and old-view acquisition/drop stay outside.
CPU 4 is pinned on a shared host; helpers use CPUs 6–15 and 22–31.

Values below are **microseconds per applied group**, not client-request latency.
Negative changes mean less elapsed time. Group sizes vary.

| Case | Mean, baseline → new (us) | Mean change | p99, baseline → new (us) | p99 change |
| --- | ---: | ---: | ---: | ---: |
| Original / overwrite | 109.611 → 113.847 | +3.864% | 305.335 → 314.932 | +3.143% |
| Original / overwrite + old view | 120.382 → 123.346 | +2.462% | 318.519 → 331.343 | +4.026% |
| Original / unique insert | 253.082 → 261.408 | +3.290% | 738.405 → 800.650 | +8.430% |
| Original / unique insert + old view | 332.714 → 354.295 | +6.486% | 974.542 → 1050.693 | +7.814% |
| Long / overwrite | 110.249 → 106.702 | -3.218% | 301.127 → 297.931 | -1.061% |
| Long / overwrite + old view | 124.825 → 127.329 | +2.006% | 330.451 → 338.355 | +2.392% |
| Long / unique insert | 312.336 → 295.666 | -5.337% | 1114.310 → 1076.009 | -3.437% |
| Long / unique insert + old view | 396.855 → 385.990 | -2.738% | 1443.039 → 1441.136 | -0.132% |

All original-key cases fail the material mean, pooled p99 and both-order mean
requirements. Long pinned overwrite also fails mean and p99 bounds. Its mean
regression is 2.005823%, which remains a failure rather than being rounded down
to the 2% limit. The remaining read/snapshot/range/delete stage is not run.
No selection threshold is relaxed.

## Effect of the accessor change alone

The concurrent single-buffer control retains the old checked-prefix accessor.
New versus control pooled changes are:

| Case | Mean change | p99 change |
| --- | ---: | ---: |
| Original / overwrite | +1.024% | +2.118% |
| Original / overwrite + old view | -0.054% | +0.881% |
| Original / unique insert | +1.375% | +3.352% |
| Original / unique insert + old view | -0.043% | -0.580% |
| Long / overwrite | -2.760% | +0.108% |
| Long / overwrite + old view | -0.769% | -0.272% |
| Long / unique insert | +0.224% | +9.228% |
| Long / unique insert + old view | +1.499% | +12.263% |

Original unpinned overwrite and unique insertion regress in both order
directions. Pinned original means are nearly unchanged and reverse direction
between orders. Long unique pinned insertion regresses in mean and p99 in both
orders. Removing the failure branches therefore does not repair this variant.
This comparison measures the source change as compiled, including resulting
code layout; it does not isolate an individual instruction's CPU cost.
Both order directions and all raw values remain in the
[analysis](key-clamp-performance-v1/analysis.json).

## Next structural experiment

The [retained selected-index CPU profile](RESIDENT-SELECTED-CPU-PROFILE.md)
identified traversal/ownership as its largest exclusive category. Allocation
coalescing and this accessor rewrite have not removed that work. Stop these
variants and investigate a persistent path-compressed byte-radix index with
ordered range and snapshot semantics. This is a new hypothesis, not a selected
replacement for rpds.

The primary [ART paper](https://db.in.tum.de/~leis/papers/ART.pdf) motivates byte
routing, path compression and adaptive nodes while retaining key order. Its
benchmarks and space bounds do not establish the behavior of a new copy-on-write
Rust implementation. We must prove and measure our own update/snapshot design.

An offline static census independently checks every key and 512 misses in each
validated dataset. Ideal compressed-radix hit paths visit four nodes for the
4,096-key maps, and mean 6.035636 / maximum seven for the 100,096-key maps.
The former have 273 branches; the latter 14,407. Forty empty/binary/prefix model
keys check terminal routing. These are topology counts only: no allocator
layout, mutation cost, physical cache behavior or timing is inferred. There is
no Rust radix implementation yet.

The [next plan](key-clamp-performance-v1/next-radix-plan.json) calls for a safe
isolated prototype, source-bound algorithm invariants/refinement, independent
mutation/snapshot models and layout qualification before timing. It explicitly
covers deep prefix ladders and iterative reclamation, preserving arbitrary
public key lengths. Future tests include varied-prefix data so the original
24-byte common prefix cannot alone justify selection. Prior packed/prefix,
inline40, callback, owned-buffer and borrowed-upsert failures stay retained.

The [packet](key-clamp-performance-v1/README.md) includes qualification, the
failed preparation and the complete corrected screen. No new distributed
recovery, MinIO, actual Chaos Mesh or Redis result is claimed. Those gates,
material matched database benefit and industrial requirements remain open.
CI stays local and bulk output remains under `/mnt/data/kv9-work`.
