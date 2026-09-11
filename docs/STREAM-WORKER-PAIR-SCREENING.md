# Two persistent stream workers: better p99, lower throughput

Reject `9b7427470402c698cc95e7c1f8f3102a0d2be34f` as the next performance
increment. Two persistent handler tasks per stream lose **4.503% / 4.808%
GET throughput** and **4.584% / 5.443% BatchGet(1) throughput** against accepted
`5ee897a`. Mean latency worsens in both repetitions for both APIs. All four
p99 buckets improve; retain that finding, but it does not meet the combined
throughput and latency objective. The first independent audit accepts the
measurement. The accepted implementation remains unchanged.

## Implementation and correctness scope

The [source and conditional proof](https://github.com/c4pt0r/kv9/blob/9b7427470402c698cc95e7c1f8f3102a0d2be34f/docs/STREAM-WORKER-PAIR.md)
replace one spawned task per request with two supervised persistent tasks,
each polling a bounded inbox and FuturesUnordered. The receive owner transfers
each request's original response reservation and absolute deadline. A shared
reservation invariant bounds pending receive, queued jobs, active work and
buffered responses together by CHANNEL_LIMIT. Per-worker cooperative budget
consumption retains cancellation/scheduling progress under the documented
finite-callback and fairness assumptions.

Each queued job and worker holds the original stream grant through actual
cleanup. Worker panic closes the generation after failure observation; clean
EOF drains both workers. Per-frame authentication, unknown-write handling,
independently settling write admission, fresh quorum and applied-index guards
are unchanged. The source excludes instrumentation and rejected scheduler
changes. Its adapter correspondence argument does not complete the outstanding
machine-checked protocol composition or implementation refinement.

Local focused checks pass **74 tests**: 26 stream, 15 async-read, eight async-batch
and 25 peer-transport tests, plus formatting and server all-target Clippy. New
controls exercise sibling progress during blocked authentication, queued
credential revocation and original deadlines, full-inbox abort before first
poll, and clean EOF with held/queued work. The existing real-client held-destructor,
panic/owned-write and full-response backpressure regressions also pass.

The initial server peer filter selected zero tests and was not accepted as
coverage; the correct Raft peer filter passes 25. A separate report-only count
expectation omitted three additional gRPC batch tests selected by the broader
filter; all eight tests had passed. Both observations remain retained. No test
failure was rerun or predicate weakened to obtain this result.

Production-runtime streaming/unary leader-loss and original-directory restart
histories independently pass **358 calls: 326 OK, 32 unknown**. Both atomic
histories, point/batch overlap, contained loss/restart successes, fresh drains
and all five voter/two client lifetimes pass. A separate six-arm correctness
smoke passes **8,731,615 calls**. These are not timed-performance samples.

## Matched five-second results

QPS counts successful calls. Batch size is one, so calls/s equals items/s here.
p99 columns contain histogram intervals, not exact or averaged percentiles.

| API / repetition | Source | Calls/s | Mean us | p99 bucket us |
| --- | --- | ---: | ---: | --- |
| GET / 1 | Control `5ee897a` | 344,602.0 | 185.593 | 360.448–364.543 |
| GET / 1 | Candidate `9b74274` | 329,085.4 | 194.342 | 344.064–348.159 |
| GET / 1 | Redis GET | 510,098.3 | 125.353 | 231.424–233.471 |
| GET / 2 | Control `5ee897a` | 344,255.6 | 185.777 | 356.352–360.447 |
| GET / 2 | Candidate `9b74274` | 327,702.7 | 195.165 | 344.064–348.159 |
| GET / 2 | Redis GET | 502,824.2 | 127.166 | 231.424–233.471 |
| BatchGet(1) / 1 | Control `5ee897a` | 336,614.3 | 189.965 | 368.640–372.735 |
| BatchGet(1) / 1 | Candidate `9b74274` | 321,182.9 | 199.092 | 352.256–356.351 |
| BatchGet(1) / 1 | Redis MGET(1) | 496,495.7 | 128.778 | 231.424–233.471 |
| BatchGet(1) / 2 | Control `5ee897a` | 337,498.7 | 189.468 | 368.640–372.735 |
| BatchGet(1) / 2 | Candidate `9b74274` | 319,129.7 | 200.373 | 348.160–352.255 |
| BatchGet(1) / 2 | Redis MGET(1) | 499,648.2 | 127.969 | 233.472–235.519 |

GET mean latency increases **4.715% / 5.054%**; batch mean increases
**4.804% / 5.755%**. Pooled point throughput falls from **344,428.832** to
**328,394.057/s** (-4.655%); batch falls from **337,056.501** to **320,156.296/s**
(-5.014%). The measured regression belongs to this entire scheduling change;
it does not separately attribute cost to inboxes, allocations or wake coalescing.

The inherited protocol uses 64 closed-loop workers, 128-byte values, 4,096 keys
plus sentinel, 128 warmup calls and a 1,500-ms deadline. Repetition two reverses
the six-arm order. Clients use CPUs 0–1 and all voters share 2–5; owned background
containers use 6–15,22–31 and regain their original masks. No build, test, fault,
profile or audit overlaps timing. Unrelated host services remain unconstrained.
KV9 uses three Raft voters, volatile tmpfs WAL and ordinary sync calls; Redis is
standalone with save/AOF disabled and one outstanding command per connection.
This does not establish equal fault tolerance or durability, sustained capacity,
write/mixed, larger-batch or cross-host performance.

## Independent acceptance and retained evidence

The sole timed run (session 46219) exits zero. All **23,346,575 measured calls**
succeed, including **13,300,788 native calls**, with one SDK attempt per native
call, no other outcomes and no dropped slots. The first frozen independent
audit passes 40 exited lifetimes, 24 fresh drains, 24 writer/listener bindings,
1,172 resource observations, 2,328 source checks and 264 retained files /
44,378,787 bytes. Container masks are restored and namespace identities preserved.

Initial statistics retain their pending-audit state; final statistics record
the accepted measurement and rejected candidate. No broad workspace or candidate
Chaos campaign follows this screen. Accepted `5ee897a` retains its separate
[exact-source workspace and actual Chaos evidence](RPC-WORKER-PAIR-ACCEPTANCE.md).
All work is local; no hosted CI is dispatched. Master/default promotion remains open.

Raw: `/tmp/kv9-stream-worker-pair-matched-diagnostic-attempt1/cohorts`.
Preparation/audit: `/tmp/kv9-stream-worker-pair-comparison-preparation`.
Process audit: `/tmp/kv9-stream-worker-pair-process-independent-first`.
Source/release: `/tmp/kv9-stream-worker-pair`, `/tmp/kv9-stream-worker-pair-release-first`.

| Artifact | SHA-256 |
| --- | --- |
| Candidate server | `a9bdd9d85b24a9ec1e4ae710403a0c4d2baed84fb24b890624856e95036fa17a` |
| Build manifest | `75c9a23f93b7eb1fe001699db0fd79589b0aa304c780b01337a696efcafb60f3` |
| Matched audit | `7db037149923e69d61e8f072aa0ace8f779904149d4b08525511f01b589990bc` |
| Independent statistics | `4369f79a9189252f39cc63e780288bbffa91cbaabe9b056508ae1ccca2e0e12f` |
| Process audit | `e6b8e6aca78ffbd34d390dfe336dab815046bf1a7ace9886be17550d40ce1d62` |

## Next measurement

Completed as the [24-cohort GET concurrency diagnostic](GET-CONCURRENCY-CURVE.md).
It exposes approximately 38 us mean c1 latency versus Redis 5.8 us and substantial
admission-count refusals at c128/c256. The original plan below is historical;
the executed upper points stay within SDK capacity but exceed fixed server
public admission. Use the diagnostic's current follow-up and scope.

Measure a matched single-GET concurrency curve for the accepted control and
Redis, including low concurrency, the existing c64 anchor and higher concurrency
within the public limit. Keep per-command semantics, payload, CPU budget and
source/client identities fixed; publish mean/p99 and successful throughput at
each point. The present closed-loop c64 rate mostly reflects request turnaround
and does not establish maximum server capacity. Locate the low-load latency,
saturation region and tail-latency growth before choosing the next handoff or
confirmation-path change. This curve is planned, not yet measured. Existing
ReadIndex grouping remains in place; no consistency relaxation is proposed.
