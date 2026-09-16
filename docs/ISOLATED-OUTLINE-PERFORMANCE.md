# Isolated shared-clone performance result

Date: 2026-09-16 UTC. Execution base: `d0337befbb1418104f2b698f705b262815e2c977`.

The [qualified triomphe-only candidate](ISOLATED-OUTLINE-QUALIFICATION.md) remains
held. Actual engine write means improve **10.803% / 3.690%** for overwrite and
**4.250% / 1.526%** for unique insertion, without/with an old snapshot. All four
means improve in both execution orders, but three miss the declared 10% material
threshold. Small-map resident warm GET hit regresses **5.509%** per call and
**9.844%** per 512-query pass. The component gate fails; production is unchanged.

All **136 processes / 152 timing rows / 76 allocation rows** complete. Independent
analysis validates every raw timing statistic and every allocator window. Both
arms have identical per-window allocator requests and requested live-byte
behavior in all 38 comparison cells. Correctness and accounting pass; the
performance acceptance criteria do not. No new database QPS follows.

## Exact scope and method

The timing pair is reused byte-for-byte from qualification, with original
registry archery/rpds and only the candidate triomphe extraction. There is no
timing rebuild, alignment flag, counter instrumentation or production source
change. Both arms use ordinary ThinLTO release settings and jemalloc. The
previous 523-test / 29-conditional-lemma / 14-control qualification is inherited
through checked source and binary identities, not presented as a new test run.

The [published plan](isolated-outline-performance-v1/declared-plan.json) fixes
22 cases, ABBA timing, twelve write passes and twelve read epochs. Each read epoch
uses one snapshot: first 512 probes, eight untimed same-view passes, then twelve
measured warm passes. Snapshot acquisition, owned `get`, resident `get_resident`,
first-probe/warm and per-call/whole-pass scopes stay separate. First-probe does
not mean cache-flushed. Every read epoch verifies its old view after a subsequent
write advances position/revision from 106 to 107.

The separate [allocation companion](../scripts/engine-interface-counting/README.md)
reuses the existing counter module verbatim. A strict source projection verifies
the same operation and validation bodies, removing only instrumentation, output
metadata and one untimed snapshot-size observation. Counts emit **no elapsed
time**. Counter buffers, bookkeeping and validation are outside counted windows;
read results remain alive until after each pass. Input batch construction and
old-view acquisition/drop are excluded, while consumed-batch destruction is
inside `write_applied`, matching the timing scope.

Four fresh preparation processes validate **1,696 live prefix states / 848 old
views**, exact applied positions/revisions, untouched Lock/Write column families,
all final keys/values and four independently reconstructed probe sequences.
The 44 counting processes and their independent allocator gate pass before any
of the 88 timing processes start. Both counting builds pass release Clippy,
explicit cache invalidation/freshness and source/dependency checks. All child
lifetimes terminate normally. CPU 4 is used for measurement; helpers exclude it
and its sibling. The host remains shared.

Before execution, one contradictory inherited plan sentence saying there was
no allocation evidence was corrected to agree with its explicit allocation
companion section. The original plan is retained; cases, binaries, counts,
timing scopes and thresholds are unchanged. There is no post-result relaxation.

## Writes and snapshots

Write values are **microseconds per retained group**, not client request latency.
Each pooled timing arm contains 2,544 group windows. Snapshot values are
nanoseconds per acquisition, with 12,288 pooled samples per arm. All samples
and both order directions remain in the [full analysis](isolated-outline-performance-v1/analysis.json).

| Case | Baseline mean | Candidate mean | Mean change | Baseline p99 | Candidate p99 | p99 change |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Overwrite, unpinned (us/group) | 109.390 | 97.572 | -10.803% | 305.231 | 269.955 | -11.557% |
| Overwrite, pinned (us/group) | 122.790 | 118.259 | -3.690% | 327.513 | 312.895 | -4.463% |
| Unique insert, unpinned (us/group) | 255.029 | 244.191 | -4.250% | 779.097 | 708.835 | -9.018% |
| Unique insert, pinned (us/group) | 341.501 | 336.289 | -1.526% | 984.791 | 978.710 | -0.617% |
| Small-map snapshot (ns/call) | 36.456 | 35.610 | -2.320% | 41 | 41 | 0.000% |
| Large-map snapshot (ns/call) | 35.559 | 35.895 | +0.946% | 41 | 41 | 0.000% |

## Reads

The following warm hit rows show the main tradeoff. Per-call units are
nanoseconds per captured-view read. Whole-pass units are nanoseconds per
**512-query pass**; their p99 is not individual request p99.

| Map / API / timer | Baseline mean | Candidate mean | Mean change | Baseline p99 | Candidate p99 |
| --- | ---: | ---: | ---: | ---: | ---: |
| Small / owned / per call | 72.669 | 70.300 | -3.260% | 160 | 151 |
| Small / owned / whole pass | 25,862.618 | 25,262.028 | -2.322% | 48,370 | 49,342 |
| Small / resident / per call | 70.077 | 73.937 | +5.509% | 141 | 150 |
| Small / resident / whole pass | 21,402.833 | 23,509.642 | +9.844% | 48,621 | 50,635 |
| Large / owned / per call | 154.272 | 156.053 | +1.155% | 310 | 311 |
| Large / owned / whole pass | 63,545.691 | 63,718.517 | +0.272% | 101,069 | 106,930 |
| Large / resident / per call | 128.232 | 129.945 | +1.336% | 281 | 290 |
| Large / resident / whole pass | 49,857.684 | 50,210.934 | +0.709% | 99,977 | 101,490 |

Small resident warm hit is worse in both orders and both timer modes. Its
per-call p99 rises 6.383%, and small resident warm miss mean rises 4.097%.
In total, **16 read-panel cells** exceed the declared 2% pooled mean/p99 bound;
some exceed it only in whole-pass p99. The 2.010% small owned warm pass-p99
increase is retained even though it is close to the threshold. There is no
rounding-based exception or claim of statistical significance from two orders.

The candidate does not remove the read tradeoff: small owned warm hit improves
in this screen, while resident warm hit worsens. The [combined candidate's
earlier result](ENGINE-INTERFACE-SCREEN.md) remains separately attributed to its
own binaries and run. These observations do not identify a cache/branch cause
or prove that one library edit alone causes the differing read pattern.

## Allocator observations

Every counted window matches between arms, including all six allocation request
fields, net requested live-byte change and peak extra requested live bytes.
The [allocation analysis](isolated-outline-performance-v1/allocation-analysis.json)
retains each cell, with raw window records in the packet.

| Write mode | Allocation requests per mutation | Requested allocation bytes per mutation |
| --- | ---: | ---: |
| Overwrite, unpinned | 3.000 | 211.000 |
| Overwrite, pinned | 3.924 | 262.728 |
| Unique insert, unpinned | 4.000 | 275.000 |
| Unique insert, pinned | 8.198 | 510.114 |

Each row covers 1,201,152 mutations in one counting process per arm. Source
inspection shows separately cloned key/value buffers entering the persistent
map. The callback extraction does not reduce these allocator requests.
Owned read hits allocate one returned value, with bytes checked against the
independent model. Borrowed hits and both kinds of misses allocate nothing
inside their windows. Snapshot acquisition allocates one **104-byte** box,
confirmed by the separately observed dynamic object size.

These are observations of instrumented Rust allocation requests, not RSS,
jemalloc arena footprint or proof of physical allocation in the uninstrumented
timing ELF. Some write windows have negative net live-byte change because
input buffers are allocated before the window and destroyed inside the consumed
write call; the values are not a whole-engine memory-growth measurement.

## Decision and next changed variable

Hold the triomphe-only and combined variants, and stop their unchanged screens
and alignment sweeps. The [next qualification plan](isolated-outline-performance-v1/next-inline-key-plan.json)
investigates a different cost: short keys stored inside the persistent-map entry
instead of in separate heap buffers. Current original/unique keys are 27/35
bytes; a safe private representation with up to 40 inline bytes and a Vec fallback
can be investigated without restricting arbitrary user keys. It has **not** been
implemented or measured.

Start from selected original dependencies. Check actual type/entry layout and
constructor allocation before timing. Prove byte identity, Eq/Ord/Borrow
consistency and ordered operation preservation; enum discriminants must never
define key ordering. Test empty/binary/prefix-sharing keys, threshold lengths,
long-key fallback, mixed representations, all CFs, forward/reverse/range access,
deletes and stable snapshots against an independent model. Preserve wire/storage
formats, mutation order, refusal/revision rules and Raft/WAL/read authorization.
This is not another owned-WriteBatch-consumption experiment; that rejected
variant remains held. Long-key footprint and latency regressions must be measured
before any selection claim.

Promotion still requires material matched database benefit, full local
correctness, ordinary recovery, actual Chaos Mesh and a matched three-copy Redis
comparison. Latest database results remain **137,873.776 Put calls/s / 1,022,750.059
BatchPut(64) items/s**, source `86aa6fc`, c64, 128-byte values, three Raft replicas
and the shared-host volatile tmpfs fixture. No new database/Redis measurement,
Chaos coverage or industrial checkbox closure is claimed. CI stays local and
bulk evidence stays under `/mnt/data/kv9-work`.
