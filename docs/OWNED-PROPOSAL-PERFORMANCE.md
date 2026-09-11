# Owned-proposal write screen: retain the CRC baseline

The first complete write-only screen does **not** support selecting proposal
buffer candidate `71c9d996e1dcc0897da14be1886e669f0c3ea773`. The development
baseline remains `ca0002c7f8e9ee6f595efcc9f4151085ccce87cb` on master, using
tonic streaming gRPC. At concurrency 64, candidate point-write throughput
falls **0.734%** and BatchPut(64) throughput falls **9.449%**. Batch-write
whole-call p99 rises from **9.306-9.437 ms to 25.428-25.690 ms** when the two
repetitions' raw histograms are pooled. All four pooled workload/concurrency
combinations lose throughput and increase mean latency.

The two batch-write repetitions differ substantially: candidate throughput
changes **+1.125% / -19.863%**, and its p99 is worse in both. These short
shared-host observations do not establish a causal allocator, scheduler or
cache explanation. Neither repetition is discarded or rerun. The candidate
is rejected for promotion on this evidence; its process-recovery acceptance
remains valid within its separate scope.

## Current baseline and same-recording Redis

Each value has 128 bytes. Rates pool two ten-second forward/reverse cohorts;
mean latency weights every completed call. Percentiles merge the original raw
histograms and are reported as bucket intervals. Batch latency covers all
64 keys in one call and is never divided by the batch size.

### Concurrency 64

| Workload | Version | Calls/s | Keys/s | Mean us | p95 us | p99 us |
| --- | --- | ---: | ---: | ---: | --- | --- |
| PUT/SET | Selected CRC | 123,527 | 123,527 | 517.971 | 704.512-712.703 | 827.392-835.583 |
| PUT/SET | Proposal candidate | 122,619 | 122,619 | 521.805 | 704.512-712.703 | 835.584-843.775 |
| PUT/SET | Redis | 494,238 | 494,238 | 129.339 | 163.840-165.887 | 235.520-237.567 |
| BatchPut/MSET(64) | Selected CRC | 13,481 | 862,759 | 4,746.206 | 6,815.744-6,881.279 | 9,306.112-9,437.183 |
| BatchPut/MSET(64) | Proposal candidate | 12,207 | 781,233 | 5,241.703 | 7,274.496-7,340.031 | 25,427.968-25,690.111 |
| BatchPut/MSET(64) | Redis | 95,639 | 6,120,871 | 664.510 | 950.272-958.463 | 999.424-1,007.615 |

Redis's observed throughput is **4.00x** the selected baseline for point writes
and **7.09x** for batch writes. The previous broader recording measured
125,136 PUT/s and 872,821 batch-write keys/s for the identical baseline.
The new recording is approximately **1.29% / 1.15%** lower. This comparison
across runs is not a source improvement or a new baseline-code regression.

### Concurrency 1

| Workload | Version | Calls/s | Keys/s | Mean us | p95 us | p99 us |
| --- | --- | ---: | ---: | ---: | --- | --- |
| PUT/SET | Selected CRC | 16,815 | 16,815 | 59.368 | 69.632-70.655 | 81.920-82.943 |
| PUT/SET | Proposal candidate | 16,668 | 16,668 | 59.895 | 70.656-71.679 | 82.944-83.967 |
| PUT/SET | Redis | 170,583 | 170,583 | 5.742 | 6.144-6.207 | 7.872-7.935 |
| BatchPut/MSET(64) | Selected CRC | 5,410 | 346,217 | 184.743 | 206.848-208.895 | 249.856-251.903 |
| BatchPut/MSET(64) | Proposal candidate | 5,399 | 345,560 | 185.094 | 208.896-210.943 | 251.904-253.951 |
| BatchPut/MSET(64) | Redis | 42,381 | 2,712,380 | 22.458 | 23.296-23.551 | 31.488-31.743 |

The selected baseline's isolated point-write mean remains about **10.34x**
Redis's. Loaded throughput does not remove this turnaround gap.

### Reads were not rerun in this screen

The latest accepted read numbers remain the [previous complete matrix](OWNED-BUFFER-PERFORMANCE.md):
at c64, selected CRC has **346,056 GET/s**, mean **184.812 us** and p99
**352.256-356.351 us**, versus Redis **509,859 GET/s**, mean **125.410 us** and
p99 **231.424-233.471 us**. BatchGet(64) has **2,267,868 keys/s** versus Redis
**5,925,161 keys/s**. At c1, GET means are **37.735 us / 5.695 us**.
These read measurements belong to the earlier recording; they must not be
presented as new measurements from this write-only screen.

## Complete write-screen evidence

The frozen protocol covers c1/c64, point PUT/BatchPut(64), CRC/proposal/Redis
and two whole forward/reverse repetitions: **24 timed cohorts**, preceded by
**12 independent two-second smoke cohorts**. It uses 4,096 keys plus a sentinel,
128 warmup calls, seed 71, a 1,500-ms deadline, a ten-million-call cap and a
closed-loop client. Complete-cohort elapsed time includes cutoff completions.
The inherited fixture uses a 16-MiB public byte limit; the production default
remains 64 MiB. Admission counts and write-completion semantics are unchanged.

Clients use CPUs 0-1; all three KV9 voters, or Redis, share CPUs 2-5. Helpers
and three owned background containers use CPUs 6-15,22-31 during timing, with
their original settings restored afterward. Other host services remain
unconstrained. No build, test, profile, fault injection or actual audit overlaps
the timed recording.

KV9 retains three-voter Raft quorum, sync calls and commit/apply-before-success,
with volatile tmpfs WAL. Redis 7.0.15 runs standalone, with save/AOF and
pipelining disabled and one I/O thread. The comparison measures memory
execution under different replication/durability semantics. It establishes
neither equal durability nor sustained capacity; the Redis batch client also
approaches its two-core budget.

Smoke accepts **1,790,613 successful measured calls**. Timing and the unchanged
independent audit accept **22,379,974 measured calls / 242,284,183 input keys**.
All measured calls succeed on one observed attempt. Refusals, unknown writes,
read failures, client rejections and dropped slots are zero. The audit verifies
**80 exited lifetimes, 48 fresh drains, 48 writer/listener bindings**, 2,343
role/source checks and 4,643 resource samples, with owned-container restoration
and historical namespace maps preserved. Original retention contains 2,082
files / 53,076,813,405 bytes; the input inventory has 2,899 entries.

The [unaltered statistical readout](../scripts/redis-reference/owned-proposal-screen-v1/statistics/READOUT.md)
contains all twelve pooled rows and all eight paired repetitions. The
[publication index](../scripts/redis-reference/owned-proposal-screen-v1/index.json)
binds exact original reports, resource coverage, frozen helpers/contracts,
audit, inventories, source/build bindings and terminal/statistics records.
It contains **160 indexed copies / 35,587,837 decoded bytes / 5,539,505 stored
bytes**, including all 24 reports and their 24 resource-coverage records.
Compressed reports retain both stored and decoded SHA-256 values. Raw WALs,
executables and unrelated host process listings remain local; the compact
publication is not a self-contained runtime archive.

One ancillary root readback initially used `cleanup_errors` instead of the
wrapper's actual `child_cleanup_errors` field. That failed readback and the
corrected schema read are retained before the first independent audit.
No runtime or validation predicate was changed or rerun to remove a cohort.
The throughput fixture verifies readback, sentinels and nonce bounds but does
not retain the complete issued-operation ledger or prove linearizability.

## Exact-source process recovery

The original default-feature release is bound to all **591 source files** at
`71c9d996`. Its original server/workload Cargo records show opt-level 3 and the
default engine/Raft/server graphs. The copied server SHA-256 is
`b740e3d3c7ca1ad6cac2ed901d2c8a52b128c7c6456f95407f479bb61f414033`;
the build-manifest SHA-256 is
`b3cd4c20bd5033bf3651ec9c2bbee008ecdee10a81b724f40a9e513fd91448b3`.
These are original build records, not retrospective replacements.

The separate process fixture and unchanged independent auditor accept both
complete atomic histories: **347 calls, 321 OK and 26 unknown**. Streaming has
173 calls (159 OK / 14 unknown); explicit unary has 174 (162 OK / 12 unknown).
Both cover overlapping point/batch operations, leader SIGKILL and restart from
the original directories. Two client and five server lifetimes exit, and each
case obtains fresh three-voter drains. All unknowns remain in the checked
histories and witnesses. Source, binaries and original inputs remain bound.

The [process publication index](../scripts/redis-reference/owned-proposal-process-v1/index.json)
retains **60 exact copies / 2,139,518 bytes** of original build, invocation,
terminal, audit and full-history records. Publication-only aliases preserve
the actual source paths in the index. This is one-host ordinary-WAL process
recovery, not disk/power-loss or a whole-system refinement proof.

The candidate image was built, payload-checked and imported for the prepared
Chaos fixture, but **no candidate Chaos runtime or fault was launched**.
The performance rejection supplies no new Chaos coverage. Earlier candidates'
Chaos histories apply only to their own bound source revisions and scopes.

## Next work

Fewer proposal copies did not produce an accepted gain. Keep the selected CRC
baseline, Raft confirmation, fences, bounded admission and the dual-WAL recovery
contract. Redis throughput and c1/loaded latency parity remain open; dynamic
multi-Raft and automatic range splits remain subsequent work.

The next separate [experiment at 74d24116](https://github.com/c4pt0r/kv9/blob/74d241161eadf2121128f6fd9101ead18e562765/docs/INDEXED-APPLY-RECEIPTS.md)
targets bounded apply-receipt lookup and FIFO
eviction, with a checked ordering condition and unchanged duplicate-index
fallback. It has a written representation-equivalence argument and passes two
focused regressions, 711 workspace tests/doctests (23 ignored), Clippy and
formatting after a retained test-fixture type correction. It still requires
its own original release/recovery/performance evidence before selection. No
speedup is claimed for that unmeasured source. Full formal composition and
industrial Chaos/storage-fault coverage remain open. All CI in this phase is
local; no hosted workflow was dispatched. The
[source-check index](../scripts/redis-reference/indexed-receipt-source-v1/index.json)
retains 32 exact copies / 680,617 bytes, including the failed first compilation,
corrected checks and supporting CPU-summary/coverage-failure excerpts. The
written representation proof is scoped to the ring API and standard-container
contracts; it does not complete machine-checked Rust/Raft composition.
