# Owned-apply performance: reject the candidate, retain the CRC baseline

The later [proposal-buffer write screen](OWNED-PROPOSAL-PERFORMANCE.md) now
provides newer write measurements and rejects that separate candidate too.
The read/mixed measurements below remain the latest accepted complete-matrix
results; they were not rerun in the later write-only screen.

The complete first 72-cohort recording does **not** support promoting
`9be0c1963515ff974426faadef80482b847b13a5`. Keep the selected development source
`ca0002c7f8e9ee6f595efcc9f4151085ccce87cb` on master, with tonic streaming gRPC.
At c64 the candidate improves batch-write throughput only **0.73%** and batch
mixed **1.98%**, while batch reads regress **2.59%**. All three c1 batch
workloads have lower throughput and higher mean latency in both repetitions.
Point writes also lose throughput at c64; batch-write p99 worsens substantially
in the second repetition. These tradeoffs do not justify a mainline change.

The candidate's separately accepted [process and Chaos histories](OWNED-BUFFER-ACCEPTANCE.md)
remain valid for their stated scope. Performance rejection is not a newly
observed consistency failure. Conversely, fewer source-level clones and
passing correctness checks do not establish a speedup. No cache, allocator or
scheduler explanation for the small changes has been measured in this run.

## Latest selected-baseline performance

These are new measurements of the **unchanged CRC baseline**, not a new source
improvement. The control role is `old` (CRC), `new` is the rejected owned-apply
candidate, and `redis` is the original Redis reference. All values below come
from this same recording. Values contain 128 bytes; mixed workloads have 50%
reads. Two ten-second forward/reverse repetitions are pooled for each row.

### Concurrency 64

| Workload | CRC calls/s | CRC keys/s | Redis calls/s | Redis keys/s | CRC mean us | Redis mean us | CRC p99 us | Redis p99 us |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- |
| Point PUT/SET | 125,136 | 125,136 | 495,476 | 495,476 | 511.317 | 129.017 | 819.200-827.391 | 235.520-237.567 |
| Point mixed | 170,717 | 170,717 | 500,638 | 500,638 | 374.749 | 127.704 | 614.400-622.591 | 233.472-235.519 |
| Point GET | 346,056 | 346,056 | 509,859 | 509,859 | 184.812 | 125.410 | 352.256-356.351 | 231.424-233.471 |
| BatchPut/MSET(64) | 13,638 | 872,821 | 94,345 | 6,038,080 | 4,691.387 | 673.520 | 8,781.824-8,912.895 | 1,015.808-1,023.999 |
| Batch mixed(64) | 20,505 | 1,312,289 | 90,228 | 5,774,596 | 3,117.813 | 705.390 | 6,094.848-6,160.383 | 1,081.344-1,097.727 |
| BatchGet/MGET(64) | 35,435 | 2,267,868 | 92,581 | 5,925,161 | 1,800.745 | 689.251 | 3,014.656-3,047.423 | 1,040.384-1,048.575 |

Redis's observed throughput is **1.47x** CRC for point GET, **3.96x** for point
PUT, **2.61x** for batch reads and **6.92x** for batch writes. Batch keys/s must
not be reported as point-request QPS. Latency covers the entire call, including
all 64 keys in a batch. Percentiles are histogram bucket intervals.

### Concurrency 1

| Workload | CRC calls/s | CRC keys/s | Redis calls/s | Redis keys/s | CRC mean us | Redis mean us | CRC p99 us | Redis p99 us |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- |
| Point PUT/SET | 16,735 | 16,735 | 172,146 | 172,146 | 59.654 | 5.690 | 79.872-80.895 | 7.424-7.487 |
| Point mixed | 18,796 | 18,796 | 171,753 | 171,753 | 53.096 | 5.743 | 76.800-77.823 | 7.424-7.487 |
| Point GET | 26,417 | 26,417 | 173,226 | 173,226 | 37.735 | 5.695 | 52.736-53.247 | 7.616-7.679 |
| BatchPut/MSET(64) | 5,433 | 347,694 | 42,552 | 2,723,299 | 183.959 | 22.364 | 241.664-243.711 | 28.672-28.927 |
| Batch mixed(64) | 6,534 | 418,151 | 40,405 | 2,585,945 | 152.543 | 23.600 | 231.424-233.471 | 30.464-30.719 |
| BatchGet/MGET(64) | 9,624 | 615,933 | 39,090 | 2,501,741 | 103.012 | 24.431 | 135.168-137.215 | 30.464-30.719 |

The isolated GET mean is **37.735 us** versus Redis **5.695 us**, a **6.63x**
ratio. Loaded throughput can amortize coordination while leaving this large
per-request turnaround gap. c1 latency and c64 mean/p95/p99 remain separate
acceptance criteria; a throughput-only comparison is insufficient.

## Candidate deltas and tail regressions

| Concurrency | Workload | Candidate QPS change | Candidate mean change | CRC p99 us | Candidate p99 us |
| ---: | --- | ---: | ---: | --- | --- |
| 1 | Point PUT/SET | +0.181% | -0.179% | 79.872-80.895 | 80.896-81.919 |
| 1 | Point mixed | +0.600% | -0.606% | 76.800-77.823 | 75.776-76.799 |
| 1 | Point GET | -0.253% | +0.253% | 52.736-53.247 | 52.224-52.735 |
| 1 | BatchPut/MSET(64) | -1.261% | +1.278% | 241.664-243.711 | 241.664-243.711 |
| 1 | Batch mixed(64) | -0.900% | +0.912% | 231.424-233.471 | 233.472-235.519 |
| 1 | BatchGet/MGET(64) | -1.532% | +1.571% | 135.168-137.215 | 139.264-141.311 |
| 64 | Point PUT/SET | -0.696% | +0.700% | 819.200-827.391 | 827.392-835.583 |
| 64 | Point mixed | +0.531% | -0.529% | 614.400-622.591 | 614.400-622.591 |
| 64 | Point GET | -0.557% | +0.562% | 352.256-356.351 | 352.256-356.351 |
| 64 | BatchPut/MSET(64) | +0.729% | -0.721% | 8,781.824-8,912.895 | 9,043.968-9,175.039 |
| 64 | Batch mixed(64) | +1.983% | -1.947% | 6,094.848-6,160.383 | 5,636.096-5,701.631 |
| 64 | BatchGet/MGET(64) | -2.586% | +2.665% | 3,014.656-3,047.423 | 3,112.960-3,145.727 |

The [unaltered full readout](../scripts/redis-reference/owned-buffer-broad-v1/statistics/READOUT.md)
publishes all 36 pooled rows and all 24 paired repetitions, including candidate
absolute rates, mean/p95/p99 and measured CPU scopes. In the two c64 batch-read
repetitions, throughput falls **2.709% / 2.463%** and p99 is higher in both.
For c64 batch writes, p99 improves in repeat 0, but changes from
**8.782-8.913 ms to 9.699-9.830 ms** in repeat 1. No cohort is discarded.
Two short repetitions on a shared host do not establish statistical
significance, a sustained capacity limit or an intrinsic Redis ceiling.

## Protocol, complete accounting and limits

The frozen protocol retains c1/c64, point/batch64, read percentages 0/50/100,
CRC/candidate/Redis and two complete forward/reverse repetitions. Each cohort
has ten nominal timed seconds after 128 warmup calls, 4,096 keys plus a
sentinel, seed 71, a 1,500-ms deadline and a ten-million-call cap. The workload
is closed loop. Rates divide summed completed calls/keys by summed complete
cohort time, means are call-weighted, and percentile histograms are merged.
Operations completing after nominal cutoff remain in complete-cohort results.

Timed clients use CPUs 0-1; all three KV9 voters, or Redis, share CPUs 2-5.
Helpers and three owned background containers use CPUs 6-15,22-31 and their
original settings are restored afterward. Other host services remain
unconstrained. No build, test, profile, fault injection or actual audit overlaps
timing. Separate-source editing and static test preparation do not modify
any measured source or executable.

KV9 uses normal three-voter Raft quorum, sync calls and committed/applied write
acknowledgements, with volatile tmpfs WAL. Redis 7.0.15 runs standalone with
save/AOF disabled, one I/O thread and no pipelining. This compares memory
execution costs with explicitly different replication/durability semantics;
it does not establish disk durability or equal fault guarantees. The Redis
batch client approaches its two-core budget, limiting capacity conclusions.

All 36 separate two-second smoke cohorts pass: **6,384,145 measured calls**.
Timing and the independent audit accept **80,198,970 measured calls** and
**812,121,630 input keys**. Measured calls equal issued calls and attempts;
all succeed. Refusals, unknown writes, read failures, client rejections and
dropped slots are zero. Initialization has 48 additional leader-routing
attempts; setup, warmup and final verification populations remain separate.
The native-only attempt-reason count of 31,752,023 is not the all-role total.

The audit accepts 240 exited lifetimes, 144 fresh drains, 144 writer/listener
bindings, 13,989 resource samples and 2,344 role/source checks. All three owned
background containers regain their original settings; eight historical
namespace UIDs are preserved. The original 4,320 retained files total
94,449,175,837 bytes; the audit binds 6,753 input-inventory entries.

Final checks cover deterministic readback, sentinel values, nonce bounds and
write-key membership. This throughput fixture does not retain an exact issued
nonce ledger or a complete operation history. Separate process/Chaos histories
cover their own workloads; this benchmark adds no new chaos coverage or formal
proof. The industrial proof, storage-fault and broader fault-matrix work stays
open. All runtime and independent audit invocations terminate successfully
without a timing rerun. One preparation-only source-field lookup failure is
retained with its metadata correction; executable predicates are unchanged.

## Published evidence and next experiment

The [publication index](../scripts/redis-reference/owned-buffer-broad-v1/index.json)
binds **216 files / 110,860,851 decoded bytes / 9,878,233 stored bytes**. It
includes 72 original reports compressed with zero gzip timestamps, their
resource coverage, frozen driver/auditor/statistics sources, original audit,
input inventories, smoke/launch/terminal records and restoration outputs.
Both encoded and decoded lengths/SHA-256 values are recorded; every decoded
report is byte-identical to its original. Large raw WALs and the host-wide
process listing remain local. This compact bundle is not a self-contained
runtime archive. Historical generic pending text in raw statistics remains
unchanged; this document records the subsequent rejection decision.

The next separate [proposal-buffer candidate](https://github.com/c4pt0r/kv9/blob/71c9d996e1dcc0897da14be1886e669f0c3ea773/docs/OWNED-PROPOSAL-BUFFERS.md)
starts from CRC and consumes buffers during batch planning and fenced-command
construction. Its first local checks pass three focused regressions and
712 workspace tests (23 ignored), Clippy and formatting. The
[source-check records](../scripts/redis-reference/owned-proposal-source-v1/index.json)
retain original invocations, logs, exits and source hashes. Its own original
release and process recovery subsequently pass (347 calls, 321 OK / 26 unknown).
The later write-only screen rejects it for promotion: c64 batch-write
throughput falls 9.449% with substantially worse tails. No candidate Chaos
runtime was launched. The baseline remains selected.

Read-path execution ownership and actual Raft tick-service measurement remain
separate work. Do not infer delivered ticks from pump iterations or status
export progress. Keep quorum, fencing, bounded admission and apply-before-success;
retain the existing dual-WAL recovery contract. Redis parity precedes dynamic
multi-Raft and automatic range splits. All CI stays local for this phase.
