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

The [event-one screen and original evidence](https://github.com/c4pt0r/kv9/blob/a7d6ccf/docs/RPC-EVENT-ONE-PERFORMANCE.md)
complete 24 cohorts: two forward/reverse ten-second repetitions, fixed v3
clients, 4,096 keys, 128-byte values, closed-loop load and fixed CPU placement.
KV9 uses three voters with ordinary quorum/sync on **tmpfs WAL**. Redis is
standalone with persistence and pipelining disabled. This is shared-host loopback,
not equal-durability, real-disk, cross-host or sustained-capacity evidence.

| Metric | Selected CRC | Event one | Redis |
| --- | ---: | ---: | ---: |
| c1 GET calls/s | 26,619 | 24,935 | 173,073 |
| c1 GET mean us | 37.449 | 39.987 | 5.701 |
| c1 GET p99 interval us | 49.664--50.175 | 51.200--51.711 | 7.552--7.615 |
| c64 GET calls/s | 345,535 | 300,321 | 510,849 |
| c64 GET mean us | 185.097 | 212.978 | 125.172 |
| c64 GET p99 interval us | 352.256--356.351 | 413.696--417.791 | 231.424--233.471 |
| c64 mixed combined calls/s | 171,794 | 162,542 | 502,676 |
| c64 mixed GET mean us | 384.665 | 408.502 | 127.179 |
| c64 mixed GET p99 interval us | 622.592--630.783 | 671.744--679.935 | 233.472--235.519 |

**Reject event one; keep CRC selected.** The only runtime change makes the
existing two-worker executor poll I/O every task rather than every eight tasks.
It loses 6.325% c1 GET, 13.085% c64 GET and 5.385% c64 mixed throughput. Means
and tails worsen in both repetitions, including separate mixed GET latency.
No full-matrix or candidate Chaos expansion follows this regression.

All 48,550,288 measured calls succeed in one attempt. Across all client phases,
48,848,344 successful calls / 48,848,360 attempts retain 16 initialization
routing attempts. The first full timing and independent audit accept 80 exited
lifetimes, 48 fresh drains/bindings, 4,679 resource samples and exact CPU/namespace
restoration. Source checks and 372-call ordinary recovery pass (338 OK / 34
unknown). The root statistics-wrapper inventory-shape failure is retained; its
correction does not rerun the workload or alter the frozen arithmetic.

Before timing, root reclaims only reviewed Go compiler-cache objects, recovering
5,155,196,928 available bytes. Original binaries, evidence and WALs stay intact;
Rust caches, downloads and uv stay unchanged. The 96-GiB retention floor remains.
Both repetitions, separate operation histograms and all outcomes are published.
This is a screen, not statistical significance or a general no-regression bound.

## Completed diagnosis and current prototype

The [request-body handoff diagnostic](https://github.com/c4pt0r/kv9/blob/a75c005/docs/RAFT-BODY-HANDOFF-RESULTS.md)
passes both fixed cells with 984,862 measured successes, eight exited lifetimes,
six drains and 12 metric documents. C1 heartbeat/response offer-to-poll means
are 0.444--0.475 us. At c64 mixed, leader heartbeat batches average 0.987 us and
mixed batches 3.244 us, with p99 32.768--65.535 us. These local populations are
not additive GET phases or wire/ACK latency. It does not justify replacing the
batch channel as the primary read optimization. Source tests and 368-call
ordinary recovery pass; original failed Clippy and its test-only fix are retained.

The [outbound peer-executor prototype](https://github.com/c4pt0r/kv9/commit/36ae89a774131b368cf1ed28e95df02ba325c8d4)
keeps two public workers/event8 and places existing outbound Raft tasks and
connections on one extra per-node worker. Inbound peer RPCs keep the existing
listener. No new per-message task, queue, port or cluster-wide service is added.
Total async workers rise from two to three; process CPU affinity stays unchanged.
The [source correspondence and ownership contract](https://github.com/c4pt0r/kv9/blob/36ae89a774131b368cf1ed28e95df02ba325c8d4/docs/PEER-EXECUTOR-ISOLATION.md)
preserves the existing transport state machine and consensus guards.

Default/experimental server suites pass 224/234 tests and doctests (overlapping,
one pre-existing ignored test each), formatting and all-target Clippy. Original
release/source readback binds 595 files and 11 freshly compiled first-party units.
The [ordinary recovery gate and original evidence](https://github.com/c4pt0r/kv9/blob/79ea149/docs/PEER-EXECUTOR-ISOLATION-VALIDATION.md)
pass 356 calls (328 OK / 28 unknown), complete stream/unary histories, five
server/two client lifetimes and six drains. Five process contracts pass. These
gates do not independently inject runtime-constructor failure or establish
actual Chaos/host failure, complete drop-order refinement or a performance gain.
No performance result is available for this prototype. It is not selected, and
its additional worker cost must remain visible in any comparison.

## Next development steps

1. **Screen outbound executor placement:** after source/recovery gates, compare
   the exact uninstrumented prototype with selected CRC and Redis at c1/c64 GET
   and mixed load. Keep both complete repetitions, GET mean/p95/p99, PUT outcomes,
   source/CPU identity and storage guards. Preserve originals while preparing
   enough retention space; do not relax the acceptance floor.
2. **Separate placement from resource count:** if promising, compare a shared
   three-worker/event8 control under the same CPU budget before attributing a
   gain to isolation. Inbound Raft still shares public HTTP2 handling; do not
   describe this prototype as complete network isolation. A regression ends its
   expansion. Avoid more unmotivated frequency sweeps or held-candidate replays.
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
