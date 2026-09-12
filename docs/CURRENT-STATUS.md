# Development checkpoint — 2026-09-11

Tracking: [#9](https://github.com/c4pt0r/kv9/issues/9). The target remains an
industrial distributed database. The active priority is Redis-class memory
RawKV reads, followed by dynamic multi-Raft and automatic range splits.

## Delivered foundation and remaining product limits

The project has a three-voter RawKV foundation with self-hosted metadata,
Raft-ordered durable writes, linearizable ReadIndex reads, MinIO checkpoints,
WAL-tail recovery, streaming RPC and atomic native batch APIs. The selected
runtime behavior is CRC `ca0002c7`; `86a689c8` additionally fixes retained-build
cache safety without changing database behavior. This documentation checkpoint
does not select a new runtime.

The full dataset still resides in RAM. Incremental bounded storage, complete
core-protocol implementation proofs, the full Chaos Mesh failure matrix,
independent host-failure acceptance, dynamic data groups and automatic splits
remain open. An abstract proof or one-host fault run does not close these gates.

## Latest completed uninstrumented performance

The [inbox vector reuse screen and exact evidence](https://github.com/c4pt0r/kv9/blob/51efc6598324c10c1e27cd6fef869d4b9a39e7c4/docs/INBOX-VECTOR-REUSE-PERFORMANCE.md)
complete 24 cohorts: two forward/reverse ten-second repetitions, fixed v3
clients, 4,096 keys, 128-byte values, closed-loop load and fixed CPU placement.
KV9 uses three voters with ordinary quorum/sync on **tmpfs WAL**. Redis is
standalone with persistence and pipelining disabled. This is shared-host
loopback, not equal-durability, disk, cross-host or sustained-capacity evidence.

| Metric | Selected CRC | Inbox vector reuse | Redis |
| --- | ---: | ---: | ---: |
| c1 GET calls/s | 26,725 | 26,593 | 174,388 |
| c1 GET mean us | 37.305 | 37.489 | 5.659 |
| c1 GET p99 interval us | 49.664-50.175 | 49.152-49.663 | 7.360-7.423 |
| c64 GET calls/s | 345,439 | 345,339 | 513,870 |
| c64 GET mean us | 185.144 | 185.202 | 124.437 |
| c64 GET p99 interval us | 352.256-356.351 | 356.352-360.447 | 229.376-231.423 |
| c64 mixed combined calls/s | 171,845 | 172,567 | 505,972 |
| c64 mixed GET mean us | 384.352 | 382.613 | 126.373 |
| c64 mixed GET p99 interval us | 622.592-630.783 | 622.592-630.783 | 231.424-233.471 |

Mixed c64 throughput improves **0.420%** (+0.255% / +0.585%), with a lower GET
mean in both repeats but unchanged GET p99. Pure c1 throughput falls **0.494%**
(-0.588% / -0.400%), with higher mean in both repeats. Pure c64 throughput is
**-0.029%** with opposite repeat signs; p95 worsens in both repeats, and p99
worsens in repeat 0. Pooled c1 p99 improves despite unchanged per-repeat buckets;
that is not a repeat-specific tail improvement. **Keep CRC selected; hold
1045755 and stop its full-matrix/Chaos expansion.**

All **49,860,618 measured calls** succeed in one attempt. The first complete
recording, independent audit and statistics pass, including 80 exited lifetimes,
48 fresh drains/bindings, 4,675 resource samples and exact restoration. Before
runtime, root reclaims only rebuildable pip HTTP downloads after a reference/link
census; source/binary identities, original evidence and storage guards remain
unchanged. Both repetitions and separate GET/PUT histograms are published.
This screen is not significance, a no-regression bound or full point/batch
acceptance. Benchmark sentinels do not replace complete histories.

The earlier [bounded Append-payload screen](https://github.com/c4pt0r/kv9/blob/381d0973c078522d3698a922ecac8594cc9df348/docs/RAFT-APPEND-PAYLOAD-PERFORMANCE.md)
remains held: +0.790% c64 GET, only +0.031% mixed throughput and unchanged mixed
GET p99. Its first 18/24 attempt stops on the unchanged 96-GiB retention floor;
a separate full second attempt passes after dev-cache reclamation. Both attempts
are retained, and only the complete second attempt supplies its statistics.

The previous [metadata/CRC combination](https://github.com/c4pt0r/kv9/blob/3641d0ff0644f700cf3ea8c9c1dbda8fd7462f00/docs/STREAM-METADATA-CRC-PERFORMANCE.md)
remains held: +0.010% c64 GET, -0.453% mixed throughput and worse mixed GET mean.

The [vector lookup experiment](https://github.com/c4pt0r/kv9/blob/8a47afabbb890f9a02485f593d6bbf75540e96cf/docs/VECTOR-RECEIPT-PERFORMANCE.md)
retains its 2.589% c64 mixed gain and pure-read tradeoffs. The previous
[deque/indexed experiment](https://github.com/c4pt0r/kv9/blob/3c7273e1df255e53a67bcfd477fd71453783127a/docs/INDEXED-RECEIPT-READ-MIXED.md)
and owner-pump/two-context experiments remain held for their documented
tradeoffs. Different recordings do not establish causal comparisons between
candidates. The [72-cohort matrix](https://github.com/c4pt0r/kv9/blob/aeee652135bc52e4527ef4fffd540ec184adc362/docs/READ-CREDIT-CRC-PERFORMANCE.md)
remains the latest broad comparison.

## Current experiment and correctness evidence

The [request-body handoff diagnostic](https://github.com/c4pt0r/kv9/blob/a75c005/docs/RAFT-BODY-HANDOFF-RESULTS.md)
now completes both fixed cells with 984,862 measured successes, eight exited
lifetimes, six drains and 12 checked metric documents. At c1, heartbeat/response
batch offer-to-poll means are 0.444--0.475 us; at c64 mixed, leader heartbeat
averages 0.987 us and mixed batches 3.244 us (p99 32.768--65.535 us). These
independent local populations are not additive GET phases or socket/ACK latency.
442 default / 234 experimental overlapping tests, two partition regressions,
formatting/Clippy and 368-call ordinary recovery pass. The first Clippy failure
and test-only unused-helper removal are retained. This diagnostic does not
justify replacing the batch channel as the primary optimization.

The clean [event-one candidate](https://github.com/c4pt0r/kv9/commit/5bed688c3b829770092ee6802078102b894ef6d5)
changes only event_interval(8) to (1), preserving two RPC workers and adaptive
global-queue scheduling. Its 224 default / 234 experimental overlapping server
tests, formatting and Clippy pass, and its original default release is bound.
The complete uninstrumented comparison and actual-runtime recovery remain pending.
No performance gain is claimed; CRC stays selected.

The [confirmation-queue diagnostic](https://github.com/c4pt0r/kv9/blob/c423d3c605bf6b88bda3521b85e6997c2b203120/docs/CONFIRMATION-QUEUE-RESULTS.md) is complete on independent
source `6530239`. Both fixed cells pass readback: 974,650 measured successes,
8 exited lifetimes, 6 drains and 12 metric documents. All-client accounting is
999,488 successful calls / 999,490 attempts. Under mixed load, the leader's
heartbeat sender queue averages 30.654 us and heartbeat-response inbox 20.758 us;
its batch-channel admission averages 0.268 us. These different message populations
are not additive request phases, and admission is not wire delivery. All 90
stage/kind and 630 outcome rows are published. Source gates and 369-call ordinary
recovery pass; this is diagnostic evidence, not a new performance selection.

The [inbox vector reuse candidate](https://github.com/c4pt0r/kv9/commit/1045755eac4292a2ccfdfa3d71741ed60e2f4668)
changes one runtime source function relative to CRC: it returns the inbox's
owned bounded FIFO vector and filters in place in testing builds. The
[source argument](https://github.com/c4pt0r/kv9/blob/1045755eac4292a2ccfdfa3d71741ed60e2f4668/docs/INBOX-VECTOR-REUSE.md)
preserves order, bounds, partition observations and wakeups. Local release-profile
gates pass 435 Raft/server tests/doctests (one ignored), two explicit testing
partition regressions, formatting and Clippy. The original clean default release
binds all 595 source files. Fresh ordinary stream/unary recovery passes:
359 calls, 330 OK and 29 unknown, five server/two client lifetimes and six drains.
Seven driver/source-binding and 17 auditor checks pass before runtime.
This source/ordinary-process evidence does not establish actual Chaos or a
whole-implementation proof. The [completed screen](https://github.com/c4pt0r/kv9/blob/51efc6598324c10c1e27cd6fef869d4b9a39e7c4/docs/INBOX-VECTOR-REUSE-PERFORMANCE.md)
shows a small mixed gain with pure-read tradeoffs, so no promotion follows.
The previous Append source gates and 357-call recovery remain published in
its report; catch-up correctness does not establish catch-up performance.

### Previous held metadata combination

[Candidate bb13e43](https://github.com/c4pt0r/kv9/commit/bb13e4313c313ca910969d1f01ef862619b57906)
integrates immutable stream metadata with selected CRC, allocator and read
workers. Its six adapter Rust files and metadata-independent consumer sections
match the original adapter. Every frame authenticates freshly. The existing
[source checks](https://github.com/c4pt0r/kv9/blob/8a47afabbb890f9a02485f593d6bbf75540e96cf/docs/stream-metadata-crc-source-v1/README.md)
pass 228 default and 238 experimental server tests/doctests, one ignored in each
overlapping configuration, formatting and experimental server Clippy. All 596
source files and original release hashes are freshly revalidated; Raft,
storage, metadata and main runtime source are unchanged from selected CRC.

New complete stream/unary leader-loss/restart histories pass independent
checking: **370 calls, 340 OK and 30 unknown**, with all five server/two client
lifetimes exited and six fresh voter drains. Unknowns remain in both histories.
The screen's first preparation audit caught one stale argument-file hash;
its one-literal correction passes all 17 auditor contracts. The first failure
and all six passing driver contracts are retained. No runtime was rerun for it.
Actual exact-bb13 Chaos and full-workspace qualification have not run. Static
Chaos preparation is preserved but will not expand for this held candidate.
Core proof composition and industrial availability gates remain open.

## Next development steps

The [existing CRC lifecycle diagnostic](https://github.com/c4pt0r/kv9/blob/3641d0ff0644f700cf3ea8c9c1dbda8fd7462f00/docs/READ-LIFECYCLE-CRC-CREDIT.md)
measures 20.887 us from submission to observed confirmation within a 24.285 us
c1 sampled barrier. C64 mixed records 168.753 us in confirmation and 56.070 us
in notification. These whole-client successful sample populations do not
isolate network RTT or the measurement-only request latency. The
[CPU/thread profile](https://github.com/c4pt0r/kv9/blob/6fb3434c7cafcbd8d93b04f298208d5785fa292a/docs/READ-PATH-CPU-PROFILE.md)
also does not assign async requests to OS-thread waits.

1. **Complete the event-one comparison:** evaluate the one-setting candidate
   against selected CRC and Redis using the fixed forward/reverse 24-cohort
   screen, after original-binary recovery checks. Compare c1/c64 GET and mixed
   throughput, GET means and tails together. More frequent executor maintenance
   may reduce waiting or add overhead; retain both repetitions and all outcomes.
2. **Follow measured scheduling or transport costs:** keep the completed body
   diagnostic and held candidates as evidence, without blind reruns. A useful
   uninstrumented change needs applicable proof, full point/batch and actual
   exact-source Chaos before promotion. If event-one regresses, hold it and
   select a specific remaining ownership/encoding/socket boundary. Preserve fresh
   Safe ReadIndex, durable writes and full pump/apply/view fences. DPDK needs
   cross-host/NIC evidence. Selected runtime remains CRC.
3. **Consistency and availability closure:** continue implementation/proof mapping,
   remote admission bounds, persistence failure cuts and actual Chaos Mesh E2E.
   Cover metadata, routing and scheduling without a service-critical singleton;
   only the object-store dependency is exempt. Local Kind cannot prove host loss.
4. **Scale-out after the read milestone:** implement bounded RegionManager and
   dynamic multi-Raft (#22), epoch-fenced routing (#23), recoverable learner
   attachment/membership (#24), then durable automatic split intent, ownership
   fencing, data handoff and idempotent recovery (#25). Follow with placement and
   independent-hotspot scaling (#27). Keep snapshot, retention and storage
   prerequisites from the [development path](DEVELOPMENT-PATH.md).

Write durability is mandatory. Optimize batching, pipelining and CPU work where
measured, but evaluate disk costs on real storage with equivalent acknowledgement
requirements. Do not sacrifice Raft consistency to match standalone Redis writes.
Advanced TLS remains later work. Run CI locally; dispatch hosted CI only for a
release or an explicitly chosen key milestone. No hosted CI ran for this checkpoint.
