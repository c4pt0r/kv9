# Engine interface screen and next isolated mutation candidate

Date: 2026-09-16 UTC. Base: `a18545d8cf969291fbfe174f7a5123c008b5afa6`.

The [outlined mutation candidate](OUTLINED-MUTATION-PATH.md) reduces actual
`MemEngine::write_applied` mean time by **4.600%–13.903%** across the four
declared write cases. Both mean and p99 improve in both execution orders.
However, warm owned GET-hit mean regresses **7.407%** on the small map and
**2.599%** on the large map; whole-pass timing also regresses. The candidate
remains isolated. This diagnostic does not pass its earlier failed selection
gate or establish a database QPS improvement.

All **90 processes / 152 timing rows / 38 comparisons** complete and pass
independent input/statistic/source verification. Production dependencies, Raft,
WAL, read authorization, release settings and runtime selection are unchanged.
No industrial work-package checkbox closes. Local CI remains the policy.

## What was measured

The [prospective plan](engine-interface-screen-v1/plan.json) uses the exact
qualified baseline/candidate dependency sources from the outlined experiment.
The only dependency changes are its static `as_ptr` callback and extracted
shared-owner mutation branch. Both arms use unchanged production engine/common
sources, ordinary ThinLTO release settings and jemalloc. Builds retain resolved
dependency checksums, source hashes, explicit shared-cache invalidation and
non-fresh engine/common/index artifacts. Release Clippy passes for both arms.

The original corpus contains 106 groups / 100,096 mutations. The overwrite map
has 4,096 keys; appending an ordinal gives 100,096 unique keys. Two untouched
Lock/Write sentinel rows exercise the other column families. Preparation checks
**848 live prefix states / 424 old views**, all positions/revisions, every final
key/value and four independently reconstructed seeded query sequences. Every
read epoch additionally checks the old view after position 106 advances to 107.

All 22 cases run baseline/candidate/candidate/baseline on CPU 4; helpers exclude
that CPU and its sibling. The host is shared. There is one excluded write pass
and twelve measured passes per process. Each read process builds twelve fresh
engines and captures one view per engine: first 512 probes, validation, eight
untimed passes, then twelve measured warm passes on that same view. Construction
already touches the map; first-probe timing is not a cache-flush guarantee.

The [harness](../scripts/engine-interface-experiment/README.md) separates:

- `write_applied`: input batch construction and optional old-view acquisition /
  destruction are outside timing; internal key/value clones, batch consumption
  and applied-position publication are inside. Each group is one window.
- Snapshot acquisition: boxing/acquisition are inside, destruction outside.
- Captured-view reads: owned `get` includes value copying; `get_resident` borrows
  from the view. Neither includes snapshot acquisition, a Raft barrier or RPC.
- Per-call timing: one query plus return/error check and black box; result storage
  follows the timer. Whole-pass timing includes 512 queries and output stores in
  a reserved vector. Both retain results until validation/drop after the pass.

Compiled-path review retains the actual production snapshot/read/apply symbols
and two monomorphized read loops, 1,545 and 2,022 bytes in both executables.
Each loop calls the real `ReadView` vtable slot (`0x20` resident / `0x18` owned)
once per query, with distinct call sites for the two timer modes. API selection
is outside the loops. This is static correspondence, not an instruction trace
or a new proof of the Rust implementation. The previously qualified conditional
dependency proof scope is unchanged; no new production algorithm is introduced.

## Write and snapshot results

Write values are **microseconds per retained group**, not client request latency.
Each pooled arm contains 2,544 groups. Pinned means one old snapshot is held during
each apply. Snapshot values are nanoseconds per acquisition, with 12,288 pooled
samples per arm. All raw samples are retained.

| Case | Baseline mean | Candidate mean | Mean change | Baseline p99 | Candidate p99 | p99 change |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Overwrite, unpinned (us/group) | 110.728 | 95.334 | -13.903% | 305.300 | 261.078 | -14.485% |
| Overwrite, pinned (us/group) | 122.376 | 112.856 | -7.779% | 327.772 | 303.126 | -7.519% |
| Unique insert, unpinned (us/group) | 251.815 | 227.722 | -9.568% | 746.815 | 697.192 | -6.645% |
| Unique insert, pinned (us/group) | 345.536 | 329.641 | -4.600% | 1,071.842 | 1,013.984 | -5.398% |
| Small-map snapshot (ns/call) | 35.656 | 35.485 | -0.479% | 41 | 41 | 0.000% |
| Large-map snapshot (ns/call) | 35.712 | 35.736 | +0.067% | 41 | 41 | 0.000% |

## Read results and interpretation

These warm GET-hit rows summarize the main result; all hit/miss, first-probe,
timer-mode and order results are in the [complete analysis](engine-interface-screen-v1/analysis.json).
Per-call values are nanoseconds per read. Whole-pass values are nanoseconds per
**512-query pass**; their p99 must not be presented as request p99.

| Map / API / timer | Baseline mean | Candidate mean | Mean change | Baseline p99 | Candidate p99 |
| --- | ---: | ---: | ---: | ---: | ---: |
| Small / owned / per call | 71.853 | 77.175 | +7.407% | 151 | 161 |
| Small / owned / whole pass | 25,402.569 | 26,747.288 | +5.294% | 49,753 | 51,536 |
| Small / resident / per call | 68.140 | 68.387 | +0.363% | 140 | 141 |
| Small / resident / whole pass | 22,116.920 | 21,286.840 | -3.753% | 49,171 | 50,124 |
| Large / owned / per call | 164.768 | 169.051 | +2.599% | 321 | 340 |
| Large / owned / whole pass | 68,395.604 | 71,694.528 | +4.823% | 104,395 | 109,665 |
| Large / resident / per call | 141.310 | 141.862 | +0.391% | 300 | 301 |
| Large / resident / whole pass | 56,012.806 | 56,460.302 | +0.799% | 105,246 | 106,268 |

Owned hit regressions have the same direction in both orders and both timing
modes. Warm owned misses also regress in per-call timing: **+7.330% / +2.854%**
small/large. Misses copy no value, so value-copy cost alone does not explain the
gap. Small owned-miss whole-pass mean is nearly unchanged (+0.252%, mixed orders);
large owned-miss whole-pass mean rises 2.420% in both orders. Read conclusions
must preserve interface and timer scope.

The isolated-index warm hit gap of approximately 15% does not carry uniformly
through engine interfaces: resident hit per-call means are nearly unchanged,
while owned reads retain smaller regressions. This is evidence against treating
that one index number as a universal database slowdown. It does not identify
the cache/branch cause, prove equivalence of allocator/physical layouts, or
authorize the candidate. Per-call clock overhead remains in the observations;
whole-pass output stores and return-value lifetimes are explicit rather than
silently equated to single-request execution.

## Decision and next implementation

Keep the selected production index and stop repeating the unchanged outlined,
alignment and engine matrices. The original 10% material-write / 2% read gate
remains failed. This bounded diagnostic confirms useful write work reduction,
but leaves owned read regressions and smaller snapshot-heavy insertion gains.

The [next candidate plan](engine-interface-screen-v1/next-isolated-outline-plan.json)
separates the two dependency changes: retain the original archery pointer
callback and investigate **only** the triomphe shared-clone extraction. This
composition has not been qualified or measured. It tests whether the mutation
benefit survives without also changing the pointer-access callback used across
the index. It is a changed source variable, not an alignment sweep or a repeat
of the combined candidate. A favorable result would not excuse the combined
candidate's failed gate.

First qualify the isolated source/dependency graph, unchanged pointer library,
conditional extraction proof correspondence and actual release mutation path.
Stop before timing if extraction does not remove work from the unique-owner
path. Then validate ordered state, applied positions and stable old views before
declaring a bounded paired screen with owned/resident read and allocation
evidence. Reuse completed observations; do not infer a cache or allocator cause.

The [experiment index](PERFORMANCE-EXPERIMENT-INDEX.md) also records an already
rejected owned-mutation-buffer implementation and its complete 72-cohort result.
Removing those key/value clones again is not a new candidate and is not scheduled.

Any production promotion still requires material matched database benefit, full
local correctness, ordinary recovery, actual Chaos Mesh and a matched three-copy
Redis throughput/latency comparison. No such promotion work ran in this diagnostic.
Latest database results remain **137,873.776 Put calls/s** and **1,022,750.059
BatchPut(64) items/s**, source `86aa6fc`, c64, 128-byte values, three Raft replicas,
shared host and volatile tmpfs fixture. This screen adds no Redis measurement.

## Retained evidence

The [evidence packet](engine-interface-screen-v1/README.md) binds sources,
dependency/build records, static call-boundary review, independent preparations,
all launch/terminal records and raw samples. The original corpus and dependency
qualification are referenced from the prior outlined packet instead of duplicated.
All owned processes exited. Bulk output remains under `/mnt/data/kv9-work`.
