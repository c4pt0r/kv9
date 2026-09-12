# Development checkpoint — 2026-09-12

Tracking: [#9](https://github.com/c4pt0r/kv9/issues/9). The target remains an
industrial distributed database. Memory RawKV read performance comes before
dynamic multi-Raft and automatic range splits.

## Delivered foundation and product limits

The current foundation provides three-voter RawKV, self-hosted metadata,
Raft-ordered durable writes, fresh linearizable ReadIndex reads, MinIO
checkpoints, WAL-tail recovery, streaming RPC and atomic native batch APIs.
Main now selects [ThinLTO integration `11113f6`](RELEASE-THIN-LTO-MAIN-INTEGRATION.md).
The fresh default server reproduces the qualified candidate executable exactly.
Other experimental branches remain separate.

The full dataset still resides in RAM. Incremental bounded storage, complete
core-protocol implementation proofs, the full actual Chaos Mesh failure matrix,
independent host-failure acceptance, dynamic data groups and automatic splits
remain open. An abstract proof or one-host fault run does not close these gates.

## Latest fixed-rate write diagnosis

The [eight-cohort fixed-rate result](BATCH-WRITE-FIXED-RATE-RESULTS.md) is now
accepted. It preserves the write-tail problem: at 8k offered BatchPut(64) calls/s,
pooled p99 is **4.850–4.915 ms CRC / 5.439–5.505 ms ThinLTO**; at 12k,
**6.750–6.816 ms / 7.143–7.209 ms**. Scheduled-to-completion p99 also rises in
both run orders. All 797,956 issued calls succeed with one attempt, but 2,044
of 800,000 offered slots are dropped by the client; equal realized work is not
established. This is a new diagnosis, not a new optimization or maximum-QPS run.

Four smoke/eight timed cohorts, 32 exited timed lifetimes, 24 fresh drains and
all twelve byte-retention records pass independent readback. The first reader's
obsolete Redis pairing lookup failed after all eight per-cohort checks; its
six-line repair and original failure remain published. No cohort was rerun.
The next main implementation investigation is the exact single-GET quorum path.

## Latest completed optimization experiment

[ThinLTO complete72 qualification](RELEASE-THIN-LTO-FULL72.md) improves throughput
and mean latency in all 12 point/batch workload cells and both run orders.
C64 GET throughput rises **8.465%**, c1 GET **6.806%**, and c64 point mixed
throughput **10.390%**. Loaded BatchPut(64) has a tail tradeoff: pooled p99 rises
from **8.258–8.323 ms to 8.389–8.520 ms**, despite better throughput and mean.
The other 11 pooled cells improve p99; all individual repetitions remain visible.

| Same-run metric | CRC baseline | Qualified ThinLTO | Redis |
| --- | ---: | ---: | ---: |
| c1 GET calls/s | 26,510.350 | 28,314.673 | 174,264.654 |
| c1 GET mean us | 37.605 | 35.206 | 5.660 |
| c1 GET p99 us | 49.664–50.175 | 44.544–45.055 | 7.104–7.167 |
| c64 GET calls/s | 346,550.757 | 375,885.286 | 512,498.244 |
| c64 GET mean us | 184.555 | 170.140 | 124.766 |
| c64 GET p99 us | 352.256–356.351 | 323.584–327.679 | 231.424–233.471 |
| c64 BatchGet(64) items/s | 2,277,629.998 | 2,342,052.591 | 5,964,711.568 |
| c64 BatchPut(64) items/s | 878,613.230 | 905,638.841 | 6,091,968.177 |

All 36 smoke and 72 timed cohorts complete. Independent acceptance verifies
**81,648,272 measured single-attempt successes**, zero dropped slots, 240 exited
lifetimes, 144 fresh drains/bindings and exact CPU/namespace restoration. All
24 smoke and 48 timed native retention records were independently decoded.
Current full72 originals are cold; their compressed objects remain local and
original-path audits require rehydration. The full report separates logical,
compressed and allocated byte totals.

[Validation](RELEASE-THIN-LTO-VALIDATION.md) retains 709 workspace tests/doctests
(23 existing ignored), formatting/Clippy and 359 complete ordinary recovery
operations. Candidate `02d0c01` also passes the
[21-window actual Chaos matrix](RELEASE-THIN-LTO-CHAOS.md): **9,923 complete
history operations** (9,371 OK / 35 refused / 517 unknown), positive fault
effects, four fresh final drains and 33 server lifetimes / 25 containers
confirmed exited or removed. Original failures retain their original scope.

[Fresh main integration](RELEASE-THIN-LTO-MAIN-INTEGRATION.md) now passes 709
workspace tests (23 existing ignored), 438 observer-feature tests (1 ignored),
formatting and both Clippy configurations. A separate default build reproduces
the qualified server/client bytes; ordinary recovery accepts 291 operations
(262 OK / 29 unknown), six fresh drains and seven exited lifetimes. All three
voters retain the default 26 metrics. The missing verbose-build-log failure and
its logging-only repair remain recorded. The original full72 result reaches
**73.344% of Redis c64 GET throughput**; isolated GET mean remains
**6.220 times Redis**. Read parity is not complete.

Scope: fixed clients, default uninstrumented production builds, shared-host
loopback and ordinary three-voter quorum/sync on **tmpfs WAL**, versus Redis
without persistence/pipelining. No equal-durability, real-disk, cross-host,
sustained-capacity or significance claim. Compression runs after writer exit
between cohorts; no codec overlaps measurement, but gaps can affect cache and
thermal state. Storage floors and caps remain unchanged.

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
ThinLTO is now integrated after the full72 comparison, original 21-window fault
gate and fresh main-source/default-build/recovery confirmation. Original timing
and Chaos receipts retain their source/build identities. The completed
[common offered-load diagnosis](BATCH-WRITE-FIXED-RATE-RESULTS.md) retains higher
write tails and client scheduling/drop limitations; it does not replace the
original closed-loop result. The
[quorum-path plan](QUORUM-LATENCY-NEXT.md) now maps exact source
boundaries and rejects ambiguous repeated-context/route-generation timing.
Keep notification candidate `42e0117` separate. Redis read parity remains open
before dynamic multi-Raft and automatic splits.

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

1. **Reduce isolated GET latency:** localize serial RPC, owner-service and
   completion-wait costs with the bounded quorum-path diagnostic. Preserve the
   selected shared executor and consensus boundaries; current evidence does not
   justify another worker-count or transport sweep. Keep repeated-context and
   route-generation ambiguity explicit; do not repeat screens without a cause.
2. **Retain the write-tail problem and separate arrival calibration:** the fixed
   offered-load diagnosis is complete, with higher ThinLTO p99 and nonzero client
   drops. Check the pinned client's timer/strided-slot fidelity separately before
   claiming matched work or a server-specific cause. Preserve the original
   results. Keep `42e0117` separate; combinations require their own qualification.
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
