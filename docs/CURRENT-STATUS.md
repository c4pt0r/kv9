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

## Latest completed uninstrumented performance

The [outbound executor screen and original evidence](https://github.com/c4pt0r/kv9/blob/3ed8661de5e2f5a3a1f5cc1cdf31ac8cd76a655c/docs/PEER-EXECUTOR-ISOLATION-PERFORMANCE.md)
complete 24 cohorts: two forward/reverse ten-second repetitions, fixed v3
clients, 4,096 keys, 128-byte values, closed-loop load and fixed CPU placement.
KV9 uses three voters with ordinary quorum/sync on **tmpfs WAL**. Redis is
standalone with persistence and pipelining disabled. This is shared-host loopback,
not equal-durability, real-disk, cross-host or sustained-capacity evidence.

| Metric | Selected CRC | Outbound isolation | Redis |
| --- | ---: | ---: | ---: |
| c1 GET calls/s | 26,751 | 27,863 | 173,283 |
| c1 GET mean us | 37.265 | 35.774 | 5.693 |
| c1 GET p99 interval us | 49.664--50.175 | 47.616--48.127 | 7.680--7.743 |
| c64 GET calls/s | 345,508 | 330,252 | 507,233 |
| c64 GET mean us | 185.108 | 193.665 | 126.062 |
| c64 GET p99 interval us | 352.256--356.351 | 364.544--368.639 | 229.376--231.423 |
| c64 mixed combined calls/s | 171,768 | 168,220 | 500,948 |
| c64 mixed GET mean us | 384.481 | 395.922 | 127.628 |
| c64 mixed GET p99 interval us | 622.592--630.783 | 679.936--688.127 | 233.472--235.519 |

**Do not promote outbound isolation; keep CRC selected.** Its extra per-node
worker improves c1 GET by 4.159%, but loses 4.415% c64 GET and 2.066% c64 mixed
throughput. Both repetitions have the same direction. C64 means and tails worsen,
including separate mixed GET. The default read-performance gate fails, so no
full-matrix or candidate Chaos expansion follows. The selected CRC remains about
6.48x below same-run Redis c1 throughput and 1.47x below c64 throughput.

All 49,236,254 measured calls succeed in one attempt. Across all client phases,
49,534,310 successful calls / 49,534,326 attempts retain 16 initialization routing
attempts. The first complete timing and independent audit accept 80 exited
lifetimes, 48 fresh drains/bindings, 4,675 resource samples and exact CPU/namespace
restoration. Seven driver/source-binding and 17 auditor contracts pass, as do
12 smoke cells. Previously completed source/recovery gates remain accepted and
are not rerun: 224/234 overlapping server tests/doctests, formatting/Clippy and
356-call ordinary recovery (328 OK / 28 unknown).

Before timing, root reclaims only reviewed release compiler-cache objects and
archive-backed extracted Cargo packages, recovering 2,897,895,424 available bytes.
Original binaries, source/recovery evidence, WALs, crate downloads and named
build/package locks remain intact. The original file-only capacity rejection
and its derived directory-allocation accounting remain published. Actual storage
guards are unchanged. Both repetitions, separate operation histograms, all
outcomes and original preparation failures are retained. This is a screen,
not statistical significance or a general no-regression bound.

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

A concrete next target is `WorkSignal::notify`: it coalesces `pending` but still
calls `notify_one` repeatedly. Recovered notification stacks account for about
4% of each CPU population, including futex work; the removable fraction remains
unmeasured. The next candidate should suppress notifications while `pending` is
already true, with a mechanized no-lost-wakeup refinement and concrete race tests.
Original setup/reader failures remain published; both environment restorations
pass. All work is local and no hosted CI is dispatched.

## Next development steps

1. **Coalesce redundant owner wakeups:** start from CRC and notify only on the
   false-to-true `pending` transition under the existing mutex. Preserve stop,
   publication/drain ordering, the atomic park boundary and independent ticks.
   Mechanize equivalence of `RSNotify`/`RSHint` under `RSParkedSignal`, then check
   concrete producer/drain/park/stop races and recovery. The existing evidence
   identifies a candidate mechanism, not its effect size.
2. **Measure the identified change:** quantify suppressed notifications and
   syscall/CPU cost, then run the same uninstrumented c1 GET, c64 GET and c64
   mixed screen with full outcomes and operation-specific means/tails. Keep
   existing queues, ownership, fairness and the selected shared executor. Avoid
   further runtime sweeps without new evidence.
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
