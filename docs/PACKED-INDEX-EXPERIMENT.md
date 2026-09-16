# Packed persistent index: first experiment

Recorded: 2026-09-16 UTC. **Reject this variant for runtime integration.**
Packing multiple keys per node improves new-key insertion and reduces retained
requested memory, but regresses overwrite and point lookup. Production remains
unchanged. These are standalone index timings, not database QPS.

## Implementation and correctness

The [standalone source](../scripts/resident-packed/README.md) uses safe Rust,
32-entry leaves / 32-child branches, 16-slot non-root minima, shared immutable
entries and copy-on-write paths. Deletion repairs underflow by borrowing or
merging; roots contract when only one child remains. Ordered lookup, predecessor
and bounded forward ranges preserve bytewise key semantics.

Six optimized tests pass against an independent `BTreeMap` model, with structural
checks for ordering, child minima, occupancy, uniform depth and exact size:

- Four ascending/descending insertion/deletion combinations over 2,049 keys.
- Three 6,000-operation mixed histories with multiple retained snapshots.
- All 120 insertion permutations paired with all 120 deletion permutations for
  five empty/binary boundary keys.
- Shared-root/subtree checks, absent-deletion identity and old-value isolation.
- A 20,000-key four-level tree with shuffled deletion and retained generations.
- Corrupted-size/separator rejection controls.

Both benchmark configurations pass warnings-denied Clippy. The library/test
sources remain byte-identical to the successful test build. Original failed
Clippy attempts are retained: the test used a manual divisibility check, and the
counting harness used unit-valued timestamp bindings. Both were corrected before
measurement; neither failure changed the tree algorithm or produced timing data.

## Fixed write comparison

The same retained 106 engine groups / 100,096 mutations are used. Overwrite starts
with all 4,096 original keys; initial fill starts empty. Synthetic unique insert
appends an eight-byte ordinal to every key and finishes with 100,096 keys. With
snapshots enabled, one pre-group root is retained through each group's writes.
The original key/value cloning at the index boundary is preserved for both arms.

Each case runs rpds/packed/packed/rpds, twelve passes per row, after warmup.
Rows use fresh maps; setup, snapshots, validation and map teardown are outside
timing. Each pooled arm contains 2,544 groups. The shared host uses CPU 4,
jemalloc 0.6.1, release ThinLTO and the retained Rust/Cargo 1.94 toolchain.

| Workload | Snapshot | rpds mean/group | Packed mean/group | Mean change | rpds p99 | Packed p99 |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| Overwrite | No | 98.882 us | 125.538 us | +26.957% | 267.651 us | 342.321 us |
| Overwrite | Yes | 118.615 us | 133.327 us | +12.403% | 315.541 us | 354.524 us |
| Initial fill | No | 103.682 us | 132.360 us | +27.660% | 278.932 us | 354.253 us |
| Initial fill | Yes | 120.318 us | 140.003 us | +16.361% | 314.789 us | 370.584 us |
| Unique insert | No | 261.263 us | 238.587 us | -8.679% | 754.993 us | 710.189 us |
| Unique insert | Yes | 344.364 us | 266.819 us | -22.518% | 1,074.802 us | 858.547 us |

Both orders retain each write direction. The predeclared gate required at least
10% mean improvement for overwrite and unique insert in both snapshot modes,
improving means in all write orders, and no greater than 2% pooled mean/p99 read
regression. All three conditions fail. Do not advance this unchanged variant.

## Reads and a diagnosed sampling limitation

Read timings cover 512 deterministic probes per case, twelve passes per row and
both orders. Point/predecessor outputs are borrowed; scan16 returns owned rows,
with result destruction outside timing. Per-operation clock overhead remains.

| Resident keys | Operation | rpds mean | Packed mean | rpds p99 | Packed p99 |
| ---: | --- | ---: | ---: | ---: | ---: |
| 4,096 | GET hit | 43.634 ns | 134.029 ns | 120 ns | 160 ns |
| 4,096 | GET miss | 59.028 ns | 133.264 ns | 140 ns | 151 ns |
| 4,096 | Predecessor | 182.483 ns | 135.460 ns | 241 ns | 160 ns |
| 4,096 | Scan16 | 415.967 ns | 357.170 ns | 591 ns | 431 ns |
| 100,096 | GET hit | 232.808 ns | 296.319 ns | 962 ns | 902 ns |
| 100,096 | GET miss | 239.435 ns | 277.890 ns | 992 ns | 882 ns |
| 100,096 | Predecessor | 480.982 ns | 284.693 ns | 1,382 ns | 892 ns |
| 100,096 | Scan16 | 891.291 ns | 691.725 ns | 2,735 ns | 1,784 ns |

**The fixed-stride present-key probe set favors shallow rpds nodes in the 4,096-key
fixture.** A separate order-equivalent comparison-count diagnostic checks the
same sorted insertion sequence: selected hits average **8.027 comparisons**,
versus **11.003** over all keys. Misses average **12.002 selected / 12.000 all**,
so that hit-selection explanation does not cover the miss regression. Comparison
counts are not CPU-time attribution. Preserve these actual rows with this
limitation; do not generalize the hit ratio to the database or rerun solely to
replace an unfavorable result. Write regressions independently reject the variant.

## Allocation and state evidence

The separate counting executable emits no latency measurements. For 100,096
overwrites without snapshots, both indexes request exactly **300,288 allocations**;
the packed layout does not remove the per-update entry/key/value allocations.
For unique insert with snapshots, allocation calls fall from **820,633 to
485,864**, while packed-vector growth adds 50,088 reallocation requests. Requested
sizes are not bytes copied or actual allocator/RSS consumption.

Final live requested bytes for the 4,096-key overwrite maps are 1,093,632 for rpds,
versus 1,004,714 unpinned / 958,730 pinned for packed. For unique insert they are
27,526,400 for rpds, versus 24,481,626 / 24,970,314 for packed. Copy-on-write vector
capacities account for different pinned/unpinned footprints. Map/snapshot teardown
returns the exact requested-byte count to its pre-map baseline in every count row.

Each executable checks **1,272 live group prefixes and 636 old views** against
the independent ordered model before measuring. All timed/count rows validate
final states, and all read probe outputs are checked. The independent Python
analysis reconstructs final datasets and exact query identities from original
group bytes, verifies source/executable/process bindings and recomputes every
statistic and the failed advancement gate. All 56 timing and 56 count rows pass
their accounting checks. Complete metadata archive readback also passes.

## Next changed experiment

The original keys are 27 bytes long and share a 24-byte prefix. This gives a
concrete hypothesis for reducing repeated full-key comparisons within packed
nodes; it does not establish their CPU share. Next inspect/count comparison and
minimum-refresh work, then consider a per-node shared-prefix comparator only if
that evidence supports it. Prove suffix comparison preserves bytewise order and
validate prefix maintenance on insert, split, deletion/merge and snapshots before
timing a changed kernel. Include varied-prefix keys to test generality.

Any future read screen must use a predeclared seeded selection without replacement
and expose comparison-depth coverage; the original biased rows stay separate.
Do not sweep fanout or repeat the rejected matrix. A complete implementation-bound
proof of the packed index, engine integration, ordinary recovery, actual Chaos
Mesh and matched database timing remain required before production selection.
No industrial checkbox is closed. See the [retained packet](packed-index-experiment-v1/README.md).
