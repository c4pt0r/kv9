# Inbox-vector reuse: four-repeat matched performance screen

Do not select `c3131800` as the next performance increment. The completed
four-repeat screen shows small pooled gains and mixed directions: c64 GET
improves **0.422%**, while BatchGet(1) changes **0.006%**. Seven of sixteen
individual QPS pairs regress. p99 is lower in ten pairs, unchanged/overlapping
in three and higher in three. This does not establish a consistent useful
throughput-and-latency benefit, nor does it prove the change intrinsically
slower. Retain the source and every observation without advancing this
candidate to a broader fault campaign.

The first complete recording and unchanged independent audit pass all 48
cohorts: **117,886,444 calls = issued = attempts = successes**, with zero
non-success outcomes/reasons, connection failures or dropped slots. The
experiment's parent remains `57ff6851`; `5ee897a` remains the general baseline.

Candidate `c3131800b665f6f160a11c804f542237e77ab83a` directly returns the owned
vector from the Raft inbox on the default gRPC transport path. The previous
implementation allocated a second vector and moved every drained message into
it. Testing builds retain the same partition-mask predicate and message order.
The [source argument](https://github.com/c4pt0r/kv9/blob/c3131800b665f6f160a11c804f542237e77ab83a/docs/RAFT-INBOX-DRAIN.md)
establishes local sequence equivalence; this change introduces no new consensus
action or authority. Queue bounds, notifications, read confirmation, apply
fences, deadlines and write acknowledgments remain unchanged.

The experiment's control is direct parent `57ff6851`, which isolates the vector
change from the preceding ReadIndex credit change. `5ee897a` remains the general
baseline. Fixed clients remain native `03c1c776` and Redis `b8ec38f`, with the
same retained release binaries; the candidate build's workload is not used in
the timed comparison.

Local validation passes 218 Raft tests/doctests, formatting and warnings-denied
Clippy. The clean default release has 595 verified source files. Process E2E
and its unchanged independent audit pass 364 calls, 335 OK / 29 unknown / zero
refused, on streaming and unary paths. All four leader-loss/restart progress
windows, six fresh drains and seven exited lifetimes pass. This is one-host
process evidence; the parent's Chaos acceptance is not inherited as a runtime
test of this revision.

## Predeclared measurement protocol

Protocol `kv9-inbox-drain-c1-c64-v1` fixes six arms: old/new point GET and
BatchGet(1), Redis GET and MGET(1), at c1 and c64. Four repetitions traverse the
entire twelve-cell list in forward/reverse/reverse/forward order, with ten-second
windows: 48 timed cohorts. All sixteen old/new comparisons and sixteen Redis
controls must be reported, retaining each p99 histogram interval. Percentiles
are not averaged, and pooled point estimates imply neither significance nor
no regression.

The twelve-cell, five-second correctness-only smoke is separate and excluded
from timing. Six driver and twelve independent auditor contracts pass before
recording. The actual 48 driver descriptors match the independent auditor,
independent test fixture and frozen protocol. All source/build pins are fixed;
no runtime outcome, resource, histogram, lifetime, drain, retention or isolation
predicate is weakened. Duration, schedule and their derived exact counts are
declared before timing.

The workload remains read-only closed-loop, 4,096 keys plus sentinel, 128-byte
values, batch size one, 128 warmup calls, a ten-million-call cap and 1,500-ms
deadlines. Public admission is 64 requests / 16 MiB, async reads 128, and SDK
in-flight capacity equals concurrency. Accepted timing requires one actual
attempt per call. Redis uses one preconnected connection per worker, one
outstanding command per connection and no pipelining.

Clients use CPUs 0-1; three KV9 voters share CPUs 2-5. Observers and three owned
background containers use 6-15,22-31. Unrelated host services remain unconstrained.
KV9 retains normal sync calls on volatile tmpfs WAL; Redis 7.0.15 is standalone
with persistence disabled and one I/O thread. This shared-host diagnostic does
not establish equivalent durability, sustained capacity or real-NIC limits.
No build, test, fault, profile or independent audit overlaps timing.

The first root summary adapter incorrectly assumed native and Redis smoke
cleanup fields had the same name. The native record uses `cleanup_complete`;
Redis uses `cleanup_errors`. The corrected adapter checks both original fields
without changing the runtime or acceptance predicates. The failed adapter is
retained, and the successful smoke is not rerun. A corrected auditor-preparation
metadata function-name list is also retained; executable checks are unchanged.

Original preparation: `/tmp/kv9-inbox-drain-comparison-preparation`.
Raw recording: `/tmp/kv9-inbox-drain-matched-diagnostic-attempt1`.

## Results

Pooled QPS is total calls divided by total cohort elapsed time; pooled mean
uses total whole-call latency divided by call count. Each arm has four separate
ten-second windows, not one continuous forty-second recording.

| Workers | API | Parent calls/s | Candidate calls/s | Change | Parent mean us | Candidate mean us |
| ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 1 | GET | 26,458.126 | 26,495.263 | +0.140% | 37.669979 | 37.614516 |
| 1 | BatchGet(1) | 26,254.394 | 26,279.146 | +0.094% | 37.952159 | 37.918016 |
| 64 | GET | 371,609.668 | 373,178.659 | +0.422% | 172.097683 | 171.373898 |
| 64 | BatchGet(1) | 368,652.599 | 368,675.162 | +0.006% | 173.446878 | 173.436168 |

Preserve all per-repeat throughput directions alongside these pooled values:

| Workers | API | Repeat 0 | Repeat 1 | Repeat 2 | Repeat 3 |
| ---: | --- | ---: | ---: | ---: | ---: |
| 1 | GET | +0.055% | +0.155% | +0.553% | -0.199% |
| 1 | BatchGet(1) | +0.022% | +0.605% | -0.018% | -0.228% |
| 64 | GET | +1.439% | +0.623% | -0.305% | -0.053% |
| 64 | BatchGet(1) | -0.075% | -0.850% | +0.550% | +0.404% |

For c64 GET, parent -> candidate p99 intervals in microseconds are
303.104-307.199 -> 299.008-303.103, 307.200-311.295 -> 294.912-299.007,
299.008-303.103 -> 299.008-303.103, and
299.008-303.103 -> 303.104-307.199. Thus the last repeat's tail worsens even
though pooled mean improves. No pooled percentile or statistical significance
is inferred.

Same-study Redis GET has pooled 172,444.970 calls/s / 5.723453 us mean at c1
and 508,767.750 calls/s / 125.681387 us at c64. All sixteen paired populations,
all sixteen Redis controls and their original p99 intervals are in the
[complete readout](../scripts/redis-reference/inbox-drain-v1/READOUT.md).
The source-level allocation saving is real; these results do not establish
that it explains a material portion of the remaining Redis gap.

Root smoke session 92541, root timing session 98766 and independent audit
session 4119 all exit 0. The audit accepts 160 exited owned lifetimes, 96
qualifying drains and writer/listener bindings, 9,392 resource samples,
2,348 role/source checks and 1,056 retained files / 177,511,111 bytes. All
three owned container cpusets restore exactly to configured/effective `0-31`;
historical namespace identities remain unchanged. No timing recording is
discarded or rerun for acceptance.

The [retained evidence](../scripts/redis-reference/inbox-drain-v1/README.md)
binds the exact preparation, helpers, contracts, readout and accepted audit.
Audit SHA-256:
`448528041ee2ccc42476b5faefa8279d42babc362b14c9a89441371dc839eafc`.

## Next work

The [next isolated experiment](https://github.com/c4pt0r/kv9/blob/1b70dba62e492cd8e10dd20f643fee74d39abc57/docs/WORK-SIGNAL-COALESCING.md)
is implemented at `1b70dba6`, starting from `57ff6851`, and coalesces redundant
physical WorkSignal notifications while retaining the existing pending bit,
state mutex, publication-before-notify and clear-before-drain ordering. The
local source argument and independent review identify the single-owner
premise and explain why a skipped wake cannot allow pending work to sleep.
The candidate includes an actual-delivery test for concurrent producers in
the drain/park window. All 219 Raft tests/doctests and formatting/Clippy pass.
A deliberately lost pending turn makes the new actual-delivery test fail with
its bounded timeout; exact source restoration makes it pass. The baseline
package gate, failed control and restored targeted test are retained without
repeating the full suite. The default release and unchanged independent
process E2E also pass: **351 calls, 318 OK / 33 unknown / zero refused**,
all four leader-loss/restart progress windows, six fresh drains and seven
exited lifetimes. All 595 source and default release/runtime bindings pass.
This ordinary-WAL process evidence is separate from actual Chaos acceptance.
Its performance remains unmeasured.

Continue to require quorum confirmation, successful-pump and applied-index
fencing, original deadlines/cancellation, bounded resources and typed outcomes.
Sustained reads, writes/mixed traffic, larger batches, full formal composition
and the broader industrial fault matrix remain open. No hosted CI runs.
