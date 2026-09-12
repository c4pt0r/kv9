# Bounded Append payloads: no useful mixed-read gain

Keep CRC `ca0002c7` selected. Candidate
[74b958a](https://github.com/c4pt0r/kv9/commit/74b958a8bcdf25252ab55ba6149876a1cddc0637)
raises raft-rs's Append entry-payload target from its one-entry default to
64 KiB. Its complete second 24-cohort screen records **+0.790% c64 pure GET**,
but only **+0.031% c64 mixed throughput** with opposite repeat signs and unchanged
mixed GET p99. This does not demonstrate the intended reduction in mixed-load
replication overhead. Hold the candidate; do not expand its full-matrix/Chaos
qualification or promote it as a performance improvement. These short repeats
are not a significance test or a no-regression bound.

## Same-run performance

Both forward/reverse repetitions remain in the statistics. QPS is total calls
divided by total elapsed time; mean latency pools whole-call nanoseconds and
counts. p99 is an interval from merged original histogram buckets, never an
average of percentiles. Mixed GET and PUT/SET populations are separate.

| Metric | Selected CRC | 64-KiB Append | Redis |
| --- | ---: | ---: | ---: |
| c1 GET calls/s | 26,443 | 26,492 | 174,086 |
| c1 GET mean us | 37.701 | 37.625 | 5.667 |
| c1 GET p99 us | 50.176-50.687 | 50.176-50.687 | 7.232-7.295 |
| c64 GET calls/s | 343,972 | 346,690 | 509,202 |
| c64 GET mean us | 185.934 | 184.477 | 125.576 |
| c64 GET p99 us | 352.256-356.351 | 352.256-356.351 | 231.424-233.471 |
| c64 mixed combined calls/s | 171,766 | 171,818 | 502,854 |
| c64 mixed GET mean us | 384.575 | 384.376 | 127.140 |
| c64 mixed GET p99 us | 622.592-630.783 | 622.592-630.783 | 233.472-235.519 |
| c64 mixed PUT/SET mean us | 360.364 | 360.339 | 127.133 |

| Workload | Repeat 0 QPS change | Repeat 1 QPS change | Pooled QPS change |
| --- | ---: | ---: | ---: |
| c1 GET | +0.863% | -0.488% | +0.184% |
| c64 GET | +0.145% | +1.437% | +0.790% |
| c1 mixed | -1.156% | +0.581% | -0.293% |
| c64 mixed | +0.096% | -0.034% | +0.031% |

C64 pure GET p99 worsens in repeat 0 (352.256-356.351 to
356.352-360.447 us) and improves in repeat 1 (to 348.160-352.255 us).
C1 pure GET p99 also moves in opposite directions. Mixed c64 GET p99 is
unchanged in both repeats; its mean improves by 0.398 us in repeat 0 and is
0.003 us worse in repeat 1. Mixed PUT p99 worsens in repeat 1. The
[complete readout](raft-append-payload-v1/READOUT.md) and
[per-repeat table](raft-append-payload-v1/PER-REPEAT.md) retain all means,
p50/p95/p99 intervals and outcomes. A small pure-read change cannot be attributed
to reduced Append work from this screen alone.

## Implementation and correctness scope

The [candidate design and source argument](https://github.com/c4pt0r/kv9/blob/74b958a8bcdf25252ab55ba6149876a1cddc0637/docs/RAFT-APPEND-PAYLOAD.md)
uses raft-rs 0.7.0's existing contiguous log slicing. `batch_append` remains
false; there is no new batching timer. The target covers encoded entries,
not envelope headers, and allows one legal oversized entry for progress.
Changing the target does not collect future proposals into one Append. A
healthy follower can still receive one newly available entry at a time.
This is a source-based explanation for limited benefit, not an observed
message-count attribution from the uninstrumented timing run.

Local gates pass **436 Raft/server tests and doctests**, plus **234 experimental
server tests and doctests**, with overlap and one ignored per configuration,
formatting and Clippy. The lag/rejoin regression observes actual multi-entry
Append messages, contiguous indexes, payload bounds including the oversized
single-entry exception, and final values on all three replicas. It does not
benchmark catch-up speed. All 595 source files bind to the original clean
release; no rebuild replaces that executable during either timing attempt.

Ordinary stream/unary leader-loss and original-directory restart histories pass
independent checking: **357 calls, 327 OK and 30 unknown**, five server and two
client lifetimes, and six fresh voter drains. Unknown outcomes remain in both
complete histories. This is process-recovery evidence, not actual Chaos Mesh,
power-loss acceptance or a whole Rust implementation proof. No exact-candidate
Chaos campaign or full point/batch matrix runs for this held change.

## Failed first attempt and complete second attempt

The first attempt stops after 18 of 24 cohorts, before cohort 018 starts:
retention available space is 102,793,814,016 bytes, below the predeclared
103,079,215,104-byte (96-GiB) preflight floor. The outer wrapper exits 1,
restores CPU configuration and preserves all partial records. That attempt is
**incomplete and excluded from every published performance statistic**.

A read-only environment investigation finds no live references or external
hard links to the rebuildable dev cache. Root then runs `cargo clean --profile
dev` under the shared retained-build lock. This releases 8,172,728,320 available
bytes, preserves the release and evidence trees, and revalidates the original
control/candidate/client source and binary identities. The next screen starts
with 7,882,715,136 bytes above the unchanged retention floor. This cleanup is
cache reclamation, not a build or a test pass.

The second attempt reruns all 24 cohorts into a new directory. Only output paths
and available cache space differ; protocols, binaries, order, CPU placement,
storage guards and audit predicates remain unchanged. No partial cohort is
resumed or pooled. The second run exits 0; the independent 24-case audit and
statistics each pass on their first execution.

All **49,675,313 measured calls = issued = attempts = successes**, with zero
other measured outcomes or dropped slots. Across initialization, warmup,
measurement and verification, 49,973,369 calls succeed in 49,973,385 attempts;
the extra 16 attempts occur only during initial leader routing. The independent
audit accepts 80 exited lifetimes, 48 fresh drains/bindings, 2,347 role/source
checks, 4,678 resource samples and exact CPU/namespace restoration. It retains
636 files / 4,562,369,306 bytes. Six driver and 17 auditor contracts pass before
runtime; 12 separate smoke cohorts pass.

The [compact evidence bundle](raft-append-payload-v1/README.md) retains source,
release and recovery records; both timing attempts including the storage failure;
cache cleanup; frozen harnesses; all raw timing reports; independent acceptance;
and exact statistics. Its verifier checks byte integrity only. Original WALs,
executables and bulky host observations remain local under the inventories.

## Environment and next development step

The fixed v3 client `0be806d9` uses c1/c64 point read100/read50, 4,096 keys plus a
sentinel, 128-byte values, seed 71, 128 warmup calls, ten-second cohorts and a
1,500-ms deadline. Clients use CPUs 0-1, three voters or Redis share 2-5, and
helpers use 6-15,22-31. KV9 performs ordinary quorum/sync on volatile **tmpfs WAL**;
Redis is standalone with persistence and pipelining disabled. This shared-host
loopback result is not equal-durability, physical-disk, cross-host or sustained
capacity evidence. No builds, tests, profiling or faults overlap timing.
No hosted CI is dispatched.

The [confirmation-queue diagnostic](CONFIRMATION-QUEUE-RESULTS.md) remains the
useful causal starting point: sender/inbox residence grows under mixed traffic,
but low batch-channel admission does not measure time until its receiver polls,
HTTP/2 flush or remote delivery. Do not repeat the coarse diagnostic or try
another Append-size setting without a new reason.

Next, remove the source-visible redundant message-vector construction in
`GrpcTransport::drain`: the inbox already returns an owned bounded FIFO vector,
which is currently moved into another vector. Preserve partition filtering,
ordering, admission bounds and wakeups. Check default and testing-feature paths,
then use the same complete screen before claiming a gain. This is a proposed
implementation step, not a measured improvement. If it is insufficient, inspect
the existing peer-worker to request-body channel handoff and ownership before
changing scheduling; stream-progress timeout and route-generation cancellation
must remain effective. DPDK still needs cross-host/NIC evidence.

Fresh Safe ReadIndex, sealed groups, complete pump/apply/view fences, durable
acknowledgements and cancellation ownership remain mandatory. Full proof
composition, actual Chaos coverage and independent host-failure acceptance
remain open. Dynamic multi-Raft and automatic splits follow the read milestone.
No broader issue #9 checklist item is completed by this experiment.
