# Peer idle watchdog: better p99, insufficient throughput

Do not select candidate `f62c08ed9dc59bf56336a64a24c76c2b775f634e` as the next
performance increment. **Seven of eight matched throughput comparisons regress
and seven mean latencies worsen**, despite lower p99 buckets in all eight
pairs. At c64, GET throughput falls 0.618% / 2.915% and BatchGet(1) falls
3.189% / 3.457%. The accepted production control remains `5ee897a`.

The first complete recording and independent audit pass with **27,821,713
successful calls**, one observed attempt per call, no dropped slots and no
non-success outcomes. This accepts the evidence, not the candidate. No broad
workspace or actual Chaos Mesh campaign was run to promote the rejected source.

## Implementation and correctness

The [committed candidate and deadline argument](https://github.com/c4pt0r/kv9/blob/f62c08ed9dc59bf56336a64a24c76c2b775f634e/docs/PEER-IDLE-WATCHDOG.md)
follow rejected direct-body source `6707bcc`. Enqueue still timestamps new
backlog and wakes the tonic body, but no longer notifies the independent owner
watchdog. An empty observation at time `t` arms a check at `t+B` under the same
queue lock. A later enqueue at `e>=t` has deadline `e+B>=t+B`. That check observes
the backlog by its policy deadline without the producer notification.

The three-second budget is unchanged. Idle timer expiry only rechecks/rearms;
current progress is revalidated before declaring a stall. Lifecycle notifications,
independent supervision, per-RPC/route fencing, queue bounds and cooperative
budget accounting remain. This removes one explicit notification edge, with
periodic idle-check work as the tradeoff. It does not eliminate every owner wake.

Local source validation passes **228 Raft tests**, Clippy with warnings denied,
and **14 source-control triples / 42 compiled executions**. New tests cover
distinct body/owner wakes, real enqueue before and after idle arming, persistent
idle checks across multiple budgets, ordinary polls retaining the timer, prompt
closure and current-progress revalidation after an old timer becomes ready.
The original artificial backdated-after-idle fixture was replaced because its
history violates `e>=t`; the real three-second production budget was retained.

All 14 mutants fail their intended assertion with the ordinary test-failure
exit code between passing baseline and restored runs. Independent readback
verifies 168 per-phase production/test bindings, the 42 exact selected-test
logs and their failure markers. Source and control checks passed on their first
recordings. The local argument is conditional source reasoning, not a new
machine-checked whole-system refinement proof. Core Raft/read/apply algorithms,
authentication, public admission and runtime settings are unchanged.

The independently checked process E2E records **379 calls: 346 OK, 33 unknown**.
Streaming records 195 (175/20), unary 184 (171/13), with overlapping point/batch
operations, leader loss, original-directory restart and fresh final drains.
Unknown outcomes stay in the complete histories. All five voter and two client
lifetimes exit. This proves neither power-loss durability nor actual Chaos Mesh
acceptance of this candidate. All routine checks ran locally; no hosted workflow
was dispatched.

## Same-run paired results

QPS counts successful logical calls. Repetitions below retain raw numbering
0 and 1. p99 values are original histogram intervals in microseconds; percentiles
are not averaged. Every measured call succeeded.

| Concurrency | Repeat | API | Control QPS | Candidate QPS | QPS change | Mean us, control -> candidate | p99 us, control -> candidate |
| --- | --- | --- | ---: | ---: | ---: | --- | --- |
| 1 | 0 | GET | 26,257.2 | 26,035.9 | -0.843% | 37.962 -> 38.276 | 59.392-59.903 -> 57.856-58.367 |
| 1 | 1 | GET | 26,199.3 | 26,252.1 | +0.202% | 38.036 -> 37.970 | 60.416-60.927 -> 56.832-57.343 |
| 1 | 0 | BatchGet(1) | 26,050.7 | 25,441.3 | -2.339% | 38.254 -> 39.173 | 59.392-59.903 -> 58.880-59.391 |
| 1 | 1 | BatchGet(1) | 26,225.7 | 26,002.1 | -0.853% | 38.010 -> 38.335 | 60.416-60.927 -> 57.344-57.855 |
| 64 | 0 | GET | 334,590.4 | 332,523.1 | -0.618% | 191.146 -> 192.335 | 368.640-372.735 -> 344.064-348.159 |
| 64 | 1 | GET | 342,121.0 | 332,147.2 | -2.915% | 186.937 -> 192.555 | 356.352-360.447 -> 348.160-352.255 |
| 64 | 0 | BatchGet(1) | 337,318.7 | 326,561.3 | -3.189% | 189.568 -> 195.816 | 364.544-368.639 -> 352.256-356.351 |
| 64 | 1 | BatchGet(1) | 338,242.1 | 326,548.4 | -3.457% | 189.049 -> 195.824 | 368.640-372.735 -> 352.256-356.351 |

Same-run standalone Redis GET:

| Concurrency | Repeat | GET/s | Mean us | p99 interval us |
| --- | --- | ---: | ---: | --- |
| 1 | 0 | 170,632.5 | 5.785 | 8.064-8.127 |
| 1 | 1 | 170,754.6 | 5.780 | 8.064-8.127 |
| 64 | 0 | 509,322.5 | 125.546 | 231.424-233.471 |
| 64 | 1 | 506,378.4 | 126.277 | 233.472-235.519 |

The [full 24-cohort readout](../scripts/redis-reference/peer-idle-watchdog-v1/READOUT.md)
also retains Redis MGET(1). [Independent comparison statistics](../scripts/redis-reference/peer-idle-watchdog-v1/statistics-first.json)
preserve the exact arithmetic; a separate root calculation matches all eight
QPS and mean deltas. The [selection record](../scripts/redis-reference/peer-idle-watchdog-v1/selection.json)
distinguishes evidence acceptance from performance rejection.

This is a comparison against accepted `5ee897a`, not a paired experiment against
`6707bcc`. Cross-run differences cannot establish how much the removed watchdog
notification helped. The small mixed c1 GET deltas do not justify a speedup claim.

## Protocol, environment and provenance

Protocol `kv9-peer-idle-watchdog-c1-c64-v1` preserves the preceding 24-cohort
design: c1/c64, control and candidate GET/BatchGet(1), Redis GET/MGET(1), two
repetitions with the full twelve-cohort order reversed in the second. Each
cohort uses five seconds, 4096 keys plus sentinel, 128-byte values, batch size
one, read-only closed-loop traffic, 128 configured warmup calls, a 10-million
call cap and original 1500-ms deadlines. Native retry configuration is unchanged;
every measured call uses one observed attempt. SDK max-in-flight equals worker
concurrency; server admission and runtime bounds remain fixed.

Native client `03c1c776` and Redis client `b8ec38f` use their original releases.
Redis has one preconnected connection per worker, one outstanding command per
connection, no pipeline, no replicas, save/AOF off and `io-threads=1`. KV9 has
three voters with volatile tmpfs WAL and normal sync calls. This is not equal
durability, cross-host networking or sustained-capacity evidence.

Clients use CPUs 0-1 and voters share 2-5. The observer and three owned
background containers use 6-15,22-31. Original configured/effective 0-31 masks
are restored, with historical namespace identities and the existing one-shot
fault object preserved. Unrelated host services remain unconstrained. No build,
test, fault, profile or independent audit overlaps timing.

The exact pin/path/protocol-only helper derivative preserves all validators;
six driver and ten auditor contracts pass before runtime. The separate first
12-cohort correctness smoke records 10,657,726 successful calls and 40 exited
lifetimes. Its timing is excluded. Root timed session **3097** and independent
audit session **77755** both exit 0. The audit verifies **80 exited lifetimes,
48 fresh drains, 48 voter/listener bindings, 2331 source-file checks, 2344 resource
observations and 528 retained files / 88,755,271 bytes**.

Two inline exploratory readbacks required schema corrections: Redis cleanup
uses `errors`/`pid_absent_after_reap`, and status counters include decimal strings.
Both first errors and corrections are recorded in `preparation-notes.json`.
No executed helper, raw input or predicate changed, and no fixture/timing/audit
was rerun to obtain acceptance. The earlier unfinished helper-pin preparation
also remains retained.

Raw timing: `/tmp/kv9-peer-idle-watchdog-matched-diagnostic-attempt1/cohorts`.
Raw independent result:
`/tmp/kv9-peer-idle-watchdog-comparison-preparation/results-first/audit.json`.
[Executed helpers and artifact inventory](../scripts/redis-reference/peer-idle-watchdog-v1/README.md)
record exact source paths, dependencies, byte counts and hashes.

| Artifact | SHA-256 |
| --- | --- |
| Candidate server | `c050cdf9b109816c28a186a3e954e4fe32a6488288f1f2398c68f83bd6d4609b` |
| Candidate build manifest | `a0a3258c5dcaf4eeb1bb9c35f4e07b32f42d1fbc7d2253e76b442ec9dc3e83e1` |
| Timing driver | `ebaa51b3598706748ab3bcfa979f39f17ee6f120eb74a6df5d97e13a5a108ef1` |
| Independent auditor | `6fade55411d5ac02d57fef87801fbff96f57cb74d8a4ddce394ddab5cf9fcc52` |
| Independent result | `b23a9ae4ac1e639f834d3e7fb999e55ff9c0bdb6673763d141c6c30ebb84f343` |
| Process E2E result | `5225482714e9c7d909bb3b954d3ee3b50b8db3491e73bffeecd80f35a8e06092` |
| Source-control manifest | `4fe438eec1dabf10ee51be45529d3191f2d1984b6c5e27397775dc6b66a1861c` |

## Next experiment: read-group admission and amortization

The same retained recording exposes a separate scheduling opportunity in the
accepted control. Cumulative admitted-member/group counter deltas give:

| API | Concurrency | Repeat 0 members/group | Repeat 1 members/group |
| --- | ---: | ---: | ---: |
| GET | 1 | 1.0000 | 1.0000 |
| BatchGet(1) | 1 | 1.0000 | 1.0000 |
| GET | 64 | 2.0333 | 2.0337 |
| BatchGet(1) | 64 | 2.0107 | 2.0115 |

These endpoint spans include setup, warmup and verification; they are not
timed-call-only counts or evidence that grouping causes the full latency gap.
[The source-bound deltas](../scripts/redis-reference/peer-idle-watchdog-v1/read-group-amortization.json)
retain all 16 native cohorts and per-voter counts, including the candidate.
An [independent counter and admission review](../scripts/redis-reference/peer-idle-watchdog-v1/read-group-admission-next-review.md)
recomputes all 16 rows from 32 SHA-bound endpoint files and checks stable process,
boot, root/store identity and drained boundaries. Every row contains 8322 more
admitted members than measured calls, reinforcing the whole-cohort scope.

On accepted `5ee897a`, `AsyncReads::submit` seals up to 64 already-queued members
per pump but does not bound admission by the number of unconfirmed groups.
Investigate a bounded number of outstanding unconfirmed groups, allowing a
later queued prefix to accumulate while preceding confirmation is pending.
An idle c1 read must remain immediately eligible; no fixed batching sleep is
proposed. No later caller may join an already-sealed group or borrow its quorum
confirmation. Applied-index coverage and successful whole-pump completion
remain mandatory.

This needs a separate safety/progress argument and adversarial tests before
implementation screening. Both submit's retained-prefix notification and the
driver's queued-work notification must respect group capacity without spinning.
Queued cancellation/expiry cleanup, inbound pumping and ticks must remain
serviceable while capacity is full. Confirmed groups awaiting apply must not
consume unconfirmed-group capacity. The design must distinguish live registry
groups from outstanding Raft ReadIndex contexts: removing a canceled group's
last member does not retract its already-submitted Raft request. Confirmation,
cancellation, expiry and leadership changes need explicit eligibility and
cleanup transitions; a local registry count alone cannot prove a protocol-wide
outstanding-context bound.
Global request bounds, exact identity, original deadlines and failure cleanup
must remain intact. The added waiting may hurt latency and must be measured.
This is the next hypothesis, not an implemented or accepted optimization.
