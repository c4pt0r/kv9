# Outlined shared-mutation path: qualification and component screen

Date: 2026-09-16 UTC. Source base: `6a385d737b6b62fcedc5ccc2353ff883384b2ab7`.

The combined static-pointer/outlined-clone candidate reduces original overwrite
index-group mean **18.220% / 11.893%** without/with snapshots. Original unique
insertion improves only **7.722% / 1.922%**; its pinned p99 worsens **5.665%**.
All 18 write means improve in both orders, with identical allocation/live-byte
counts. The predeclared material-write and read-regression gates **fail**.
Keep the candidate isolated; production CRC/rpds and the database QPS baseline
are unchanged. The component result is not a Redis comparison.

The new read protocol exposes a repeatable small-map warm GET-hit regression:
mean rises **14.504–15.353%** across all three key distributions, in both orders.
Do not explain it away as the prior first-pass/warmup artifact. The complete
48-panel read comparison has **19** failing mean/p99 cells, including seven
warm cells. No sample is removed and the 2% acceptance bound is unchanged.

## Disproved named adapter

The [previous proposal](STATIC-POINTER-CALLBACK-PERFORMANCE.md) passed 58 archery
and 276 rpds tests, 18 guard plus six conditional adapter lemmas, seven rejecting
controls and four ordinary engine smokes. However, the 421-byte mutation closure
merely moved into `FnOnce::call_once`; the 1,147-byte engine insertion helper still
calls it. Symbol disappearance was not sufficient. Paired benchmark builds and
map-identity protocol tests completed, but **no named-adapter timing matrix ran**.
Its codegen gate remains closed. Sources and all failed/accepted build attempts
are retained separately from the actual outlined candidate below.

## Actual change and qualification

The candidate keeps the earlier static `as_ptr` adapter. It does not use the
failed named mutation adapter. In triomphe 0.1.16, `Arc::make_mut` retains its
single original Acquire uniqueness check and unsafe mutable-access expression.
Only the exact `*this = Arc::new(T::clone(this))` statement moves into a cold,
non-inlined helper; the public wrapper is always inlined. Clone evaluation,
allocation, owner replacement and existing archery unwind restoration remain
in their original order. There is no second count observation or reference
operation. The Cargo registry and production dependency configuration are untouched.

Local validation completes 54 triomphe production-feature tests, 58 triomphe
stable-feature tests (including arc-swap/unsize), 58 archery tests including the
retained panic/drop controls, and 276 rpds tests: **446 passes**. The initial
all-feature attempt failed during offline dependency resolution; optional unsize
was then fetched and the corrected stable configurations were tested. The
nightly-only drop-check feature is not claimed as tested.

The [Lean model](../proofs/lean/outlined-mutation-path/Outline.lean) adds eight
conditional extraction lemmas to the 18 guard lemmas. Nine deliberately invalid
proof/source controls are rejected. The checker binds the original/candidate
sources, single Acquire observation, identical unsafe-access tail and exact
clone-before-assignment expression. There are no admitted holes or custom axioms
in the accepted proof. This proves the reviewed abstract control-flow extraction
under the original Arc and Rust/compiler/provenance/unwind contracts, **not**
verified Rust extraction, correctness of all of Arc, or a new Raft proof. Stack
and code-layout identities are outside the modeled observables.

The qualified ordinary-release engine executable passes four one-pass semantic
smokes: 424 prefix states, 212 old views and 400,384 further mutations, with the
same final digests as the prior baseline. Its actual insertion assembly directly
checks the count and falls through on the unique-owner path; only the shared
branch calls the extracted helper. The engine insertion/helper sizes are
1,161/397 bytes. The separate timing executable confirms the same effect with
1,149/384-byte symbols. These are different harnesses and their sizes must not
be conflated. No new full workspace, recovery or Chaos Mesh gate ran for this
unselected candidate.

## Matched method

Identical ordinary ThinLTO/jemalloc harness sources are compiled with baseline
archery or the static-pointer archery plus outlined triomphe. Other resolved
registry versions/checksums match. Shared-cache invalidation, fresh dependency
artifacts, source hashes, both Clippy runs and both map-identity tests pass.
Timing and allocation counting have separate binaries. An initial named-harness
build-order failure is retained; tests run after binary copies in the corrected
builds, so test artifacts cannot satisfy the timing freshness checks.

Each case uses baseline/candidate/candidate/baseline, one process at a time on
CPU 4 of the shared host. The original 106 groups/100,096 mutations, three key
distributions, seeded probes and write kernels remain unchanged. Write timing
uses one excluded pass and 12 measured passes. Owned input cloning is timed;
map construction, snapshots and validation are outside the write windows.

Each read case now constructs 12 fresh maps. On each map it first times one
512-probe pass, then validates all outputs, performs eight untimed probe passes,
and measures 12 warm passes on **that same live map**. Construction touches
nodes: the first-probe panel is not a hardware cache flush. Each panel remains
separate, including every epoch and sample. The map-identity test checks this
lifetime protocol. Counting uses one map and one pass per panel. Point and
predecessor outputs are borrowed; scan16 owns its results and drops them outside
timing. Per-call timer overhead remains.

All **338 processes**, **264 timing rows** and **264 counting rows** finish.
The 42 execution cases yield 66 comparison cells: 18 writes and 48 read panels.
Independent reconstruction checks **3,816 live prefix states / 1,908 old views**,
all final maps and all 24 probe sets. Independent analysis rechecks identities,
terminal records and every reported statistic from raw samples. Each timing
row has 1,272 write-group windows, 6,144 first-probe windows or 73,728 warm
windows. Each pooled arm combines two such rows. Allocation counts and requested
live/peak/released bytes match across both arms and both orders. Requested bytes
are not RSS or allocator usable sizes.

## Write results

Means below are microseconds per retained mutation group, not per database call.
Negative changes mean less time. Every write mean improves in both orders;
original pinned unique insertion is the only pooled p99 regression and regresses
in both orders (+1.002% / +9.223%).

| Keys | Workload | Snapshot | Baseline mean (us) | Candidate mean (us) | Mean change | p99 change |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| original24 | overwrite | none | 100.966 | 82.570 | -18.220% | -17.361% |
| original24 | overwrite | pinned | 116.041 | 102.240 | -11.893% | -11.569% |
| original24 | initial_fill | none | 101.525 | 84.237 | -17.029% | -14.624% |
| original24 | initial_fill | pinned | 117.830 | 104.570 | -11.254% | -11.689% |
| original24 | unique_insert | none | 267.172 | 246.541 | -7.722% | -10.576% |
| original24 | unique_insert | pinned | 331.006 | 324.645 | -1.922% | +5.665% |
| short4 | overwrite | none | 143.229 | 120.832 | -15.638% | -14.817% |
| short4 | overwrite | pinned | 185.129 | 172.573 | -6.782% | -9.325% |
| short4 | initial_fill | none | 142.391 | 117.747 | -17.307% | -19.790% |
| short4 | initial_fill | pinned | 183.308 | 172.519 | -5.886% | -7.458% |
| short4 | unique_insert | none | 419.959 | 396.611 | -5.560% | -3.181% |
| short4 | unique_insert | pinned | 556.839 | 548.123 | -1.565% | -0.253% |
| dispersed0 | overwrite | none | 141.645 | 120.648 | -14.824% | -15.477% |
| dispersed0 | overwrite | pinned | 182.454 | 171.285 | -6.122% | -7.848% |
| dispersed0 | initial_fill | none | 139.715 | 116.932 | -16.307% | -16.999% |
| dispersed0 | initial_fill | pinned | 182.341 | 171.549 | -5.919% | -9.279% |
| dispersed0 | unique_insert | none | 421.645 | 400.164 | -5.095% | -4.899% |
| dispersed0 | unique_insert | pinned | 560.645 | 545.736 | -2.659% | -4.273% |

## Read limits and next investigation

These are component calls, including timing overhead, with no engine locks,
Raft, RPC or snapshot creation. Full baselines, candidate values and both-order
comparisons for all 66 cells are in the [analysis](outlined-mutation-path-v1/analysis.json).
The failing read cells are:

| Keys | Map | Operation | Panel | Mean change | p99 change |
| --- | --- | --- | --- | ---: | ---: |
| original24 | overwrite | get_hit | first_touch | +2.431% | -6.645% |
| original24 | overwrite | get_hit | warm | +14.693% | +18.812% |
| original24 | overwrite | get_miss | first_touch | +1.715% | +6.452% |
| original24 | overwrite | get_miss | warm | +6.914% | +8.333% |
| original24 | unique_insert | scan16 | first_touch | -3.798% | +5.803% |
| short4 | overwrite | get_hit | first_touch | +3.079% | +0.000% |
| short4 | overwrite | get_hit | warm | +14.504% | +18.812% |
| short4 | overwrite | get_miss | warm | +3.354% | +8.333% |
| short4 | overwrite | predecessor | first_touch | +2.911% | +31.020% |
| short4 | overwrite | scan16 | first_touch | -0.255% | +2.357% |
| short4 | unique_insert | get_hit | first_touch | -2.883% | +2.508% |
| short4 | unique_insert | get_miss | first_touch | -1.916% | +2.422% |
| short4 | unique_insert | predecessor | first_touch | -2.112% | +9.947% |
| short4 | unique_insert | scan16 | first_touch | +11.157% | +6.950% |
| dispersed0 | overwrite | get_hit | first_touch | +2.888% | +2.804% |
| dispersed0 | overwrite | get_hit | warm | +15.353% | +9.091% |
| dispersed0 | overwrite | get_miss | first_touch | +3.598% | +12.085% |
| dispersed0 | overwrite | get_miss | warm | +8.241% | +17.117% |
| dispersed0 | unique_insert | predecessor | warm | +0.798% | +3.766% |

Original small-map warm GET hit rises from 51.796 to 59.406 ns; pooled p99 rises
from 101 to 120 ns. This direction repeats in both orders and the other key
distributions. It is a retained local observation, not a 15% database regression.
First-probe and warm measurements are both retained; changing warmup did not
make this candidate pass.

Static inspection finds equal sizes and operand shapes for the actual `answer`
and `read_panels` functions (2,184 and 14,014 bytes), after removing instruction
addresses and RIP-relative displacements. Addresses change by 0x230. This does
not resolve relocation targets, data/cache placement, branch prediction or host
noise, and does not establish which causes the regression. The observation
supports diagnosing the existing executables before another algorithm rewrite.

Next inspect the retained GET dispatch/loop and resolved targets, then design
one bounded diagnostic for the repeatable warm-read regression and the shared
clone/new-key cost. A profile must qualify its own sampling and overhead; do not
rerun the unchanged full matrix or modify the measured dependencies. A future
changed candidate gets its own prospective plan. Keep both prior and current
failed gates; no post-hoc claim of acceptance or comparison pooled with older
callback-only measurements. Production promotion still needs full local
correctness, ordinary recovery, actual Chaos Mesh and matched database/three-copy
Redis throughput and latency. Industrial roadmap checkbox states stay open.

Latest recorded database values remain **137,873.776 Put/s** and
**1,022,750.059 BatchPut(64) items/s** for `86aa6fc`, three Raft replicas,
c64/128-byte values on the volatile tmpfs fixture. There is no new Redis parity
or durable-storage result. All bulk output uses `/mnt/data/kv9-work`; hosted CI
was not requested.

## Reproduction and evidence

The [outlined harness](../scripts/resident-outlined-mutation/README.md) and
[failed named harness](../scripts/resident-named-mutation/README.md) document the
protocol. The [evidence packet](outlined-mutation-path-v1/README.md) includes
both dependency variants, proof inputs/controls, build logs, disassembly,
engine smokes, plans, every sample/count row and independent reconstruction.
Original roots are `/mnt/data/kv9-work/named-mutation-adapter-20260916-first`
and `/mnt/data/kv9-work/outlined-mutation-path-20260916-first`.

Run the proof checkers with pinned Lean 4.33.1, `--work` pointing to the relevant
extracted dependency root and a fresh `--output`. Execution helpers retain the
original absolute workspace paths; when reproducing elsewhere, copy to a fresh
root and update those path constants before resolving/building. Do not overwrite
any published attempt. Binary identities remain recorded; executables and Lean
compiled artifacts stay in the original roots rather than the source packet.
