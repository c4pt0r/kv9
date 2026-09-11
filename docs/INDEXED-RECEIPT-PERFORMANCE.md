# Indexed apply receipts: point-write improvement, batch qualification open

Candidate `74d241161eadf2121128f6fd9101ead18e562765` improves c64 point PUT
throughput **3.812%**, with lower mean and p99 latency in both repetitions.
BatchPut(64) remains inconclusive: throughput changes **+2.264% / -4.361%**
across the two repetitions, pooling to **-1.064%**, with a worse second-run
tail. Keep the candidate experimental. The selected development source remains
`ca0002c7f8e9ee6f595efcc9f4151085ccce87cb` on master, using tonic streaming gRPC.
These observations support neither general promotion nor discarding the
repeatable point-write benefit. They do not establish the cause of batch
variation or a statistically significant population-wide effect.

The change replaces full-suffix receipt eviction with a bounded deque and
linear lookup with binary search only while a checked strictly increasing
index certificate holds. Duplicate or reordered indexes permanently restore
the original first-match lookup. Complete receipt payloads, term/fence checks,
eviction uncertainty, Raft confirmation and the dual-WAL contract are retained.
See the [source and representation argument](https://github.com/c4pt0r/kv9/blob/74d241161eadf2121128f6fd9101ead18e562765/docs/INDEXED-APPLY-RECEIPTS.md).

## Latest same-recording writes

Rates pool both ten-second forward/reverse cohorts; mean latency weights all
completed calls. Percentiles merge original raw histogram populations and are
reported as bucket intervals. Batch latency covers the entire 64-key call.
Values are 128 bytes.

### Concurrency 64

| Workload | Version | Calls/s | Keys/s | Mean us | p95 us | p99 us |
| --- | --- | ---: | ---: | ---: | --- | --- |
| PUT/SET | Selected CRC | 123,846 | 123,846 | 516.636 | 704.512-712.703 | 835.584-843.775 |
| PUT/SET | Indexed candidate | 128,566 | 128,566 | 497.664 | 679.936-688.127 | 802.816-811.007 |
| PUT/SET | Redis | 494,304 | 494,304 | 129.324 | 165.888-167.935 | 233.472-235.519 |
| BatchPut/MSET(64) | Selected CRC | 13,495 | 863,705 | 4,741.068 | 6,815.744-6,881.279 | 8,912.896-9,043.967 |
| BatchPut/MSET(64) | Indexed candidate | 13,352 | 854,511 | 4,792.237 | 7,077.888-7,143.423 | 8,912.896-9,043.967 |
| BatchPut/MSET(64) | Redis | 94,549 | 6,051,110 | 672.101 | 958.464-966.655 | 1,015.808-1,023.999 |

Redis's observed throughput is **3.84x** the candidate for point writes and
**7.08x** for batch writes. The corresponding selected-CRC gaps are **3.99x**
and **7.01x**. This is still far from Redis-class write performance.

### Concurrency 1

| Workload | Version | Calls/s | Keys/s | Mean us | p95 us | p99 us |
| --- | --- | ---: | ---: | ---: | --- | --- |
| PUT/SET | Selected CRC | 16,574 | 16,574 | 60.234 | 71.680-72.703 | 87.040-88.063 |
| PUT/SET | Indexed candidate | 16,779 | 16,779 | 59.496 | 70.656-71.679 | 86.016-87.039 |
| PUT/SET | Redis | 170,402 | 170,402 | 5.749 | 6.016-6.079 | 8.064-8.127 |
| BatchPut/MSET(64) | Selected CRC | 5,392 | 345,077 | 185.359 | 210.944-212.991 | 262.144-266.239 |
| BatchPut/MSET(64) | Indexed candidate | 5,437 | 347,973 | 183.814 | 206.848-208.895 | 262.144-266.239 |
| BatchPut/MSET(64) | Redis | 42,194 | 2,700,431 | 22.562 | 23.552-23.807 | 32.768-33.279 |

The candidate's isolated point-write mean remains **10.35x** Redis's. Its
c1 point/batch throughput improves **1.236% / 0.839%** in the pooled recording;
the second c1 point p99 is slightly worse. Higher concurrency does not resolve
the isolated-request turnaround gap.

### Both repetitions remain visible

| Concurrency | Workload | Repeat 0 QPS change | Repeat 1 QPS change | Pooled QPS change | Pooled mean change |
| ---: | --- | ---: | ---: | ---: | ---: |
| 1 | PUT | +2.052% | +0.425% | +1.236% | -1.224% |
| 1 | BatchPut(64) | +0.333% | +1.348% | +0.839% | -0.833% |
| 64 | PUT | +4.513% | +3.123% | +3.812% | -3.672% |
| 64 | BatchPut(64) | +2.264% | -4.361% | -1.064% | +1.079% |

For c64 batch writes, repeat 0 p99 improves from **8.782-8.913 ms** to
**8.323-8.389 ms**; repeat 1 worsens from **9.044-9.175 ms** to
**9.568-9.699 ms**. The pooled equal p99 bucket must not hide that difference.
No repetition was discarded or rerun. The unchanged baseline's variation
across different experiments is not a new source change.

### Reads and mixed workloads retain their earlier recording

This screen does not measure reads or mixed traffic. The [previous complete
matrix](OWNED-BUFFER-PERFORMANCE.md) remains the latest selected-source read
evidence: c64 GET **346,056/s**, mean **184.812 us**, p99
**352.256-356.351 us**, versus Redis **509,859/s**, **125.410 us** and
**231.424-233.471 us**. BatchGet(64) reaches **2,267,868 keys/s** versus Redis
**5,925,161 keys/s**. At c1, GET means remain **37.735 us / 5.695 us**.
These numbers do not establish the indexed candidate's read behavior.

## Protocol and complete accounting

The frozen write-only protocol covers c1/c64, PUT/BatchPut(64), CRC/indexed/Redis
and two whole forward/reverse repetitions: **24 ten-second timed cohorts**,
preceded by **12 independent two-second smoke cohorts**. It uses 4,096 keys
plus a sentinel, 128 warmup calls, seed 71, a 1,500-ms deadline, a
ten-million-call cap and closed-loop clients. Complete-cohort elapsed time
includes cutoff completions. The inherited fixture public byte limit is
16 MiB; the production default remains 64 MiB.

Clients use CPUs 0-1; all three KV9 voters, or Redis, share CPUs 2-5. Helpers
and three owned background containers use CPUs 6-15,22-31 during timing, with
original settings restored afterward. Other host services remain unconstrained.
No build, test, model checker, profile, fault injection or actual audit overlaps
the timed recording.

KV9 retains three-voter quorum, sync calls and commit/apply-before-success,
with volatile tmpfs WAL. Redis 7.0.15 runs standalone with save/AOF and
pipelining disabled and one I/O thread. This compares memory execution under
different replication/durability semantics. It is not equal-durability,
cross-host or sustained-capacity acceptance. Redis batch clients also approach
their two-core budget; their measured rate is not an intrinsic Redis ceiling.

Smoke accepts **1,882,289 measured calls**. Timing and the first independent
audit accept **22,498,455 measured calls / 242,280,129 input keys**. Every
measured call succeeds on one observed attempt. Refusals, unknown writes,
read failures, client rejections and dropped slots are zero. Auditing verifies
**80 exited lifetimes, 48 fresh drains, 48 writer/listener bindings**, 2,344
role/source checks and 4,643 resource samples, with owned-container restoration
and historical namespace UIDs preserved. Original retention contains 2,124
files / 54,717,627,908 bytes; the audit input inventory has 2,941 entries.

The [original statistical readout](../scripts/redis-reference/indexed-receipt-screen-v1/statistics/READOUT.md)
contains every pooled row and paired repetition. The [screen index](../scripts/redis-reference/indexed-receipt-screen-v1/index.json)
binds **165 copies / 35,560,784 decoded bytes / 5,511,836 stored bytes**, including
all 24 original reports and 24 resource-coverage records, frozen preparation,
source/build bindings, audit and terminal/statistical records. Gzip reports
retain both encoded and decoded hashes. Raw WALs, binaries and unrelated host
process listings remain local; this compact publication is not a complete
runtime archive. Readback, sentinels and nonce bounds do not substitute for a
complete issued-operation history or prove benchmark linearizability.

## Exact-source process recovery and proof scope

The candidate's own original default-feature release binds all **592 source
files**, original server/workload Cargo records and copied runtime binaries.
Runtime engine/Raft/server libraries and binaries use opt-level 3; Cargo
custom-build artifacts normally use opt-level 0. Server SHA-256 is
`3fc1c4c0c8f7b2c8ffead879b8228aef139aa2c443b647f38c7722a8e33668f0`;
manifest SHA-256 is
`406c397f56e8d7f177aa540dbddbdda21ba9d0d9f159b0cb83694acfc92275f6`.

The separate process fixture and unchanged independent auditor accept both
complete atomic histories: **378 calls, 347 OK and 31 unknown**. Streaming has
194 calls (177 OK / 17 unknown); explicit unary has 184 (170 OK / 14 unknown).
Both cover overlapping point/batch operations, leader SIGKILL and restart from
the original directories. Two client and five server lifetimes exit; both
cases obtain fresh three-voter drains. Unknowns remain in the histories and
witnesses. Source, binaries and original inputs remain bound.

The [process index](../scripts/redis-reference/indexed-receipt-process-v1/index.json)
retains **63 exact copies / 2,237,047 bytes**, including original build,
invocation, terminal, audit and complete-history records. The first ancillary
build reader incorrectly required opt-level 3 for custom-build artifacts;
its failure and corrected classification remain retained. A publication-only
listing schema failure is also retained. No release, process, timing, audit
or statistics was rerun to remove a failed workload.

This is one-host ordinary-WAL process recovery. **No indexed-candidate Chaos
runtime has run.** Earlier candidates' Chaos results do not transfer to this
source. Power-loss, storage-fault and whole-system refinement claims remain
open. Local source checks pass two focused regressions, 711 workspace
tests/doctests (23 ignored), Clippy and formatting after the retained test
fixture type correction; [source evidence](../scripts/redis-reference/indexed-receipt-source-v1/index.json)
is unchanged.

The [parameterized TLA+/TLAPS representation proof](INDEXED-RECEIPT-PROOF.md)
now passes 13 theorems, 122 fresh obligations and all controlled checks. Its
scope is the ring API under standard container/search and caller-locking
contracts, not whole-Rust/Raft composition. The measured source remains frozen.

## Next development step

Keep CRC selected and the indexed candidate available. Obtain accepted batch
CPU attribution on the selected source, then target the dominant removable
work while retaining quorum, commit/apply acknowledgements, fences and the
dual-WAL recovery contract. The [new post-CRC diagnostic](POST-CRC-CPU-PROFILE.md)
passes unchanged sample coverage checks and identifies engine CRC at 18.079%
of selected batch CPU samples, versus receipt lookup at 0.870%. The previous
rejected prefix recording remains retained. Neither diagnostic supplies
replacement QPS evidence; the next CRC prototype remains unselected.

The candidate still needs broader read/mixed and exact-source Chaos acceptance
before general selection. Redis throughput and c1/loaded-tail parity remain
open; dynamic multi-Raft and automatic range splits follow that milestone.
All CI for this phase is local. No hosted workflow was dispatched.
