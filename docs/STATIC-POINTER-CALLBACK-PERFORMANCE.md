# Static pointer callback performance screen

Date: 2026-09-16 UTC. Source base: `1427e8ac792b0c099b73052894e4bfabf77e095a`.

The [qualified callback patch](STATIC-POINTER-CALLBACK.md) reduces every measured
write mean in both orders. On the original key distribution, overwrite group
mean falls **9.951% / 7.069%** without/with snapshots; unique insertion falls
**6.071% / 5.478%**. Allocations and requested live bytes are identical.

The predeclared selection gate nevertheless fails: the original four write
cells do not all reach 10%, and three read cells exceed the 2% mean/p99 bound.
Keep this as an isolated improvement; do not promote it or rerun it unchanged.
There is no new database QPS or Redis comparison.

## Matched method and correctness

Two isolated dependency workspaces build identical benchmark source with
baseline archery or the qualified static callback patch. All registry versions
and checksums match. Ordinary ThinLTO, one codegen unit and jemalloc are used;
no forced frame pointers. Timing and allocation-counting executables are
separate. Shared-cache clean/build/copy checks and both Clippy runs pass.

The driver starts a fresh process for each case/arm/order, pinned to CPU 4 on
the same shared host. It runs baseline/candidate/candidate/baseline for each
case. Timing uses 12 passes and one excluded invocation; counting uses one
pass. Map construction, snapshot creation/drop, final validation and query
preparation are outside measured windows. Writes include owned input clones.
Point/predecessor answers are borrowed; scan16 allocates owned results and
drops them outside the timer. Per-call timer overhead remains.

The corpus, write/read kernels, three key transformations, seed-71 probes and
allocator counter are unchanged from the prior harness; only the driver and
backend selection change. Old packed/prefix timing results are not pooled.
Preparation checks **3,816 live prefixes / 1,908 old views** across both arms.
Independent reconstruction verifies all nine final maps and 24 probe sets
before timing. All **338 processes**, **168 timing rows** and **168 counting
rows** finish successfully. All final states, query identities, raw statistics
and allocation/lifetime checks pass independent readback.

## Write results

Values are pooled over the two orders. Negative changes mean lower latency.
The unit is one retained mutation group, not one database request. All 36
individual write-order mean comparisons improve; pooled write p99 improves
in all 18 cells, though original pinned unique-insert p99 rises 0.870% in the
reverse order.

| Keys | Workload | Snapshot | Baseline mean (us/group) | Candidate mean (us/group) | Mean change | p99 change |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| original24 | overwrite | none | 100.426 | 90.433 | -9.951% | -9.833% |
| original24 | overwrite | pinned | 117.278 | 108.987 | -7.069% | -8.534% |
| original24 | initial_fill | none | 103.184 | 94.703 | -8.219% | -9.601% |
| original24 | initial_fill | pinned | 119.423 | 111.210 | -6.877% | -6.814% |
| original24 | unique_insert | none | 269.467 | 253.108 | -6.071% | -5.568% |
| original24 | unique_insert | pinned | 354.578 | 335.155 | -5.478% | -2.176% |
| short4 | overwrite | none | 144.635 | 133.597 | -7.631% | -8.297% |
| short4 | overwrite | pinned | 185.548 | 179.486 | -3.267% | -3.105% |
| short4 | initial_fill | none | 139.563 | 128.965 | -7.594% | -5.144% |
| short4 | initial_fill | pinned | 184.776 | 178.676 | -3.301% | -2.985% |
| short4 | unique_insert | none | 438.042 | 421.043 | -3.881% | -5.037% |
| short4 | unique_insert | pinned | 577.111 | 551.955 | -4.359% | -5.243% |
| dispersed0 | overwrite | none | 145.359 | 132.249 | -9.019% | -9.025% |
| dispersed0 | overwrite | pinned | 185.671 | 179.550 | -3.297% | -4.864% |
| dispersed0 | initial_fill | none | 140.313 | 128.625 | -8.330% | -8.798% |
| dispersed0 | initial_fill | pinned | 182.419 | 177.359 | -2.774% | -3.114% |
| dispersed0 | unique_insert | none | 448.681 | 423.119 | -5.697% | -4.264% |
| dispersed0 | unique_insert | pinned | 582.848 | 557.213 | -4.398% | -3.899% |

Allocation operation counts, requested bytes, map peaks and final live bytes
match exactly across both arms and both orders in every cell. Every measured
map releases its requested bytes after drop. This supports a call/codegen
effect; it establishes no memory-capacity improvement.

## Read results and first-pass limitation

Each arm contributes 12,288 samples per cell. Values below are nanoseconds per
component call, including instrumentation. They exclude engine locks, snapshot
creation, RPC and Raft. The full sample arrays and order comparisons remain in
the [evidence](static-pointer-callback-performance-v1/README.md).

| Keys | Final map | Operation | Mean baseline / candidate (ns) | Mean change | p99 baseline / candidate (ns) | p99 change |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| original24 | overwrite | get_hit | 55.655 / 55.336 | -0.573% | 150 / 150 | +0.000% |
| original24 | overwrite | get_miss | 60.447 / 61.469 | +1.691% | 151 / 151 | +0.000% |
| original24 | overwrite | predecessor | 166.701 / 167.846 | +0.687% | 220 / 220 | +0.000% |
| original24 | overwrite | scan16 | 425.451 / 422.101 | -0.788% | 541 / 541 | +0.000% |
| original24 | unique_insert | get_hit | 164.354 / 152.032 | -7.497% | 792 / 531 | -32.955% |
| original24 | unique_insert | get_miss | 144.334 / 162.920 | +12.877% | 390 / 691 | +77.179% |
| original24 | unique_insert | predecessor | 375.477 / 358.856 | -4.427% | 982 / 511 | -47.963% |
| original24 | unique_insert | scan16 | 776.969 / 741.715 | -4.537% | 2184 / 1944 | -10.989% |
| short4 | overwrite | get_hit | 55.351 / 55.264 | -0.158% | 150 / 150 | +0.000% |
| short4 | overwrite | get_miss | 59.489 / 60.320 | +1.396% | 151 / 151 | +0.000% |
| short4 | overwrite | predecessor | 166.966 / 168.312 | +0.806% | 220 / 220 | +0.000% |
| short4 | overwrite | scan16 | 427.143 / 424.553 | -0.606% | 541 / 541 | +0.000% |
| short4 | unique_insert | get_hit | 150.254 / 137.760 | -8.315% | 571 / 411 | -28.021% |
| short4 | unique_insert | get_miss | 151.817 / 150.417 | -0.922% | 501 / 501 | +0.000% |
| short4 | unique_insert | predecessor | 409.491 / 387.322 | -5.414% | 862 / 841 | -2.436% |
| short4 | unique_insert | scan16 | 678.454 / 659.606 | -2.778% | 1603 / 1753 | +9.357% |
| dispersed0 | overwrite | get_hit | 55.696 / 55.540 | -0.280% | 150 / 150 | +0.000% |
| dispersed0 | overwrite | get_miss | 61.593 / 62.071 | +0.775% | 151 / 151 | +0.000% |
| dispersed0 | overwrite | predecessor | 167.928 / 167.454 | -0.282% | 221 / 220 | -0.452% |
| dispersed0 | overwrite | scan16 | 424.616 / 430.716 | +1.437% | 541 / 561 | +3.697% |
| dispersed0 | unique_insert | get_hit | 159.357 / 144.041 | -9.611% | 572 / 461 | -19.406% |
| dispersed0 | unique_insert | get_miss | 166.717 / 152.434 | -8.567% | 682 / 491 | -28.006% |
| dispersed0 | unique_insert | predecessor | 465.730 / 380.469 | -18.307% | 1272 / 792 | -37.736% |
| dispersed0 | unique_insert | scan16 | 693.213 / 656.884 | -5.241% | 2244 / 1493 | -33.467% |

The failing cells are original large-map GET miss (+12.877% mean, +77.179%
p99), short-prefix large-map scan16 (+9.357% p99), and dispersed small-map
scan16 (+3.697% p99). The first two have opposing tail directions between
orders; the last is worse in both orders. Do not infer a stable 77% regression
from this one order-sensitive observation. It still fails the declared gate.

A [post-hoc pass decomposition](static-pointer-callback-performance-v1/read-pass-review.json)
retains every read sample and covers all 96 read rows. The excluded warmup
invokes `read_case`, which creates and drops its own map. The measured
invocation creates another map: it does not reuse that warm map. Although
query outputs are validated before timing, early measured passes remain
different from later passes. In the problematic GET-miss candidate reverse
order, 113 of 119 samples above the pooled candidate p99 occur in its first
512-query pass. Its pass mean falls from 549.2 ns to 115.4 ns; the other
candidate order ends at 114.9 ns. This locates the tail in the retained data;
the cause of the between-run variation is not established.

No samples are discarded and no acceptance rule changes retroactively. A
future changed-candidate measurement must declare first-touch and same-map
warm steady-state panels separately. Preserving the first-touch panel prevents
a warm-only result from hiding an initialization/cache cost. The material
write-gain gate fails independently of this read limitation.

## Code generation and next hypothesis

The actual timing executables confirm the intended effect: baseline retains
an 8-byte `Arc::as_ptr` symbol and five references; candidate has neither.
These are this harness's counts, distinct from the earlier MemEngine harness's
21 references. A remaining ArcTK mutation closure is still a separate
411-byte function in the candidate. Its common unique-owner path still
crosses the callback boundary; shared-owner cloning remains in the same body.

Next qualify one changed hypothesis: make the existing mutation callback a
named `#[inline(always)]` adapter, preserving the same `Arc::make_mut` call,
raw-pointer conversion, reference counts and normal/unwind guard. A proposed
patch and exact remaining helper assembly are retained, but this follow-up
has **not** been compiled, proved, tested or timed. Inspect ordinary codegen
and source/ownership equivalence before its own matched comparison. Keep the
first callback-only results as a separate building block, not a selected
production optimization.

Promotion still requires a material component win without accepted read
regressions, full local correctness, ordinary recovery, actual Chaos Mesh and
matched database/three-copy Redis throughput and latency. CRC/rpds remain
selected. Previously losing tree, clone-removal and borrowed-upsert variants
stay held. Current recorded database results remain 137,873.776 Put/s and
1,022,750.059 BatchPut(64) items/s on the volatile c64 fixture.

## Reproduction and artifacts

The [harness](../scripts/resident-static-callback/README.md) describes the
protocol. The [packet](static-pointer-callback-performance-v1/README.md) retains
the exact plan, source/build evidence, execution helpers, every timing/count
row, independent analysis and read-pass review. Original execution root:
`/mnt/data/kv9-work/static-pointer-callback-performance-20260916-first`.
All new bulk output is on `/mnt/data`; no hosted CI was requested.
