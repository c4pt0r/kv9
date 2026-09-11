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

The latest **24-cohort point read/mixed screen** uses two forward/reverse
repetitions, ten seconds each, fixed v3 clients, 4,096 keys and 128-byte values,
closed-loop load, shared-host loopback and fixed CPU placement. KV9 uses three
voters with normal quorum/sync calls on **tmpfs WAL**. Redis is standalone with
persistence disabled and no pipelining. Write durability is not equivalent.

| GET metric | Selected CRC | Two-context experiment | Redis |
| --- | ---: | ---: | ---: |
| c1 calls/s | 26,638 | 26,381 | 174,164 |
| c1 mean us | 37.424 | 37.785 | 5.665 |
| c64 calls/s | 345,323 | 367,127 | 511,325 |
| c64 mean us | 185.210 | 174.199 | 125.051 |
| c64 p99 histogram interval us | 352.256–356.351 | 311.296–315.391 | 229.376–231.423 |

The two-context experiment improves c64 GET throughput **6.314%**, but c1
throughput falls **0.962%** and mixed c64 GET mean worsens **3.008%**
(383.676 -> 395.217 us), with worse GET p99 in both repetitions. Keep CRC
selected. The candidate still has a **1.393x** Redis throughput gap at c64
and **6.670x** Redis mean latency at c1. All **50,152,139 measured calls**
succeed in one attempt; 80 owned lifetimes exit and exact restoration passes.

The [complete screen and all repetitions](https://github.com/c4pt0r/kv9/blob/119a49cc2f5f35c7f68b8d2301a71e563a36140c/docs/READ-WINDOW-SCREEN.md)
are published. No batch or write-only performance was measured in this screen.
The previous [72-cohort one-context matrix](https://github.com/c4pt0r/kv9/blob/aeee652135bc52e4527ef4fffd540ec184adc362/docs/READ-CREDIT-CRC-PERFORMANCE.md)
remains the latest broad workload comparison: its candidate reaches 2,296,816
BatchGet(64) keys/s versus Redis 5,954,693. It also remains experimental. Do not
compare candidate speeds across these two different runs as a causal effect.

## Current experiment

The [fresh instrumented lifecycle report](https://github.com/c4pt0r/kv9/blob/aeee652135bc52e4527ef4fffd540ec184adc362/docs/READ-LIFECYCLE-CRC-CREDIT.md)
finds mixed-read queue mean rising from **19.893 to 82.557 us** under the
one-context cap, exceeding savings after admission. Its four cohorts contain
1,986,314 successful measured calls, with independently checked metric deltas.
It is a diagnostic, not a new uninstrumented performance result.

[Two-context candidate 5654ea59](https://github.com/c4pt0r/kv9/commit/5654ea593fe561c5fd8d800cf498f716eb5bca23)
allows two local ReadIndex contexts to overlap. Fresh Safe ReadIndex, sealed
group identities, cancellation/deadline ownership and pump/apply/view fences
remain required. Local checks pass:

- 718 workspace tests/doctests, with 23 ignored; formatting and warnings-denied Clippy.
- 14 compiled semantic-control triples; all 42 selected test units freshly compiled.
- Counted-window formal gate: 29 cases, nine theorems and 23 baseline obligations.
- Prior admission formal gate: 37 cases, 28 theorems and 265 baseline obligations.
- Exact-source ordinary three-voter recovery: stream and unary histories with
  372 operations, 347 successes and 25 unknown outcomes, both independently valid.
  Leader loss, surviving-quorum progress, restart, fresh drains and owned cleanup
  pass. This is SIGKILL/restart evidence, not actual Chaos Mesh or power loss.

The parameterized bound is conditional on guarded queue-increasing transitions.
Authenticated remote MsgReadIndex currently bypasses local admission; a universal
ingress bound is separate work. The counter and admission proofs do not establish
whole Rust/Raft refinement or per-caller fairness.

The candidate's original release is built and retained with 11 first-observed
project units freshly compiled. Recovery audit:
`/tmp/kv9-read-window-runtime-preparation-first/process-results-first/audit.json`.
The point-read/mixed screen now completes and rejects promotion because of read
regressions. The full batch and exact-source Chaos Mesh promotion campaign is
not pursued for this candidate. [Published recovery histories](https://github.com/c4pt0r/kv9/blob/62d5be870ae6b41a765543f501324d2d0410d316/docs/READ-LIFECYCLE-CRC-CREDIT.md)
retain its separate correctness result; it does not override performance.

## Next development steps

1. **Confirmation-path costs:** stop widening the read window without a new
   measured reason. Profile one processing or task-handoff mechanism on the
   current source, retaining independent quorum authorization and sealed groups.
   Prior rejected transport/wake experiments remain evidence; repeat one only
   with a new causal hypothesis. Tonic streaming remains selected. DPDK requires
   actual cross-host/NIC evidence.
2. **Short qualification loop:** run focused correctness and ordinary recovery,
   then matched c1/c64 GET and mixed screening. Report GET separately from PUT,
   both repetitions and raw tail histograms. For a promising candidate, run the
   full point/batch matrix and exact-source Chaos Mesh/proof mapping before
   promotion. Preserve deadlines, ownership and apply/read-view fences.
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
