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

## Latest diagnosis and active experiment

[Matched asynchronous read stages](READ-STAGE-RESULTS.md) now isolate the next
performance work. In the c1 diagnostic lifecycle, quorum confirmation averages
20.636 us (85.72% of the observed read wait); at c64, receiver resumption averages
42.449 us (37.47%). These successful stage chains include initialization and
verification, and are not measurement-only latency or new uninstrumented QPS.

Source `40f014f` passes default435/diagnostic438 tests and both Clippy variants.
Two original fixtures pass independent acceptance with 1,811,433 measured
single-attempt successes, eight exited lifetimes, six drains and 12 metric
documents. The next candidate explicitly checks the RPC executor's global task
queue every eight selections, targeting remote completion wakes while local
RPC tasks stay busy. Its correctness/recovery/performance gates are in progress;
no candidate is selected. The c1 quorum round trip remains a separate target.

## Latest completed uninstrumented performance

The [notification-coalescing screen](COALESCED-OWNER-PERFORMANCE.md) completes
12 smoke and 24 timed cohorts using two opposite ten-second run orders, fixed
v3 clients, 4,096 keys, 128-byte values and identical CPU placement. Candidate
`42e0117` only suppresses duplicate owner wakes; selected runtime remains CRC.

| Metric | Selected CRC | Notification candidate | Redis |
| --- | ---: | ---: | ---: |
| c1 GET calls/s | 26,461.551 | 26,395.976 | 174,159.224 |
| c1 GET mean us | 37.674 | 37.772 | 5.665 |
| c1 GET p99 interval us | 50.176–50.687 | 50.176–50.687 | 7.360–7.423 |
| c64 GET calls/s | 345,626.302 | 349,507.003 | 513,504.422 |
| c64 GET mean us | 185.046 | 182.990 | 124.523 |
| c64 GET p99 interval us | 352.256–356.351 | 331.776–335.871 | 229.376–231.423 |
| c64 mixed combined calls/s | 171,594.529 | 176,278.501 | 503,716.521 |
| c64 mixed GET mean us | 384.744 | 375.026 | 126.920 |
| c64 mixed GET p99 interval us | 622.592–630.783 | 606.208–614.399 | 233.472–235.519 |

C64 GET throughput improves **1.123%** and mixed throughput **2.730%**, with
better loaded/mixed GET mean and p99 in both repetitions. C1 GET is slightly
slower: **-0.248% throughput / +0.260% mean**. C1 mixed moves in different
directions across repeats. Keep this modest loaded-read candidate experimental;
broader API measurements and actual exact-source Chaos remain pending. C1 mean
is still about 6.67x Redis and c64 GET throughput about 68.1% of Redis.

All **49,944,895 measured calls** succeed in one attempt. Complete phase accounting
retains initialization routing attempts. The independent audit accepts 80 exited
lifetimes, 48 fresh drains/bindings, 4,679 resource samples and exact CPU/namespace
restoration. Seven driver/binding and 17 auditor contracts pass. No runtime
cohort was repeated or omitted. See the report for all four cells and both repeats.

[Local validation](COALESCED-OWNER-VALIDATION.md) passes 14 new TLAPS theorems /
64 obligations plus unchanged scheduling dependency 33 / 294, 438 default
Raft/Server tests/doctests (one existing ignored), 214 overlapping Raft testing
tests/doctests, formatting/Clippy, and ordinary recovery with 369 operations
(341 OK / 28 unknown). Three new concurrent tests cover producer/drain/stop races.
This does not close full Rust/database proofs, candidate Chaos or host failure.

Scope: shared-host loopback, ordinary three-voter quorum/sync on **tmpfs WAL**,
versus standalone Redis with persistence/pipelining disabled. No equal-durability,
real-disk, cross-host, sustained-capacity or statistical-significance claim.
Root reclaimed only 8.14 GiB of obsolete debug intermediates to preserve the
unchanged storage guards; original executables, evidence and WALs remain intact.

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
