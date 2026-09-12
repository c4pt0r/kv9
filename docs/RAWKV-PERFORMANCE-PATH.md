# RawKV performance development path

> Latest direction: [ThinLTO qualification](RELEASE-THIN-LTO-PERFORMANCE.md)
> improves c64 GET 8.511%, c1 GET 6.402% and c64 mixed throughput 10.424%, with
> better means and p99 in both orders. Freeze `02d0c01` for broader API and actual
> exact-build Chaos Mesh gates; CRC remains selected. Consult the
> [experiment index](PERFORMANCE-EXPERIMENT-INDEX.md) before another candidate.
> Redis read parity and industrial gates remain open.

Tracking: #13, #20 and #9. The product target is Redis-class performance for
memory-resident RawKV data. Data residency, durable acknowledgement, replication
and API overhead are separate dimensions. Performance work must improve the
implementation while keeping each measured mode's guarantees explicit.

## Product sequence, updated 2026-09-10

First bring client-visible memory RawKV throughput to Redis-class performance,
then implement dynamic multi-Raft and automatic range partitioning/splits.
The current GET/PUT/mixed rates remain below that target. Evaluate the same
payloads, concurrency and resource budget with repeated paired Redis runs;
report latency, refusals and unknown outcomes alongside throughput. Raft quorum,
linearizable reads and committed/applied write acknowledgements are mandatory.
Keep volatile-memory diagnostics separate from durable-storage results.

After that performance gate, execute the existing scale-out work in this order:

1. #22: a bounded RegionManager that creates, recovers and schedules independent
   Raft groups dynamically, with shared transport and fair resource accounting.
2. #23 and #24: range-aware routing with epoch fencing, learner attachment and
   recoverable membership/ownership changes. Stale routing must never admit a
   write to an obsolete owner; unknown writes must not be retried blindly.
3. #25: size/load-triggered automatic splits with durable intent, fenced parent
   and child ownership, data handoff and idempotent crash recovery. Establish
   split safety and conditional progress with source-mapped proofs and actual
   Chaos Mesh histories under leader loss, partition and restart.
4. #27: demonstrate balanced placement and throughput scaling with multiple
   groups and failure domains; retain #26's separate merge/recovery dependency
   before claiming the complete scale-out package.

Metadata and scheduling authority must be replicated or safely replaceable;
there must be no service-critical singleton except the object-store dependency.
Existing snapshot, retention and storage prerequisites remain required before
accepting scale-out. This sequence does not mark those prerequisites complete.

## Current checkpoint

The [current status](CURRENT-STATUS.md) is authoritative for runtime selection.
Selected behavior remains CRC `ca0002c7`. The [ThinLTO screen](RELEASE-THIN-LTO-PERFORMANCE.md)
completes all 24 timed cohorts: c64 GET +8.511%, c1 GET +6.402% and mixed
throughput +13.891% at c1 / +10.424% at c64. Mean/p99 improve in both orders,
including separate mixed GET/PUT. The candidate reaches 376,202 GET/s at c64;
isolated GET mean is 35.226 us, about 6.21 times the same-run Redis mean.

Freeze `02d0c01` for broader point/batch API checks and actual exact-build Chaos
Mesh. Its [source and ordinary recovery validation](RELEASE-THIN-LTO-VALIDATION.md)
is complete within the documented scope. Separate notification candidate
`42e0117` remains experimental; combining it with ThinLTO requires a distinct
matched qualification. Fresh ReadIndex, sealed groups, successful pump/apply/view
fences and durable writes remain mandatory. Historical rejected scheduling
screens remain rejected; review the index and all branch history first.

## Earlier development evidence

**Streaming gRPC with tonic is selected** for the next point-transport integration
and performance work. The [same-artifact five-arm comparison](../scripts/redis-reference/results/40e813f-streaming-rpc-c64-tmpfs-diagnostic.md)
at clean `40e813f` measures 208,849 GET/s, 96,078 PUT/s and 123,137 mixed
operations/s: +10.56%, +2.06% and +5.42% versus surrounding tarpc, and +65.71%,
+29.26% and +40.71% versus surrounding unary. Both repetitions exceed all
surrounding control repetitions for every workload. The independent audit
passes all 60 cohorts, 10 full-history guards and 5,411,907 measured KV9 successes
without measured retries. Redis remains 2.38x, 4.99x and 3.95x faster.
This is a short single-host volatile-memory diagnostic, with the same 1.5-second
protocol and a new clean build. Integrate the selected stream into the ordinary
service/client path and complete exact-source actual Chaos/proof gates before
default runtime promotion. Then profile and optimize the selected Raft/WAL write
path; retain the other transports as reproducible controls.

The [bounded streaming-gRPC control](https://github.com/c4pt0r/kv9/blob/40e813f/docs/RPC-STREAM-CONTROL.md)
is now committed on the experiment branch. It shares the existing handlers and
Raft semantics, correlates multiplexed responses, bounds unfinished/buffered work
and closes uncertain stream generations without replaying writes. Fifteen
focused transport tests pass, alongside 670 feature-enabled and 645 default
workspace tests/doctests (23 ignored in each workspace run). The independently
audited three-transport leader-loss/restart run validates all 495 history
operations, including 43 retained unknown outcomes, and fresh final resource
drain. Exact integrated-source actual Chaos remains open; the selected stream
has not yet replaced the default production transport.

The new opt-in [RPC framework comparison](../scripts/redis-reference/results/b49a2f6-rpc-framework-c64-tmpfs-diagnostic.md)
uses the same clean `b49a2f6` feature-enabled release client/server for both unary
gRPC and tarpc/TCP. In its repeated c64 tmpfs bracket, tarpc reaches 185,665 GET/s,
93,692 PUT/s and 115,714 mixed operations/s: +48.07%, +26.31% and +32.98% against
the surrounding unary arms. The paired Redis reference remains 2.67x, 5.16x and
4.23x faster. All 36 measured cohorts completed, with zero failed/unknown/refused
operations or extra measured KV9 attempts. This is a new 1.5-second diagnostic;
its feature build, client and window differ from the older a00e comparison.
This earlier tarpc comparison remains a retained reference. The later five-arm
comparison selects streaming gRPC, with integration and exact-source
Chaos/refinement work still required. Redis-class performance remains the
prerequisite for the next dynamic multi-Raft/automatic-split mainline stage.

Main now includes the selected runtime through asynchronous exact-apply waiting
and process-status identity, integrated in `2627825`. All 131 runtime/build-tree
files match exact `23bc58b`: the inherited append/Raw/Ready grouping, asynchronous
read preparation, sealed read groups, resident GET execution and asynchronous
write waiting are included. The [integration record](ASYNC-WRITE-MAIN-INTEGRATION.md)
records fresh local checks and the exact-source [full Chaos Mesh acceptance](ASYNC-WRITE-CHAOS-ACCEPTANCE.md).
Unselected proposal-queue, indexed-receipt, FIFO, serialization, CRC and RPC
experiments remain separate; retained structures follow the exact selected source.

This is a staged integration after correctness validation. Whole-lineage
machine-checked composition, Redis-class performance and broader production
acceptance remain open. Earlier [scheduling/completion proofs](RAFT-SCHEDULING-ACCEPTANCE.md)
and the [Raw group composition proof](RAW-GROUP-PROOF.md) keep their stated source
boundaries. Development-time gate statements below retain their original scope;
the linked integration record states the current runtime selection.

The latest measured memory-path implementation is asynchronous exact-apply waiting
`a00e39f`, built on resident GET execution `11cae97`. In its fresh c64 tmpfs
bracket, GET reaches 136,171/s, PUT 78,501/s and mixed 92,712/s. PUT improves by
18.4% and mixed by 10.6% against the pooled surrounding resident baseline;
GET differs by -0.26%. Both candidate PUT repetitions exceed all four baseline
repetitions. PUT is still approximately 16% of its contemporaneous standalone
Redis reference, so the Redis-class target remains open. The comparison appears under increment 4 below; `23bc58b` adds only diagnostic
process identity and is not relabeled as a new performance measurement.

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

## Earlier implementation increments

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

The completed fresh
[groups-before/resident/groups-after bracket](../scripts/redis-reference/results/2cbbe26-11cae97-c64-tmpfs-diagnostic.md)
passes all 36 cohorts with complete successful outcomes and drained registries.
All 63 owned process lifetimes exit. Pooled successful operations/s are:

| Workload | Groups before | Resident candidate | Groups after | Candidate's paired Redis |
|---|---:|---:|---:|---:|
| GET | 119,586.6 | 136,227.1 | 118,208.8 | 503,462.3 |
| PUT | 66,810.7 | 66,203.1 | 67,368.8 | 491,936.4 |
| Mixed | 79,782.8 | 83,435.7 | 80,012.8 | 499,728.3 |

Relative to the pooled surrounding baseline, GET improves by 14.6% and mixed
by 4.4%, while PUT decreases by 1.3%. Preserve that write result; the experiment
does not establish a no-regression claim. The candidate GET repetitions are
136,887/135,567 operations/s. GET voter CPU is 3.091/3.150 aggregate cores, and
client CPU is 1.413/1.397 of two allowed cores. GET p99 remains in the
0.524288-1.048575 ms bucket; PUT/mixed remain in 1.048576-2.097151 ms. Redis
remains in 0.131072-0.262143 ms. There is no p99 bucket improvement.

Whole-trial inline success accounts for 99.8771%/99.8909% of the observed GET
execution-path counters, and 98.9644%/98.8646% for mixed traffic. The nonzero
fallback is expected under the try-lock contract. These counters include setup,
warmup and verification; they are not measurement-only hit rates. An additional
observer initially assumed zero fallback and was rejected. Its source/log are
retained, and only that observer was corrected to validate the intended branch
accounting and drained boundaries. No cohort, runtime or shared validator was
changed or rerun to address the observer error.

The local correctness archive is
`target/correctness-evidence/2026-09-10-11cae97-resident-read-local-first.tar.gz`
(57,614,894 bytes; 624 entries), SHA-256
`3cfcf7f686d76703e4db983566a2b8e2c9c9ba2a194d462c38ac3c32fb8ec826`.
Every member was read back and verified, with original inputs unchanged.
Performance evidence is inventoried separately.

The exact resident `11cae97` [full Chaos Mesh acceptance](RESIDENT-READ-CHAOS-ACCEPTANCE.md)
now passes all 21 original fault windows and independent full-history/effect
checks. The accepted second run contains 5,109 CLI operations (4,650 successful,
453 unknown, six refused) and 1,405 persistent-client operations (1,378
successful, 23 unknown, four refused). Every invocation has a terminal record;
unknown writes remain unknown and are not retried. Actual fault coverage includes
formation/seed loss, every voter's Pod faults, partition/admission pressure,
delay, all six EIO/ENOSPC cases, missing logs, replacement PVCs and endpoint
migration with the retained original stores.

A separate PID-aware observer attests 35 sampled runtime lifetimes and 468
wrapped-child samples. It binds actual child executables to Pod UID, status PID
and process start ticks before/after observation. Positive resident-read counter
growth occurs in the partition, delay and recovered endpoint envelopes with
independently checked persistent GETs; this is envelope-scoped activity, not
per-response attribution. All four final public/read-group ledgers drained,
owned fixtures exited/cleaned, and eight preexisting namespace UIDs remained
unchanged. The first full matrix and three rejected extra observer audits are
retained separately: its original history/effect checks passed, but that extra
observer's hard-coded limits and PID-1-only attestation were incomplete. Only the
owned observer was repaired before running the unchanged complete second matrix.

This evidence applies to exact resident `11cae97` on one Kind host, not the later
asynchronous-write candidate, cross-host loss or physical power failure.
Machine-checked protocol composition remains open; master runtime is unchanged.

The next performance priority is the write path: it remains around 66,000/s
against a roughly 492,000/s Redis memory reference in this bracket. The exact
resident [PUT CPU profile](../scripts/redis-reference/results/11cae97-put-cpu-profile.md)
is now complete: 5,203 usable measurement samples and 427,101 successful PUTs.
Allocation/copy leaves account for 23.91% and RPC/framing/buffer leaves for
18.60%. Actual partial stacks identify command-buffer growth and completion /
blocking-worker wakeups. Most recovered allocation stacks have no KV9 caller;
these broad populations cannot be assigned wholesale to serialization or used
to predict a speedup. This instrumented tmpfs recording overlaps the separate
proof gate and is not throughput acceptance.

The first write allocation candidate is
[`fb25950`](https://github.com/c4pt0r/kv9/commit/fb2595030f7cc6d7e12a81afa13a027545bc9afe).
It moves an already owned RawKV batch into the same fenced command without
copying key/value buffers, and reserves the command/WAL payload capacity before
running unchanged encoding loops. Its
[representation argument](https://github.com/c4pt0r/kv9/blob/fb2595030f7cc6d7e12a81afa13a027545bc9afe/docs/WRITE-SERIALIZATION.md)
proves ordered-effect and encoded-byte equality under successful allocation;
it does not replace the inherited core-protocol proof gates. Proposal retries,
unknown outcomes, apply receipts, storage syncs and public admission ownership
remain unchanged.

Local validation passes 631 workspace tests/doctests (23 ignored), all-target
Clippy with warnings denied, and the exact default-binary three-process
failover/delete/restart fixture. All four observed process lifetimes executed
the recorded binary and exited. The retained correctness archive is
`target/correctness-evidence/2026-09-10-fb25950-write-serialization-local-first.tar.gz`
(56,064,677 bytes; 465 entries), SHA-256
`9d031a07bc31092cfdac5f62d792607e28538f194995c57c20c9c9d611867cc2`;
all members and original inputs were independently read back. The release
executable has SHA-256
`e187af09941fab74098aa138ddeb0cfd87899cd0926088618ace2687825b30be`
and default features.

The completed [parent / candidate / parent bracket](../scripts/redis-reference/results/11cae97-fb25950-c64-tmpfs-diagnostic.md)
observes only small differences:

| Operation | Resident before | Serialization | Resident after | Paired Redis |
| --- | ---: | ---: | ---: | ---: |
| GET | 135,445.4 | 137,157.4 | 137,102.0 | 509,076.9 |
| PUT | 67,009.5 | 68,010.4 | 66,605.2 | 493,819.5 |
| Mixed | 83,973.2 | 85,526.8 | 83,883.2 | 501,555.3 |

Rates are successful operations/s in the same explicitly volatile c64 tmpfs
diagnostic. Against the pooled surrounding baseline, GET differs by +0.65%,
PUT by +1.80%, and mixed by +1.91%. Candidate GET essentially equals the final
baseline; candidate PUT repetitions overlap the baseline repetition range.
This does not establish a meaningful or general throughput improvement.
All 5,189,120 KV9 and 26,992,217 Redis measured operations succeeded; all 36
cohorts and 63 process lifetimes passed the independent identity/outcome/drain
checks. All observed p99 buckets remain unchanged. Candidate GET is still only
about 27% and PUT about 14% of the contemporaneous standalone Redis reference.

Retain serialization as an isolated representation experiment without a
performance-promotion claim. Continue the performance mainline from resident
`11cae97`, prioritizing write completion and blocking-worker wakeup costs.
Any asynchronous write wait must retain exact receipt/fence verdicts and the
original deadline across replacement retries; cancellation after submission
must not release admission capacity while its work can still be running.
Candidate-specific actual Chaos Mesh and inherited checked composition remain
open. Prioritize exact resident fault acceptance while those larger read gains
are being evaluated; neither experiment has been promoted into the main runtime.


The next write-wait implementation is now published as
[`a00e39f`](https://github.com/c4pt0r/kv9/commit/a00e39f9f8da7bb381df19afd2eed63e8a1f1b75),
starting from resident `11cae97`. Point PUT, batch PUT and point DELETE retain
blocking validation/submission, then release the worker while awaiting the exact
apply result from the existing Raft owner. A bounded 128-reservation registry
counts submission, queue and extracted handoff ownership. The same receipt
inspection and settlement functions serve synchronous and asynchronous callers.
Only a known-not-applied replacement retries the same command with the original
absolute deadline. A private task retains the public reservation across normal
RPC cancellation until the logical result/deadline; the proposal can still apply
after an unknown timeout. This is not a hard interruption of blocked storage.
The [conditional refinement proof and implementation contract](https://github.com/c4pt0r/kv9/blob/a00e39f9f8da7bb381df19afd2eed63e8a1f1b75/docs/ASYNC-WRITE-WAIT.md)
state the exact ownership, evidence, progress assumptions and remaining gates.

Local validation passes 643 workspace tests/doctests (23 ignored), all-target
Clippy with warnings denied, seven compiled semantic controls with all 21
baseline/mutant/restored phases, and the unchanged default three-process
failover/delete/original-directory restart fixture. All four observed executing
process lifetimes match default binary SHA-256
`72f72c49bf16307f1b9c91a99db7ef41555e847f752e9da78e51623a168c8628`
and exited. Tests discriminate committed-but-unapplied success, missing
registration wakeups, evicted receipts, proposal-before-admission, early public
reservation release, unknown retries and extended retry deadlines.

The retained archive is
`target/correctness-evidence/2026-09-10-a00e39f-async-write-wait-local-first.tar.gz`
(2,369,910 bytes; 509 entries), SHA-256
`b621a5d34d1198daae843eff8fc1c536ba08d9b76278d43a26467529aa237ab5`.
Every archive member and original input was read back. This archive covers local
implementation tests; performance artifacts are separately retained below.

The completed [resident-before / async-write / resident-after bracket](../scripts/redis-reference/results/11cae97-a00e39f-c64-tmpfs-diagnostic.md)
passes all 36 cohorts and six unchanged matrix/outcome checks. Pooled successful
operations/s in the same explicitly volatile c64 tmpfs diagnostic are:

| Operation | Resident before | Async write | Resident after | Change vs pooled baseline |
| --- | ---: | ---: | ---: | ---: |
| GET | 136,110.3 | 136,171.3 | 136,948.0 | -0.26% |
| PUT | 66,315.0 | 78,501.3 | 66,292.8 | +18.40% |
| Mixed | 84,218.4 | 92,711.6 | 83,419.0 | +10.61% |

The candidate PUT repetitions are 78,363.6/78,638.9 operations/s, above all four
baseline repetitions (65,955.1-66,674.8). Every one of the 5,284,937 measured KV9
operations succeeded, and all 63 owned process lifetimes exited. Existing public,
read-group and inline/fallback checks pass. The additional read-only async-apply
audit uses existing per-PID snapshots without adding live polling: the candidate
leader's process-lifetime peak increases from 1 to 64 during the first PUT trial,
remains bounded by 128, and all before/after queue and in-flight observations are
zero with stopped=false. These are whole-trial/lifetime observations, not
per-response or measurement-only counts.

Client p99 buckets remain unchanged: GET 0.524288-1.048575 ms, PUT/mixed
1.048576-2.097151 ms, and Redis 0.131072-0.262143 ms. Existing whole-trial stage
observations show candidate PUT queue means of 20.84/16.45 microseconds versus
39.33-42.44 around it, and application-wait means of 429.29/439.44 versus
515.48-520.40. Submission means remain approximately 9-11 microseconds. These
are overlapping wall-time intervals with export/setup/drain boundaries, not
additive CPU attribution or a measurement-only request breakdown.

The complete performance inventory contains 1,714 files / 2,369,292,591 bytes,
all independently reopened and hash-verified; SHA-256
`eddd8086c5eb7be1f006c6a5a435978d8ae435547c7d68664b97bdf59f2bf916`.
All 26,968,000 Redis measured operations also succeeded. Original matrices,
identities, full outcomes, CPU/histograms, and volatile-data copies are retained.

Retain `a00e39f` as the next performance candidate. Its short volatile-memory
write improvement is separated from the surrounding baseline, while GET does
not improve. The paired standalone Redis PUT rate is 492,632.4/s, leaving a large
remaining gap. No result here establishes disk durability, sustained capacity
or cross-host performance. The normal quorum/sync code and fixed Ready/Redis
clients remained unchanged; only new proof launches were paused, and all active
provers finished before timing. The same proof scheduler resumed immediately
after timing and owned fixture cleanup.

Resident-read actual Chaos validation is accepted for exact `11cae97`; the later
async-write candidate needs its own full fault/history acceptance. The separate
test task uses a frozen exact-source default image and the unchanged complete
21-window Chaos Mesh fixture, with a PID-aware observer for the new bounded
async-apply registry. Performance timing waits until fault injection finishes.

The next bounded implementation experiment is now published as
[`c7313ec`](https://github.com/c4pt0r/kv9/commit/c7313ecd9918c3f1a8269dcf4bcf31ba5ba69c73),
starting from `a00e39f`. It replaces the 1,024-entry command receipt Vec with a
VecDeque, evicting the oldest receipt before appending to a full container.
This removes per-command shifting and avoids growing an already full allocation.
Chronological contents and all existing exact receipt/eviction decisions remain
unchanged; lookup still scans oldest first. The
[sequence-equivalence proof](https://github.com/c4pt0r/kv9/blob/c7313ecd9918c3f1a8269dcf4bcf31ba5ba69c73/docs/RECEIPT-FIFO.md)
uses induction over arbitrary finite receipt streams and requires no ordering,
uniqueness or contiguity premise about indices. It assumes successful standard
container operations and the existing mutex contract; it does not prove the
underlying consensus, persistence or publication protocol.

Local validation passes 644 workspace tests/doctests (zero failures, 23 ignored),
all-target Clippy with warnings denied, formatting, three compiled FIFO controls
and seven inherited async-write controls. All 30 baseline/mutant/restored phases
pass their acceptance checks. A wraparound test compares complete chronological
suffixes, exact verdicts, front/back observations and stable full allocation.
The unchanged default three-process failover/delete/original-directory restart
fixture passes; all four observed executing lifetimes match default binary
SHA-256 `e9a235f13741ed17b9945b3ee39c0e9ecf7a2ed86cc563d48e2451a97b30a084`
and exited. The raw pre-commit manifests retain parent HEAD `a00e39f` and their
recorded modified source state. Every recorded control and process source input
was rehashed against committed `c7313ec` and matches; those manifests have not
been relabeled after the commit.

The local correctness archive is
`target/correctness-evidence/2026-09-10-c7313ec-receipt-fifo-local-first.tar.gz`
(2,755,866 bytes; 525 entries), SHA-256
`b8fd64ed1ab55c37f30ce46200d236ba75534b5518b2eaf203434eb9a57f3313`.
All members and original inputs passed readback. A subsequent clean exact-source
default-feature release build has SHA-256
`f2402a9321e19dd937c980496255df2402b090b1aa08445139223d555fb27150`;
its 417 recorded source files still match, and the separately identified Ready
benchmark client remains unchanged. This build is preparation for measurement.

The completed [async-write / FIFO / async-write bracket](../scripts/redis-reference/results/a00e39f-c7313ec-c64-tmpfs-diagnostic.md)
does not establish a FIFO performance benefit. Successful operations/s in the
unchanged c64 volatile tmpfs diagnostic are:

| Operation | Async write before | Receipt FIFO | Async write after | Change vs pooled baseline |
| --- | ---: | ---: | ---: | ---: |
| GET | 136,331.3 | 135,889.9 | 137,110.5 | -0.61% |
| PUT | 78,917.2 | 78,113.5 | 78,397.0 | -0.69% |
| Mixed | 93,163.8 | 92,086.6 | 92,746.4 | -0.93% |

Both candidate PUT repetitions are below all four surrounding baseline
repetitions; GET/mixed ranges overlap. Client p99 buckets remain unchanged.
All 5,537,388 KV9 and 26,975,936 Redis measured operations succeeded, all 36
cohorts and six unchanged checks passed, and all 63 owned executing lifetimes
exited. Existing read/group/inline/public checks and async-apply occupancy,
monotonic lifetime peak and boundary drain checks pass across all three
matrices. The complete inventory contains 1,728 files / 2,587,602,557 bytes,
all reopened and verified, SHA-256
`65cb50a13283aeaadaf55d9a1974d016499793e871cd218f475732379f2238af`.

Retain FIFO as an isolated representation experiment. Continue the performance
mainline from `a00e39f`; the earlier 18.4% PUT improvement belongs only to its
async-write comparison. The new exact-source
[GET/PUT CPU profiles](../scripts/redis-reference/results/a00e39f-get-put-cpu-profile.md)
are complete: GET RPC/framing/serialization/buffer leaves account for 27.08%,
generic allocation/copy/comparison for 20.67%, and scheduler/generic
synchronization for 12.62%. PUT RPC and allocation shares are 19.40% and 23.60%;
182 samples fall in the exact executable's WAL CRC loop (5.61%, a subset of
engine samples). These are instrumented on-CPU populations, not additive
inclusive stack costs, end-to-end latency fractions or throughput gains.
The report retains both failed GET attempts and the first audit's dense
follower-coverage failure; accepted GET aggregate/leader coverage is complete,
while each sparse follower lacks the first measurement bin.

Prioritize RPC framework comparisons: current unary gRPC, streaming gRPC as a
control, and the scaffolded tarpc/TCP prototype. The scaffold has no completed
measurement result. Preserve payloads, bounded admission/backpressure,
cancellation ownership, deadlines, retry/unknown accounting, quorum-confirmed
reads and exact committed/applied write receipts. Raft consistency remains a
mandatory constraint for every experiment and integration. Consider DPDK or
other kernel bypass only after measurements establish a relevant network
bottleneck; these loopback profiles do not establish NIC limits. The separate
table-CRC prototype `89b9755` remains unselected with three focused tests only;
RPC experiments supersede it in priority.

Earlier profiles describe earlier runtimes; lower representation complexity
alone is insufficient evidence of an end-to-end gain. Exact-FIFO Chaos and
inherited checked protocol composition remain open. Main runtime promotion and
the original roadmap checklist remain unchanged; no hosted workflow was
dispatched.

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
