# Development checkpoint — 2026-09-11

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

[Global queue polling at interval eight](GLOBAL-QUEUE-PERFORMANCE.md) is rejected:
c64 GET throughput falls **2.390%**, with **2.450%** higher mean and worse p99 in
both run orders. C1 GET and c64 mixed throughput also fall in both repetitions.
Selected runtime remains CRC. The prior notification candidate remains separate;
this screen does not compare the two candidates directly.

| Same-run metric | Selected CRC | Rejected interval-eight | Redis |
| --- | ---: | ---: | ---: |
| c1 GET calls/s | 26,542.953 | 26,391.062 | 174,382.402 |
| c1 GET mean us | 37.562 | 37.779 | 5.658 |
| c1 GET p99 us | 49.664–50.175 | 50.176–50.687 | 7.296–7.359 |
| c64 GET calls/s | 346,069.116 | 337,799.779 | 511,088.898 |
| c64 GET mean us | 184.807 | 189.335 | 125.114 |
| c64 GET p99 us | 352.256–356.351 | 356.352–360.447 | 231.424–233.471 |
| c64 mixed combined calls/s | 170,326.223 | 168,640.129 | 503,781.883 |
| c64 mixed GET mean us | 387.535 | 390.292 | 126.912 |

All 12 smoke and 24 timed cohorts complete, with **49,504,037 measured calls**
succeeding in one attempt. Independent acceptance checks 80 exited lifetimes,
48 fresh drains/bindings, 4,678 resource samples and exact CPU/namespace
restoration. Seven driver/binding and 17 auditor contracts plus the statistics
contract pass. No original runtime is rerun or omitted. [Correctness and ordinary
recovery](GLOBAL-QUEUE-VALIDATION.md) pass 435 tests/doctests (one existing ignored),
formatting/Clippy and full histories with 363 operations (330 OK / 33 unknown).

Scope: default-feature uninstrumented builds, shared-host loopback, ordinary
three-voter quorum/sync on **tmpfs WAL**, versus standalone Redis without
persistence/pipelining. No equal-durability, real-disk, cross-host or significance
claim. Precisely reviewed obsolete debug intermediates supplied 6.06 GiB of
space; original binaries, WAL, histories, sources and storage guards remain.

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
The active distinct candidate tests ThinLTO and one release codegen unit while
keeping the fixed clients and all runtime/consistency semantics unchanged.
Its full release workspace checks pass 709 tests/doctests (23 existing ignored),
formatting and all-target Clippy. The retained production build is source-bound with actual ThinLTO/codegen flags
and panic unwinding verified; ordinary recovery and matched measurement remain
pending.
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

1. **Finish candidate acceptance:** keep `42e0117` frozen while completing
   applicable broader point/batch measurements and actual candidate Chaos Mesh.
   The short loaded-read improvement does not itself justify default promotion.
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
