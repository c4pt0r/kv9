# Raft owner wake coalescing: matched performance screen

Do not select `1b70dba6` as a general performance increment. It improves all four
c64 GET throughput/mean pairs, with a pooled **0.798%** throughput gain, but c1
GET throughput falls **0.290%** and mean latency rises **0.307%**. Four of sixteen
individual QPS pairs regress. p99 is lower in five pairs, unchanged in four and
higher in seven. The observations support a small c64 benefit in this recording;
they do not establish a useful improvement across throughput and latency, a
statistically significant effect or absence of regression. Keep the source and
all observations without advancing this candidate to a broader fault campaign.

The general baseline remains `5ee897a`; direct parent `57ff6851` is the control
for this isolated experiment. The earlier inbox-vector change is excluded.
No server implementation is promoted by this screen.

## Source and correctness

Candidate `1b70dba62e492cd8e10dd20f643fee74d39abc57` calls the WorkSignal
condition variable only when the mutex-protected pending bit changes from false
to true. It preserves producer publication, clear-before-drain, stop broadcast,
one-owner authority and logical pending state. No delay, spin loop or new queue
is added. The [local source argument](https://github.com/c4pt0r/kv9/blob/1b70dba62e492cd8e10dd20f643fee74d39abc57/docs/WORK-SIGNAL-COALESCING.md)
and independent review explain the drain/park/stop races; this is not complete
Raft/Ready/Rust proof composition.

Previously completed exact-source gates pass 219 Raft tests/doctests, format and
Clippy, a lost-pending-turn negative control with exact source restoration,
default release and independently checked process E2E. That process history has
351 calls, 318 OK / 33 unknown, all four leader-loss/restart progress windows,
six fresh drains and seven exited lifetimes. Its evidence remains in the
[preceding source report](RAFT-INBOX-DRAIN-SCREENING.md). Candidate-specific
actual Chaos remains unrun; its parent's fault campaign is separate evidence.

## Fixed protocol and complete results

Protocol `kv9-work-signal-c1-c64-v1` retains the preceding screen's entire
schedule and acceptance predicates. Six arms cover old/new GET and BatchGet(1),
Redis GET and MGET(1), at c1 and c64. Four ten-second repetitions use whole-list
forward/reverse/reverse/forward order: 48 timed cohorts. The separate twelve-cell
five-second correctness smoke is excluded. Six driver and twelve independent
auditor contracts pass before recording; every descriptor matches both the
independent expectations and the predecessor. Only candidate pins/paths and
protocol identity change. All auditor function bodies remain byte-identical.

Fixed clients are native `03c1c776` and Redis `b8ec38f`, with their original
release binaries. The workload is 4,096 keys plus sentinel, 128-byte values,
batch size one, 128 warmup calls, seed 71, closed-loop traffic, 1,500-ms deadlines
and a ten-million-call cap. SDK concurrency equals worker count; the public
limit remains 64 requests / 16 MiB and async reads 128. Accepted calls use one
actual attempt. Redis has one outstanding command per connection, no pipeline,
one I/O thread and save/AOF disabled. Three KV9 voters use normal sync on tmpfs.

Clients use CPUs 0-1, voters share 2-5, and observers plus three owned background
containers use 6-15,22-31. No build, test, fault, profile or audit overlaps timing.
Light reads and v3 measurement-client edits in a separate worktree run while
fixed v2 measurement sources remain unchanged. Unrelated host services remain
unconstrained. This shared-host volatile diagnostic establishes neither equal
durability, real-NIC limits nor sustained production capacity.

Pooled QPS uses total calls / total cohort elapsed; pooled means use summed
whole-call latency / count. All four repetitions are included. Percentiles are
not averaged.

| Workers | API | Parent calls/s | Candidate calls/s | QPS change | Parent mean us | Candidate mean us |
| ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 1 | GET | 26,536.505 | 26,459.542 | -0.290% | 37.555366 | 37.670622 |
| 1 | BatchGet(1) | 26,268.420 | 26,265.960 | -0.009% | 37.935641 | 37.941827 |
| 64 | GET | 374,812.531 | 377,804.713 | +0.798% | 170.629087 | 169.276052 |
| 64 | BatchGet(1) | 368,978.621 | 369,901.873 | +0.250% | 173.293636 | 172.860253 |

Same-recording Redis GET reaches 172,609.493 calls/s / 5.717858 us at c1 and
512,144.140 calls/s / 124.854313 us at c64. All sixteen native pairs and sixteen
Redis controls, including exact histogram intervals, are in the
[complete readout](../scripts/redis-reference/work-signal-v1/READOUT.md).
For c64 BatchGet(1), the final two repetitions' p99 worsens from
303.104-307.199 to 311.296-315.391 us and from 299.008-303.103 to
303.104-307.199 us. The favorable aggregate QPS does not erase those tails.

The first recording and frozen independent audit accept **118,432,733 calls =
issued = attempts = successes**, zero other outcomes/reasons, retries, dropped
slots or connection failures. Root smoke 58438, root timing 58205 and independent
audit 88927 all exit 0. Acceptance covers 160 exited lifetimes, 96 fresh drains
and writer/listener bindings, 9,386 resource samples, 2,348 role-source checks
and 1,056 retained files / 177,511,414 bytes. All three owned container CPU masks
restore exactly to configured/effective 0-31; historical namespace identities
remain unchanged. No recording is discarded or rerun for acceptance.

Audit SHA-256: `28a80e747f46cdc0fee27fb0b4bfa91dbb1e4d70cacb406355b7cdb370171a77`.
Original preparation: `/tmp/kv9-work-signal-comparison-preparation`.
Raw recording: `/tmp/kv9-work-signal-matched-diagnostic-attempt1`.
[Retained source and evidence](../scripts/redis-reference/work-signal-v1/README.md).

## Next development

Refresh writes, mixed traffic and genuine batch APIs before another isolated
pure-read micro-optimization. The fixed v2 point-read clients only permit pure
GET, and writes use BatchPut/MSET, so they cannot establish actual point PUT/SET
or point GET/PUT mixed behavior. The new
[explicit v3 measurement clients](https://github.com/c4pt0r/kv9/blob/0be806d9671e2c50701a64aa7889c8859b7648ba/docs/POINT-WRITE-MEASUREMENT.md)
add actual point writes and mixed API selection, preserving v1/v2 behavior and
shared deterministic inputs. Local native/Redis Rust tests pass 15/19 cases;
independent Python tests pass 19/17, with focused Clippy and format checks.
Both clean default-release clients build successfully from `0be806d9`. The
first release preflight refused an unrelated legacy Redis `main.rs` formatting
edit introduced by the root's broad formatter command, before compilation.
That formatter-only edit was saved and restored to HEAD; a scoped benchmark
format check passes and consumed benchmark inputs are unchanged. The refusal,
both file versions, final clean source inventories and successful builds are
retained. No benchmark source was changed or test rerun to hide the refusal.

The first real-runtime smoke exits 0 with all twelve cases passing: native
GET/PUT and Redis GET/SET at batch size one, plus BatchGet/BatchPut and MGET/MSET
at batch size four, each with 0/50/100 percent reads. It uses three ordinary
`5ee897a` voters, six fresh standalone Redis lifetimes, c4, 128 keys plus sentinel,
128-byte values, 32 warmup calls, 500-ms duration and a 100,000-call cap. All
670,684 calls across phases succeed in one actual attempt, including 667,384
measured calls. Eighteen fresh drains cover all three voters; all 21 managed
lifetimes are reaped. All six Redis cases hit the operation cap and correctly
report timing ineligible. This is correctness-only evidence with performance
acceptance disabled for every case; no v3 performance result is claimed.

Independent final scans validate all 129 deterministic values, configured nonce
bounds, write-key membership and the untouched sentinel. They do not establish
the exact issued nonce set or full-history linearizability: client allocation
can race the measurement cutoff. The preparation reviewer caught the draft's
unsupported contiguous-nonce assumption before runtime; the original draft and
explicitly bounded correction remain retained. No Chaos or protocol acceptance
is inferred from this client smoke.

The first independent result audit exits 0 and rehashes all 313 retained files
(30,513,735 bytes). It accepts exact source/release bindings, selected APIs and
pairing, all-phase populations, deterministic scans, 18 drains / 54 voter
bindings and cleanup. Audit SHA-256:
`e718e4376fb8147f553a6928fd59aa2f9d6d406f2f5dc5724079a746b302b4a7`.
[Independent smoke report](../scripts/redis-reference/work-signal-v1/next-client/smoke-independent/REPORT.md).

Use the same new clients on every arm of the next declared comparison. Report
calls/s, keys/s, whole-call mean and tails, refused/unknown outcomes and resource
usage. Preserve quorum, exact committed/applied receipts, read/apply fencing,
original deadlines and no replay of unknown writes. Redis parity, full formal
composition, the broader fault matrix and automatic range splitting remain
open. CI remains local; no hosted workflow is dispatched.
