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

The [vector-receipt screen and evidence](https://github.com/c4pt0r/kv9/blob/8a47afabbb890f9a02485f593d6bbf75540e96cf/docs/VECTOR-RECEIPT-PERFORMANCE.md)
complete 24 cohorts: two forward/reverse ten-second repetitions, fixed v3
clients, 4,096 keys, 128-byte values, closed-loop load and fixed CPU placement.
KV9 uses three voters with ordinary quorum/sync on **tmpfs WAL**. Redis is
standalone with persistence and pipelining disabled. This is shared-host
loopback, not equal-durability, disk, cross-host or sustained-capacity evidence.

| Metric | Selected CRC | Vector lookup | Redis |
| --- | ---: | ---: | ---: |
| c1 GET calls/s | 26,537 | 26,479 | 173,699 |
| c1 GET mean us | 37.571 | 37.650 | 5.680 |
| c1 GET p99 interval us | 50.688-51.199 | 49.664-50.175 | 7.296-7.359 |
| c64 GET calls/s | 345,613 | 344,406 | 508,212 |
| c64 GET mean us | 185.051 | 185.702 | 125.820 |
| c64 GET p99 interval us | 348.160-352.255 | 352.256-356.351 | 231.424-233.471 |
| c64 mixed combined calls/s | 171,677 | 176,121 | 502,276 |
| c64 mixed GET mean us | 384.797 | 374.367 | 127.309 |
| c64 mixed GET p99 interval us | 622.592-630.783 | 606.208-614.399 | 233.472-235.519 |

C64 mixed throughput improves **2.589%** and GET mean improves **2.711%**;
GET and PUT p99 improve in both repetitions. Pure c64 throughput falls
**0.349%** pooled (-0.536% / -0.163%); p99 is unchanged in the first repeat
and worse in the second. Pure c1 throughput falls 0.220% and mean increases
0.212%, although its p95/p99 improve in both repetitions. C1 mixed throughput
falls 0.809%. These observations do not establish significance or a
no-regression bound. **Keep CRC selected; hold general promotion.**

All **49,724,855 measured calls** succeed in one attempt. The first independent
audit accepts 80 exited lifetimes, 48 fresh drains/bindings, 4,677 resource
samples and exact restoration. Both repetitions, separate GET/PUT histograms
and all outcome populations are published. No cohort is removed or rerun.
Benchmark sentinels do not replace complete linearizability histories.
This screen contains no batch or write-only comparison.

The [previous deque/indexed screen](https://github.com/c4pt0r/kv9/blob/3c7273e1df255e53a67bcfd477fd71453783127a/docs/INDEXED-RECEIPT-READ-MIXED.md)
retains its mixed/write gains and pure-read tradeoffs. This vector-only lookup
experiment still shows a small pure-read shift; it does not establish that
the deque caused the prior shift. Different recordings are not causal
comparisons between candidates. The [72-cohort matrix](https://github.com/c4pt0r/kv9/blob/aeee652135bc52e4527ef4fffd540ec184adc362/docs/READ-CREDIT-CRC-PERFORMANCE.md)
remains the latest broad comparison; earlier owner-pump and two-context
experiments remain held for their documented tradeoffs.

## Current experiments and correctness evidence

[Vector candidate 7c8cd25](https://github.com/c4pt0r/kv9/commit/7c8cd25e9531809c9d5bd40faee116a7d6d5a3ff)
keeps original vector push/drain retention, adding binary lookup under a checked
index-order certificate. Duplicate or reordered indexes permanently restore
first-match lookup. Complete receipt outcomes and eviction uncertainty remain
unchanged. It passes 213 Raft tests/doctests, formatting, Raft Clippy, and two
compiled negative controls with passing baselines/restorations. Its conditional
local TLA+/TLAPS proof passes 13 parameterized theorems and 122 obligations per
positive run; four positive runs are not four times as many distinct obligations.
Complete ordinary stream/unary leader-loss/restart histories pass: 351 calls,
321 OK and 30 unknown. All 596 tested source files bind to the clean commit.
No full-workspace or exact-vector-source Chaos Mesh run is claimed.

[Stream metadata candidate bb13e43](https://github.com/c4pt0r/kv9/commit/bb13e4313c313ca910969d1f01ef862619b57906)
now integrates the earlier immutable metadata adapter onto selected CRC,
including the current allocator and read-worker configuration. Its six Rust
files match the original adapter; the five Raw handlers, identity/context
readers and admission consumer sections also match. Its [source validation](https://github.com/c4pt0r/kv9/blob/8a47afabbb890f9a02485f593d6bbf75540e96cf/docs/stream-metadata-crc-source-v1/README.md)
passes 228 default and
238 experimental server tests/doctests, one ignored in each overlapping
configuration, formatting and experimental server Clippy. Every frame still
authenticates freshly. Current-combination performance, process recovery and
Chaos acceptance are pending; the older isolated candidate's evidence does
not transfer. Neither experiment changes selected runtime behavior.

## Next development steps

The [fresh selected-source CPU/thread diagnostic](https://github.com/c4pt0r/kv9/blob/6fb3434c7cafcbd8d93b04f298208d5785fa292a/docs/READ-PATH-CPU-PROFILE.md)
passes c1 GET/c64 mixed recording and analysis. Exact-binary attribution puts
4.189% of mixed CPU samples in linear apply-receipt search. Instrumented CPU
and thread intervals are not new QPS results or per-request wait attribution.

1. **Remove per-request RPC metadata allocation:** finish qualifying the new
   CRC/immutable-metadata combination. Reuse only one bounded representation
   per admitted stream; preserve per-frame authentication, dynamic revocation,
   role/principal changes, fallback credentials and all admission ownership.
   The prior system-allocator screen provides a hypothesis, not a combined gain.
2. **Measure pure and mixed reads:** after exact-source recovery and smoke,
   compare c1/c64 with the same fixed client/control/Redis protocol. Retain
   both repeats and raw histograms. A useful candidate still requires the full
   point/batch matrix and exact-source actual Chaos Mesh before promotion.
   Preserve deadlines, fresh Safe ReadIndex and full pump/apply/view fences.
   Tonic streaming remains selected; DPDK needs actual cross-host/NIC evidence.
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
