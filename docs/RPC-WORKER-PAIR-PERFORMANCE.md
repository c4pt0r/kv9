# Two RPC workers: measured throughput, latency and memory improvement

The isolated `5ee897a2f58c57bdf17ea1757c96224adf0f0dbb` candidate improves GET
throughput **4.795% / 5.381%** against unchanged event8 `917243f`, reaching
**344,412–344,790 calls/s**. Mean whole-call latency falls **4.574% / 5.110%**
to **185.490–185.700 us**. Point p99 improves in both repetitions.

BatchGet(1) reaches **335,694–337,822 calls/s**, gaining **5.069% / 5.434%**;
mean latency falls **4.827% / 5.156%** to **189.288–190.487 us**. Batch p99
improves in repetition one and remains in the same bucket in repetition two.
Pooled point/batch gains are **5.087% / 5.252%**. These are calls/s, equal to
items/s only for batch size one. Same-run Redis GET is **497,503–505,913/s**,
about **1.44–1.47x** the candidate's paired throughput. Parity remains open.

The first frozen independent audit accepts the entire matrix. Broader local
workspace checks and [exact-source Chaos acceptance](RPC-WORKER-PAIR-ACCEPTANCE.md)
pass. Select this as an isolated read-performance increment. Master/default
promotion, sustained/write/mixed behavior and other CPU budgets remain open.

## Exact change and safety scope

The only executable delta selects two async workers on the existing
multi-threaded Tokio runtime. Event interval eight, adaptive global scheduling,
blocking pool, all drivers and the dedicated Raft owner remain unchanged.
The unsuccessful peer-enqueue and one-worker changes are excluded.

The [source contract](https://github.com/c4pt0r/kv9/blob/5ee897a2f58c57bdf17ea1757c96224adf0f0dbb/docs/RPC-WORKER-PAIR.md)
provides a conditional exact-delta safety argument: worker selection changes
interleaving, while application guards and the base's arbitrary-interleaving
invariants remain premises. Quorum, successful pump, applied-index coverage,
view/epoch checks and write acknowledgement conditions are unchanged. This does
not close historical machine-checked composition or executable-refinement gates.

Two workers retain useful parallelism while reducing the competing async-worker
population under this shared CPU budget. This is a measured configuration choice,
not proof of a universally optimal count or a pure causal decomposition.
Dynamic-member authentication can synchronously access metadata; decoding and
resident copies also occupy workers. The initial-voter GET fixture does not
cover that authentication path or establish progress under stalled callbacks.

## Matched five-second results

The inherited protocol uses 64 closed-loop workers, 128-byte values, 4,096 keys
plus sentinel, batch size one, 128 warmup calls and 1,500-ms per-call deadlines.
Two repetitions reverse six-arm order; the fixed native03 and Redis b8 client
binaries remain unchanged. Clients use CPUs 0–1; all three voters share 2–5.
No build, test, fault, profiling or audit work overlaps timing. Owned container
masks are restored; unrelated host services remain unconstrained.

KV9 uses three ordinary Raft voters with volatile tmpfs WAL and normal sync
calls. Standalone Redis has save/AOF disabled. Replication and durability are
not equivalent. These short pure-read runs do not prove sustained capacity,
write/mixed performance, larger-batch behavior or cross-host scaling. Quantiles
are histogram bucket intervals, not exact values or averaged percentiles.

| Cohort | Calls/s | Mean us | p99 bucket us |
| --- | ---: | ---: | --- |
| 000-old-point-p00064 | 328,652.9 | 194.601 | 364.544–368.639 |
| 001-old-batch1-p00064 | 319,498.3 | 200.149 | 372.736–376.831 |
| 002-new-point-p00064 | 344,411.8 | 185.700 | 356.352–360.447 |
| 003-new-batch1-p00064 | 335,693.9 | 190.487 | 364.544–368.639 |
| 004-redis-mget1-p00064 | 487,197.9 | 131.236 | 233.472–235.519 |
| 005-redis-get1-p00064 | 505,913.2 | 126.391 | 233.472–235.519 |
| 006-redis-get1-p10064 | 497,503.4 | 128.528 | 233.472–235.519 |
| 007-redis-mget1-p10064 | 502,433.9 | 127.264 | 233.472–235.519 |
| 008-new-batch1-p10064 | 337,821.7 | 189.288 | 368.640–372.735 |
| 009-new-point-p10064 | 344,789.8 | 185.490 | 360.448–364.543 |
| 010-old-batch1-p10064 | 320,409.6 | 199.577 | 368.640–372.735 |
| 011-old-point-p10064 | 327,183.9 | 195.479 | 364.544–368.639 |

Summed mean voter RSS falls from **55.97/55.92 to 51.68/51.90 MiB** for GET,
and **56.58/55.69 to 51.74/51.89 MiB** for BatchGet(1), approximately
**3.8–4.8 MiB less**. This is sampled RSS over the small working set; process
high-water marks include setup and are not interval-only peaks. Long-running
memory behavior remains unmeasured.

## Remaining bottleneck and next measurements

This screen establishes continued sensitivity to execution scheduling, with
about a 5% gain under the fixed CPU allocation. It does not establish a fixed
throughput ceiling or a generally optimal worker count. The separate
[one-worker experiment](RPC-WORKER-SCREENING.md) loses roughly 35% throughput;
the [peer enqueue experiment](PEER-BATCH-ENQUEUE-SCREENING.md) gains less than
0.4% for GET and worsens one p99 repetition. Both remain rejected.

The earlier [lifecycle diagnostic](READ-LIFECYCLE-WAIT-RESULTS.md) found
confirmation and receiver scheduling dominant in its sampled barrier, with
on-CPU work spread across scheduling, RPC/buffers and allocation/copying.
That diagnostic predates event8 and two workers. Its 82/38-us stage means
and CPU percentages are hypotheses for the current source, not its measured
cost decomposition. No NIC/kernel ceiling or DPDK benefit is established.

The [refreshed two-worker trace](RPC-PAIR-READ-LIFECYCLE.md) now passes its
original CPU/fixture checks and independent lifecycle readback. Confirmation
remains about 67 us and notification about 41 us in that sampled barrier.
Next test global-queue polling on this two-worker control as a separate
completion-scheduling interaction experiment. Change one mechanism while
retaining persistent peer streaming, exact group identity, fresh quorum,
successful-pump and applied-index fences. Do not infer gains by adding earlier
percentages or extending these short runs to sustained capacity.

Before general promotion, check sustained point traffic, writes/mixed loads,
larger batches and bounded slow dynamic-auth/storage callbacks. The shared
executor makes this pure-read improvement insufficient to establish performance
or progress for those workloads. Redis-class RawKV and automatic splits remain
separate unfinished product gates.

## Validation and artifacts

Focused checks pass 68 peer/read/batch/stream tests, formatting and Clippy.
Full default and experimental-RPC workspace checks pass 707 and 717 tests,
respectively, with 23 existing ignored tests in each overlapping configuration.
Workspace all-target experimental Clippy passes with warnings denied. The exact
release source remains unchanged after those checks.
Actual production-runtime stream/unary leader-kill/original-directory restart
histories independently pass **367 calls: 338 OK, 29 unknown**, with all seven
processes exited and no replay after uncertain writes. The six-arm correctness
smoke passes **9,299,019 calls** before timing.

The exact-source eleven-window Chaos campaign passes once with 2,114 complete
history calls (1,816 OK, 232 refused, 66 unknown), 183 new same-leaf drops,
fresh empty drains and complete owned cleanup. The unchanged independent
history/effect audit, leaf readback and source/build verification pass. Its
one-host volatile-tmpfs client-link/quorum scope is separate from performance
and the remaining storage/cross-host qualification.

The sole matched runtime (session81302) and first frozen independent audit pass
**23,258,382 successful measured calls**, including 13,292,722 native calls,
with no other outcomes or dropped slots. Acceptance checks 40 exited lifetimes,
24 drains, 24 writer/listener bindings, 1,171 resource observations, 2,326 source
checks and 264 retained files / 44,378,703 bytes. Original container masks and
historical namespace maps are preserved. Source, default-feature/O3 builds,
clients, exact datasets and inputs are bound and retained.

- Source/release: `/tmp/kv9-rpc-worker-pair`, `/tmp/kv9-rpc-pair-release-first`.
- Raw matrix: `/tmp/kv9-rpc-pair-matched-diagnostic-attempt1/cohorts`.
- Frozen preparation/audit: `/tmp/kv9-rpc-pair-comparison-preparation`.
- Process audit: `/tmp/kv9-rpc-pair-process-independent-first`.
- Workspace checks: `/tmp/kv9-rpc-pair-workspace-first`.

| Artifact | SHA-256 |
| --- | --- |
| Candidate server | `0d5ffa081482945d213b88aef12222afab44a44ed03bf46cf27bd292db7b1711` |
| Build manifest | `1bc2c98587f344f0066033450018d0a4c81b4f16a5c25a691b18136659ebe417` |
| Matched audit | `375a7579265e2bca31b0afd1745572a9f679f958123e2c233a5b79904c857b3b` |
| Independent statistics | `bd85b3f3f4c4950d6ee70ab8ed064404187589615a75303175d60f98ac53eccb` |
| Root statistics | `69af292d87e55bc5bf91c7f5f1a3b543609053f41e063d3f1bf6c9ffa27932e7` |
| Process audit | `e803d6f16c110354cbc224900d96ed7f5a6d6155dde47af765374d812a07eb00` |

No hosted CI was dispatched. Broader roadmap items are not completed by this
screen.
