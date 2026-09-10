# Async-write runtime: bounded GET and PUT CPU profiles

The exact `a00e39f` profiles identify RPC framing/serialization/buffers and
generic allocation/copy/comparison as substantial CPU populations. GET assigns
27.08% of selected leaf samples to RPC and 20.67% to allocation; PUT assigns
19.40% and 23.60%, respectively. GET scheduler/generic synchronization leaves
account for another 12.62%. These observations support comparing RPC transports
and their allocation/dispatch paths while preserving Raft consistency.

These are **two instrumented CPU diagnostics on one shared host with volatile
tmpfs voter data**. They establish neither a throughput improvement nor disk,
power-loss or cross-host acceptance. On-CPU sample shares omit blocked time and
are not fractions of end-to-end request latency. The earlier unchanged-protocol
[async-write performance bracket](11cae97-a00e39f-c64-tmpfs-diagnostic.md) remains
separate evidence.

## Exact runtime and protocol

| Input | Identity |
| --- | --- |
| Server revision | `a00e39f9f8da7bb381df19afd2eed63e8a1f1b75` |
| Default-feature release server SHA-256 | `b50da6f48e4ed88416892a3490653afb979a410a8c8c935abfc5d1e6008ccfbc` |
| Retained client revision | `892b2a178450309859113c942f1738c070130eb5` |
| Retained client SHA-256 | `b47d5f408cd9a36ce7a8117917eb0fba530b0616125b4fff38980a255ddd12dc` |
| Executing perf binary | `/usr/lib/linux-tools/6.17.0-19-generic/perf` |
| Executing perf SHA-256 | `a9dde21212f2b4bfa54d66f9c40b4121556367940f0f7c6f7f64c7885e07ff45` |

All 415 server source inputs and 406 separately identified client source inputs
match their respective Git revisions and are retained. The server build has
features `[]`. The client is the unchanged Ready release artifact; its manifest
has not been relabeled as an async-write build. This report also does not apply
the recordings to later status, receipt-FIFO, CRC or RPC candidates.

Each cohort uses 64 persistent workers, 64 hot keys, 23-byte keys, 128-byte
values, 32 warmup operations and a five-second measurement window. GET100 and
PUT100 use separate fixtures. The one-million-operation cap, 1,500-ms deadline,
six maximum attempts, 5-ms routing backoff and 64-call client limit are unchanged.
Only explicit NotLeader refusals may be retried; unknown writes are not retried.
Normal three-voter Raft quorum and sync calls remain enabled. Retaining a disk
copy of tmpfs data after execution does not make the runtime durable.

The affinity observer verifies clients on CPUs 0–1, all three voters on CPUs
2–5 and the recorder on CPUs 6–31. These are logical masks on a shared physical
host with background processes, not exclusive physical-core isolation. Perf
attaches only to the three owned voter PIDs. It uses user-and-kernel `cpu-clock`
samples at 199 Hz, monotonic timestamps and DWARF call graphs with a 16 KiB stack
window. Each recording is bounded to 20 seconds and 128 MiB. Analysis selects
the client's measurement interval, excluding one millisecond from each edge;
setup, warmup and drain samples are excluded. The measured clock-anchor offset
spreads are 100 ns for GET and 230 ns for PUT.

## Outcomes and coverage

| Observation | GET | PUT |
| --- | ---: | ---: |
| Successful measured operations | 681,563 | 380,314 |
| Measured failures, unknown outcomes or extra attempts | 0 | 0 |
| Recorded samples | 2,988 | 3,304 |
| Selected measurement samples | 2,932 | 3,242 |
| Raw recording bytes | 49,924,384 | 59,712,048 |
| Lost samples | 0 | 0 |

Both workloads stop on duration, below the operation cap. The original complete
workload, outcome/histogram and resource validators pass unchanged. Fresh perf
decoding is byte-for-byte identical to the original rendering; the recorder cap
is not reached. Aggregate samples and the leader cover all 32 equal measurement
bins in both runs. PUT also covers all 32 bins on every voter.

GET follower coverage is sparse: voter 1 has 275 samples with its first sample
255.369 ms after the selected interval begins; voter 3 has 262 with its first
at 289.754 ms. Both lack bin 0. Leader 2 has 2,395 selected samples and complete
bin coverage. The report therefore makes no all-voter edge-completeness claim
for GET. Absence of an on-CPU sample does not measure whether a follower was
blocked or idle during that gap.

All three voters, the client and the observed recorder lifetime exited in each
run; recorder launcher PIDs are also absent. Public admission, async read/group
and async apply queue/in-flight occupancy are zero at both boundaries, with
stopped=false and unchanged limits. The leader's async-apply lifetime peak is
1 in the GET fixture, including setup writes, and 64 in the PUT fixture; both
are below the 128-reservation limit. These are boundary/lifetime observations,
not per-response or measurement-only occupancy counts.

## Sampled CPU populations

These are exclusive leaf-symbol categories under the retained analyzer rules.
Generic allocation, locks and copying remain separate from specific callers.

| Leaf population | GET samples | GET share | PUT samples | PUT share |
| --- | ---: | ---: | ---: | ---: |
| RPC framing, serialization and buffers | 794 | 27.08% | 629 | 19.40% |
| Generic allocation, copying and comparison | 606 | 20.67% | 765 | 23.60% |
| Scheduler and generic synchronization | 370 | 12.62% | 239 | 7.37% |
| Kernel network symbols | 158 | 5.39% | 149 | 4.60% |
| Kernel generic spinlocks | 168 | 5.73% | 177 | 5.46% |
| Kernel futex/scheduler symbols | 84 | 2.86% | 77 | 2.38% |
| Engine symbols | 20 | 0.68% | 214 | 6.60% |
| Raft driver/consensus symbols | 29 | 0.99% | 172 | 5.31% |
| Unknown or unsymbolized leaves | 103 | 3.51% | 58 | 1.79% |

Other categories remain in the complete summaries. Optimized release unwinding
recovers only one symbolized frame in 1,217 GET samples and 1,437 PUT samples;
async boundaries lose additional caller context. An absent symbol does not
establish zero cost. Recovered inclusive stack populations overlap each other
and these leaf categories, so they must not be added as independent costs.

The exact executable's `WalSegment::append` symbol starts at `0x84ccf0`.
Independent `objdump` replay matches the retained disassembly byte-for-byte.
Its inlined reflected CRC loop uses polynomial `0xedb88320`; the half-open
relative instruction range `[0x55b, 0x5eb)` contains **182 PUT leaf samples,
5.61% of the selected PUT population**. The whole append symbol contains 186
samples. CRC samples are a subset of engine samples, not an extra category.
No instruction offsets from an earlier build are reused, and this attribution
does not predict the end-to-end effect of changing the CRC implementation.

The retained hotspot table also identifies 132 GET and 27 PUT contended-mutex
leaf samples with a recovered frame whose symbol starts with `h2::`. This is a
specific partial-stack selection, overlapping other categories. It does not
assign the remaining locks to Raft or prove that HTTP/2 causes all of their cost.

## Next experiments

1. Compare the current unary gRPC path with streaming gRPC as a control and a
   tarpc/TCP prototype. The tarpc scaffold has no completed measurement or
   performance result. Hold the workload, admission bounds, payloads, deadlines,
   retry rules and success/unknown accounting constant. Separate framing,
   serialization, allocation and dispatch changes where the experiment permits.
2. Preserve quorum-confirmed reads and exact committed/applied write receipts.
   A transport benchmark alone cannot accept a database integration: retain
   bounded queues, cancellation ownership, backpressure and peer identity, then
   check protocol refinement, local semantic controls and actual Chaos Mesh
   histories before promotion. No RPC choice may weaken Raft consistency.
3. Consider DPDK or another kernel-bypass experiment only when measurements
   establish a network bottleneck worth addressing. These loopback CPU profiles
   neither measure NIC limits nor establish that bypassing the kernel is the
   dominant opportunity. RPC experiments have priority.

The table-CRC prototype `89b9755` is an unselected, separate experiment with
three focused passing tests. It has no measured performance result or completed
integration acceptance. The verified CRC hotspot remains useful evidence, but
RPC comparisons supersede that prototype in the current work order.

## Retained evidence and unsuccessful attempts

| Evidence | Local path |
| --- | --- |
| Accepted GET recording and original artifacts | `/tmp/kv9-async-write-get-profile-attempt3` |
| Accepted PUT recording and original artifacts | `/tmp/kv9-async-write-put-profile-attempt1` |
| GET attempt 1, retained failure | `/tmp/kv9-async-write-get-profile-attempt1` |
| GET attempt 2, retained failure | `/tmp/kv9-async-write-get-profile-attempt2` |
| Original strict coverage audit failure | `/tmp/kv9-async-write-profile-independent-audit-v1` |
| Accepted scoped audit and independent renders | `/tmp/kv9-async-write-profile-independent-audit-v2` |
| Original hotspot table and disassembly | `/tmp/kv9-async-write-profile-analysis` |
| Independent hotspot replay and source copies | `/tmp/kv9-async-write-profile-report-evidence-v2` |

GET attempt 1 requested eight seconds and stopped at the operation cap; the
unchanged trial validator rejected its truncated measurement. Attempt 2 requested
two million operations and was refused by the unchanged client configuration
validator. Neither is accepted or relabeled. Attempt 3 reduces the requested
window to five seconds while retaining the one-million cap and succeeds.

The first independent audit inherited a dense per-voter edge/32-bin condition
from an earlier PUT diagnostic and rejected the sparse GET followers. Its source,
log and failure remain. The accepted v2 audit requires full aggregate and leader
coverage, recording containment and explicit individual-voter coverage. It does
not discard missing follower bins. During report closeout, the first hotspot
replay used any `h2::` substring and disagreed with the table's narrower prefix
rule. That failed script/log is retained; the second replay states the exact
prefix rule and reproduces the original counts. Neither audit correction
reruns a workload or changes any runtime, shared validator or recorded sample.

The unprivileged PID-scoped perf probe was denied; the successful probe and
recording used existing noninteractive sudo access for the owned voter PIDs.
No sysctls were changed and no unrelated process was sampled. Retained tmpfs
data comprises 33 files / 297,295 bytes for GET and 42 files / 437,660,427 bytes
for PUT; every file was rehashed and both original scratch directories removed.

The v2 audit is `/tmp/kv9-async-write-profile-independent-audit-v2/audit.json`,
SHA-256 `dbaabe043659d0f9bb3e99339206184f2175cf86e048b42201f9f2c5e220c203`.
The independent hotspot audit is
`/tmp/kv9-async-write-profile-report-evidence-v2/hotspot-audit.json`, SHA-256
`4f29d00c762c8a8d8b6d2edad40f525a1c26822063d47b8d2469a5135a8bbb05`.

The frozen inventory `/tmp/kv9-async-write-profile-inventory.json` contains
**2,083 files / 697,859,974 bytes**, SHA-256
`7481df8e0fa786e92847cb8c3063d21250aadb1fc285ba2ef989e78ebd8559d8`.
Every original was reopened and hash-verified; the second pass is recorded in
`/tmp/kv9-async-write-profile-inventory-verification.json`. It includes raw
recordings, renders, selected samples, outcomes/resources, exact source/build
inputs, preparation/observer versions and all failures above. This report is
excluded to avoid a circular hash. Raw perf SHA-256 values are:

- GET: `5afa2fad7862f34af6021937bb46d7492ee0b17ad9df1c5b804ffae66e0be89e`.
- PUT: `4f086f2bd7fbd24487ea2fc65572299e06008a78fe12f33fe37324f8b4a483c7`.

This closeout adds documentation only. It dispatches no hosted CI and makes no
runtime promotion, throughput, full-history correctness or durability claim.
