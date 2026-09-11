# CRC workload matrix and baseline selection

The complete 72-cohort comparison supports selecting `ca0002c7` as the next
development baseline. At concurrency 64, point PUT throughput increases
**5.04%**, BatchPut(64) **37.78%**, and 50/50 batch read/write **33.09%** over
`5ee897a`. GET is effectively unchanged. There is a small batch-read tradeoff:
pooled throughput decreases **0.39%**, and p99 increases in both repetitions.
Selection reflects the write/mixed gains with this explicit limitation; it is
not a no-regression or Redis-parity claim.

The selected source is `ca0002c7f8e9ee6f595efcc9f4151085ccce87cb`. It retains
streaming gRPC, normal Raft quorum, sync calls and committed/applied write
acknowledgements. The CRC change preserves the checksum polynomial, byte
coverage, fragments, log format and ordering. The separate experimental
read-credit and owned-buffer candidates are not part of these results.

## Throughput and loaded latency

These pooled results cover two ten-second forward/reverse repetitions at
concurrency 64. Values are 128 bytes. Batch operations contain 64 keys;
mixed workloads use 50% reads and 50% writes.

| Workload | KV9 calls/s | KV9 keys/s | Redis calls/s | Redis keys/s | KV9 mean us | Redis mean us | KV9 p99 us | Redis p99 us |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- |
| Point PUT/SET | 123,767 | 123,767 | 492,248 | 492,248 | 516.968 | 129.864 | 827.392-835.583 | 235.520-237.567 |
| Point mixed | 170,333 | 170,333 | 495,229 | 495,229 | 375.593 | 129.096 | 614.400-622.591 | 235.520-237.567 |
| Point GET | 344,111 | 344,111 | 508,831 | 508,831 | 185.860 | 125.669 | 352.256-356.351 | 231.424-233.471 |
| BatchPut/MSET(64) | 13,489 | 863,301 | 94,340 | 6,037,751 | 4,743.255 | 673.581 | 9,175.040-9,306.111 | 1,015.808-1,023.999 |
| Batch mixed(64) | 20,390 | 1,304,970 | 89,701 | 5,740,894 | 3,135.211 | 709.550 | 5,832.704-5,898.239 | 1,081.344-1,097.727 |
| BatchGet/MGET(64) | 34,890 | 2,232,969 | 92,763 | 5,936,844 | 1,829.039 | 687.929 | 3,112.960-3,145.727 | 1,040.384-1,048.575 |

Latency covers the complete call, including the complete batch. Percentiles
are histogram bucket intervals. Rates use summed calls/keys divided by summed
complete cohort time; means are call-weighted and percentiles merge the
original histograms. Batch key throughput is not point-request throughput.

Redis remains **1.48x** faster for point GET, **3.98x** for point writes,
**2.66x** for batch reads and **6.99x** for batch writes in this recording.

| Workload | c1 QPS change | c64 QPS change | c64 mean change |
| --- | ---: | ---: | ---: |
| Point PUT | +4.037% | +5.036% | -4.796% |
| Point mixed | +1.486% | +3.872% | -3.729% |
| Point GET | +0.111% | +0.116% | -0.116% |
| BatchPut(64) | +17.474% | +37.780% | -27.419% |
| Batch mixed(64) | +11.716% | +33.094% | -24.880% |
| BatchGet(64) | +0.456% | -0.389% | +0.395% |

All eight write/mixed workload-by-concurrency comparisons improve throughput,
mean and p99 in both original repetitions. The c64 batch-read p99 changes are
3.113-3.146 to 3.146-3.178 ms in repeat 0 and 3.015-3.047 to 3.080-3.113 ms
in repeat 1. The c1 batch-read p95 is also slightly higher. Both directions
remain visible; two repeats on a shared host do not establish statistical
significance or a sustained capacity limit.

## Isolated request turnaround

At concurrency 1, KV9 GET achieves **26,295 calls/s**, mean **37.914 us** and
p99 **53.760-54.271 us**. Redis reaches **172,994 calls/s**, mean **5.703 us**
and p99 **7.808-7.871 us**. The remaining mean-latency ratio is **6.65x**.
KV9 point PUT averages **59.155 us**, versus Redis **5.707 us**.

High concurrency amortizes coordination but does not eliminate per-request
work. CRC addresses a measured write CPU hotspot; it does not shorten the
read request path. The next execution-path work must therefore measure c1
turnaround and loaded tails separately, alongside throughput.

## Protocol, accounting and correctness scope

The frozen matrix covers c1/c64, point/batch64, read percentages 0/50/100,
control/candidate/Redis, and two complete forward/reverse repetitions. Each
timed cohort lasts ten nominal seconds after 128 warmup calls, with 4,096 keys
plus a sentinel, seed 71, a 1,500-ms deadline and a ten-million-call cap.
Clients use CPUs 0-1; all three KV9 voters, or Redis, share CPUs 2-5.
Helpers and three owned background containers use CPUs 6-15,22-31 during
timing and are restored afterward. Other host services remain unconstrained.

KV9 uses volatile tmpfs WAL while retaining normal Raft quorum and sync calls.
The memory reference is standalone Redis 7.0.15, save/AOF disabled, one I/O
thread and no pipelining. This compares client-visible memory execution costs,
not equal durability or fault semantics. The Redis batch client is close to
its two-core budget; the observed throughput is not an intrinsic Redis ceiling.

The 36 separate two-second smoke cohorts pass with **6,690,939** measured calls.
The complete timing and independent audit accept **79,349,358 measured calls**
and **795,634,977 input keys**. Every measured call succeeds on one observed
attempt; refusals, unknown writes, read failures, client rejections and dropped
slots are zero. Of these calls, 2,335 finish after the nominal cutoff and stay
in the complete cohort denominator. Initialization has 48 additional
leader-routing attempts; all setup, warmup and verification logical calls
succeed and remain separately accounted for.

The audit binds 240 exited lifetimes, 144 fresh drains, 144 exact writer/listener
bindings, 13,984 resource samples, 2,336 role/source checks and 4,012 retained
files totaling 83,887,851,985 bytes. CPU settings and all eight historical
namespace UIDs are restored/preserved. No build, test, profile, fault injection
or actual audit overlaps the timing interval.

Final readback checks deterministic values, the sentinel, nonce bounds and
write-key membership. This benchmark does not retain a complete operation
history or exact issued-nonce ledger. Separate exact-source
[process recovery](CRC-WRITE-SCREENING.md) and
[actual Chaos Mesh acceptance](CRC-CHAOS-ACCEPTANCE.md) supply complete-history
correctness evidence for their own workloads. The scoped CRC proof checks
22 theorems, all 256 compiled table entries and five rejected controls; it is
not a whole-Rust/Raft refinement proof. Local workspace tests, Clippy and
formatting also pass. Industrial fault coverage and proof composition remain
open requirements.

One ancillary smoke-summary reader initially expected a native cleanup field
on a Redis row. That failure is retained; a separately named corrected reader
uses each original schema. Timing starts only after the corrected gate passes.
No fixture is rerun and no frozen runtime or audit predicate is changed.

## Reproducible evidence and next work

The [publication index](../scripts/redis-reference/crc-broad-v1/index.json)
binds **212 files**, **110,692,880 decoded bytes** and **9,714,047 stored bytes**.
The 72 original client reports use gzip compression with zero timestamps;
stored and decompressed hashes and lengths are both recorded. The remaining
140 files are unchanged identity copies. To recover a report, use
`gzip -dc report.json.gz > report.json` and compare its SHA-256 with the index's
`decoded_sha256`. Compression changes no report field or acceptance predicate.
Large raw WALs and the host-wide process listing remain local at the recorded
inventory paths. The index does not claim to publish the entire 83.9-GB run.

The frozen statistical reader's original output retains
`performance_promotion=false` and its generic pending-acceptance text: the
reader derives evidence, while this document records the subsequent selection
decision. Historical raw outputs are not rewritten as later decisions.

The next isolated candidate, `9be0c1963515ff974426faadef80482b847b13a5`, moves
already-owned mutation buffers into the resident index. It passes 712 workspace
tests (23 ignored), Clippy and its own independently audited process recovery:
367 complete-history calls, 331 OK and 36 unknown. Unknown outcomes remain in
both streaming/unary histories. Its own
[actual Chaos fixture now also passes](OWNED-BUFFER-ACCEPTANCE.md), retaining
2,047 complete-history calls and 154 verified netem leaf drops. There is no
measured performance gain for this source yet.

Next compare owned-buffer performance against this CRC baseline.
Read-path scheduling and ownership transfers remain a separate measured
optimization. Retain the existing dual-WAL architecture decision in
`docs/SEGMENTED-WAL.md`: removing the engine WAL requires its own recovery,
group-aware retention and migration proof. Redis parity must precede the
planned dynamic multi-Raft and automatic range-split work. CI remains local;
no hosted workflow is dispatched for this phase.
