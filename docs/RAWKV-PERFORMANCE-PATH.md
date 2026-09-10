# RawKV performance development path

Tracking: #13, #20 and #9. The product target is Redis-class performance for
memory-resident RawKV data. Data residency, durable acknowledgement, replication
and API overhead are separate dimensions. Performance work must improve the
implementation while keeping each measured mode's guarantees explicit.

## Current evidence

The latest completed paired measurement is the `892b2a1` Ready candidate. At 64
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
  lookup work, but its actual contribution requires measurement.

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

Introduce an owner-consumed proposal queue with explicit count and byte limits
only after the measured wait budget supports it. Admission and queueing must not
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

Profile the persistent-client GET path after the volatile diagnostic. Candidate
changes include indexed exact-context receipt lookup, fewer duplicate context
decodes/allocations, and bounded read-barrier batching. Each change gets its own
unchanged-protocol comparison so CPU and latency effects can be attributed.

The first indexed-receipt implementation is pushed as `73ddb0d`. It passes 611
local tests/doctests, three implementation-control triples, the default-build
three-process failover/restart fixture and a separately identified
`partition-testing` typed read-refusal fixture. Its own full paired measurement
is running; no speedup or complete acceptance is claimed yet. The
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
