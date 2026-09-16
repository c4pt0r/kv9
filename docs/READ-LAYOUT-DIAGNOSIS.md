# Read layout diagnosis and next engine comparison

Date: 2026-09-16 UTC. Base: `9ff177672659c0f99954812461c73defed2445aa`.

The [outlined mutation candidate](OUTLINED-MUTATION-PATH.md) remains held.
A bounded control reproduces the small-map warm GET-hit regression: **51.924 →
60.203 ns (+15.943%)**, with both orders worse. Forcing all Rust functions to
64-byte alignment does not fix it: **55.550 → 62.418 ns (+12.364%)**, again in
both orders. Both absolute GET-hit means become slower in the aligned build.
Do not select this compiler flag or reinterpret the original failed gate.

Four debugger-only first-map captures independently validate **16,384 nodes**.
Within each build configuration, baseline and candidate have identical tree
shape, key/value bytes, reference counts, capacities, relative virtual allocation
positions and page offsets. They also resolve GET comparison to the same libc
implementation. These observations do not locate a cache/branch cause or prove
that all later maps behave identically. No database read regression, new QPS or
Redis comparison follows from this component diagnostic.

## Prospective method and validation

The [plan](read-layout-diagnosis-v1/timing-plan.json) was fixed before measurement.
It reuses the exact ordinary binaries from the prior screen and builds identical
sources/dependencies with only `-Cllvm-args=--align-all-functions=6`. There is no
index-source patch, C flag change, sampling profiler or extra timing observer.
The flag is scoped to isolated builds; production release settings are unchanged.
Shared-cache invalidation/freshness, all source/dependency identities and both
same-map protocol tests pass. The post-run helper rename described below does
not change any measured Rust source, executable or plan.

Only original small-map GET hit, GET miss and pinned unique insertion run. Each
case interleaves ordinary-baseline, ordinary-candidate, aligned-candidate,
aligned-baseline, aligned-baseline, aligned-candidate, ordinary-candidate,
ordinary-baseline. The helper CPUs exclude measured CPU 4 and its sibling. The
host remains shared. The inherited 42-case namespace is not rerun.

All **26 processes / 40 timing rows / 10 comparisons** complete. Independent
reconstruction checks both aligned preparations: **3,816 live prefix states /
1,908 old views**, every final map and all probe identities. Independent analysis
recomputes every statistic from raw samples and verifies all terminal and source
records. Ordinary builds reuse their qualified sources/binaries; this diagnostic
does not add allocation-count evidence or repeat dependency tests.

The prior first-probe and same-map warm protocol is preserved: 12 fresh maps,
512 queries before query validation, eight untimed passes on that same map, then
12 measured warm passes. Construction touches nodes; this is not a cache flush.
Each pooled arm has 12,288 first-probe or 147,456 warm samples. Writes retain
2,544 pooled group samples per arm. Every sample is retained. Per-call timing
overhead remains, and scan/value ownership conventions remain unchanged.

## Results

Read values are nanoseconds per component call; write values are microseconds
per retained group. These are different units and neither is a database request
latency. The [full analysis](read-layout-diagnosis-v1/analysis.json) retains both
orders as well as pooled values.

| Case | Panel | Build | Unit | Baseline mean | Candidate mean | Mean change | Baseline p99 | Candidate p99 | p99 change |
| --- | --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| GET hit | first_touch | ordinary | ns/call | 127.030 | 129.781 | +2.165% | 330.000 | 281.000 | -14.848% |
| GET hit | first_touch | aligned64 | ns/call | 132.563 | 131.305 | -0.949% | 341.000 | 320.000 | -6.158% |
| GET hit | warm | ordinary | ns/call | 51.924 | 60.203 | +15.943% | 110.000 | 120.000 | +9.091% |
| GET hit | warm | aligned64 | ns/call | 55.550 | 62.418 | +12.364% | 110.000 | 120.000 | +9.091% |
| GET miss | first_touch | ordinary | ns/call | 139.839 | 136.974 | -2.048% | 311.000 | 260.000 | -16.399% |
| GET miss | first_touch | aligned64 | ns/call | 142.716 | 140.269 | -1.714% | 331.000 | 341.000 | +3.021% |
| GET miss | warm | ordinary | ns/call | 56.838 | 58.098 | +2.218% | 120.000 | 130.000 | +8.333% |
| GET miss | warm | aligned64 | ns/call | 58.093 | 56.732 | -2.343% | 130.000 | 120.000 | -7.692% |
| Pinned unique insertion | group | ordinary | us/group | 347.510 | 327.013 | -5.898% | 1009.003 | 1034.450 | +2.522% |
| Pinned unique insertion | group | aligned64 | us/group | 354.751 | 328.985 | -7.263% | 1095.656 | 913.784 | -16.599% |

Ordinary warm GET-hit mean regresses 15.352% / 16.540% by order, meeting the
prospective >5% reproduction criterion. Aligned warm GET hit still regresses
11.423% / 13.296%. GET miss changes direction between configurations, and the
pinned insertion p99 also changes direction. This shows sensitivity to the
broader build context, without identifying a single responsible instruction
or making the isolated candidate pass. This small diagnostic does not replace
the original full write/read matrix or its material-gain requirement.

## Code and runtime-map correspondence

The exact `answer` and `read_panels` functions remain 2,184 and 14,014 bytes in
all four executables. Their instruction/operand shapes and direct call target
names match after removing addresses and RIP-relative displacements. Their
six-entry operation jump tables resolve to identical offsets within `answer`.
The GET path references `memcmp@GLIBC_2.2.5` in every ELF. This is stronger than
checking symbol names alone, but is not an executed-instruction or cache proof.

| Function | Ordinary baseline mod 64 | Ordinary candidate mod 64 | Aligned baseline mod 64 | Aligned candidate mod 64 |
| --- | ---: | ---: | ---: | ---: |
| answer | 48 | 0 | 0 | 0 |
| read_panels | 16 | 32 | 0 | 0 |

The separate GDB capture stops at the first GET entry and only reads child
memory. It checks the actual first probe, traverses all 4,096 nodes and resumes
the child to normal completion. All four captured maps match the independently
reconstructed original24/overwrite model, satisfy strict key ordering and
red-black invariants, and have complete graph coverage. The runtime `memcmp`
GOT entry resolves to identical libc file offsets, library SHA-256 and first
64 instruction bytes in all captures.

Within ordinary and within aligned builds, every node/entry/key/value position
relative to its allocation origin—and to the combined origin—matches across
baseline/candidate. Page offsets match too. Across ordinary versus aligned
builds, relative virtual layouts differ, although shape, bytes and page offsets
still match. Therefore this is a broad build-layout intervention, not an isolated
instruction-alignment experiment. It does not establish matching physical pages,
cache state, branch history or later-epoch allocation positions. All debugger
results are explicitly excluded from the performance analysis.

The first decoder incorrectly applied an ArcInner-relative color offset to a
Node data pointer and was refused. The corrected offset is checked by complete
model and tree-invariant correspondence. Both failed attempts remain retained.
The first independent comparison also refused a list-versus-dictionary mismatch;
its corrected comparison sorts decoded pairs before comparing the unchanged
independent model. Python's error hook exposed a local `inspect.py` namespace
collision, so the helper is now `inspect_elf.py`. The exact original build helper
is retained with its original source hash in `retained-tool-bindings.json`.
No measured executable, Rust source, plan or sample was modified or rerun by
these tooling corrections.

## Next action and limits

The cause of the warm GET-hit gap remains unresolved. Stop alignment-flag sweeps
and unchanged index matrices. Move the next bounded diagnostic to the existing
engine interfaces: `ReplicatedEngine::write_applied`, `ReadView::get` and
`ReadView::get_resident`. The latter two distinguish owned value copying from
borrowed resident lookup; snapshot acquisition must be reported separately.
The current standalone benchmark's multi-operation `answer` dispatcher and its
per-call timestamps are not the server's complete read path.

A fresh prospective plan must verify compiled-path correspondence, exact input
and old-view/applied-position semantics, then retain separate first-probe and
same-snapshot warm panels. Complementary whole-pass measurements can help assess
timing context, but their p99 must never be labeled request p99. Use ordinary
release settings and one bounded representative comparison; do not introduce
another full index grid. The [next scope](read-layout-diagnosis-v1/next-engine-diagnostic.json)
is a plan only: no such engine comparison was built or timed in this stage.

The original component selection gate remains failed. A new diagnostic cannot
retroactively pass it or establish a production optimization. Promotion still
requires material benefit, full local correctness, ordinary recovery, actual
Chaos Mesh and matched database/three-copy Redis throughput and latency. CRC/rpds,
ThinLTO/Safe ReadIndex and industrial roadmap checkboxes are unchanged. No new
Raft, lease or durability behavior is introduced.

Latest recorded database results remain **137,873.776 Put/s** and
**1,022,750.059 BatchPut(64) items/s** on the `86aa6fc` three-replica,
c64/128-byte volatile tmpfs fixture. All new output is under `/mnt/data/kv9-work`;
no hosted CI was requested.

See the [tools](../scripts/read-layout-diagnostic/README.md) and
[evidence packet](read-layout-diagnosis-v1/README.md). Original execution root:
`/mnt/data/kv9-work/read-layout-diagnosis-20260916-first`.
