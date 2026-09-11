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

Two forward/reverse repetitions, ten seconds per cohort, fixed v3 clients,
4,096 keys and 128-byte values, closed loop, shared-host loopback and fixed CPU
placement. KV9 uses three voters with normal quorum/sync calls on **tmpfs WAL**.
Redis is standalone with persistence disabled and no pipelining. These are
memory-path observations, not an equal-durability write comparison.

| GET metric | Selected CRC | One-context experiment | Redis |
| --- | ---: | ---: | ---: |
| c1 calls/s | 26,461 | 26,550 | 173,925 |
| c1 mean us | 37.676 | 37.549 | 5.673 |
| c64 calls/s | 344,473 | 372,211 | 513,378 |
| c64 mean us | 185.665 | 171.820 | 124.556 |
| c64 p99 histogram interval us | 352.256–356.351 | 294.912–299.007 | 229.376–231.423 |

The one-context experiment improves c64 GET throughput **8.052%**. Redis still
has **1.379x** its throughput and **6.619x** lower c1 mean latency. Mixed-load
GET mean worsens **4.548%**, and mixed read tails regress in both repetitions;
the experiment therefore remains unselected. C64 BatchGet(64) reaches
**2,296,816 keys/s**, versus Redis **5,954,693 keys/s**.

The complete 72-cohort report, including writes, errors and all repetitions, is
[published here](https://github.com/c4pt0r/kv9/blob/aeee652135bc52e4527ef4fffd540ec184adc362/docs/READ-CREDIT-CRC-PERFORMANCE.md).

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
Performance and exact-source Chaos Mesh remain pending. The preceding published
diagnostic report describes the earlier source-only checkpoint; recovery above
completed subsequently. No candidate speedup is claimed yet.

## Next development steps

1. **Read admission overlap:** screen CRC/window2/Redis at c1/c64, point GET and
   50% GET/PUT, in paired forward/reverse order. Report per-operation throughput,
   mean and tail latency, refusal/unknown outcomes and all repetitions. If useful,
   run the full point/batch read/write/mixed matrix. Reject repeatable mixed-read
   regressions. Complete exact-source Chaos Mesh histories before promotion.
2. **Single GET turnaround:** c1 confirmation observation dominates the sampled
   barrier. Identify one scheduling/transport handoff with current-source evidence,
   preserve independent quorum authorization, then repeat the same comparison.
   Tonic streaming remains selected. DPDK requires actual cross-host/NIC evidence.
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
