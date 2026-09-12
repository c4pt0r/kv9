# kv9 development roadmap

> Latest direction: [ThinLTO full72 qualification](RELEASE-THIN-LTO-FULL72.md)
> improves throughput and mean latency in all 12 point/batch cells and both orders.
> C64 GET improves 8.465%; loaded BatchPut has a documented p99 tradeoff.
> [Main integration `11113f6`](RELEASE-THIN-LTO-MAIN-INTEGRATION.md) now passes fresh
> local checks, default-build and recovery confirmation. Consult the
> [experiment index](PERFORMANCE-EXPERIMENT-INDEX.md) before another candidate.
> The [bounded owner-poll screen](OWNER-POLL-PERFORMANCE.md) now rejects the
> 32-us candidate: every matched workload/order loses throughput and worsens
> latency, with more server CPU. Main remains on `11113f6`.
> The [leader-lease proof](LEADER-LEASE-PROOF.md) now establishes a conditional
> path to zero per-read quorum RTT, with a [proved transition model](LEASE-AUTHORITY-MODEL.md).
> Implementation, clock qualification and
> actual lease Chaos acceptance remain ahead of any runtime selection.
> The [Rust controller component](LEASE-CONTROLLER.md) now passes its local source
> and integer-timing gates. The [voting adapter](LEASE-VOTE-BINDING.md) now binds
> durable peer installation and elections. The [renewal adapter](LEASE-RENEWAL-PUBLICATION.md)
> now binds grants and whole-pump publication. The [read-view integration](LEASE-READ-VIEW.md)
> passes local source/service/fault controls with default startup still on Safe
> ReadIndex; clock qualification and actual lease Chaos remain next.
> Redis read parity and industrial gates remain open.

Updated: 2026-09-12. This file defines delivery order. `DESIGN.md` preserves the long-term architecture;
[TAKEOVER-AUDIT.md](TAKEOVER-AUDIT.md) maps that architecture to the current implementation.

The execution breakdown is in [DEVELOPMENT-PATH.md](DEVELOPMENT-PATH.md), with 25 work packages,
explicit dependencies, implementation steps and acceptance criteria. Track delivery in
[GitHub issue #9](https://github.com/c4pt0r/kv9/issues/9).

The [current checkpoint](CURRENT-STATUS.md) records accepted ThinLTO full72
performance and the exact-build Chaos matrix, including 9,923 complete history
operations. The 36 smoke / 72 timed point/batch cohorts and independent audit
now pass. Main now selects the exact qualified source after fresh local release,
observer-feature and default-build/recovery checks. The retained server/client
bytes match the original candidate. Preserve the loaded batch-write p99
regression. The [completed fixed-rate diagnosis](BATCH-WRITE-FIXED-RATE-RESULTS.md)
retains higher whole-call and scheduled p99 at both offered rates and both
orders, with 2,044 client-dropped slots preventing strict matched-work inference.
All 797,956 issued calls succeed; no performance cohort was rerun. Client timer
calibration is a separate environment question. The [quorum-path capture](QUORUM-TRACE-RESULTS.md)
now preserves the original lossy recorder result and a qualified immutable-slot
repair. Six repaired prefixes have zero loss, 2,318 matched leader-local round-trip
candidates and 1,159 exact group chains. The 17.286 / 17.102-us round-trip means
include transport and remote work; 0.285 / 0.297-us confirmation-to-eligibility
means retain the original successful pump/apply fence. This is diagnosis, not
a new selected runtime speedup. The [32-us owner-poll source checkpoint](https://github.com/c4pt0r/kv9/blob/2ca5fccb157b26b6c3c79eb52f7c7838f10a5c8c/docs/BOUNDED-OWNER-POLL.md)
now passes 14 new TLAPS theorems / 49 obligations, model/negative controls,
714 default tests and 443 standalone read-stage tests, with overlapping
populations. Original failed drafts and command/build-inventory failures remain
retained. The [clean default release and ordinary recovery](OWNER-POLL-RECOVERY.md)
now pass, including 363 complete operations, 29 unknown outcomes and seven
exited lifetimes. The [24-cohort c1/c64 GET/mixed screen](OWNER-POLL-PERFORMANCE.md)
now passes accounting but rejects the candidate: every workload/order loses
throughput and worsens mean/p99, with more server CPU. Main still selects
`11113f6`. Reuse existing profiles and capture a concrete selected-build CPU cost
before another rewrite; do not retune this poll budget or repeat losing cohorts.
Keep notification candidate `42e0117` separate; its gains cannot be added to
ThinLTO's measurements. The [next latency investigation](QUORUM-LATENCY-NEXT.md)
targets unresolved intervals inside a fresh quorum round, reusing completed
body-handoff evidence and preserving all earlier rejected experiment decisions.
Preserve fresh quorum reads, sealed groups, successful pump/apply/view fences
and durable acknowledgements.
The separately requested lease path now has a complete conditional mathematical
argument, eight checked TLAPS lemmas / 23 obligations and real-clock SMT
containment. The [fixed-configuration transition proof](LEASE-AUTHORITY-MODEL.md)
adds 27 theorems / 348 obligations and bounded fault-model evidence for renewal,
revocation, restart promises and the local view gate. The
[Rust component](LEASE-CONTROLLER.md) passes 205 Raft library tests, source fault
controls and integer-timing proofs. Its [voting adapter](LEASE-VOTE-BINDING.md)
now binds durable peer installation and actual elections. The
[renewal adapter](LEASE-RENEWAL-PUBLICATION.md) adds exact envelopes, leader lifetime
binding and whole-pump certificate publication, with 231 passing Raft tests.
The [read-view integration](LEASE-READ-VIEW.md) now binds fresh commit capture,
the exact owned applied view, final authority and the original admission budget
to experimental GET/BatchGet. Its local gate passes 586 tests (one existing
server test ignored) and nine compiled source fault controls. Unary/streaming
RPC traversal and deferred same-view metadata/value checks pass. Default startup
remains Safe ReadIndex. Expired authority plus unavailable quorum fails closed.
Next qualify the clock and actual fault histories before
matched throughput/latency comparison. See [the proof and implementation gates](LEADER-LEASE-PROOF.md).
The product sequence remains memory RawKV read performance, then dynamic
multi-Raft and automatic splits, without waiving storage/proof/fault prerequisites.
Earlier rejected executor/handoff experiments retain their decisions. Evaluate
throughput and latency together, including mixed-read tails. CI runs locally
except at releases or explicitly selected key milestones.

The target is an industrial-grade distributed database. Prioritize consistency, recovery, measured throughput
and scalable architecture. Complex private-network TLS configuration is not a prerequisite for the current
milestones. Existing identity, root descriptor, token authentication and epoch checks remain part of the baseline.

Hard gates for every stage: rigorous core-protocol proofs with explicit implementation refinement,
real Chaos Mesh fault-injection E2E, and no service-critical single point of failure except the object-store
dependency. See [CORRECTNESS-GATES.md](CORRECTNESS-GATES.md) for proof obligations, evidence requirements
and the distinction between replica failure and host-failure isolation.

## Current baseline: working distributed Raw KV

- One `META_REGION_0` Raft group replicates both self-hosted metadata and Raw KV. Real multi-process tests cover
  bootstrap, failover, restart, learner registration and promotion.
- Writes pass through Raft; data and exact `(term,index)` share a WAL v2 record and fsync before publication.
  Raw and public catalog reads establish a quorum ReadIndex barrier.
- The catalog has typed tables, indexes, foreign keys and transaction overlays. Production mutations acquire
  the catalog mutex, commit an ordering barrier, plan against a stable snapshot and propose in the same leader term.
- With `KV9_STORAGE=minio`, a leader freezes full state, uploads immutable SSTs, confirms PUT and readability,
  and proposes manifest CAS. Each replica adopts only after local ordered apply, persists a recovery pointer,
  and reclaims the covered catalog WAL prefix.
- Cold recovery reads MinIO SSTs and replays the surviving WAL tail. Pending flushes verify cluster and committed
  history, recheck remote bytes, and recover their original identity before another attempt.
- Legacy in-band applied markers can be validated against committed Raft history and upgraded atomically.
  When the legacy full state exceeds the 64 MiB record limit, the old prefix remains and only a verified position
  transition is appended, preserving the ability to reopen large old stores.

This is a basic implementation, not an industrial capacity or performance claim. The entire dataset remains in RAM,
full checkpoints have a 48 MiB serialized ceiling, and catalog WAL reclamation copies the surviving tail before rename.
Raft logs, historical manifests and SSTs are retained. Incremental LSM, block cache, multiple running data groups,
split/merge and usable transaction execution are still future work.

Acknowledged unflushed writes depend on the replicas' local durable logs. A bucket alone cannot reconstruct cluster
identity, protocol state or an unuploaded tail.

## P0: make consistency and recovery continuously verifiable

Complete this gate before capacity or throughput claims:

1. Add named cuts around WAL append/fsync, Ready persistence/send, upload confirmation, manifest apply,
   checkpoint pointers and WAL replacement. Extend deterministic and real-process fault matrices to disk-full,
   EIO, reordered/duplicated messages, partitions/healing and interrupted membership changes.
2. Record concurrent Raw KV and catalog histories and run an independent linearizability checker.
   Distinguish success, proven refusal and unknown outcomes. Matching final values is insufficient.
3. Preserve the delivered `catalog.pending` recovery and exact historical-winner reconciliation.
   Missing files or a latest pair beyond the reconciliation window cannot prove failure.
   Integrate retained evidence with snapshot/GC pins before deleting history.
4. Define backup/recovery anchors and format compatibility. WAL v2 currently requires an offline upgrade;
   mixed-version rolling upgrade and downgrade are not promised.
5. Retain selected counts, failure cuts, logs and exclusive completion markers. Keep real MinIO process tests
   in the PR checks and establish metrics and repeatable single-group benchmarks early.

**Exit gate:** under a documented failure model, acknowledged writes survive, deletes do not reappear, minority
leaders cannot establish successful reads, and replaced log positions cannot produce false success receipts.
Bad objects or protocol anchors fail recovery explicitly. Each covered cut has reproducible positive and negative
controls; uncovered power-loss/fsync behavior remains identified.

The bounded public admission increment (#38) is followed by fixed local latency
observations (#39), documented in [LATENCY-OBSERVABILITY.md](LATENCY-OBSERVABILITY.md).
The client foundation for C03 (#40), including its retry/deadline proof and real
connection tests, is documented in [PERSISTENT-CLIENT.md](PERSISTENT-CLIENT.md).
The remaining workload increment must record end-to-end
latency independently, and separate refused requests, unresolved writes and
acknowledged operations before any throughput claim. Instrumentation itself is
not benchmark acceptance; exact-revision evidence is tracked on the issues.

## P1: bounded storage and measured throughput

1. Implement active/immutable memtables, incremental SSTs, stable versioned views, range tombstones, leveled
   compaction and a bounded block cache. Read remote SST blocks directly so memory no longer scales with live data.
2. Replace O(tail) synchronous copying with segmented WAL and whole closed-segment reclamation.
   Add atomically installable Raft snapshots before protocol-log truncation.
   Record the dual-WAL versus unified-log decision; any unification must preserve atomic data/position recovery.
3. Batch proposals and persistence, add group commit, and bound upload concurrency and queues.
   Propagate admission/backpressure for slow disks, slow MinIO and compaction debt.
4. Benchmark fixed CPU/RAM/disk/network/MinIO topologies, key/value sizes, distributions and concurrency.
   Report throughput, p50/p95/p99, error rates, fsync latency, apply lag, backlog, amplification, memory and recovery.
   Measure one group before many groups; do not invent QPS targets.
5. Enable actual SST deletion only after references, reader/checkpoint/snapshot pins, orphan handling and durable
   delete intents are implemented. Pending proposals, transfers and backups must retain their inputs.

**Exit gate:** data larger than memory remains serviceable; memory and disk usage stay bounded across multiple
compaction cycles; recovery converges within measured limits; performance improvements pass the same P0 correctness
suite and publish reproducible before/after results.

## P2: multiple Raft groups and scale-out

1. Separate metadata and user data groups with RegionManager, shared transport, batched scheduling,
   per-group lifecycle and explicit resource budgets.
2. Bind catalog regions to real groups. Implement routing-cache invalidation, epoch fences, leader hints and
   bounded client rerouting. Define cross-region Raw operation semantics before exposing them.
3. Attach learners through snapshots and shared SSTs, catch up tails, and change membership safely after log
   truncation. Recover ConfState, data and position consistently.
4. Implement durable pre-shard, split, merge and migration protocols before automatic placement.
   Detect throughput/queue hotspots and use hysteresis and budgets to avoid oscillation.
5. Measure lazy attachment and cold-cache costs. Claim metadata-only transfer only when byte measurements
   exclude a hidden full copy. Introduce L0/L1 metadata sharding only after measuring the bottleneck.

**Exit gate:** independent hotspots benefit from added nodes; migration, split and membership changes preserve
single ownership and complete coverage. Shared SST attachment transfers metadata and an unflushed tail, with
object-store reads reported separately.

## P3: transactions and tenant service quality

Implement MVCC, durable TSO/timeline generations, Percolator SI, lock recovery and deadlock detection after
Raw KV, recovery and multi-group ownership are stable. Start within one keyspace and transaction group,
including multiple regions in that domain. Keep unknown commit outcomes explicit.

Add tenant admission, weighted fairness, cache quotas, background-work attribution, resource accounting and
bounded tenant metrics. This work can overlap late P2 once the shared scheduling interfaces are stable.

**Exit gate:** independent histories satisfy SI; uncertain results and retries cannot cause duplicate execution;
hot tenants and slow uploads do not indefinitely starve others. MVCC GC, SST GC, snapshots and long transactions
share an accountable retention protocol.

## P4: operational completeness

Deliver complete backup/PITR, versioned migrations, verified rolling upgrades, operational APIs, diagnostics,
capacity guidance and long-duration soak. Run short fault/nightly tests from P0; P4 adds sustained release acceptance.

Add deployment-specific TLS/mTLS, certificate rotation, cloud identities and finer authorization here.
Keep an explicit HTTP MinIO/token-authenticated development workflow so core protocol testing stays accessible.

Isolate MinIO behind `ObjectStore` and the maintained S3 client. Pin the test image digest and verify at least one
maintained S3-compatible service or cloud S3 before production backend claims.
The [official MinIO repository](https://github.com/minio/minio) was marked archived when checked on 2026-09-08.

## Existing issues and execution planning

The takeover checked [#5](https://github.com/c4pt0r/kv9/issues/5),
[#6](https://github.com/c4pt0r/kv9/issues/6), [#7](https://github.com/c4pt0r/kv9/issues/7) and
[#8](https://github.com/c4pt0r/kv9/issues/8). Their implementation evidence is in
[TAKEOVER-AUDIT.md](TAKEOVER-AUDIT.md). Publishing code, observing CI and closing an issue are distinct steps.

Bounded storage, recoverable log compaction, measurement and multi-group ownership precede transaction expansion.
Complex private-network TLS configuration remains later work.
