# Two-worker global-queue interval: reject the regression

Do not select `c893834950dc2a94d022c4f989115a3e34249cf6` as a performance
increment. Fixed global-queue interval eight on the accepted two-worker/event8
control loses **3.217% / 2.614% GET throughput** and **3.142% / 2.736%
BatchGet(1) throughput**. Mean latency worsens in all four comparisons;
batch p99 also worsens in both repetitions. The first independent audit accepts
the measurement, not the candidate. Retain `5ee897a` and its adaptive interval.

## Exact experiment

The sole executable delta from `5ee897a2f58c57bdf17ea1757c96224adf0f0dbb`
is `.global_queue_interval(8)` on the existing two-worker Tokio runtime.
Event interval eight and all Raft, admission, deadline and cancellation
requirements remain unchanged. The candidate excludes diagnostic instrumentation
and earlier rejected changes. The [conditional source argument](https://github.com/c4pt0r/kv9/blob/c893834950dc2a94d022c4f989115a3e34249cf6/docs/RPC-PAIR-GLOBAL-QUEUE.md)
states the scheduling and progress assumptions; it is not a proof of Tokio or
complete implementation refinement.

The [refreshed lifecycle trace](RPC-PAIR-READ-LIFECYCLE.md) motivated this
interaction test after the worker-count change. Its approximately 41-us result
notification interval includes delivery and receiver scheduling. The actual
adaptive polling interval was not recorded. Fixed eight can poll less often
than adaptive values two through seven; neither this experiment nor the trace
attributes the entire notification interval to the global injection queue.
The earlier [default-worker global-queue screen](RPC-GLOBAL-QUEUE-SCREENING.md)
also regressed and remains rejected under its separate source scope.

## Matched results

Each row represents one complete five-second cohort. QPS counts successful
calls; batch size is one. p99 values are histogram bucket intervals, not exact
or averaged percentiles.

| API / repetition | Source | Calls/s | Mean us | p99 bucket us |
| --- | --- | ---: | ---: | --- |
| GET / 1 | Control `5ee897a` | 344,184.2 | 185.816 | 364.544–368.639 |
| GET / 1 | Candidate `c893834` | 333,111.4 | 192.000 | 364.544–368.639 |
| GET / 1 | Redis GET | 504,639.0 | 126.701 | 231.424–233.471 |
| GET / 2 | Control `5ee897a` | 342,527.7 | 186.714 | 360.448–364.543 |
| GET / 2 | Candidate `c893834` | 333,572.5 | 191.731 | 356.352–360.447 |
| GET / 2 | Redis GET | 506,598.5 | 126.218 | 231.424–233.471 |
| BatchGet(1) / 1 | Control `5ee897a` | 339,117.3 | 188.563 | 360.448–364.543 |
| BatchGet(1) / 1 | Candidate `c893834` | 328,463.6 | 194.685 | 368.640–372.735 |
| BatchGet(1) / 1 | Redis MGET(1) | 497,089.8 | 128.621 | 231.424–233.471 |
| BatchGet(1) / 2 | Control `5ee897a` | 339,052.0 | 188.595 | 364.544–368.639 |
| BatchGet(1) / 2 | Candidate `c893834` | 329,777.1 | 193.906 | 368.640–372.735 |
| BatchGet(1) / 2 | Redis MGET(1) | 499,470.3 | 128.016 | 233.472–235.519 |

GET mean latency increases **3.328% / 2.687%**; batch mean increases
**3.247% / 2.816%**. One improved GET p99 bucket does not offset the repeated
throughput and mean regressions. These two repetitions are a local screen,
not a statistical estimate of a general runtime optimum.

The unchanged protocol uses 64 closed-loop workers, 128-byte values, 4,096 keys
plus sentinel, 128 warmup calls and a 1,500-ms request deadline. Repetition two
reverses the six-arm order. Clients use CPUs 0–1; all three voters share 2–5.
Owned background containers use 6–15,22–31 and regain their original masks.
No builds, tests, faults, profiles or audits overlap timing; unrelated host
services remain unconstrained. KV9 uses three Raft voters, volatile tmpfs WAL
and ordinary sync calls; Redis is standalone with save/AOF disabled. This
does not establish sustained, durable-disk, write/mixed, larger-batch or
cross-host performance, and the systems have different fault tolerance.

## Validation and evidence

Focused local validation passes **68 tests**, formatting and server all-target
Clippy. Production-runtime streaming/unary leader-loss and original-directory
restart histories pass **374 calls: 341 OK, 33 unknown**, with independent
history and lifetime audit. The six-arm correctness smoke passes **8,690,683
calls** before timing. These are separate correctness runs, not timed samples.

The sole matched run (session 69656) exits zero. All **23,488,956 measured
calls** succeed, including **13,449,487 native calls**, with one SDK attempt
per native call, zero other outcomes and zero dropped slots. The first frozen
independent audit accepts 40 exited fixture lifetimes, 24 fresh drains,
24 writer/listener bindings, 1,170 resource observations, 2,328 source checks
and 264 retained files / 44,378,425 bytes. All three container masks are
restored and historical namespace identities remain unchanged.

The initial statistics file retains its pending-audit state. A separate final
statistics file records the accepted measurement and rejected selection.
No broad workspace or candidate Chaos campaign follows this rejected screen.
The previously accepted control's [workspace and actual Chaos evidence](RPC-WORKER-PAIR-ACCEPTANCE.md)
remains scoped to that source. All work is local; no hosted CI is dispatched.

Raw matrix: `/tmp/kv9-rpc-pair-global-matched-diagnostic-attempt1/cohorts`.
Preparation/audit: `/tmp/kv9-rpc-pair-global-comparison-preparation`.
Process audit: `/tmp/kv9-rpc-pair-global-process-independent-first`.
Source/release: `/tmp/kv9-rpc-pair-global-queue`,
`/tmp/kv9-rpc-pair-global-release-first`.

| Artifact | SHA-256 |
| --- | --- |
| Candidate server | `a77c4b8960ad02b4e1e1af53551ce943f539624d974ebdac50c86a09fcd023d0` |
| Build manifest | `508aba69257ac0e3947326445b89621b35938f9eb42438eb26e7751ed46eb52b` |
| Matched audit | `638deaa9c26cf94d385d479ed0055543b102314ce3a8a47e28d6bb70ee460c82` |
| Independent statistics | `8d446308010af403bf253de5d31e5eeacda0234e4fb1150b9d5d55045f131d35` |
| Final root statistics | `85bc9aeaa95f78a10257bbe6187d577cc2f29e685deafc1125b88b48b322f010` |
| Process audit | `8a1d88f7896833887a9630846a9eea3ecfe14f0263ef0f33b40afe8be547cd92` |

## Next structural experiment

Investigate two persistent handler tasks per stream, each polling a bounded
set of request futures, instead of spawning one task per request. This can
coalesce completion notifications while retaining two independently scheduled
polling contexts. The [earlier single-owner design](https://github.com/c4pt0r/kv9/blob/f2c4e856d03f75ac6e4718f0aac7f4e7a1cf3d02/docs/PARALLEL-STREAM-REQUESTS.md)
lost useful parallelism, so simply reverting it is not justified.

The bounded design review is complete; implementation and performance evidence
are still pending. Preserve the original response reservations and frame-time
deadlines across queueing, authenticate every frame, retain stream admission
through actual cleanup, close the generation on worker panic, and preserve
independently settling write admission without replay. Test cancellation,
backpressure, EOF drain and sibling progress before screening. Do not wait to
fill batches or weaken fresh-quorum and applied-index fences. No gain is assumed.
