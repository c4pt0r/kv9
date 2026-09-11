# RawKV performance development path

Tracking: #13, #20 and #9. The product target is Redis-class performance for
memory-resident RawKV data. Data residency, durable acknowledgement, replication
and API overhead are separate dimensions. Performance work must improve the
implementation while keeping each measured mode's guarantees explicit.

## Product sequence, updated 2026-09-11

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

## Performance experiment order

Use a short feedback loop for each narrowly scoped optimization:

1. State the measured bottleneck hypothesis and preserve a clean control.
   Change one mechanism, retaining the consistency and admission contracts.
2. Run focused correctness tests and a correctness smoke, then measure the
   exact candidate and control with matched resources and clients. Mark these
   measurements provisional until the remaining acceptance gates pass.
3. Reject changes without a useful throughput and latency result. Preserve
   their results; do not spend a full fault campaign trying to justify them.
4. For a promising candidate, complete the appropriate workspace checks,
   source-mapped proof obligations, process histories and actual Chaos Mesh
   acceptance before selecting or promoting it. Recheck exact source and
   artifact identity so the measured and validated implementations coincide.

Never overlap timing with builds, tests, fault injection, profiling or audits.
Test-environment preparation can proceed independently between timing windows.
Acceptance still requires the same consistency evidence; this order moves
performance screening earlier so a rejected experiment costs less time.

## Current evidence

The [two-worker comparison](RPC-WORKER-PAIR-PERFORMANCE.md) reaches
**344,412–344,790 GET/s**, improving **4.795% / 5.381%** over event8 under
the same CPU budget. Mean latency falls to **185.490–185.700 us**, with
improved p99 in both repetitions. BatchGet(1) reaches 335,694–337,822 calls/s
(+5.069% / +5.434%); mean voter RSS sums fall about 3.8–4.8 MiB across the
read comparisons. Same-run Redis GET remains 497,503–505,913/s, a paired
1.44–1.47x throughput gap. All 23,258,382 measured calls succeed and the
independent audit passes. Default/experimental workspace checks pass 707/717
tests and all-target Clippy passes. [Exact-source Chaos acceptance](RPC-WORKER-PAIR-ACCEPTANCE.md)
passes all eleven windows, with 2,114 complete-history calls, 183 verified
new leaf drops, fresh drains and owned cleanup. Select this as the next
isolated short pure-read increment; master/default promotion remains open.

The [refreshed two-worker lifecycle diagnostic](RPC-PAIR-READ-LIFECYCLE.md)
now passes: GET's sampled 112.716-us barrier includes 66.634 us to observed
confirmation and 40.770 us from result send to receiver observation. BatchGet(1)
shows the same pattern. All 3,324,772 measured calls, CPU/fixture readbacks and
the independent twelve-document lifecycle analysis pass. This is instrumented
diagnostic evidence, separate from the accepted uninstrumented QPS result.

The [two-worker global-queue screen](RPC-PAIR-GLOBAL-QUEUE-SCREENING.md) now
rejects fixed interval eight: GET throughput falls **3.217% / 2.614%** and
BatchGet(1) falls **3.142% / 2.736%**, with worse mean latency in both repeats.
All 23,488,956 measured calls and the first independent audit pass. Retain the
adaptive interval; this rejected candidate does not advance to broad acceptance.

The [two persistent stream-worker experiment](STREAM-WORKER-PAIR-SCREENING.md)
is implemented and screened as `9b74274`, then rejected: GET throughput falls
**4.503% / 4.808%** and batch throughput falls **4.584% / 5.443%**. Every p99
bucket improves, but mean latency worsens in all four comparisons. All 23,346,575
measured calls and the first independent audit pass; 74 focused tests and the
358-call process histories also pass. Retain the per-request task control.

The [single-GET concurrency curve](GET-CONCURRENCY-CURVE.md) now passes its
first complete 24-cohort run and independent audit. At c1, KV9 mean latency is
37.975–38.012 us versus Redis 5.789–5.806 us, approximately a 6.5x turnaround
gap. At c64, KV9 reaches 342,979–344,681 successful calls/s with all calls
succeeding. Offered c128/c256 cross the fixture's unchanged public admission
limit of 64: approximately 30% / 50% of calls are refused, and c256 successful
throughput falls to 306,181–308,856/s. High-concurrency totals therefore do not
establish service capacity or Redis parity. All populations remain published.

The [c1 lifecycle recording](LOW-CONCURRENCY-READ-LIFECYCLE.md) now measures
a 24.448-us sampled GET barrier, including 20.997 us (85.9%) from admitted
invocation to observed confirmation and 1.693 us for result notification.
The first recording, fixture readback and independent twelve-document analysis
pass. All 258,976 measured calls succeed. Historical c64 also used perf, so the
recordings do not isolate a concurrency-only effect.

The [direct peer body screen](DIRECT-PEER-BODY-SCREENING.md) rejects `6707bcc`:
GET throughput falls **4.093% / 3.760% at c1** and **3.456% / 2.290% at c64**.
BatchGet(1) also regresses; mean latency worsens in all eight comparisons.
All 27,724,506 measured calls and the first independent audit pass. Source
validation passes 224 Raft tests, nine source-control triples and the 352-call
process E2E. The accepted control remains `5ee897a`; this candidate does not
advance to broad workspace/Chaos acceptance.

The [idle watchdog follow-on](PEER-IDLE-WATCHDOG-SCREENING.md) also rejects
`f62c08e`: seven of eight throughput/mean pairs regress, despite better p99 in
all eight. c64 GET changes -0.618% / -2.915% and BatchGet(1) -3.189% / -3.457%.
All 27,821,713 measured calls succeed and the first independent audit passes.
Source gates pass 228 Raft tests, 14 source-control triples and independently
checked 379-call process E2E. Accepted `5ee897a` remains unchanged.

The [pending ReadIndex credit screen](READ-GROUP-CREDIT-SCREENING.md) now
retains `57ff685` as a promising c64 candidate. The actual upstream pending
queue bounds local submission to one unconfirmed context, allowing queued
readers to form a later sealed group without an idle batching timer. Both c64
GET repetitions improve by 8.031% / 8.220%, reaching **371,789-374,086 GET/s**;
BatchGet(1) improves by 8.479% / 7.919%. Mean and p99 improve in all four c64
pairs. All **28,847,557 measured calls succeed** and the first independent
audit passes. Source validation includes 714 workspace tests, 218 Raft tests,
14 compiled control triples, 265 parameterized proof obligations and the
independently checked 353-operation process E2E.

The initial screen's c1 point GET throughput falls 0.083% / 0.573%, with worse
mean/p99 in both repetitions. A separate predeclared
[four-repeat, 30-second c1 follow-up](READ-CREDIT-C1-FOLLOWUP.md) now passes all
24 cohorts and its unchanged independent audit: **53,783,436 calls, all
successful, one attempt each**. Pooled GET changes 26,335.687 -> 26,447.329/s
(+0.424%) and 37.848 -> 37.681 us mean; p99 improves in two repeats and worsens
in two. All earlier observations remain retained. This does not establish
significance or c1 no-regression; `5ee897a` remains the general baseline.

Exact-source [eleven-window local Chaos acceptance](READ-CREDIT-CHAOS-ACCEPTANCE.md)
also passes: **2,079 calls, 1,780 OK / 224 refused / 75 unknown**, actual fault
effects, recovery, contained negative windows and complete owned cleanup.
Keep sustained traffic, writes/mixed, larger batches, dynamic-auth/storage
callback progress and whole grouped-read/Ready/Rust proof composition in the
promotion gates. The new proof establishes a single-invocation admission
projection and conditional progress, not full implementation refinement.

The [Redis execution analysis](REDIS-EXECUTION-PATH.md) separates quorum
amortization from shortening individual request latency. Preserve fresh
confirmation, sealed membership and applied-index checks while measuring
individual transport/owner handoffs and reducing unnecessary representations.
Previous queue and scheduler regressions remain evidence against selecting a
change on architectural intuition alone. Neither the profile nor fixed public
admission curve establishes an intrinsic ceiling. Redis parity and automatic
splits remain open.

The [inbox-vector screen](RAFT-INBOX-DRAIN-SCREENING.md) completes 48 cohorts
and its unchanged independent audit: **117,886,444 calls, all successful**.
Candidate `c3131800` has pooled c64 GET +0.422% and BatchGet(1) +0.006%, with
seven of sixteen individual QPS pairs regressing and mixed p99. It does not
establish a consistent useful throughput-and-latency benefit and is not
selected for the next performance increment or a broader fault campaign.
Its source gate and 364-call process E2E remain valid, separate correctness
evidence. Keep every observation; `5ee897a` remains the general baseline.

The [next isolated candidate](https://github.com/c4pt0r/kv9/blob/1b70dba62e492cd8e10dd20f643fee74d39abc57/docs/WORK-SIGNAL-COALESCING.md)
is implemented at `1b70dba6`, based on `57ff6851`, and coalesces physical
WorkSignal notifications on the pending bit's false-to-true transition. It
preserves the same state mutex and single-owner work protocol, with a local
argument for publication/drain/parking races and an actual-delivery concurrency
test. All 219 Raft tests/doctests and formatting/Clippy pass. A controlled lost
pending turn fails that test and exact source restoration passes. The default
release and independently checked 351-call process E2E pass all four progress
windows, six fresh drains and seven exited lifetimes. This targets redundant
synchronization work; performance and candidate-specific Chaos remain open.

### Earlier experiments

The following records retain their historical source, measurement and selection
scope. Their original next-step decisions are superseded by the current path
above; gains from different baselines must not be added together.

The [one-worker screen](RPC-WORKER-SCREENING.md) rejects `711631b`: GET
throughput falls about 35%, mean latency rises about 53–54%, and both APIs'
p99 worsens. All 20,736,623 measured calls and the independent audit pass.
Keep event8 and test two async workers as a separate intermediate experiment;
the one-worker result does not justify more reductions in useful parallelism.

The [peer batch enqueue screen](PEER-BATCH-ENQUEUE-SCREENING.md) rejects
`ea5f498` as the next performance increment: GET gains only 0.399% / 0.303%
and second-repeat p99 regresses. The complete 23,025,553-call matched matrix
and independent audit pass. Retain the existing event8 control and test the
async worker population next; no full Chaos campaign follows this rejected
screen.

The [I/O event interval candidate](RPC-EVENT-INTERVAL-PERFORMANCE.md) shows a
useful gain on the unchanged jemalloc control: the five-second follow-up
reaches 326,793–327,857 GET/s (+8.649% / +8.585%), with mean latency about
195 us and unchanged/lower p99 buckets. Redis GET remains about 507,000/s.
BatchGet(1) gains about 8%. The preceding short screen has second-repeat
p99 regressions, which remain documented; there is no universal tail claim.
Both complete matched matrices and independent audits pass. Exact-source
[eleven-window Chaos acceptance](RPC-EVENT-INTERVAL-ACCEPTANCE.md) now passes
2,106 calls with 177 new verified leaf drops and complete owned cleanup.
Default/experimental workspace checks pass 707/717 tests and Clippy passes.
Select this as the next isolated read-performance increment; master/default
promotion, write/mixed, sustained and memory behavior remain separate gates.

The [fixed global queue interval screening](RPC-GLOBAL-QUEUE-SCREENING.md)
rejects scheduler candidate `5e46a61`: GET throughput falls 1.952% / 1.213%
versus the unchanged jemalloc control, with worse mean and p99 latency.
BatchGet(1) also regresses. Focused checks, actual process histories, smoke
and the independent twelve-cohort audit pass, so this is a measured negative
experiment. Preserve it without promotion or another full fault campaign.
Next examine peer-message processing and I/O scheduling within the observed
confirmation interval. A fixed global queue polling interval alone has not
improved the sampled completion-wait bottleneck.

The latest [authorization-metadata screening](STREAM-AUTHORIZATION-METADATA-SCREENING.md)
retains an isolated candidate for broader validation: single GET gains
4.425% / 3.641% against the same-run f2 control, with improved mean and p99.
All 6,460,852 measured calls and the separate reader pass. The candidate uses
the system allocator and has not been combined with jemalloc; exact-source
[eleven-window Chaos acceptance](STREAM-AUTHORIZATION-METADATA-ACCEPTANCE.md) now passes; broader promotion obligations remain open. It does not supersede the best
retained jemalloc performance or establish Redis parity.

The completed [sampled read lifecycle profile](READ-LIFECYCLE-WAIT-RESULTS.md)
narrows the next bottleneck: GET's 131.316-us sampled barrier includes 82.119 us
from successful ReadIndex admission invocation to observed confirmation, and
37.779 us from result send to receiver observation. Both APIs pass the original
CPU analysis and fixture readback; all 2,943,458 measured calls succeeded.
These instrumented, whole-cohort histograms are not new capacity results or
an exact decomposition of measured end-to-end latency. Prioritize completion
delivery and receiver scheduling, then peer dispatch and owner wakeups, with
matched throughput/latency screening. The confirmation interval includes local
processing and scheduling, not only network time. No hardware ceiling or DPDK
benefit is established. Normal Raft read and write semantics remain mandatory.
The earlier [read-barrier counter diagnostic](READ-BARRIER-WAIT-DIAGNOSTIC.md)
is retained as the motivation for this completed measurement.

The subsequent [typed authenticated dispatch screening](TYPED-POINT-DISPATCH-SCREENING.md)
is **not selected** as the next performance version. Single GET improves only
0.966% / 0.019% versus its same-run f2 control; BatchGet(1) gains 1.867% / 2.521%.
All 6,452,825 measured calls and the independent audit passed, but this does not
establish the desired meaningful step. Preserve the isolated experiment and
stop before a full fault campaign. That allocation hypothesis is now measured separately above, with per-frame
authentication retained.
This comparison used the system allocator and does not replace the separate
jemalloc result below.

The latest [Linux allocator experiment](JEMALLOC-SERVER-PERFORMANCE.md)
reaches **304,863–305,903 single GET calls/s**, a paired 4.47–6.24% improvement
over the same-run parallel-stream control. Mean latency is 209.1–209.8 us,
with improved p99 buckets. Redis GET reaches 507,769–508,136 calls/s, leaving
about **1.66x** throughput headroom to the reference. All 6,558,446 measured
calls succeeded under the unchanged short c64, 128-byte paired protocol.
The cost is 10.2–11.2 MiB higher aggregate mean voter RSS, or 23–26% in this
small working set. Keep this candidate isolated pending broader memory and
write evidence; it does not by itself justify default allocator promotion.
The [exact-source acceptance record](JEMALLOC-SERVER-ACCEPTANCE.md) separates
process/Chaos evidence from the conditional allocator/source argument.

The refreshed [parallel-stream CPU profile](PARALLEL-STREAM-CPU-PROFILE.md)
finds roughly 23% of sampled leaves in allocation/copy/comparison, 15% in
scheduling/synchronization and 14% in RPC/framing/buffers. These are categories
on the pre-allocator control, not candidate latency fractions or additive
predicted gains. They support reducing request allocations and copies next,
then reassessing scheduling/serialization with measured candidate profiles.
No hardware ceiling or benefit from kernel bypass is established. Preserve
per-request authentication, bounded ownership and normal Raft barriers.
Writes/mixed workloads require their own fresh comparison before claiming
Redis-class RawKV performance or advancing to automatic splits.

The separate [first follower WAL sync-stall qualification](STORAGE-STALL-FIRST-QUALIFICATION.md)
records actual Chaos Mesh FSYNC delay on exact f2: 43 fresh sync samples average
501.087 ms and all 655 complete-history operations succeed. The original
harness still exited 1 on supervisor FIFO archive extraction; independent safe
retention, history/effect readback and scoped cleanup passed afterward. The
archive repair is separate, and this one-follower result does not close the
broader fault/storage package or accept newer runtime sources.

The separately released [repaired follower stall run](STORAGE-STALL-REPAIRED-QUALIFICATION.md) now passes the complete controller and independent audit: 655 all-OK calls and 42 fresh same-FUSE-WAL sync samples averaging 501.267 ms. All three windows, six post-client drains, original archive/FIFO retention and scoped cleanup pass. It retains the same one-follower, exact-f2 and volatile-media scope.

### Parallel stream scheduling baseline

The preceding [matched point-read comparison](PARALLEL-STREAM-GET-PERFORMANCE.md)
retains bounded parallel stream scheduling at `f2c4e85` for continued
development. Single GET reaches **287,795–288,611 calls/s**, improving
38.15–38.93% over the same-run `850f0de` control. Mean whole-call latency falls
from about 307–308 to 222 us, and both p99 buckets improve. BatchGet(1) reaches
281,983–287,208 calls/s, improving 41.33–43.35%. Actual Redis GET reaches
496,264–505,568 calls/s, leaving a 1.72–1.76x throughput gap.

This is the same short c64, 128-byte, shared-host tmpfs protocol, with two
opposite-order repetitions and unchanged native/Redis clients. All 5,941,631
measured calls succeeded. The exact candidate passed local workspace checks,
the compiled stream-slot control, process histories and actual eleven-window
Chaos Mesh acceptance before timing. See the [acceptance record](PARALLEL-STREAM-REQUESTS-ACCEPTANCE.md)
for proof scope and remaining fault gaps. Master/default promotion is still
open. Larger batches, writes, sustained load and Redis parity are not updated
by this point-read result. Continue common-path allocation/copying and
synchronization work before dynamic multi-Raft and automatic splits.

### Earlier transport selection and implementation evidence

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
