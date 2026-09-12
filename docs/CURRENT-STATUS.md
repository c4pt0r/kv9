# Development checkpoint — 2026-09-12

Tracking: [#9](https://github.com/c4pt0r/kv9/issues/9). The target remains an
industrial distributed database. Memory RawKV read performance comes before
dynamic multi-Raft and automatic range splits.

## Delivered foundation and product limits

The current foundation provides three-voter RawKV, self-hosted metadata,
Raft-ordered durable writes, fresh linearizable ReadIndex reads, MinIO
checkpoints, WAL-tail recovery, streaming RPC and atomic native batch APIs.
Selected runtime behavior remains CRC `ca0002c7`; later main commits update
build-cache safety and documentation. Experimental branches are not selected.

The full dataset still resides in RAM. Incremental bounded storage, complete
core-protocol implementation proofs, the full actual Chaos Mesh failure matrix,
independent host-failure acceptance, dynamic data groups and automatic splits
remain open. An abstract proof or one-host fault run does not close these gates.

## Latest completed optimization experiment

[ThinLTO with one release codegen unit](RELEASE-THIN-LTO-PERFORMANCE.md) improves
all four point-read/mixed cells in both run orders. C64 GET throughput rises
**8.511%**, mean falls **7.847%**, and p99 improves. C1 GET throughput rises
**6.402%**; mixed throughput rises **13.891% at c1** and **10.424% at c64**.
Separate mixed GET/PUT means and p99 improve in both repetitions.

| Same-run metric | Selected CRC | ThinLTO candidate | Redis |
| --- | ---: | ---: | ---: |
| c1 GET calls/s | 26,591.615 | 28,293.988 | 174,022.103 |
| c1 GET mean us | 37.485 | 35.226 | 5.671 |
| c1 GET p99 us | 50.176–50.687 | 45.056–45.567 | 7.424–7.487 |
| c64 GET calls/s | 346,695.956 | 376,202.163 | 510,898.184 |
| c64 GET mean us | 184.474 | 169.998 | 125.158 |
| c64 GET p99 us | 352.256–356.351 | 323.584–327.679 | 229.376–231.423 |
| c64 mixed combined calls/s | 170,992.289 | 188,817.154 | 500,132.085 |
| c64 mixed GET mean us | 386.171 | 350.297 | 127.850 |

All 12 smoke and 24 timed cohorts complete with **50,708,100 measured
single-attempt successes**, zero dropped slots and independent acceptance.
The audit verifies 80 exited lifetimes, 48 fresh drains/bindings, 4,681 resource
samples and exact CPU/namespace restoration. [Validation](RELEASE-THIN-LTO-VALIDATION.md)
passes 709 workspace tests/doctests (23 existing ignored), formatting/Clippy
and ordinary recovery histories with 359 operations (326 OK / 33 unknown).
The production compiler flags/default feature graph and source identity are
verified separately from the workspace test graph.

Candidate `02d0c01` now passes the [21-window actual Chaos matrix](RELEASE-THIN-LTO-CHAOS.md):
**9,923 complete history operations** (9,371 OK / 35 refused / 517 unknown),
with positive fault effects, four fresh final replica drains and **33 observed
server lifetimes / 25 containers** confirmed exited or removed. The original
delay-selector failure and its tested correction remain separately retained.
Broader point/batch performance qualification is pending; selected runtime
remains CRC. This correctness run adds no new QPS measurement. The result
reaches 73.635% of Redis c64 GET throughput; isolated GET mean is still about
6.21 times Redis. Read parity is not complete.

Scope: unchanged fixed clients, default uninstrumented production builds,
shared-host loopback and ordinary three-voter quorum/sync on **tmpfs WAL**,
versus standalone Redis without persistence/pipelining. No equal-durability,
real-disk, cross-host, sustained-capacity or significance claim. Completed
historical WAL artifacts were moved to verified local cold retention to make
space; original cold paths require rehydration before reuse of old full audits.
Current ThinLTO and global-queue evidence is resident. See the retention overlay
linked by the performance report; no storage guard was lowered.

## Diagnosis and next performance work

[Matched asynchronous read stages](READ-STAGE-RESULTS.md) observe c1 quorum
confirmation at 20.636 us (85.72% of the successful lifecycle read wait) and c64
receiver resumption at 42.449 us (37.47%). Those instrumented populations include
initialization/verification and do not establish pure network or scheduler time.
Source `40f014f` and its two original diagnostic fixtures are qualified and pushed.

The [cross-branch experiment index](PERFORMANCE-EXPERIMENT-INDEX.md) now links
prior decisions, including earlier global-queue and persistent-stream-worker
regressions. The latest single-owner and two-worker revisits stop before timing;
they provide no new performance result. Scheduling rewrites require a new cause.
The ThinLTO candidate now passes source, ordinary recovery, the complete
point-read/mixed screen above and the 21-window actual exact-build fault gate.
Next complete 36 smoke / 72 timed broader point/batch cohorts. A post-process
compressed-retention implementation passes 53 helper controls and actual
1,073,741,841-byte compression/decode/restore qualification. Its real CRC WAL
restore pilot also passes; the linked Chaos report records the new historical
cold-retention overlay. Further actual capacity is still required before runtime.
Storage floors and workloads remain unchanged. Finish these gates before default promotion.
Do not add its percentage to the separate notification candidate's historical improvement.
The read milestone remains open before dynamic multi-Raft and automatic splits.

The earlier [notification comparison](COALESCED-OWNER-PERFORMANCE.md) remains
historical accepted evidence: candidate `42e0117` improves c64 GET 1.123% and mixed
throughput 2.730%, with no isolated GET improvement. It remains experimental
pending broader API and actual Chaos qualification. Its proof/source/recovery
scopes are retained in [the validation report](COALESCED-OWNER-VALIDATION.md).

## Completed diagnosis and rejected prototype

The [request-body handoff diagnostic](https://github.com/c4pt0r/kv9/blob/a75c005/docs/RAFT-BODY-HANDOFF-RESULTS.md)
passes both fixed cells with 984,862 measured successes, eight exited lifetimes,
six drains and 12 metric documents. C1 heartbeat/response offer-to-poll means
are 0.444--0.475 us. At c64 mixed, leader heartbeat batches average 0.987 us and
mixed batches 3.244 us, with p99 32.768--65.535 us. These local populations are
not additive GET phases or wire/ACK latency. It does not justify replacing the
batch channel as the primary read optimization. Source tests and 368-call
ordinary recovery pass; original failed Clippy and its test-only fix are retained.

The outbound peer-executor prototype keeps two public workers/event8 and places
existing outbound Raft tasks and connections on one additional per-node worker.
Inbound peer RPCs keep the original listener; no new per-message task, queue,
port or cluster-wide service is added. Its performance gate now shows the tradeoff
above and rejects default promotion. The [source correspondence and ownership
contract](https://github.com/c4pt0r/kv9/blob/36ae89a774131b368cf1ed28e95df02ba325c8d4/docs/PEER-EXECUTOR-ISOLATION.md)
and [ordinary recovery evidence](https://github.com/c4pt0r/kv9/blob/79ea149/docs/PEER-EXECUTOR-ISOLATION-VALIDATION.md)
retain their exact scopes. Constructor failure, full implementation refinement
and actual candidate Chaos/host failure are not newly established by this screen.

## Completed CPU and scheduler investigation

The [exact-artifact diagnostic](PEER-SCHEDULING-DIAGNOSTIC.md) accepts four
instrumented c64 GET fixtures with 6,703,621 measured successes, 6,120 selected
CPU samples and 380,154 scheduler events. Sampled CPU rises from 2.9831 to 3.1701
core equivalents with outbound isolation. Switching and wake activity increase,
but different waiting populations move in different directions. Source-supported
thread roles and all unknown/ambiguous boundaries are retained. This establishes
no new uninstrumented QPS result and does not promote the rejected executor.

The identified `WorkSignal::notify` candidate is now implemented, proven and
screened as `42e0117`; see the completed validation and performance reports above.
The original sampled notification stacks were about 4% of CPU, not a predicted
speedup. The actual screen establishes a modest loaded-read improvement and no
isolated GET improvement. Original setup/reader failures remain published.

## Next development steps

1. **Finish the strongest current candidate:** keep `02d0c01` frozen after its
   favorable screen and accepted 21-window exact-build Chaos matrix. Qualify
   local retention capacity, then execute 36 smoke / 72 timed point/batch cohorts.
   Keep `42e0117` separate; any combined candidate needs
   its own source mapping and matched qualification. A short screen alone does
   not justify default promotion.
2. **Reduce isolated GET latency:** localize serial RPC, owner-service and
   completion-wait costs with a bounded diagnostic. Preserve the selected shared
   executor and consensus boundaries; the current evidence does not justify
   another worker-count or transport sweep. Do not repeat completed screens
   without a new cause.
3. **Qualify useful improvements:** applicable proof mapping, full point/batch
   checks and actual exact-source Chaos Mesh fault injection precede promotion.
   Preserve fresh Safe ReadIndex, sealed groups, durable writes and complete
   successful pump/apply/view fences. DPDK requires cross-host/NIC evidence.
4. **Close industrial correctness and availability:** compose implementation
   proofs, remote admission bounds, persistence failure cuts and the actual Chaos
   matrix. Metadata, routing and scheduling must have no service-critical
   singleton; only the object-store dependency is exempt. Local Kind cannot
   establish independent host loss.
5. **Scale out after the read milestone:** bounded RegionManager/dynamic multi-Raft
   (#22), epoch-fenced routing (#23), recoverable learner attachment/membership
   (#24), then durable split intent, ownership fencing, data handoff and
   idempotent recovery (#25). Follow with placement and independent-hotspot
   scaling (#27), retaining prerequisites in [DEVELOPMENT-PATH.md](DEVELOPMENT-PATH.md).

Earlier held [inbox-vector](https://github.com/c4pt0r/kv9/blob/51efc6598324c10c1e27cd6fef869d4b9a39e7c4/docs/INBOX-VECTOR-REUSE-PERFORMANCE.md),
[Append-payload](https://github.com/c4pt0r/kv9/blob/381d0973c078522d3698a922ecac8594cc9df348/docs/RAFT-APPEND-PAYLOAD-PERFORMANCE.md),
[metadata combination](https://github.com/c4pt0r/kv9/blob/3641d0ff0644f700cf3ea8c9c1dbda8fd7462f00/docs/STREAM-METADATA-CRC-PERFORMANCE.md),
[confirmation-queue](https://github.com/c4pt0r/kv9/blob/c423d3c605bf6b88bda3521b85e6997c2b203120/docs/CONFIRMATION-QUEUE-RESULTS.md)
and [broad 72-cohort](https://github.com/c4pt0r/kv9/blob/aeee652135bc52e4527ef4fffd540ec184adc362/docs/READ-CREDIT-CRC-PERFORMANCE.md)
evidence remain available under their original scopes.

Write durability is mandatory; optimize CPU work, batching and pipelining where
measured, and compare real-disk costs with equivalent acknowledgements. Advanced
TLS remains later work. CI runs locally; hosted workflows remain manual for
releases or explicitly selected key milestones. No hosted CI ran for this checkpoint.
