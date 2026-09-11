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

The latest [indexed-receipt read/mixed screen and exact evidence](https://github.com/c4pt0r/kv9/blob/3c7273e1df255e53a67bcfd477fd71453783127a/docs/INDEXED-RECEIPT-READ-MIXED.md)
completes 24 cohorts: two forward/reverse ten-second repetitions, fixed v3
clients, 4,096 keys, 128-byte values, closed-loop load and fixed CPU placement.
KV9 uses three voters with ordinary quorum/sync on **tmpfs WAL**. Redis is
standalone with persistence and pipelining disabled. This is shared-host
loopback, not equal-durability, disk, cross-host or sustained-capacity evidence.

| Metric | Selected CRC | Indexed receipts | Redis |
| --- | ---: | ---: | ---: |
| c1 GET calls/s | 26,408 | 26,404 | 174,356 |
| c1 GET mean us | 37.754 | 37.762 | 5.658 |
| c1 GET p99 interval us | 50.688-51.199 | 50.176-50.687 | 7.360-7.423 |
| c64 GET calls/s | 346,078 | 344,657 | 513,879 |
| c64 GET mean us | 184.806 | 185.566 | 124.434 |
| c64 GET p99 interval us | 352.256-356.351 | 356.352-360.447 | 229.376-231.423 |
| c64 mixed combined calls/s | 172,071 | 176,847 | 499,647 |
| c64 mixed GET mean us | 383.691 | 372.473 | 127.963 |
| c64 mixed GET p99 interval us | 622.592-630.783 | 606.208-614.399 | 233.472-235.519 |

C64 mixed throughput improves **2.775%** and GET mean improves **2.924%**;
GET p95/p99 improve in both repetitions. Pure c64 GET throughput falls
**0.411%** pooled (-0.211% / -0.610%), with higher p95/p99 in both repeats.
C1 pure GET is almost unchanged: -0.016% throughput, +0.022% mean. These
observations do not establish significance or a no-regression bound.
**Keep CRC selected and retain indexed receipts as a mixed/write candidate.**

All **49,825,632 measured calls** succeed in one attempt. The first independent
audit accepts 80 exited lifetimes, 48 fresh drains/bindings, 4,677 resource
samples and exact restoration. Both complete repetitions, separate GET/PUT
histograms and all outcome populations are published. No cohort is removed or
rerun. Benchmark sentinels do not substitute for complete linearizability
histories. This screen contains no batch or write-only comparison.

The [earlier indexed write screen](https://github.com/c4pt0r/kv9/blob/3c7273e1df255e53a67bcfd477fd71453783127a/docs/INDEXED-RECEIPT-PERFORMANCE.md)
retains its point PUT gain and inconclusive batch result. The
[owner-local pump](https://github.com/c4pt0r/kv9/blob/99d94fb2789d8dc5886e8d1471a658be9f600e27/docs/OWNER-READ-PUMP-SCREEN.md)
and [two-context admission](https://github.com/c4pt0r/kv9/blob/119a49cc2f5f35c7f68b8d2301a71e563a36140c/docs/READ-WINDOW-SCREEN.md)
experiments remain held for their documented tradeoffs. The
[72-cohort one-context matrix](https://github.com/c4pt0r/kv9/blob/aeee652135bc52e4527ef4fffd540ec184adc362/docs/READ-CREDIT-CRC-PERFORMANCE.md)
remains the latest broad comparison. Different recordings are not causal
comparisons between candidate speeds.

## Current candidate and correctness evidence

[Candidate 74d24116](https://github.com/c4pt0r/kv9/commit/74d241161eadf2121128f6fd9101ead18e562765)
uses a bounded deque and binary lookup under a checked index-order certificate.
Duplicate or reordered indexes permanently restore original first-match
lookup. Exact receipt payloads, term/fence verdicts and eviction uncertainty
remain unchanged.

Its existing evidence includes 711 workspace tests/doctests (23 ignored),
formatting, Clippy, and complete ordinary stream/unary leader-loss/restart
histories: 378 calls, 347 OK and 31 unknown. Its local TLA+/TLAPS representation
proof discharges 13 parameterized theorems and 122 distinct obligations under
explicit standard-container/search and locking contracts. These gates were
not rerun or enlarged for this unchanged source.

Fresh readback binds all 592 source files and the original two-stage release's
11 first-observed compiled units. The first overly strict server-only reader
and corrected stage-aware reader are retained. Six pure driver and 17 auditor
contracts pass, and the runtime retains per-cohort source/build checks.
No exact indexed-candidate Chaos Mesh runtime has run. Full Rust/Raft proof
composition, storage-failure and independent-host acceptance remain open.

## Next development steps

The [fresh selected-source CPU/thread diagnostic](https://github.com/c4pt0r/kv9/blob/6fb3434c7cafcbd8d93b04f298208d5785fa292a/docs/READ-PATH-CPU-PROFILE.md)
passes c1 GET/c64 mixed recording and analysis. Exact-binary attribution puts
4.189% of mixed CPU samples in linear apply-receipt search. Instrumented CPU
and thread intervals are not new QPS results or per-request wait attribution.

1. **Isolate receipt lookup:** preserve original vector storage/eviction while
   using checked indexed lookup. The combined deque/indexed experiment does
   not isolate either mechanism or explain the pure-read shift. Retain the
   ordering fallback, complete receipt/verdict and unknown-outcome behavior;
   map its scoped proof and run focused correctness before another screen.
2. **Qualify throughput and latency together:** compare c1/c64 pure and mixed
   GET with the same fixed client/control/Redis protocol. Retain both repeats
   and raw histograms. A useful candidate requires the full point/batch matrix
   and exact-source actual Chaos Mesh/proof mapping before promotion. Preserve
   deadlines, ownership, fresh Safe ReadIndex and full pump/apply/view fences.
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
