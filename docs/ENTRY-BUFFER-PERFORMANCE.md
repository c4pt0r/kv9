# Single-buffer engine write screen

Subsequent result: the [qualified safe-accessor three-arm screen](KEY-CLAMP-PERFORMANCE.md)
also fails. Stop the single-buffer/accessor family; the original result below
remains retained and production is unchanged.

Date: 2026-09-16 UTC. Execution base: `e17d1b33fa4d76cb03a7457942f4a1bbba6beabb`.

The qualified single key/value buffer **fails the declared write gate**. It
removes one data-buffer allocation per nonempty Put and saves 24 requested entry
bytes, but original-key overwrite improves only 1.469% without an old view and
regresses 0.143% with one. Original unique insertion regresses 2.480% / 0.430%.
Long-key insertion improves, while pinned long-key overwrite p99 regresses
2.690%. Production stays unchanged; the subsequent read/range stage is not run.
There is no new database QPS or Redis result.

## Completed scope

The [published plan](entry-buffer-qualification-v1/next-performance-plan.json)
was fixed before timing. Four ordinary ThinLTO builds use the exact
[qualified engine sources](ENTRY-BUFFER-QUALIFICATION.md), original registry
dependencies and one shared inherited harness transformation. First-party
artifacts were invalidated before each release build; Clippy denies warnings.
The timing/counting harnesses match byte-for-byte after removing only declared
instrumentation and untimed footprint metadata. Each ELF and source is pinned.

All **52 workload processes** finish: four preparations, 16 allocation-only
processes and 32 ABBA timing processes. Each timed process has twelve passes
through 106 retained groups / 100,096 mutations after one excluded pass.
Original keys are 27 bytes for overwrite or 35 bytes with unique ordinals;
long inputs are padded to 128 bytes before adding ordinals, producing 136-byte
unique keys. Values are 128 bytes. Default map sizes are 4,096 / 100,096 keys.

Every timing window is one actual `ReplicatedEngine::write_applied` group. It
includes the new whole-batch length preflight, consumed input destruction and
position/revision updates. Batch construction and old-view acquisition/drop are
outside. Windows have unequal group sizes; their percentiles are group latency,
not individual write-request latency. CPU 4 is shared-host pinned; helpers use
CPUs 6–15 and 22–31. This experiment contains no WAL, Raft or RPC latency.

Independent byte reconstruction verifies 3,392 live prefixes, 1,696 old views,
6,784 refused applications, both other-column-family sentinels, final maps and
read-probe identities. Every counted/timed pass checks final contents and
position/revision. All child PID/start-time identities, terminal outputs,
source/binary hashes, raw samples and derived statistics pass readback.

## Timing result

Pooled ABBA values below are microseconds per applied group. Negative changes
mean less elapsed time.

| Case | Mean, baseline → candidate (us) | Mean change | p99, baseline → candidate (us) | p99 change |
| --- | ---: | ---: | ---: | ---: |
| Original / overwrite | 111.220 → 109.587 | -1.469% | 304.269 → 304.420 | +0.050% |
| Original / overwrite + old view | 122.073 → 122.248 | +0.143% | 327.322 → 325.710 | -0.492% |
| Original / unique insert | 254.854 → 261.175 | +2.480% | 777.724 → 788.315 | +1.362% |
| Original / unique insert + old view | 341.471 → 342.939 | +0.430% | 1061.225 → 1037.320 | -2.253% |
| Long / overwrite | 111.398 → 107.032 | -3.919% | 303.328 → 289.752 | -4.476% |
| Long / overwrite + old view | 126.067 → 127.569 | +1.191% | 335.257 → 344.274 | +2.690% |
| Long / unique insert | 309.503 → 293.490 | -5.174% | 1101.119 → 1012.924 | -8.010% |
| Long / unique insert + old view | 397.001 → 383.919 | -3.295% | 1387.104 → 1263.583 | -8.905% |

Cases 0–3 miss the required 10% original-data mean reduction; cases 1–3 also
fail the requirement that both order directions improve. Long pinned overwrite
exceeds the 2% p99 regression bound. These five failed cases stop stage two.
No thresholds were relaxed and no completed matrix was retried.

Both order directions remain in the raw [analysis](entry-buffer-performance-v1/analysis.json).
For example, original unique unpinned mean regresses 1.414% / 3.582%. Long unique
unpinned mean improves 4.185% / 6.170%, but its per-order p99 worsens 0.977% /
0.640% even though pooled p99 improves 8.010%. Pooling quantiles can conceal
order effects; the pooled value is not evidence of a tail improvement in every
order. The failed overall gate remains failed.

## Allocation result

All **10,176 paired allocation windows** match independent group accounting.
For each group, every constructed entry saves one request and 24 requested
bytes. Freed-entry differences reflect whether an old view retains the prior
version: all overwrite entries free inside an unpinned window, while pinned
windows retain the group's previously visible distinct keys. Unique insertion
frees no replaced entry. Reallocation remains zero. The validator checks all
six request counters, live-byte differences, peak bounds and aggregate records.

A separate untimed fresh-index observation verifies full reclamation on drop:

| Dataset / map | Baseline requested bytes | Candidate requested bytes | Reduction |
| --- | ---: | ---: | ---: |
| Original / 4,096 keys | 1,093,964 | 995,612 | 98,352 |
| Original / 100,096 keys | 27,526,732 | 25,124,380 | 2,402,352 |
| Long / 4,096 keys | 1,507,660 | 1,409,308 | 98,352 |
| Long / 100,096 keys | 37,636,428 | 35,234,076 | 2,402,352 |

The difference is exactly `24 * (default_keys + 2)`, including both CF sentinels.
These are Rust allocation requests, not physical jemalloc footprint or RSS.
Fewer requests and bytes did not establish a write-performance win.

## Changed follow-up supported by actual code generation

The exact candidate timing ELF retains two unsigned `key_len > buffer_len`
failure branches before each insertion `memcmp`; the baseline comparator has
neither. Source inspection maps these to safe prefix slicing in `key()`.
The reviewed insertion excerpts and ELF hashes are in the packet. This static
difference is a hypothesis, not measured causal attribution. Combined-buffer
locality and the extra preflight scan also remain possible costs.

One separate **codegen-only** control changes only `key()` to use the safe bound
`key_len.min(bytes.len())` before slicing. For a valid entry this selects the
same prefix. In its actual ordinary-release engine comparator, the two failure
branches become conditional moves. No unsafe access is added. Constructor,
value accessor, preflight and selected dependencies are unchanged. The fifth
release build passes Clippy but **executes no workload**; the changed candidate
has not passed source-bound proof/model qualification or timing.

The [next plan](entry-buffer-performance-v1/next-key-clamp-plan.json) first proves
and checks the private valid-entry invariant and this accessor identity. After
qualification, a fixed three-arm comparison will distinguish the accessor
change from the failed single-buffer control while retaining the selected
baseline and original write-selection gates. Static branch removal alone is
not sufficient to start a production rollout.

The [packet](entry-buffer-performance-v1/README.md) retains sources, all failed
gates, complete raw windows and the static control. Industrial requirements,
actual distributed recovery/Chaos Mesh and matched three-copy Redis throughput
and latency remain open. CI stays local; bulk output remains under
`/mnt/data/kv9-work` and the repository target is reused.
