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

The latest **24-cohort owner-local ReadIndex pump screen** uses two forward/reverse
repetitions, ten seconds each, fixed v3 clients, 4,096 keys and 128-byte values,
closed-loop load, shared-host loopback and fixed CPU placement. KV9 uses three
voters with normal quorum/sync calls on **tmpfs WAL**. Redis is standalone with
persistence disabled and no pipelining. Write durability is not equivalent.

| GET metric | Selected CRC | Owner-local pump experiment | Redis |
| --- | ---: | ---: | ---: |
| c1 calls/s | 26,518 | 26,641 | 174,419 |
| c1 mean us | 37.590 | 37.425 | 5.656 |
| c1 p99 histogram interval us | 50.176–50.687 | 49.664–50.175 | 7.232–7.295 |
| c64 calls/s | 345,145 | 348,190 | 514,041 |
| c64 mean us | 185.303 | 183.681 | 124.394 |
| c64 p99 histogram interval us | 352.256–356.351 | 352.256–356.351 | 229.376–231.423 |

The experiment improves c1 GET throughput **0.462%** and c64 GET **0.882%**
pooled. Paired c64 changes are +1.216% / +0.549%; its p99 stays in the same
bucket in both repetitions. Mixed c64 throughput falls **0.243%**, while GET
mean rises **384.041 -> 384.781 us (+0.193%)** and PUT mean rises 0.298%.
Combined mixed p99 moves to a higher bucket in both repetitions; mixed GET
p99 itself stays in the same bucket. These are small screening observations,
not evidence of statistical significance or a sustained capacity improvement.
**Keep CRC selected; do not promote this candidate.**

The candidate remains **1.476x** behind Redis throughput at c64 and its c1 mean
latency is **6.616x** Redis. All **49,860,741 measured calls** succeed in one
attempt; 80 owned lifetimes exit, 48 fresh drains/bindings and exact restoration
pass. Both complete repetitions, separate mixed GET/PUT results and retained
source/proof/recovery evidence are in the
[full report](https://github.com/c4pt0r/kv9/blob/99d94fb2789d8dc5886e8d1471a658be9f600e27/docs/OWNER-READ-PUMP-SCREEN.md).

The [previous two-context screen](https://github.com/c4pt0r/kv9/blob/119a49cc2f5f35c7f68b8d2301a71e563a36140c/docs/READ-WINDOW-SCREEN.md)
reaches 367,127 c64 GET/s but is also held because c1 and mixed reads regress.
These are separate runs, not a causal comparison between candidate speeds.
No batch or write-only performance was measured in either short screen. The
[72-cohort one-context matrix](https://github.com/c4pt0r/kv9/blob/aeee652135bc52e4527ef4fffd540ec184adc362/docs/READ-CREDIT-CRC-PERFORMANCE.md)
remains the latest broad comparison; its candidate reaches 2,296,816
BatchGet(64) keys/s versus Redis 5,954,693 and also remains experimental.

## Current experiment and correctness evidence

[Candidate 3bfb63b](https://github.com/c4pt0r/kv9/commit/3bfb63bcb48e07325ab212d3e1e9d4eacb961b59)
binds internal sealed-read admission to an immediate pump of the same peer.
External synchronous submissions still notify. It removes an extra owner turn
in the isolated regression test while retaining fresh Safe ReadIndex, sealed
identities, cancellation/deadline ownership and full pump/apply/view fences.

- 714 workspace tests/doctests pass, with 23 ignored; formatting and
  warnings-denied all-target Clippy pass. All 601 tested source files match
  the clean committed tree; release compilation invalidates project artifacts
  and freshly compiles all 11 first-observed project units.
- Five real owner/quorum/apply regressions and three compiled semantic-control
  triples pass. An initial broad omit-pump mutation failed during fixture setup;
  that attempt is retained, and a narrower admission-only mutation reaches the
  intended assertion. Failed compilation or setup is not counted as that control.
- The local TLA+/TLAPS gate passes 21 cases, seven named theorems and 20 baseline
  obligations for sequencing and retained notifications in one transaction.
  It includes typed failure and abort; a pump invocation is not a quorum
  certificate. Complete Rust/Raft composition and scheduler liveness remain open.
- Exact-source ordinary stream/unary recovery checks 365 operations, 337 successes
  and 28 unknown outcomes across leader loss and original-directory restart.
  Both complete histories independently validate; all five server and two client
  lifetimes exit. This is SIGKILL/restart evidence, not actual Chaos Mesh or power loss.

The matched screen is complete and does not justify full batch/Chaos promotion
for this candidate. The [earlier lifecycle diagnostic](https://github.com/c4pt0r/kv9/blob/aeee652135bc52e4527ef4fffd540ec184adc362/docs/READ-LIFECYCLE-CRC-CREDIT.md)
remains scoped evidence for the admission-window tradeoff. Remote admission
bounds, broader implementation proofs and actual Chaos Mesh remain required.

## Next development steps

The [fresh selected-source CPU/thread diagnostic](https://github.com/c4pt0r/kv9/blob/6fb3434c7cafcbd8d93b04f298208d5785fa292a/docs/READ-PATH-CPU-PROFILE.md)
passes c1 GET and c64 mixed recording/analysis with 1,241 / 3,342 selected CPU
samples and 912,580 successful measured calls. Instrumented calls are not new
QPS evidence. Exact-binary attribution puts 4.189% of mixed CPU samples in the
linear apply-receipt search. OS-thread intervals do not identify async-task
or per-request quorum waits. The first recording's 128-MiB cap failure is
retained; a fresh 512-MiB run passes unchanged sampling and validity checks.

1. **Qualify indexed receipts for reads/mixed:** complete the missing c1/c64
   GET/mixed screen for existing `74d24116`, reusing its original checked
   source/release. It already has local source, representation-proof and
   ordinary recovery evidence; its earlier point-write gain and inconclusive
   batch result remain separate. Evaluate GET mean/p95/p99 and mixed throughput.
   Keep CRC selected until full qualification. Avoid another broad wake rewrite
   or admission-window change without evidence. Tonic streaming remains selected;
   DPDK requires actual cross-host/NIC evidence.
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
