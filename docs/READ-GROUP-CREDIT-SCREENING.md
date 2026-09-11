# Pending ReadIndex credit: higher c64 throughput in the initial screen

Follow-up: the separate [four-repeat, 30-second c1 study](READ-CREDIT-C1-FOLLOWUP.md)
and [exact-source eleven-window local Chaos gate](READ-CREDIT-CHAOS-ACCEPTANCE.md)
now pass. The longer study has slightly better pooled GET throughput/mean and
mixed p99; it does not establish c1 no-regression. The original observations
below remain unchanged. `5ee897a` remains the general baseline.

Candidate `57ff6851e40ed63c837189d6eb0a11190704725a` improves c64 GET
throughput by **8.031% / 8.220%** and BatchGet(1) by **8.479% / 7.919%**
against accepted `5ee897a`. Mean latency and p99 improve in all four c64
pairs. Retain this as a promising high-concurrency candidate, not a general
runtime promotion: c1 point GET loses **0.083% / 0.573%** throughput and its
mean and p99 worsen in both repetitions. These short shared-host observations
do not establish that the small c1 effect is negligible or statistically stable.

The first complete 24-cohort recording and unchanged independent audit pass.
All **28,847,557 issued logical calls succeed**, with one measured attempt each,
zero non-success outcomes/reasons and zero dropped slots.
Accepted control `5ee897a` remains the general baseline. At this initial screen,
exact-source Chaos Mesh acceptance and the c1 follow-up were pending; both
now complete as linked above. Sustained/write/mixed/large-batch behavior and
whole-protocol proof composition remain open. Redis parity and
automatic splitting are not established.

## Mechanism and correctness boundary

The [source and design](https://github.com/c4pt0r/kv9/blob/57ff6851e40ed63c837189d6eb0a11190704725a/docs/READ-INDEX-CREDIT.md)
bound locally admitted, unconfirmed ReadIndex requests to one using the
upstream Raft pending queue under the same lock as submission. Synchronous
and asynchronous readers share this gate. Existing queued requests can form a
later sealed group; no fixed batching delay is added to idle admission.

Canceling the last member does not release its outstanding protocol request.
An acknowledgment, configuration-quorum reevaluation or Raft reset can clear
occupancy. Singleton confirmation may bypass queue occupancy entirely. A group
that has confirmed but awaits local apply no longer consumes the pending slot.
Late reads cannot join an already admitted group. Existing read confirmation,
apply fences, original deadlines, typed refusal and write acknowledgment
requirements remain unchanged.

The implementation also makes the owner's wake hint capacity-aware. It must
park with a full slot while still processing queued cancellation/expiry,
inbound acknowledgments and role changes. The policy bounds local submission
against actual upstream occupancy; authenticated remote ReadIndex messages
do not pass through this wrapper, so it is not a universal queue-memory bound.

The [separate endpoint readout](../scripts/redis-reference/read-group-credit-v1/endpoint-counters-second.md)
confirms larger groups in the complete fixture envelopes: c64 GET averages
15.2467 / 15.2457 members per group versus 2.0274 / 2.0314 for the control;
BatchGet(1) averages 15.0812 / 15.0967 versus 2.0034 / 2.0084. All c1 cases
remain 1.0. These counters include setup, warmup and verification, with admitted
members equal to measured calls plus 8,322. They are not timed-only ratios or
a causal decomposition of either the gain or the remaining Redis gap.

The counter reader preserves the prior arithmetic. Its first adapter attempt
incorrectly assumed before-status files were in the main auditor's input
inventory and stopped. The corrected separate readout hashes both snapshots
directly and matches accepted voter start-tick identities; the failed assumption
is retained. The accepted main audit and performance readout are unchanged.

Local validation passes 714 default workspace tests/doctests with 23 ignored,
including the 218-test Raft package gate, and warnings-denied Clippy. Seven
new real-Raft tests cover freshness, cancellation, parked-owner wakeup, apply
delay, synchronous admission/deadlines and leadership transition. All 14
compiled implementation-control triples retain exact baseline/mutant/restored
0/101/0 exits and intended semantic failures; source readback independently
checks unchanged tests, source bindings and final restoration.

The parameterized TLA+/TLAPS gate freshly checks 28 theorems / 265 obligations
across inherited admission and new credit modules. All 37 retained formal
cases pass their required positive or negative verdicts. Two fingerprints
agree at safety bounds of 832 and 2,472 distinct states and six conditional
progress states. Controls include cap bypass, cancellation refund, missing
release fairness, competing refill, omitted proof and an added axiom.

The proof projects a single invocation to the unchanged admission model. Its
eventual-admission theorem assumes a stable ready leader, positive budget,
no cancellation/competing refill, and fair release and polling. It does not
prove complete grouped-read/Ready/Rust refinement or unconditional progress.
Independent formal review verifies the model/proof/control bindings and
preserves the prior failed attempts described in the source design.

Process E2E and its unchanged independent audit pass **353 complete operations:
324 OK, 29 unknown, zero refused**. Streaming and unary paths both cover
leader loss, original-directory restart and overlapping point/batch histories.
All four contained progress windows pass, all seven process lifetimes exit,
and six fresh drain checks pass. The audit checks all 594 source bindings and
the default release identity. This one-host process evidence is separate from
Chaos Mesh and power-loss acceptance.

## Matched results

All values are client-visible successful calls/s and whole-call latency.
BatchGet(1) contains one key. Histogram intervals are retained rather than
presented as exact quantiles. Each repeat is reported separately.

| c | Repeat | API | Control calls/s | Candidate calls/s | Change | Mean us, control -> candidate | p99 interval us, control -> candidate |
| ---: | ---: | --- | ---: | ---: | ---: | --- | --- |
| 1 | 0 | GET | 26,307.1 | 26,285.3 | -0.083% | 37.888 -> 37.926 | 58.368-58.879 -> 58.880-59.391 |
| 1 | 1 | GET | 26,230.6 | 26,080.3 | -0.573% | 37.998 -> 38.217 | 52.736-53.247 -> 55.808-56.319 |
| 1 | 0 | BatchGet(1) | 26,098.5 | 26,144.4 | +0.176% | 38.188 -> 38.123 | 59.904-60.415 -> 60.416-60.927 |
| 1 | 1 | BatchGet(1) | 26,220.4 | 26,245.1 | +0.094% | 38.010 -> 37.970 | 53.248-53.759 -> 52.224-52.735 |
| 64 | 0 | GET | 344,151.7 | 371,789.3 | +8.031% | 185.830 -> 172.010 | 360.448-364.543 -> 303.104-307.199 |
| 64 | 1 | GET | 345,671.0 | 374,086.3 | +8.220% | 185.018 -> 170.956 | 352.256-356.351 -> 299.008-303.103 |
| 64 | 0 | BatchGet(1) | 338,390.5 | 367,082.4 | +8.479% | 188.965 -> 174.184 | 364.544-368.639 -> 315.392-319.487 |
| 64 | 1 | BatchGet(1) | 339,745.7 | 366,649.8 | +7.919% | 188.211 -> 174.394 | 360.448-364.543 -> 307.200-311.295 |

Same-run standalone Redis GET:

| c | Repeat | GET/s | Mean us | p99 interval us |
| ---: | ---: | ---: | ---: | --- |
| 1 | 0 | 171,657.2 | 5.750 | 8.128-8.191 |
| 1 | 1 | 172,487.9 | 5.722 | 7.744-7.807 |
| 64 | 0 | 509,004.3 | 125.620 | 229.376-231.423 |
| 64 | 1 | 506,965.8 | 126.124 | 229.376-231.423 |

Redis's c64 throughput is still approximately 1.36-1.37x the candidate's,
and the candidate's c1 turnaround is approximately 6.6-6.7x Redis's. Closed-loop concurrency
can overlap distributed waiting, so the c64 gain does not imply a shorter
isolated request path. The [Redis execution analysis](REDIS-EXECUTION-PATH.md)
connects this distinction to command ownership, buffer reuse and fewer
scheduling steps without weakening KV9's consensus requirements.

## Protocol and retained evidence

Protocol `kv9-read-group-credit-c1-c64-v1` preserves the previous six-arm
c1/c64 comparison, with two reversed repetitions and five-second read-only
closed-loop windows. There are 4,096 keys plus a sentinel, 128-byte values,
128 configured warmup calls, a ten-million-call cap and 1,500-ms deadlines.
Public admission remains 64 requests / 16 MiB, asynchronous read capacity
128, and SDK in-flight capacity matches c1/c64. Measurement clients remain
native `03c1c776` and Redis `b8ec38f` with their original release binaries.

Redis 7.0.15 runs without persistence or replication, one I/O thread, one
preconnected connection per worker and one outstanding command per connection.
It does not pipeline. KV9 uses three voters, normal Raft sync calls and volatile
tmpfs WALs. These are not equivalent durability configurations. Clients use
CPUs 0-1, voters share 2-5, and observer plus three owned background containers
use 6-15,22-31. Unrelated host services remain unconstrained.

No build, test, fault, profile or independent audit overlaps timing. The
12-cell correctness-only smoke is excluded from performance. Root smoke
session 53845 and timing session 20864 both exit 0. Independent timing audit
session 20099 exits 0 on its first execution. All original predicates are
unchanged; no failed timing recording is discarded or rerun for acceptance.

The audit verifies 80 exited fixture lifetimes, 48 fresh drain/writer bindings,
2,350 resource samples, 2,337 role/source checks and 528 retained files /
88,755,243 bytes. Original configured/effective container CPU masks and
historical namespace identities are restored/preserved. Hosted CI is not run.

Exact helpers, contracts, source/proof/process summaries and accepted readout
are retained in [the evidence directory](../scripts/redis-reference/read-group-credit-v1/README.md).
The complete raw recording is `/tmp/kv9-read-group-credit-matched-diagnostic-attempt1`;
the original preparation is `/tmp/kv9-read-group-credit-comparison-preparation`.

## Next gates

The separate longer comparison retains all four c1 pairs and the observations
above. Exact-source local Chaos acceptance also completes. Neither implies
general promotion or a uniform c1 tail-latency improvement. Exercise
sustained reads, writes/mixed and larger batches: a single outstanding read
context may change fairness and queueing under other loads.

For low latency, continue measuring individual confirmation-path handoffs
before changing task ownership or representation. The earlier direct-body
and watchdog experiments regressed; their results remain part of the decision.
Protocol confirmation, apply coverage and deadline/cancellation authority are
fixed requirements throughout. A broader industrial-readiness or Redis-parity
claim requires evidence beyond this short successful read-only diagnostic.
