# RPC I/O event interval: throughput gain with explicit tail evidence

The isolated candidate `917243fd1b501843d75899cc167f7eff32b5bbb2` improves GET throughput and mean latency against the unchanged jemalloc control `629bee4`. The five-second follow-up reaches **326,793–327,857 GET calls/s**, paired gains of **8.649% / 8.585%**, with mean whole-call latency **195.074–195.708 us** versus **211.959–212.525 us**. Redis GET reaches **506,897–507,881 calls/s**, leaving approximately a **1.55x** throughput gap. Redis parity remains open.

BatchGet(1) reaches **320,880–321,634 calls/s**, improving **7.924% / 8.233%**; mean latency falls from **214.587–215.709 us** to **198.821–199.287 us**. These are calls/s and equal items/s only at batch size one. Writes, mixed traffic and larger batches need separate evidence.

The longer run has unchanged p99 buckets in repetition one and lower buckets in repetition two for both APIs. The preceding 1.5-second screen improves throughput/mean too, but has worse p99 in repetition two. Both results remain valid, retained evidence; the follow-up does not erase the earlier tail regression or prove universal tail improvement. The full workspace checks and [exact-source eleven-window Chaos acceptance](RPC-EVENT-INTERVAL-ACCEPTANCE.md) now pass. Select this as the next isolated read-performance increment. Default/master promotion remains open.

## Implementation and scope

The sole executable change replaces Runtime::new with the equivalent multi-threaded builder, all drivers enabled, and event_interval(8). Pinned Tokio 1.53.1 defaults this interval to 61. Default worker selection, adaptive global queue scheduling, allocator, peer streaming transport and all protocol settings are preserved. This does not include the rejected fixed global-queue experiment, authorization-metadata candidate or lifecycle instrumentation.

Public and peer tasks share the executor. More frequent busy-worker maintenance offers earlier socket-readiness/timer processing. Shared-driver lock contention can skip a polling attempt; idle workers already poll independently, and maintenance also handles statistics, shutdown and deferred wakeups. This is not a pure-I/O attribution or a fixed latency bound. The change preserves group sealing, exact ReadIndex confirmation, successful-pump and applied-index conditions, admission, cancellation, deadlines and write acknowledgements. The [conditional safety correspondence](RPC-EVENT-INTERVAL-SAFETY.md) covers only this scheduling delta; existing core safety arguments grant no latency or unconditional progress theorem.

Both matrices use exact clean binaries and the fixed native03/Redis b8 clients: 64 closed-loop workers, 4,096 keys plus sentinel, 128-byte values, batch size one, 100% reads, 128 warmup calls, a 10-million-call cap and 1,500-ms per-request deadlines. Two repetitions reverse six-arm order. The follow-up changes only measurement duration from 1,500 to 5,000 ms plus owned artifact paths; exact validator expectations change correspondingly. It has no rebuilt source, discarded arm, relaxed predicate or repeated correctness smoke. The original short-run tail concern specifically motivated this follow-up.

Clients use CPUs 0–1; all measured servers share 2–5; owned background containers are constrained to 6–15,22–31 and restored. No builds, tests, faults, profiles or audits overlap timing. Other host services remain unconstrained. KV9 uses three ordinary Raft voters and tmpfs WAL with normal sync calls; standalone Redis has save/AOF disabled. Durability and fault tolerance are not equivalent. Neither five seconds nor these two repetitions establish sustained capacity or offered-load behavior.

## Five-second follow-up

| Cohort | Calls/s | Mean us | p99 bucket us |
|---|---:|---:|---|
| 000-old-point-p00064 | 301,757.9 | 211.959 | 368.640–372.735 |
| 001-old-batch1-p00064 | 298,017.9 | 214.587 | 372.736–376.831 |
| 002-new-point-p00064 | 327,856.9 | 195.074 | 368.640–372.735 |
| 003-new-batch1-p00064 | 321,633.5 | 198.821 | 372.736–376.831 |
| 004-redis-mget1-p00064 | 501,622.1 | 127.455 | 231.424–233.471 |
| 005-redis-get1-p00064 | 506,897.1 | 126.141 | 231.424–233.471 |
| 006-redis-get1-p10064 | 507,880.9 | 125.901 | 231.424–233.471 |
| 007-redis-mget1-p10064 | 501,806.2 | 127.422 | 231.424–233.471 |
| 008-new-batch1-p10064 | 320,879.5 | 199.287 | 368.640–372.735 |
| 009-new-point-p10064 | 326,792.9 | 195.708 | 364.544–368.639 |
| 010-old-batch1-p10064 | 296,470.0 | 215.709 | 372.736–376.831 |
| 011-old-point-p10064 | 300,955.5 | 212.525 | 368.640–372.735 |

## Original 1.5-second screen

| Cohort | Calls/s | Mean us | p99 bucket us |
|---|---:|---:|---|
| 000-old-point-p00064 | 304,797.1 | 209.827 | 364.544–368.639 |
| 001-old-batch1-p00064 | 298,534.0 | 214.194 | 376.832–380.927 |
| 002-new-point-p00064 | 325,200.0 | 196.651 | 364.544–368.639 |
| 003-new-batch1-p00064 | 321,033.8 | 199.176 | 368.640–372.735 |
| 004-redis-mget1-p00064 | 501,444.4 | 127.473 | 229.376–231.423 |
| 005-redis-get1-p00064 | 511,642.7 | 124.944 | 231.424–233.471 |
| 006-redis-get1-p10064 | 492,030.6 | 129.928 | 231.424–233.471 |
| 007-redis-mget1-p10064 | 503,300.6 | 127.028 | 233.472–235.519 |
| 008-new-batch1-p10064 | 315,683.0 | 202.557 | 376.832–380.927 |
| 009-new-point-p10064 | 324,823.2 | 196.878 | 376.832–380.927 |
| 010-old-batch1-p10064 | 296,946.6 | 215.338 | 368.640–372.735 |
| 011-old-point-p10064 | 303,796.1 | 210.513 | 364.544–368.639 |

Quantiles are histogram bucket intervals, not exact values or averaged percentiles. Five-second aggregate mean voter RSS rises from **54.58/54.59 MiB to 55.83/56.61 MiB** for GET, and **54.67/54.53 to 56.12/56.59 MiB** for BatchGet(1). This is roughly 1.3–2.1 MiB more over this small working set; long-running memory behavior remains unmeasured.

## Validation and artifact identity

Focused checks pass: 23 point-stream, 15 async-read, five async-batch and 25 peer transport tests, formatting and server all-target Clippy. The actual production-runtime stream/unary leader-kill/original-directory restart E2E has **357 calls: 325 OK and 32 unknown**, both complete atomic histories valid, with owned processes cleaned up and unknown writes not blindly replayed. The separate six-arm correctness smoke passed before timing. Tests creating their own runtime alone do not exercise the changed constructor; actual frozen-binary process histories provide that coverage.

The first matched runtime session46774 and independent audit each passed once: **6,749,813 successful measured calls**, 40 exited lifetimes, 24 drains, 24 writer/listener bindings, 353 resource observations and 2,324 source checks. The five-second runtime session46299 and independent audit each passed once: **22,563,781 successful measured calls**, the same lifecycle/drain/binding counts, 1,172 resource observations and 2,324 source checks. There are no additional SDK attempts or non-success measured outcomes. All original container masks and historical namespace maps are preserved. Both matrices retain exact datasets and input artifacts.

Endpoint provenance checks require absent allocator/interposition overrides and TOKIO_WORKER_THREADS. No source/build/worker or protocol adjustment is hidden in the duration change. Both reports retain latencies in nanoseconds and convert only displayed units. Default and experimental-RPC workspace runs pass 707 and 717 tests respectively, with 23 existing ignored tests in each configuration; these configurations overlap. Workspace all-target experimental Clippy passes with warnings denied. The separate exact-source Chaos campaign and unchanged full audit pass all eleven windows and the 2,106-call complete atomic history (1,803 OK, 228 refused, 75 unknown), with 177 verified new leaf drops and complete owned cleanup. This is one-host tmpfs link/quorum evidence, not disk/power-loss or sustained-load acceptance.

- Source and release: `/tmp/kv9-rpc-event-interval`, `/tmp/kv9-rpc-event-release-first`.
- Short run and audit: `/tmp/kv9-rpc-event-matched-diagnostic-attempt1/cohorts`, `/tmp/kv9-rpc-event-comparison-preparation/results-first`.
- Five-second run and audit: `/tmp/kv9-rpc-event-five-second-diagnostic-attempt1/cohorts`, `/tmp/kv9-rpc-event-five-second-preparation/results-first`.
- Local gates and actual histories: `/tmp/kv9-rpc-event-local-first`, `/tmp/kv9-rpc-event-process-e2e-first`.

| Artifact | SHA-256 |
|---|---|
| Candidate server | `12384de2df2f57e2e148a919795f7f8c255ab85fdbaa73a373e72fc5bd354217` |
| Build manifest | `0d77baabf19f89cfd903c3bcf80c408ef39d253cca1d2b8468013da03779599b` |
| Short independent audit | `992c5b4023854744ddb0b36aa96b8198e75d49da7e29267798bbc6b3b889afb6` |
| Short root statistics | `3be4e3efff67e47c940f302f17336aa13f7885a734a0c95f6f481732c92d31de` |
| Five-second independent audit | `038435e346cb31ceacf0fbc7156fb972b509accaed7a9dd362dc77036444512e` |
| Five-second root statistics | `483923a1a448d569244b59b73f3b1cd750d2ede452504606314990819418a36c` |

No hosted CI was dispatched. The original roadmap checklist is not changed by this screen.
