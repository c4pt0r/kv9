# Receipt upper-bound matched write results

Completed locally on 2026-09-15 UTC. **Keep CRC selected; hold upper-bound
promotion.** Candidate `e2e23cc` improves loaded point throughput by **3.169%**,
with better mean latency and p99 in both execution orders. Loaded BatchPut(64)
instead loses **1.419%** in the pooled comparison and its p99 increases from
**7.799–7.864 ms to 8.651–8.782 ms**. Both low-concurrency workloads and loaded
batch throughput change direction between orders. This does not establish a
consistent overall write improvement.

All eight smokes and sixteen ten-second timed cohorts pass independent
acceptance. The timed population contains **7,154,151 successful calls /
62,959,110 input items**, each with one data-command attempt. Refusals, unknown
writes, read failures, client rejections and dropped slots are all zero.
Complete deterministic datasets, 48 fresh applied drains, 48 voter/writer/
listener bindings and 64 exited timed process lifetimes pass. The eight smokes
separately contain 803,116 calls / 8,192,386 input items and 32 exited lifetimes;
their rates do not contribute to the results below.

## Throughput and whole-call latency

Both roles use the same explicitly [requalified measurement client](WRITE-CLIENT-REQUALIFICATION.md).
These are the retained CRC `bd42e60` and candidate `e2e23cc` releases. They do
not measure the later checkpoint-owner integration on current main. Historical
measurements with the missing original client are not pooled with this run.

| Concurrency / API | CRC throughput | Upper-bound throughput | Change | CRC mean / p99 | Upper-bound mean / p99 |
| --- | ---: | ---: | ---: | --- | --- |
| 1 / Put | 18,960.518 calls/s | 19,027.483 calls/s | +0.353% | 52.641 / 69.632–70.655 us | 52.453 / 68.608–69.631 us |
| 64 / Put | 135,562.425 calls/s | 139,858.145 calls/s | +3.169% | 471.982 / 778.240–786.431 us | 457.484 / 745.472–753.663 us |
| 1 / BatchPut(64) | 393,689.312 items/s | 393,558.224 items/s | -0.033% | 162.464 / 204.800–206.847 us | 162.514 / 202.752–204.799 us |
| 64 / BatchPut(64) | 1,030,729.153 items/s | 1,016,106.575 items/s | -1.419% | 3.973 / 7.799–7.864 ms | 4.030 / 8.651–8.782 ms |

Rates divide summed successful calls/items by summed actual cohort elapsed time.
Latency means use summed integer durations and counts. Percentiles merge the
original histogram counts and retain inclusive bucket bounds; percentiles are
not averaged. Batch latency covers the entire 64-item call. The portable report
also retains calls/s for batches and all p50/p95/p99 intervals.

| Workload | Old-first throughput change | New-first throughput change |
| --- | ---: | ---: |
| c1 Put | -0.354% | +1.040% |
| c64 Put | +3.677% | +2.685% |
| c1 BatchPut(64) | -0.478% | +0.404% |
| c64 BatchPut(64) | -3.437% | +0.656% |

Loaded batch p99 worsens from 7.209–7.274 to 9.044–9.175 ms in the old-first
order, then improves from 8.389–8.520 to 8.258–8.323 ms in the new-first order.
Low-concurrency Put p99 also changes direction. Both observations remain in
the result; the slower order is not discarded. Two short orders on a shared
host establish neither statistical confidence nor the cause of the differences.

Sampled three-voter CPU is 3.517 versus 3.508 cores for loaded Put and 3.251
versus 3.207 for loaded BatchPut(64). Client CPU is 0.434/0.441 and 0.448/0.460
cores respectively. These are sampled process rates, with nonidentical probe
boundaries; they are not per-request stage costs or proof of a particular cause.

## Inputs and acceptance scope

The measurement client has clean source `0be806d`, all 581 original file hashes,
and the retained Rust/Cargo 1.94 toolchain. Its executable SHA-256 is
`1b8060eb168610328c10a27480de16bd8b2d6805166d8169638f85ceef0992c4`.
It is a separately qualified executable, not the missing historical `8da9`
binary. Fresh source/build checks cover 859 CRC files, 1,116 candidate files and
581 client files. Both server releases retain default features and ThinLTO.

Each cohort uses 4,096 mutable keys plus a sentinel, 128-byte values, seed 71,
128 warmup calls and a ten-million-call cap. Point writes use singular Put;
64-item writes use atomic BatchPut. Concurrency is 1 or 64 with closed-loop
requests. Client CPUs are 0–1; three voters share CPUs 2–5. Helpers and the
three owned background containers use CPUs 6–15 and 22–31 during timing.
The original container CPU settings and namespace maps are restored exactly.

Raft quorum, WAL synchronization, durable apply and response fences remain
enabled. Active WAL storage is **volatile tmpfs**. This shared-host loopback
screen does not establish physical-disk performance, power-loss durability,
cross-host availability, read/mixed regression behavior, sustained capacity
or full-history linearizability. The candidate's separate
[source proof](WRITE-RECEIPT-UPPER-BOUND.md),
[ordinary recovery](WRITE-RECEIPT-UPPER-BOUND-RUNTIME.md) and
[actual full21 Chaos Mesh acceptance](WRITE-RECEIPT-UPPER-BOUND-CHAOS.md) remain
accepted. This performance screen does not repeat or replace those gates.
Redis was not rerun; its [earlier WAIT references](WRITE-REDIS3-BASELINE.md)
retain different confirmation and durability semantics.

Actual terminals are smoke `45876/559f87/0`, smoke readback `905b8a/0`, timing
and restoration `14114/1182dd/0`, independent audit `48940/2f82fd/0`, and report
derivation `aad8c9/0`. The final audit hash is
`7f60d63edbfaa3580160b3e80bf5ed97ba01aafe4faeb8e1342a6ba82daedb4f`.
The report uses unchanged integer histogram and weighted-rate arithmetic.

All retained outputs use `/mnt/data/kv9-work`. The full campaign launch observed
9,689,422,909,440 available bytes there, exceeding the unchanged
79,455,850,496-byte empirical requirement. Independent decoding covers all
**72,088,784,643 original logical bytes** across smoke and timing. Combined
physical retention is **50,835,156,992 bytes**. Root/data/tmpfs guards remain
separate; no codec overlaps measured work. The additional root stat observation
runs in both arms, so the complete environment is not claimed identical to an
older experiment. Available space is an observation, not a reservation.

Original local records are in `upper-bound-requalified-{preparation,execution,
report-preparation}-20260915-first` under the data-volume work directory.
The [portable metadata packet](write-receipt-upper-bound-performance-v1/README.md)
preserves the exact reports, source/build identities, original preparation
failures, execution terminals, audit records and derivation inputs. Large WAL
objects and executables remain local; the packet is not standalone WAL replay.

## Development decision

Keep CRC as the selected write implementation. Preserve the upper-bound
candidate and its repeatable loaded-point improvement without promoting its
batch tradeoff or running an unchanged matrix again. The completed observer's
high skip count is now paired with a small point-write benefit; it does not
establish a general batch improvement.

Next analyze the retained selected-runtime and candidate evidence for loaded
batch tail variation, including group formation, writer activity and queue
occupancy. Aggregate distributions cannot identify which event delayed a
particular request. Add a bounded timestamped capture only for unresolved
intervals, with explicit loss/coverage and observer-overhead checks, before
another writer, scheduling or transport rewrite. Keep the held FNV and
receipt-tail candidates separate. There is no matched Redis parity claim and
no reason here to advance the dynamic multi-Raft/split dependency gates.

Continue the C04 pre-upload crash cut and typed negative-history/abort-release
integration alongside this analysis. Legacy coverage/fencing, reader drainage
and destination installation remain required storage foundations. All checks
stay local; no hosted workflow is dispatched for this checkpoint.
