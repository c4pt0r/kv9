# Indexed receipts improve mixed traffic; pure GET remains a tradeoff

The missing read/mixed screen for `74d241161eadf2121128f6fd9101ead18e562765`
is complete. At c64, combined mixed throughput improves **2.775%**, and GET
mean improves **2.924%**, with lower GET p95/p99 in both repetitions. However,
pure GET throughput falls **0.411%** pooled and p95/p99 rise in both repetitions.
Keep CRC selected and retain indexed receipts as a mixed/write candidate.
This does not establish statistical significance or justify general promotion.

The [fresh selected-source profile](READ-PATH-CPU-PROFILE.md) motivated this
screen: 4.189% of mixed CPU samples fall in the exact executable's linear
receipt-search loop. The candidate combines indexed lookup and deque retention.
These results do not isolate their individual effects or explain the small
pure-read regression, whose normal request path does not search write receipts.

## Same-recording results

Rates pool two ten-second forward/reverse repetitions. Latency is whole-call
time; p99 entries are merged raw histogram bucket intervals, not averaged
percentiles. The mixed operation table keeps GET and PUT separate.

| Workload | Version | Calls/s | Mean us | p99 us |
| --- | --- | ---: | ---: | --- |
| c1 GET | CRC | 26,408 | 37.754 | 50.688-51.199 |
| c1 GET | Indexed | 26,404 | 37.762 | 50.176-50.687 |
| c1 GET | Redis | 174,356 | 5.658 | 7.360-7.423 |
| c64 GET | CRC | 346,078 | 184.806 | 352.256-356.351 |
| c64 GET | Indexed | 344,657 | 185.566 | 356.352-360.447 |
| c64 GET | Redis | 513,879 | 124.434 | 229.376-231.423 |
| c64 mixed combined | CRC | 172,071 | 371.799 | 614.400-622.591 |
| c64 mixed combined | Indexed | 176,847 | 361.754 | 589.824-598.015 |
| c64 mixed combined | Redis | 499,647 | 127.954 | 233.472-235.519 |

| c64 mixed operation | CRC mean us | Indexed mean us | CRC p99 us | Indexed p99 us |
| --- | ---: | ---: | --- | --- |
| GET | 383.691 | 372.473 | 622.592-630.783 | 606.208-614.399 |
| PUT | 359.929 | 351.059 | 598.016-606.207 | 573.440-581.631 |

At c1, mixed throughput improves 2.001% and GET mean improves 1.236%; both
mixed operation means and p99 improve in both repetitions. Pure c1 GET is
effectively unchanged in the pooled observation: -0.016% throughput and
+0.022% mean, with improved p99. This is not a proven no-regression bound.

| Workload | Repeat 0 QPS change | Repeat 1 QPS change | Pooled QPS change |
| --- | ---: | ---: | ---: |
| c1 mixed | +2.318% | +1.684% | +2.001% |
| c1 GET | +0.165% | -0.196% | -0.016% |
| c64 mixed | +2.799% | +2.752% | +2.775% |
| c64 GET | -0.211% | -0.610% | -0.411% |

The [full readout](indexed-read-mixed-v1/READOUT.md) and
[every repetition](indexed-read-mixed-v1/PER-REPEAT.md) retain separate means,
p50/p95/p99, GET/PUT rates and all outcome populations. No cohort was removed
or repeated to improve the result. Redis remains substantially faster: c64 GET
is about 1.49x the candidate's rate, and the candidate's c1 GET mean is about
6.67x Redis's mean.

## Protocol and complete accounting

The unchanged bounded protocol uses 24 timing cohorts plus 12 separate
two-second smoke cohorts: c1/c64, point read50/read100, CRC/indexed/Redis,
and two whole forward/reverse repetitions. It uses the fixed v3 client
`0be806d9`, 4,096 keys plus a sentinel, 128-byte values, seed 71, 128 warmup
calls, ten-second measurement, a 1,500-ms deadline and a ten-million-call cap.
The original CRC and candidate release binaries are reused after identity
checks; no source change or compilation is part of this screen.

All **49,825,632 measured calls = issued calls = attempts = successes**.
Refusals, unknown writes, read failures, client rejections and dropped slots
are zero. Across all phases, 50,123,688 calls succeed in 50,123,704 attempts;
the 16 additional attempts occur during initial leader routing, not measurement.
The first audit accepts 80 exited lifetimes, 48 fresh drains, 48 writer/listener
bindings, 2,344 role/source checks and 4,677 resource samples. It checks 640
retained files totaling 4,629,571,887 bytes. Three owned background containers'
configured/effective CPU masks and the original namespace identities are restored.

Clients use CPUs 0-1; voters or Redis share CPUs 2-5; helpers and owned
background containers use CPUs 6-15,22-31. This is shared-host loopback with
ordinary three-voter quorum/sync and volatile tmpfs WAL. Redis runs standalone
with persistence and pipelining disabled. The workloads have different
replication/durability guarantees. No build, tests, proofs, profile, fault
injection or actual audit overlaps timing. No hosted CI is dispatched.

Benchmark readback and sentinels are not a complete linearizability history.
This is a short point-read/mixed screen, not full batch, equal-durability,
disk, cross-host, sustained-capacity or host-failure acceptance.

## Correctness evidence and decision

The exact candidate already has [local source and ordinary recovery evidence](INDEXED-RECEIPT-PERFORMANCE.md):
711 workspace tests/doctests with 23 ignored, formatting, Clippy and complete
stream/unary leader-loss/restart histories containing 378 calls (347 OK,
31 unknown). The [representation proof](INDEXED-RECEIPT-PROOF.md) passes 13
parameterized TLA+/TLAPS theorems and 122 distinct obligations under explicit
container/search/locking contracts. Duplicate or reordered indexes restore
the original first-match lookup; full term/fence/verdict and eviction behavior
are preserved. These earlier gates were not rerun or enlarged here.

Fresh identity checks bind all 592 source files and the original two-stage
release's 11 first-observed compiled units. Server SHA-256 is
`3fc1c4c0c8f7b2c8ffead879b8228aef139aa2c443b647f38c7722a8e33668f0`.
The initial overly strict server-only reader and corrected stage-aware reader
remain retained. The six pure driver and 17 auditor contracts pass; the
source-reading driver contract is not rerun because root supplies the fresh
original-artifact readback. The benchmark driver and auditor retain their
per-cohort source, binary and fixture checks.

The [evidence bundle](indexed-read-mixed-v1/README.md) contains 192 exact files,
including all original reports, source/build bindings, resource coverage,
protocol, audit, statistics and terminal records. Raw WALs, runtime binaries
and unrelated host listings remain local. Its verifier checks byte integrity,
not runtime correctness.

Retain the mixed/write improvement while holding general promotion. The earlier
batch-write comparison remains inconclusive, and no exact indexed-source Chaos
Mesh runtime has run. The next bounded implementation should isolate indexed
lookup while retaining the original vector storage/eviction, separating the
search hypothesis from the deque change. Preserve the checked-order fallback,
full receipt payloads and unknown-outcome behavior; map the local proof and run
focused correctness before the same pure/mixed screen. A useful candidate still
needs full workload and actual Chaos Mesh acceptance. Fresh Safe ReadIndex,
sealed groups, durable Raft writes and full pump/apply/view fences are mandatory.
Dynamic multi-Raft and automatic range splits follow the read milestone.
