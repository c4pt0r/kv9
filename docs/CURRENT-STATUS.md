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

The [bounded Append-payload screen and exact evidence](https://github.com/c4pt0r/kv9/blob/381d0973c078522d3698a922ecac8594cc9df348/docs/RAFT-APPEND-PAYLOAD-PERFORMANCE.md)
complete a fresh 24-cohort comparison: two forward/reverse ten-second
repetitions, fixed v3 clients, 4,096 keys, 128-byte values, closed-loop load and
fixed CPU placement. KV9 uses three voters with ordinary quorum/sync on
**tmpfs WAL**. Redis is standalone with persistence and pipelining disabled.
This is shared-host loopback, not equal-durability, disk, cross-host or
sustained-capacity evidence.

| Metric | Selected CRC | 64-KiB Append | Redis |
| --- | ---: | ---: | ---: |
| c1 GET calls/s | 26,443 | 26,492 | 174,086 |
| c1 GET mean us | 37.701 | 37.625 | 5.667 |
| c1 GET p99 interval us | 50.176-50.687 | 50.176-50.687 | 7.232-7.295 |
| c64 GET calls/s | 343,972 | 346,690 | 509,202 |
| c64 GET mean us | 185.934 | 184.477 | 125.576 |
| c64 GET p99 interval us | 352.256-356.351 | 352.256-356.351 | 231.424-233.471 |
| c64 mixed combined calls/s | 171,766 | 171,818 | 502,854 |
| c64 mixed GET mean us | 384.575 | 384.376 | 127.140 |
| c64 mixed GET p99 interval us | 622.592-630.783 | 622.592-630.783 | 233.472-235.519 |

Pure c64 throughput rises **0.790%** pooled (+0.145% / +1.437%), with
opposite repeat movements in p99. Mixed throughput changes only **+0.031%**
(+0.096% / -0.034%); mixed GET p99 is unchanged in both repeats and its mean
is essentially unchanged. C1 GET changes +0.184% with opposite repeat signs.
The candidate does not demonstrate the intended mixed-load benefit.
**Keep CRC selected; hold this candidate and stop its full-matrix/Chaos expansion.**

The first timing attempt stops on the unchanged 96-GiB retention guard after
18/24 cohorts. Root preserves it, reclaims 7.611 GiB of rebuildable dev cache
under the shared lock, and revalidates original sources/binaries. A separate
whole second attempt passes with unchanged protocols, order and guards; none
of the first attempt is pooled into statistics. All **49,675,313 measured calls**
succeed in one attempt. Its independent audit accepts 80 exited lifetimes,
48 fresh drains/bindings, 4,678 resource samples and exact restoration. Both
repetitions, separate GET/PUT histograms and all outcomes are published.
This screen is not significance, a no-regression bound or full point/batch
acceptance. Benchmark sentinels do not replace complete histories.

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

The [confirmation-queue diagnostic](https://github.com/c4pt0r/kv9/blob/c423d3c605bf6b88bda3521b85e6997c2b203120/docs/CONFIRMATION-QUEUE-RESULTS.md) is complete on independent
source `6530239`. Both fixed cells pass readback: 974,650 measured successes,
8 exited lifetimes, 6 drains and 12 metric documents. All-client accounting is
999,488 successful calls / 999,490 attempts. Under mixed load, the leader's
heartbeat sender queue averages 30.654 us and heartbeat-response inbox 20.758 us;
its batch-channel admission averages 0.268 us. These different message populations
are not additive request phases, and admission is not wire delivery. All 90
stage/kind and 630 outcome rows are published. Source gates and 369-call ordinary
recovery pass; this is diagnostic evidence, not a new performance selection.

The held [bounded Append-payload candidate](https://github.com/c4pt0r/kv9/commit/74b958a8bcdf25252ab55ba6149876a1cddc0637)
sets raft-rs `max_size_per_msg` to 64 KiB instead of its zero default (one entry
per Append). It uses the existing contiguous log-slice mechanism without a new
batch timer; `batch_append` remains disabled. The real lag/rejoin test checks
multi-entry messages, the target's single-oversized-entry exception and final
replica values. Local source gates pass 436 Raft/server and 234 experimental
server tests/doctests, with overlap and one ignored per configuration, plus
formatting and Clippy. Its clean original release and independent 357-call
ordinary recovery pass (327 OK, 30 unknown; five server/two client lifetimes).
The [complete uninstrumented screen](https://github.com/c4pt0r/kv9/blob/381d0973c078522d3698a922ecac8594cc9df348/docs/RAFT-APPEND-PAYLOAD-PERFORMANCE.md)
now shows no useful mixed-load benefit. No Chaos qualification or runtime
promotion follows from this experiment; its catch-up regression is functional
evidence, not a catch-up performance measurement.

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

1. **Remove redundant inbox vector construction:** `GrpcTransport::drain`
   currently moves the inbox's owned bounded FIFO vector into another vector.
   Reuse that vector while preserving testing-feature partition filtering,
   ordering, admission bounds and wakeups. Check both feature paths, then run
   the same complete c1/c64 GET/mixed screen. This is the next proposed change,
   not an established gain. Do not repeat Append-size tuning without a new cause.
2. **Qualify only a useful change:** retain fresh Safe ReadIndex, durable writes,
   full pump/apply/view fences, queue bounds and cancellation ownership. Complete
   the applicable proof mapping, full point/batch matrix and exact-source actual
   Chaos Mesh before promotion. If vector reuse is insufficient, inspect the
   peer-worker to request-body channel handoff, including stream-progress timeout
   and route cancellation, before changing scheduling. Do not replay held
   experiments without a new cause. DPDK needs cross-host/NIC
   evidence. The selected runtime stays CRC until a candidate passes its gates.
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
