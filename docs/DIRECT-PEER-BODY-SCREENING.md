# Direct peer body: rejected at both c1 and c64

Follow-up: the [bounded idle-watchdog candidate](PEER-IDLE-WATCHDOG-SCREENING.md)
completed its matched screen and was also rejected for insufficient throughput
and mean latency, despite better p99. Accepted control `5ee897a` is unchanged.

Reject candidate `6707bcccf15ea788ff231f4263f43a9f73fa63dd` as the next
performance increment. **All eight matched throughput comparisons regress and
all eight mean latencies increase.** GET throughput changes by -4.093% / -3.760%
at c1 and -3.456% / -2.290% at c64. Seven p99 buckets improve, but the first
c1 GET p99 worsens. The accepted production control remains `5ee897a`.

The first complete recording and frozen independent audit pass: all
**27,724,506 measured calls succeed**, with one observed attempt per call and
no dropped slots or non-success outcomes. This accepts the measurement's
correctness and provenance, not the candidate's performance. No broad
workspace/Chaos campaign is run to promote this rejected source.

## Implementation and pre-screening gates

The [committed implementation and source-mapped argument](https://github.com/c4pt0r/kv9/blob/6707bcccf15ea788ff231f4263f43a9f73fa63dd/docs/DIRECT-PEER-BODY.md)
replace the original envelope mpsc and intermediate 16-batch channel with one
mutex-protected bounded queue polled by the active tonic body. A fresh per-RPC
token prevents retained bodies from consuming replacement traffic or replacing
its waker, including same-route reconnect and A/B/A routing. The independent
watchdog handles unpolled bodies; explicit Tokio cooperative accounting bounds
consecutive ready batches.

This changes queue implementation, batching ownership and progress supervision
together; it is not an isolated measurement of the cost of one channel.
Raft quorum, fresh read confirmation, applied-index and exact write checks,
inbound authority, public admission and server runtime settings are unchanged.

Local pre-screening passes **224 kv9-raft tests** (193 library, 19 integration,
12 documentation), Clippy with warnings denied, and **nine source-control
triples / 27 compiled executions**. The final control manifest explicitly hashes
the split-out tests with production source. The first control attempt remains
rejected: a watchdog test asserted while borrowing a MutexGuard, and its expected
mutation failure poisoned the queue and caused cleanup SIGABRT. Copying the
timestamp before the same comparison fixes the test failure path; abnormal
termination was not accepted as an expected failure. Separately, checking expiry
before `select` removes dependence on continuously ready notification ordering.

The independently audited process E2E passes **352 calls: 324 OK, 28 unknown**.
Streaming and unary each record 176 complete-history calls with overlapping
point/batch work, leader loss and original-directory restart. All seven owned
process lifetimes exit; fresh three-voter final drains and exact source,
default-feature and executable bindings pass. Unknown outcomes remain in the
histories. This is process recovery evidence, not power-loss or actual Chaos
Mesh acceptance. The source argument does not complete machine-checked
whole-protocol proof composition or source-level refinement.

## Complete paired comparison

QPS counts successful logical calls. All measured calls succeed here, so
successful and all-outcome latency populations coincide. p99 is an original
histogram interval, not an averaged percentile. Repetitions below are displayed
as 1 and 2; raw artifacts use 0 and 1.

| Concurrency | Repeat | API | Control QPS | Candidate QPS | QPS change | Mean us, control -> candidate | p99 us, control -> candidate |
| --- | --- | --- | ---: | ---: | ---: | --- | --- |
| 1 | 1 | GET | 26,182.9 | 25,111.1 | -4.093% | 38.067 -> 39.692 | 58.368-58.879 -> 60.416-60.927 |
| 1 | 2 | GET | 26,093.9 | 25,112.7 | -3.760% | 38.201 -> 39.698 | 60.416-60.927 -> 57.344-57.855 |
| 1 | 1 | BatchGet(1) | 26,250.8 | 24,809.1 | -5.492% | 37.946 -> 40.169 | 59.904-60.415 -> 59.392-59.903 |
| 1 | 2 | BatchGet(1) | 25,292.0 | 24,872.6 | -1.658% | 39.404 -> 40.063 | 64.000-64.511 -> 58.368-58.879 |
| 64 | 1 | GET | 343,685.9 | 331,809.6 | -3.456% | 186.084 -> 192.747 | 360.448-364.543 -> 344.064-348.159 |
| 64 | 2 | GET | 336,722.6 | 329,012.2 | -2.290% | 189.936 -> 194.392 | 364.544-368.639 -> 352.256-356.351 |
| 64 | 1 | BatchGet(1) | 337,382.4 | 324,271.7 | -3.886% | 189.531 -> 197.200 | 364.544-368.639 -> 356.352-360.447 |
| 64 | 2 | BatchGet(1) | 339,257.2 | 324,900.2 | -4.232% | 188.481 -> 196.819 | 360.448-364.543 -> 352.256-356.351 |

Same-run standalone Redis GET:

| Concurrency | Repeat | GET/s | Mean us | p99 bucket us |
| --- | --- | ---: | ---: | --- |
| 1 | 1 | 170,252.4 | 5.798 | 8.064-8.127 |
| 1 | 2 | 170,294.4 | 5.796 | 8.448-8.575 |
| 64 | 1 | 507,254.6 | 126.055 | 231.424-233.471 |
| 64 | 2 | 502,881.4 | 127.150 | 233.472-235.519 |

The [full 24-arm readout](../scripts/redis-reference/direct-peer-body-v1/READOUT.md)
also retains Redis MGET(1). [Machine-readable statistics](../scripts/redis-reference/direct-peer-body-v1/statistics.json)
preserve every count, nanosecond sum and latency interval. The control accounts
for 7,304,571 calls, the candidate 7,049,750 and Redis 13,370,185. No failed
population is removed to produce the table.

## Frozen protocol and scope

Protocol `kv9-direct-peer-body-c1-c64-v1`, version 2, reuses native client
`03c1c776a5dd7d1cc67491ab253e02ce51665bf8` and Redis client
`b8ec38f660786412705f350d96ab086f0e7f6c60` with their original releases.
Each concurrency runs control GET, control BatchGet(1), candidate GET,
candidate BatchGet(1), Redis MGET(1), and Redis GET. Repetition 0 executes c1
then c64; repetition 1 reverses the entire twelve-cohort order. Native SDK
max-in-flight equals concurrency; server admission bounds remain unchanged.

Every cohort uses five seconds, 128 warmup calls, 4096 keys plus sentinel,
128-byte values, batch size one, read-only closed-loop traffic, the original
1,500-ms deadline and six-attempt native retry configuration. All calls happen
to use one attempt. Redis has one preconnected connection per worker and one
outstanding command, no pipelining, save/AOF disabled and io-threads=1.

Three KV9 voters share CPUs 2-5, clients use 0-1, and observers plus the three
owned background containers use 6-15,22-31. Original configured/effective
0-31 container masks are restored. Historical namespace UIDs and the existing
one-shot fault object are preserved; unrelated host services remain
unconstrained. No build, test, fault, profiling or audit overlaps timing.
KV9 uses volatile tmpfs WAL with normal sync calls; Redis is standalone memory
without replicas. This is a short shared-host diagnostic, not sustained capacity,
equal durability or a cross-host network measurement. No DPDK conclusion follows.

Six driver/client compatibility tests and ten independent auditor contract
tests pass before runtime. A separate twelve-cohort smoke completes 10,605,645
successful calls and cleans up 40 processes; its timing is excluded. The first
timed recording exits 0 in root session **11757**. The first frozen audit exits
0 in **52985**, verifying **80 exited lifetimes, 48 fresh-drain documents,
48 voter/listener bindings, 2,330 source-file checks, 2,345 resource observations
and 528 retained files / 88,755,769 bytes**, final values and sentinels, and
complete environment restoration. No timing or audit was rerun to obtain acceptance.

Raw recording: `/tmp/kv9-direct-peer-body-matched-diagnostic-first/cohorts`.
Independent result:
`/tmp/kv9-direct-peer-body-comparison-preparation/results-first/audit.json`.
[Executed helpers and inventories](../scripts/redis-reference/direct-peer-body-v1/README.md)
retain exact bytes, absolute dependencies, pins and preparation failures.

| Artifact | SHA-256 |
| --- | --- |
| Candidate server | `1c4ceb3b95ecca2ad900f65641c73ad097a13afbeba5304b8b8ac63358ff3a63` |
| Candidate build manifest | `029b7653ff227d4f4e15ef1de1236da2b6a824ca7e3cf0ab27ee3ac54640ee9b` |
| Timing driver | `7be1ffee291a28c7500f6e1ad58f5f6dea34befcf69eff2d48c56db86796425a` |
| Independent auditor | `2c182758cdcb0668d133e9efc1adc06d3b610048917d23832607294543739d62` |
| Independent result | `fcb7c01b52cf83870449aa98e6e43517cec9600666eb364b715ccffb38735f90` |
| Scalar statistics | `f537a4a2041bdecbde61fe52eb6ec8aa7f23129a740e56208b6838013735b2a6` |
| Process E2E audit | `db82db752129df4ff45ba437a563fdd925034a60fb7b7f08c6d69e27979ab30b` |

## Next implementation hypothesis

Source inspection identifies a remaining scheduling edge: an empty-to-nonempty
`Sender::try_send` wakes the tonic body **and** notifies its owner watchdog.
Removing the batch channel did not remove this supervisor wake. This is not
measured attribution of the regression: mutex costs, batching and scheduling
also changed in the candidate.

Investigate replacing producer-to-watchdog notifications with bounded idle
checks, retaining lifecycle notifications. If the empty queue is inspected at
time `t` under its mutex, an idle check at `t+B` precedes or equals the deadline
`e+B` for a subsequent first enqueue at `e>=t`. Once backlog is observed, its
actual progress timestamp remains authoritative. The watchdog must stay
independent of body polling, recheck current state on timer completion and
never reconnect merely because an idle check fires. `B` remains three seconds;
clock capture and queue inspection must share the lock.

This removes one explicit notification edge for such bursts, not every owner
wake. It needs a separate source argument, real enqueue-to-expiry coverage and
negative controls before another c1/c64 comparison. The existing immediate-wake
test cannot simply be kept with an artificially backdated enqueue that violates
`e>=t`. No benefit or follow-on acceptance is claimed. Raft consistency,
writes/mixed performance, broad fault acceptance, Redis parity and automatic
splits remain open.
