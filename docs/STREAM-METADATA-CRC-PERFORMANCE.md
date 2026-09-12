# Immutable stream metadata on CRC: no useful combined gain

Keep CRC `ca0002c7` selected. Candidate
[bb13e43](https://github.com/c4pt0r/kv9/commit/bb13e4313c313ca910969d1f01ef862619b57906)
combines the earlier immutable per-stream authorization representation with
the selected allocator, read workers and CRC implementation. The complete
24-cohort screen does not reproduce a useful gain: c64 pure GET is **+0.010%**
pooled with opposite repeat signs, while mixed throughput is **-0.453%** and
mixed GET mean is worse in both repeats. Hold this experiment and stop its
full-matrix/Chaos expansion. This is not a statistical significance claim.

## Same-run throughput and latency

Rates use total calls divided by total elapsed time over both ten-second
forward/reverse repetitions. Latencies cover complete client calls; p99 values
are merged raw histogram bucket intervals, not averaged percentiles.

| Metric | Selected CRC | Metadata reuse | Redis |
| --- | ---: | ---: | ---: |
| c1 GET calls/s | 26,478 | 26,495 | 174,125 |
| c1 GET mean us | 37.647 | 37.626 | 5.668 |
| c1 GET p99 us | 50.688-51.199 | 50.176-50.687 | 7.424-7.487 |
| c64 GET calls/s | 345,421 | 345,456 | 511,909 |
| c64 GET mean us | 185.155 | 185.136 | 124.916 |
| c64 GET p99 us | 352.256-356.351 | 356.352-360.447 | 229.376-231.423 |
| c64 mixed combined calls/s | 171,752 | 170,973 | 501,859 |
| c64 mixed GET mean us | 384.598 | 386.367 | 127.400 |
| c64 mixed GET p99 us | 622.592-630.783 | 622.592-630.783 | 233.472-235.519 |
| c64 mixed PUT/SET mean us | 360.412 | 362.036 | 127.384 |

| Workload | Repeat 0 QPS change | Repeat 1 QPS change | Pooled QPS change |
| --- | ---: | ---: | ---: |
| c1 GET | +0.138% | -0.014% | +0.062% |
| c64 GET | +0.151% | -0.129% | +0.010% |
| c1 mixed | +0.398% | -1.289% | -0.443% |
| c64 mixed | -0.261% | -0.646% | -0.453% |

Pooled mixed GET p99 hides a repeat-specific regression: in repeat 1 it rises
from 622.592-630.783 to 630.784-638.975 us; repeat 0 is unchanged. Pure c64 p99
is worse in repeat 0 and unchanged in repeat 1. Pure c1 p99 improves in repeat 0
and is unchanged in repeat 1. Both complete repetitions and separate GET/PUT
means, p50/p95/p99 and outcome populations remain in the
[readout](stream-metadata-crc-screen-v1/READOUT.md) and
[per-repeat table](stream-metadata-crc-screen-v1/PER-REPEAT.md).

The earlier system-allocator adapter's favorable result does not transfer to
this combination. These recordings cannot isolate which interaction explains
the difference. No cohort was dropped or repeated to improve the numbers.

## Exact-source recovery and measurement evidence

The [source checks and clean release](stream-metadata-crc-source-v1/README.md)
already pass 228 default and 238 experimental server tests/doctests, with one
ignored in each overlapping configuration, formatting and experimental server
Clippy. The six adapter Rust files and relevant consumer sections match the
original implementation. Every frame still authenticates freshly. Raft,
storage, metadata and main runtime sources are unchanged from selected CRC.
The source correspondence is not a whole-implementation proof.

Fresh readback binds all 596 source files and the original default-feature
release. New ordinary stream/unary leader-loss and original-directory restart
histories independently pass: **370 calls, 340 OK and 30 unknown**. Each
transport contributes 185 calls. All unknowns remain in the histories;
all five server/two client lifetimes exit, and both cases have three fresh
voter drains. This is ordinary process recovery, not actual Chaos Mesh or
power-loss acceptance.

The protocol retains 24 timing cohorts and 12 separate two-second smoke
cohorts: c1/c64, point read50/read100, CRC/candidate/Redis, fixed v3 client
`0be806d9`, 4,096 keys plus a sentinel, 128-byte values, seed 71, 128 warmup
calls, a 1,500-ms deadline and ten-million-call cap. All
**49,706,706 measured calls = issued = attempts = successes**. Other measured
outcomes and dropped slots are zero. Across all client phases, 50,004,762
calls succeed in 50,004,778 attempts; 16 extra attempts occur only during
initial leader routing.

The independent audit accepts 80 exited lifetimes, 48 fresh drains,
48 voter/listener bindings, 2,348 role/source checks and 4,678 resource samples.
All three owned containers' configured/effective CPU settings and namespace
identities are restored. The audit retains 636 files / 4,552,528,169 bytes;
the compact publication has a smaller, explicitly documented selection.
Six driver contracts pass initially; the first auditor contract run catches
one stale argument-file hash. The exact one-literal correction then passes all
17 auditor contracts. The failure is preserved; no acceptance predicate changes
and no runtime failure or retry follows from it.

The [compact evidence](stream-metadata-crc-screen-v1/README.md) includes all
24 original timing reports and raw histograms, frozen protocols, failed and
corrected preparation, independent audit, statistics, terminal records and
both complete ordinary recovery histories. Its verifier checks byte integrity
only. Raw WALs, binaries and bulky host observations remain local with original
inventory bindings.

Clients use CPUs 0-1, voters or Redis share 2-5, and helpers use 6-15,22-31.
This is shared-host loopback with ordinary three-voter quorum/sync calls on
volatile **tmpfs WAL**. Redis is standalone with persistence/pipelining disabled.
It is not equal-durability, disk, cross-host, sustained capacity or full batch
acceptance. No builds, tests, audits, profiling or faults overlap timing.
No hosted CI or exact-bb13 Chaos campaign runs.

## Next work: split the confirmation interval

The [existing CRC lifecycle diagnostic](READ-LIFECYCLE-CRC-CREDIT.md) already
records a c1 sampled read-barrier mean of 24.285 us, including 20.887 us from
submission to observed confirmation. C64 mixed records 168.753 us in that
interval and 56.070 us in completion notification. These are sampled successful
reads over the drained whole-client envelope, not a measurement-only latency
decomposition. Repeating that same coarse diagnostic is unnecessary.

Current source uses one persistent BatchRaft stream per peer, bounded outbound
queues, immediate coalescing of already queued messages, and a bounded receiver
inbox that wakes the Raft owner. Public requests already use bounded parallel
JoinSet tasks; returning to a single FuturesUnordered would undo an earlier
measured scheduling improvement. Neither observation identifies a new gain.

The [next diagnostic plan](READ-CONFIRMATION-DIAGNOSTIC-PLAN.md) splits the currently merged confirmation path into
sender queue residence, stream backpressure, receiver inbox residence and owner
processing/confirmation observation. Use bounded sampling and explicit local
clock boundaries; separate leader/follower and message kinds. If samples cannot
be joined to one exact read context, report them as queue populations and do
not sum their means into request latency. Preserve source projection to the
selected protocol, run focused checks, then collect fresh c1 and c64 mixed
observations without concurrent benchmarks or fault work. Choose the next
scheduling/transport change from those results.

Fresh Safe ReadIndex, sealed groups, complete pump/apply/view fences, durable
write acknowledgements, deadlines and reservation ownership remain mandatory.
Full core-proof composition, actual Chaos coverage and independent host-failure
acceptance remain open. Dynamic multi-Raft and automatic splits follow the read
milestone. This checkpoint closes no broader issue #9 work package.
