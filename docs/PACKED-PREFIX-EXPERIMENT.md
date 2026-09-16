# Shared-prefix packed-index evaluation

Updated 2026-09-16. **Reject this candidate for integration.** Guarded prefix
skipping is order-correct under the stated node invariants, but the implementation
is slower than the unchanged packed reference in every measured write case.
The original24 overwrite mean regresses **4.145% / 4.241%** without/with a retained
snapshot versus packed, and **30.393% / 21.318%** versus production's rpds index.
Correctness/input/statistics gates pass; all three performance gates fail.
Production code and database QPS are unchanged.

## Hypothesis and implementation

A count-only copy of the original packed implementation records, per 100,096
overwrite mutations, **1,013,102 branch-routing comparisons, 501,497 leaf-write
comparisons and 200,192 minimum-refresh comparisons**. Every pair shares at
least 24 bytes; all minimum-refresh comparisons are equal. The counts are
identical with and without the pre-group snapshot. These are logical comparator
calls and equal-prefix lengths, not bytes actually executed by vectorized
`memcmp`, CPU time, cache misses or a cause attribution.

The [candidate](../scripts/resident-packed-prefix/README.md) caches a conservative
shared-prefix length in every node, covering its entire subtree. New-key
admission shortens the prefix before routing. Splits, redistribution and merges
rebuild it from actual subtree endpoints. Deletion takes a subset. Queries use
suffix comparison only after checking the prefix, with whole-key fallback.
The fanout, entry sharing, minimum refreshes, mutation order and repair algorithm
remain unchanged. No minimum-refresh suppression is mixed into this candidate.

## Correctness and proof scope

Eight release library tests pass, including reference-map correspondence,
structural invariants, many retained snapshots, all small permutations, deep
split/merge/deletion, guarded unsigned-byte comparisons and shrinking-prefix
transitions. Both timing and allocation configurations pass local Clippy.
The new harness then checks all three implementations after every batch:
**5,724 live states and 2,862 retained old views**, across three key distributions,
three write workloads and two snapshot modes. Read results are checked against
`BTreeMap` before each measured row. Every final map and query sequence is
independently reconstructed from the pinned retained corpus.

The [Lean model and strict checker](../proofs/lean/packed-prefix/README.md) pass
**18 universal lemmas and eight rejecting controls**. The theorem set proves
prefix cancellation, guarded ordering, endpoint interval coverage, insertion
shortening, subset preservation and slice bounds. Standard Lean foundations
are the only permitted transitive axioms. The exact reviewed Rust declarations
and library hash are checked before and after execution.

This is **not a full B+tree proof or mechanized Rust refinement**: sorted node
intervals, separators and safe Rust ownership are explicit premises. The four
Rust source-edit controls exercise binding rejection, not compiled fault
injection. Original proof failures and a benchmark Clippy failure are retained.
No engine integration, recovery or Chaos Mesh gate is claimed for this rejected
isolated candidate.

## Matched measurement

One pinned corpus contains 106 groups / 100,096 mutations. The datasets are:

- `original24`: original 27-byte keys with a 24-byte common prefix.
- `short4`: `raw:` plus SHA-256 bytes and a unique sorted-original-key rank.
- `dispersed0`: SHA-256 bytes and that same rank, with no fixed byte prefix.

Rekeyed base keys remain 27 bytes; the rank occupies the final eight bytes.
The first byte is masked to keep all keys below the common scan bound. Values,
mutation repetition and group boundaries are preserved; key order changes.
The unique-insert case appends an eight-byte operation ordinal to each base key.
These synthetic variations are not complete production key distributions.

The order is **rpds / packed / prefix / prefix / packed / rpds** for each case,
with twelve timing passes per row and one separate allocation-counting pass.
Release ThinLTO, jemalloc 0.6.1 and CPU 4 are fixed on the shared host. Warmup,
map initialization and snapshot creation/drop are outside the write interval.
There are **252 timing rows and 252 allocation rows**, all complete. Compact raw
timing output is 4,493,055 bytes, within the predeclared 8 MiB bound.

Future read selection was fixed before timing: xorshift64 seed 71, rejection
sampling for bounded draws, full Fisher-Yates shuffle, first 512 distinct keys.
Comparison-depth coverage on the 4,096-key map is now **10.97656 sampled versus
11.00342 all-key comparisons** for hits, and **12.00000 versus 12.00049** for
misses. On the 100,096-key map hit means are **15.71875 versus 15.71889**; miss
means are **16.74414 versus 16.71873**. All depth histograms are retained. These
new samples are not pooled with the earlier biased fixed-stride experiment.
This verifies depth coverage, not every source of cache/locality sampling bias.

## Write results

Mean-time changes below use pooled raw samples. Negative is faster. Both timing
orders show a prefix-versus-packed regression in every row.

| Dataset | Workload | Snapshot | Prefix vs rpds | Prefix vs packed |
| --- | --- | --- | ---: | ---: |
| original24 | overwrite | No | +30.393% | +4.145% |
| original24 | overwrite | Yes | +21.318% | +4.241% |
| original24 | initial_fill | No | +36.787% | +4.229% |
| original24 | initial_fill | Yes | +25.462% | +4.371% |
| original24 | unique_insert | No | -3.991% | +7.094% |
| original24 | unique_insert | Yes | -18.193% | +6.827% |
| short4 | overwrite | No | +12.201% | +5.733% |
| short4 | overwrite | Yes | +0.090% | +6.828% |
| short4 | initial_fill | No | +20.193% | +8.451% |
| short4 | initial_fill | Yes | +2.964% | +8.001% |
| short4 | unique_insert | No | -24.472% | +2.984% |
| short4 | unique_insert | Yes | -26.899% | +2.108% |
| dispersed0 | overwrite | No | +10.514% | +3.760% |
| dispersed0 | overwrite | Yes | -3.123% | +4.172% |
| dispersed0 | initial_fill | No | +14.163% | +3.876% |
| dispersed0 | initial_fill | Yes | -0.888% | +3.858% |
| dispersed0 | unique_insert | No | -25.035% | +5.741% |
| dispersed0 | unique_insert | Yes | -25.539% | +3.552% |

For original24 overwrite without a snapshot, rpds / packed / prefix means are
**99.615 / 124.721 / 129.891 us per group**, with pooled p99
**276.467 / 332.763 / 345.767 us**. With a snapshot the means are
**114.968 / 133.802 / 139.476 us**, with p99 **302.045 / 352.029 / 364.121 us**.
Unique insertion retains some advantages over rpds, but prefix skipping reduces
the packed reference's gains. Complete per-order means/p99 and pooled raw-sample
quantiles are in `analysis.json`; quantiles are never averaged across orders.

## Read results and allocation

Original24 results below are per index call in nanoseconds. These calls include
clock overhead and exclude the engine, locks, networking, WAL and Raft.

| Map / operation | rpds mean / p99 ns | Packed mean / p99 ns | Prefix mean / p99 ns |
| --- | ---: | ---: | ---: |
| overwrite / get_hit | 53.017 / 141 | 135.630 / 160 | 142.626 / 161 |
| overwrite / get_miss | 57.437 / 150 | 134.845 / 151 | 141.709 / 161 |
| overwrite / predecessor | 160.020 / 220 | 137.420 / 160 | 146.373 / 161 |
| overwrite / scan16 | 425.746 / 551 | 358.912 / 431 | 367.910 / 451 |
| unique_insert / get_hit | 165.126 / 721 | 261.320 / 771 | 291.027 / 762 |
| unique_insert / get_miss | 182.905 / 792 | 248.249 / 702 | 285.665 / 842 |
| unique_insert / predecessor | 464.503 / 1353 | 264.451 / 862 | 294.534 / 862 |
| unique_insert / scan16 | 821.839 / 2044 | 675.605 / 1744 | 722.785 / 1713 |

Prefix mean read time is worse than packed in all 24 cases. Some individual p99
values improve, but all datasets retain substantial GET mean regressions versus
rpds; the read gate fails. Larger-map tails vary between orders on this shared
host. Scan16 returns owned rows; point/predecessor results are borrowed.

Original24 unpinned overwrite still makes **300,288 allocation calls per
100,096 mutations** in all three implementations. Prefix caching does not
remove the entry/key/value allocations. Final requested map bytes are
**1,093,632 / 1,004,714 / 1,006,882** for rpds / packed / prefix; with a pre-group
snapshot they are **1,093,632 / 958,730 / 960,898**. All map-related requested
bytes return to the baseline after dropping the final map and snapshots. These
counters are neither RSS nor actual allocator usable sizes.

## Decision and next work

Do not integrate this candidate or repeat the unchanged matrix. The predeclared
10% original-write gain, both-order write improvement and <=2% read regression
gates all fail. The measured extra prefix state/guards outweigh any reduction
in comparator operands for this implementation; their separate CPU shares were
not measured. Comparison count alone was insufficient to predict a win.

Stop tuning this packed/prefix family: both overwrite and point reads remain
behind rpds. Return to attribution of the selected production index: obtain
bounded CPU evidence with complete caller coverage for allocation/copy,
comparison and tree traversal before selecting a different hot-path change.
Retain the existing borrowed-upsert insertion regression; do not rerun it or
propose value reuse without evidence for removing that regression. A viable
changed candidate still needs insert/overwrite/snapshot/read gates, complete
implementation-bound proof, ordinary recovery, actual Chaos Mesh and matched
database timing before promotion. Raft and durable acknowledgements remain
mandatory. C04 recovery ownership and automatic split/multi-Raft dependencies
remain open.

The latest database result remains **137,873.776 Put/s and 1,022,750.059
BatchPut(64) items/s** at c64 on the previously documented volatile fixture;
this experiment does not update Redis comparisons or establish parity.

See the [portable evidence packet](packed-prefix-experiment-v1/README.md).
