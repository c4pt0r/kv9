# RawKV performance development path

Tracking: #13, #20 and #9. The product target is Redis-class performance for
memory-resident RawKV data. Data residency, durable acknowledgement, replication
and API overhead are separate dimensions. Performance work must improve the
implementation while keeping each measured mode's guarantees explicit.

## Current evidence

Main now includes the accepted scheduling/completion/socket lineage through
`a9510e2`, whose runtime/build inputs match candidate `cc8bc87`. Its
[composition and local/fault acceptance](RAFT-SCHEDULING-ACCEPTANCE.md) cover
this integration. The later append, Raw and Ready grouping implementations
remain candidates, and their measurements below are identified separately.

The latest measured memory-path candidate is sealed read groups `2cbbe26`, built
on asynchronous read preparation `1ad259e`. In its fresh c64 tmpfs bracket,
GET reaches 120,359/s, PUT 67,202/s and mixed 79,767/s. GET is approximately
24% of its contemporaneous standalone Redis reference. This is a modest local
gain, not attainment of the Redis-class target or runtime promotion. The full
comparison and remaining correctness gates appear under increment 4 below.

The earlier performance development baseline was the `892b2a1` Ready candidate. At 64
outstanding calls, pooled GET throughput is 84,810/s and PUT throughput is 836/s;
the corresponding standalone Redis memory reference is 500,842/s and 491,739/s.
The two kv9 PUT repetitions are 854/s and 818/s. The previous fe candidate pooled
734/s with repetitions of 958/s and 514/s, so this comparison alone cannot
establish a repeatable 14% improvement. These short shared-host runs establish
a large remaining gap, not a stable capacity. See the
[retained report](../scripts/redis-reference/results/fe650ed-892b2a1.md).

In the single-outstanding-write sample, each voter performs two Raft syncs and
one engine sync per acknowledged write across the observed trial interval;
mean sync times are approximately 5-8 ms. In the concurrency-64 write trials,
the three servers together use approximately 0.12-0.14 CPU cores. Those counters
show substantial waiting, although they do not alone assign every millisecond
to a cause. Concurrent reads consume about 3.61 aggregate server CPU cores out
of four allowed CPUs. A storage-only optimization cannot explain away that read
CPU cost.

The independent [volatile tmpfs diagnostic](../scripts/redis-reference/results/892b2a1-tmpfs-diagnostic.md)
uses the same Ready executable and protocol code, with explicitly volatile voter
directories and a shorter concurrency sweep. At concurrency 64 it observes
74,657 GET/s and 59,534 PUT/s, with approximately 3.6 aggregate server CPU cores
and client CPU headroom. All 24 trials pass complete outcome and storage-identity
audits. The large write difference supports work on the storage wait, while the
remaining CPU cost supports optimizing the protocol path. Different substrates,
trial order and accumulated work preclude treating the ratio as a precise causal
speedup. tmpfs supplies no disk durability or power-loss guarantee.

Source inspection identifies two concrete follow-up candidates:

- `RaftPeer::process_ready` holds the peer mutex across synchronous persistence.
  `propose_in_term`, incoming message processing and Ready persistence share that
  mutex. Submission can therefore wait behind device latency.
- Quorum read confirmations are retained in a bounded vector and looked up by
  scanning from its beginning. The bound is 1,024 receipts. This is avoidable
  lookup work. The completed profile and indexed experiment below distinguish
  its measured CPU population from an established end-to-end gain.

## Measurement lanes

1. **Durable three-voter comparison.** Keep the existing clients, 64 hot keys,
   128-byte values, concurrency sweep, two reversed-order repetitions, admission
   limit and full outcome accounting. Record actual executable/source identities,
   latency histograms, CPU and residual work. This remains the performance lane
   for the current database guarantee.
2. **Volatile-storage diagnostic.** Run the same candidate with explicitly owned
   tmpfs voter directories, keeping protocol and acknowledgement code unchanged.
   Label it diagnostic-only from artifact creation. tmpfs does not satisfy the
   durable storage/power-loss premise. Its results estimate performance when
   device synchronization is removed; they cannot be substituted for durable
   throughput, a production mode, or physical power-loss evidence.
3. **Cross-host and sustained acceptance.** After the local bottlenecks are
   resolved, repeat on separate hosts/storage devices, with controlled background
   load, larger datasets and longer windows. Add open-loop offered-load sweeps
   for queueing and tail latency. Closed-loop three-second trials cannot establish
   a production latency SLO.

The speed reference is useful even though its durability differs. Keep the
ratios visible and report the actual guarantee beside each result. No throughput
increase from early success replies, stale reads, hidden refusals, changed value
sizes, or unreported operation budgets counts as progress.

## Ordered implementation increments

### 1. Finish the current synchronous batching candidate

`892b2a1` combines original Ready entries and HardState into one sync. It retains
separate LightReady commit persistence. The candidate passes 608 local tests and
doctests, four Ready implementation-control triples, three append-control triples
and the exact-source three-process failover/restart fixture. Its [recovery
argument](https://github.com/c4pt0r/kv9/blob/892b2a178450309859113c942f1738c070130eb5/docs/RAFT-READY-GROUP.md)
spells out the remaining parameterized composition/refinement gate.

The full 60-trial comparison with fe and its independent audit are complete.
Joint Ready frequency, group occupancy and synchronization counts matter;
entry-only and HardState-only calls do not save a sync. Candidate-specific MinIO,
actual Chaos Mesh and checked composition remain open. Earlier source revisions'
evidence stays separate.

### 2. Bound and batch proposal submission before persistence

The implementation is now pushed separately as
[`95fb5cd`](https://github.com/c4pt0r/kv9/commit/95fb5cd968411009a41ba4f7ed287cd6597115fa).
It reserves at most 128 requests/64 MiB of encoded commands per owner and
consumes at most 64 inspections per turn, including cancelled entries, with a
1 MiB soft turn target. Reservations include in-progress handoffs. Its
[contract and local evidence](https://github.com/c4pt0r/kv9/blob/codex/raft-proposal-queue/docs/RAFT-PROPOSAL-QUEUE.md)
cover atomic cancellation/claim, typed pre-submission refusal, exact receipt
identity, term checks, failure fencing and independent workload accounting.
Local validation passed 626 Rust tests/doctests (23 ignored), all-target Clippy,
eight compiled control triples, default three-process failover/restart and the
actual MinIO persistent-workload functional suite with twelve report-corruption
controls. The candidate remains outside the main runtime. Parameterized proof
composition, actual Chaos Mesh, overload/service fairness and repeated
performance acceptance remain open. The completed
[targeted c64 comparison](../scripts/redis-reference/results/892b2a1-95fb5cd-c64-diagnostic.md)
observes a local disk PUT increase from 838 to 1,018 operations/s, but lower
tmpfs PUT (67,160 to 61,318/s) and mixed throughput (69,081 to 63,326/s).
All 48 cohorts passed independent outcome/identity checks after retained
observer-environment failures were corrected without timing reruns. This is
not a general throughput improvement. Keep the queue as an experiment and
continue the memory read-path mainline from Ready `892b2a1`.

The acceptance contract requires explicit count and byte limits. Admission and queueing must not
require the mutex held during a device sync. Drain already available work under
an explicit per-turn budget so one busy producer cannot starve messages or ticks.
Do not add an unconditional sleep merely to form a batch.

Preserve the distinction between queue acceptance, assigned `(term, index)`,
durable quorum commitment and the exact applied receipt. Check leadership and any
expected planning term at actual Raft submission. A queued request canceled before
dequeue may be a definite refusal only if cancellation wins an atomic handoff;
once submission can have happened, a caller timeout is an unknown outcome. Keep
the original deadline and reservation ownership through that distinction.

Exit evidence includes grouped submission under concurrent callers, bounded
bytes under overload, no tick/message starvation, and leader-change/cancellation
controls that cannot misclassify a possibly applied command as safe to retry.

### 3. Overlap persistence with protocol work under an explicit state machine

Move blocking storage service off the Raft owner only with ordered persistence
tokens tied to the peer lifetime and Ready identity. An old or out-of-order
completion must never advance the current peer. Bound pending entry bytes, jobs
and completion records; stop admission before exhausting those bounds.

Raft advancement, outbound acknowledgement eligibility, committed delivery,
engine publication and client receipt eligibility must retain their required
durability order. Preserve the later LightReady commit requirement. Engine work
must remain in committed order, with metadata/configuration barriers and epoch
adjudication intact. Failed persistence fences the affected lifetime, including
already queued work; it does not become a transient retry inside that lifetime.

Model the pipeline and prove its invariants and conditional progress before
acceptance. Exercise delayed/reordered completion, old-lifetime callbacks, short
writes, EIO/ENOSPC, shutdown and leader changes. Use actual Chaos Mesh histories
to verify the integrated path. The scheduler remains per owner/group; this work
must not introduce an indispensable cluster-wide batching service.

### 4. Reduce the measured read-path CPU cost

The exact Ready [GET CPU diagnostic](../scripts/redis-reference/results/892b2a1-get-cpu-profile.md)
is complete. In the kernel-inclusive recording, RPC/framing/serialization/buffer
leaves account for 21.58% of selected samples, generic allocation/copy/comparison
for 14.87%, and receipt vector search for 7.02%. Actual stacks also show
completion publication and blocking-pool dispatch waking threads through
futexes. Inclusive stack populations overlap; optimized/async unwinding and
software sampling limit attribution. This supports reducing RPC/allocation and
blocking wakeup/dispatch costs alongside lookup work; it is not a causal speedup
estimate. Both instrumented cohorts retained complete successful outcomes and
zero residual admission occupancy.

Follow-up changes include fewer blocking dispatches, duplicate context
decodes/allocations, and bounded read-barrier batching. Each change gets its own
unchanged-protocol comparison so CPU and latency effects can be attributed.

The first indexed-receipt implementation is pushed as `73ddb0d`. It passes 611
local tests/doctests, three implementation-control triples, the default-build
three-process failover/restart fixture and a separately identified
`partition-testing` typed read-refusal fixture. Its completed, independently
checked [60-trial comparison](../scripts/redis-reference/results/892b2a1-73ddb0d.md)
does not establish an end-to-end gain: c64 GET is 79,067/s versus 84,810/s for
Ready, and results vary across concurrency levels while Redis also drifts lower.
The candidate remains an experiment and is not selected for runtime promotion.
Continue the performance mainline from Ready, using the profile to prioritize
further changes. The
[representation argument](https://github.com/c4pt0r/kv9/blob/73ddb0db123d5ee5cecfbe95bdb476e30f6380bc/docs/READ-RECEIPT-INDEX.md)
preserves FIFO retention and first-match duplicate behavior for arbitrary finite
histories. Its new process checks do not replace actual Chaos Mesh.

An indexed receipt cache must preserve exact context identity, FIFO retention,
duplicate-context behavior and the existing unknown result after eviction. A
read batch must seal membership before initiating its quorum confirmation: a
later caller cannot inherit an arbitrarily old confirmation. Each successful
caller still waits for local apply coverage and acquires its authorized engine
view afterward. Retain stale-leader partition and committed-but-unapplied
counterexamples. Lease reads would require a separate protocol and timing
contract and are outside this increment.

The next candidate, [asynchronous point-read preparation](https://github.com/c4pt0r/kv9/blob/1ad259e78c148b0b6b8d837b142a2e9521b60b3f/docs/ASYNC-READ-BARRIER.md),
is pushed as `1ad259e` from the Ready baseline. Public GET registers a bounded
owner-serviced ReadIndex request and awaits an individual completion without
occupying a blocking worker during quorum wait. One blocking engine job then
consumes the same public admission reservation and the existing established,
context-checked view. It retains exact contexts, apply coverage, and normal
Raft/read semantics; no lease or new service is introduced.

Its initial local gate passes 622 tests/doctests, seven compiled semantic-control
triples, warnings-denied Clippy, and the default-feature three-process
failover/restart fixture. Product tests cover committed-but-unapplied data and
both epoch halves changing between preparation and engine execution. Checked
protocol composition and actual Chaos Mesh on the new candidate remain open;
the master runtime is unchanged. Performance selection requires an independently
checked comparison with the same client and workload, including write/mixed
regression checks and fresh before/after Ready measurements.

The fresh c64 volatile tmpfs bracket passes all 36 cohorts and independent
outcome, identity, CPU-placement and queue-drain checks. Successful throughput
(operations/s), pooled across the two repetitions within each matrix:

| Workload | Ready before | Async candidate | Ready after |
|---|---:|---:|---:|
| GET | 86,172.5 | 112,459.5 | 86,424.3 |
| PUT | 67,151.7 | 67,347.6 | 66,927.3 |
| Mixed | 68,886.6 | 75,992.7 | 68,905.5 |

This supports approximately 30% higher GET and 10% higher mixed throughput in
this local bracket, with unchanged PUT throughput. Every measured operation
succeeded, and all three replicas' asynchronous queues and reservations drained
after verification. Candidate GET p99 moves to the 0.524-1.049 ms histogram
bucket from the Ready matrices' 1.049-2.097 ms bucket, while total GET voter CPU
falls to 3.15-3.23 cores from 3.59-3.62. The paired Redis GET reference is
494,564/s, leaving the candidate at approximately 23% of that reference.
See the [complete bracket report](../scripts/redis-reference/results/892b2a1-1ad259e-c64-tmpfs-diagnostic.md).
The initial bracket was rejected for incorrect inherited
CPU placement; a subsequent preflight-only observer failure was also retained.
Only the corrected, independently checked bracket contributes these numbers.
This is volatile-storage evidence, not disk-durability, cross-host capacity,
or complete protocol/fault acceptance. The remaining Redis gap still requires
work on bounded quorum-read grouping and RPC/allocation cost.

The initial local correctness archive is
`target/correctness-evidence/2026-09-09-1ad259e-async-read-local-second.tar.gz`
(57,192,174 bytes; 644 entries). All members were read back and hash-verified;
SHA-256 is `8dad4a64cabb7b5db649f2716faecb2b9ed0f8e37364f6ef465475de0d96467a`.
It retains source, default executable, tests, control inputs/logs, process stores,
and failed early attempts. It contains no new Chaos Mesh acceptance.

The exact `1ad259e` candidate subsequently passed one
[actual NetworkChaos isolation/recovery scenario](ASYNC-READ-NETWORKCHAOS.md).
Positive effect probes showed the old leader isolated from both remaining
voters while the majority remained connected. The majority acknowledged v2,
the same live former leader returned typed NotLeader, and recovery returned v2.
All 244 public operation records were independently checked, including 26
conservatively unknown outcomes. This is post-deposition, single-host evidence;
full fault coverage and protocol composition remain open.

The next implementation is [sealed read groups](https://github.com/c4pt0r/kv9/blob/2cbbe26a3d4273c6d265fa40c8b56c548b1a59ec/docs/READ-GROUPS.md),
pushed as `2cbbe26` from `1ad259e`. It seals at most 64 already invoked readers
before one quorum request, retaining individual deadlines/reservations and an
independent group identity when its representative cancels. A three-voter Raft
test observes one heartbeat per follower for three grouped readers and requires
a fresh confirmation for a later reader. The local gate passes 627 tests and
doctests, 11 compiled control triples, warnings-denied Clippy, and the default
three-process failover/restart fixture. Its completed fresh
[unchanged-client bracket](../scripts/redis-reference/results/1ad259e-2cbbe26-c64-tmpfs-diagnostic.md)
passes all 36 cohorts and six independent outcome/tmpfs checks. Pooled successful
operations/s across each matrix's two repetitions are:

| Workload | Async before | Sealed groups | Async after |
|---|---:|---:|---:|
| GET | 112,020.0 | 120,358.6 | 112,983.8 |
| PUT | 66,776.4 | 67,202.1 | 67,366.0 |
| Mixed | 75,655.2 | 79,766.8 | 76,276.3 |

Relative to the mean of the two surrounding baseline rates, this is about 7%
higher GET and 5% higher mixed throughput, with no meaningful PUT gain.
All measured operations succeeded. The GET p99 histogram bucket remains
0.524288-1.048575 ms and the PUT/mixed bucket remains 1.048576-2.097151 ms;
the comparison does not establish a tail-latency improvement. GET voter CPU is
3.095/3.190 cores in the candidate repetitions, versus 3.168/3.236 before and
3.141/3.217 after. The paired Redis p99 bucket is 0.131072-0.262143 ms.

GET group membership averages 2.329/2.545 requests per admitted group, and mixed
averages 3.137/3.146. These counter deltas include setup, warmup, measurement and
verification; they are not measurement-window-only batching statistics. The
maximum admitted group is a process-lifetime peak of 64. All async groups,
members and public reservations drain, and all 63 owned process lifetimes exit.
Reducing logical heartbeat broadcasts therefore yields a measured but small
end-to-end gain. The next memory-path work should reduce per-request execution
and RPC cost while retaining the same quorum/apply/view and admission contracts.
Do not infer another speedup from the earlier Ready profile without measuring
the changed path. Every prepared GET still dispatches one blocking engine job.

The local correctness archive is
`target/correctness-evidence/2026-09-09-2cbbe26-read-groups-local-first.tar.gz`
(59,083,888 bytes; 827 entries), SHA-256
`bae2d5a4410c8bf0b9c4f2f740bc205ad92075d13a082e26b7375f82fb5ede87`.
All archived members were read back and hash-verified, and original inputs
remained unchanged. It includes both the rejected first control attempt and
the accepted second attempt. Performance artifacts are inventoried separately.
Machine-checked group composition and full candidate fault coverage remain
open. The exact `2cbbe26` runtime subsequently passed one
[actual NetworkChaos isolation/recovery scenario with multi-member groups](READ-GROUP-NETWORKCHAOS.md).
Inside the installed partition, the majority admitted 241 members through
120 groups, with a maximum group of 17. The same live old leader refused a
point read after the majority acknowledged a new value; healing restored fresh
reads and drained ledgers. A 246-operation persistent history and 355-operation
CLI history were independently checked with their distinct clock scopes.
This one-host, post-deposition case does not establish full fault coverage or
apply to the later resident-read candidate. Master runtime is unchanged;
tmpfs performance results remain volatile-storage diagnostics.

The next implementation candidate is
[resident point-read execution](https://github.com/c4pt0r/kv9/blob/11cae977f15df0912c2a35561480d45447bae660/docs/RESIDENT-READ.md),
pushed as `11cae97` from `2cbbe26`. After the same quorum/apply barrier, the
concrete memory-indexed engine tries its lifecycle and snapshot locks. An
uncontended GET completes on an owned memory view without dispatching a blocking
job; contention transfers the original credential and reservation to the existing
blocking path. Both use the same context/epoch gate and data view. An unused
full-driver status query is also removed from prepared GET execution. Generic
engine contracts and other APIs retain their existing blocking boundaries.

This candidate passes 630 workspace tests/doctests (23 ignored), warnings-denied
all-target Clippy, four compiled semantic-control triples, and the default-feature
three-process failover/restart fixture to term 2/index 14. Tests distinguish an
already completed old-view read from a queued job that must reject a now-stale
epoch; they also cover committed-but-unapplied writes, writer contention and a
completed public GET while the sole blocking worker is occupied. The initial
workspace failure was an obsolete 13-line status assertion after adding two
counters; the corrected full run passed, and the original failure is retained.

No performance improvement is claimed until a fresh unchanged-client
groups-before/resident/groups-after bracket is independently checked. The new
inline-success and blocking-submission counters will distinguish actual execution
paths. Exact-candidate fault evidence and machine-checked composition remain open;
master runtime is unchanged.

### 5. Reconcile improvements before extending capacity

Publish throughput, successful-operation latency, refusal/unknown counts,
per-voter CPU, batch occupancy and storage-stage observations together. Increase
admission or add multi-group parallelism only after demonstrating bounded
resources and useful scaling; a larger queue alone does not improve service
capacity. Keep the original roadmap's storage/retention and multi-group gates.

For every completed increment, link the exact implementation and evidence on
#9 and #20. A candidate benchmark or scoped proof does not complete an entire
roadmap item. Commit messages and GitHub records remain English. Daily checks
run locally; hosted CI is reserved for releases or key milestones.
