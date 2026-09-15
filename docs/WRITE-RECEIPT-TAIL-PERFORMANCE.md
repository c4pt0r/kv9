# Receipt-tail matched write results

Completed locally on 2026-09-15 UTC. Receipt-tail candidate `a6ac335` improves
point-write throughput, mean latency and p99 in both execution orders. Low
concurrency batch throughput regresses slightly in both orders, and the loaded
batch result changes direction with execution order. **Keep CRC main `bd42e60`
selected; hold receipt-tail promotion.** Preserve the point-write improvement
as a candidate for further investigation. The separate
[proof and ordinary recovery](WRITE-RECEIPT-TAIL-HINT.md) and
[actual 21-window Chaos acceptance](WRITE-RECEIPT-TAIL-CHAOS.md) pass.

All eight smoke and sixteen timed cohorts pass. The independent final audit
accepts **7,296,894 successful one-attempt timed calls / 64,701,927 input items**,
with zero refused calls, unknown writes, read/client failures or dropped slots.
Final datasets, 48 fresh applied drains, 48 voter writer/listener bindings and
64 timed process lifetimes pass. All test processes exit and the owned
background containers' CPU settings are restored exactly. The eight smoke
cohorts separately contain 770,913 calls / 8,100,081 successful input items and
32 exited process lifetimes; smoke data is excluded from the table.

| Concurrency / API | CRC throughput | Receipt-tail throughput | Change | CRC mean / p99 | Receipt-tail mean / p99 |
| --- | ---: | ---: | ---: | --- | --- |
| 1 / Put | 19,388.881 calls/s | 19,584.972 calls/s | +1.011% | 51.474 / 66.560–67.583 us | 50.956 / 65.024–65.535 us |
| 64 / Put | 138,099.031 calls/s | 142,202.415 calls/s | +2.971% | 463.312 / 745.472–753.663 us | 449.937 / 720.896–729.087 us |
| 1 / BatchPut(64) | 399,984.491 items/s | 397,191.431 items/s | -0.698% | 159.899 / 206.848–208.895 us | 161.018 / 204.800–206.847 us |
| 64 / BatchPut(64) | 1,050,119.752 items/s | 1,068,082.311 items/s | +1.711% | 3.899 / 7.864–7.930 ms | 3.834 / 7.012–7.078 ms |

Rates divide summed successful counts by summed actual cohort elapsed time.
Mean latency divides summed latency nanoseconds by summed calls. P99 comes from
merged original integer histograms; ranges are bucket bounds. Batch latency is
for the complete 64-item call. Historical runs and Redis results are not pooled.

Both complete target orders remain visible:

| Workload | Old-first throughput change | New-first throughput change |
| --- | ---: | ---: |
| c1 Put | +1.313% | +0.711% |
| c64 Put | +2.611% | +3.334% |
| c1 BatchPut(64) | -0.410% | -0.986% |
| c64 BatchPut(64) | +3.572% | -0.089% |

Loaded batch p99 improves from 8.389–8.520 to 6.881–6.947 ms in the old-first
order, but worsens from 7.078–7.143 to 7.209–7.274 ms in the new-first order.
The pooled improvement does not establish a consistent batch benefit. Low
concurrency batch p99 improves or stays in the same bucket, despite slower
throughput and mean latency. These two ten-second orders on a shared host do
not establish statistical confidence or the cause of the differences.

Sampled voter RSS also varies by workload: duration-weighted sums of voter mean
RSS change by +2.741% for loaded point writes and +0.794% for loaded batch
writes. The retained analysis includes every voter's samples and lifetime HWM.
Separate voter maxima need not coincide; these observations do not establish
a memory leak or sustained resource bound.

## Measurement and acceptance

The fixed native v3 client `0be806d` uses 4,096 keys plus a sentinel, 128-byte
values, seed 71, 128 warmup calls and ten-second closed-loop measurements.
Client CPUs are 0–1; three voters share CPUs 2–5. Both servers use default
features and ThinLTO. Raft quorum, WAL synchronization, durable apply and reply
fences are unchanged. WAL storage is volatile tmpfs, and unrelated host
services remain a shared-host limitation. This screen does not establish
physical-disk performance, power-loss recovery, cross-host availability,
read/mixed-workload behavior or long-running storage bounds. Redis was not
rerun; the [earlier WAIT references](WRITE-REDIS3-BASELINE.md) have different
confirmation and durability semantics.

Actual terminals are smoke `18934/899209/0`, pre-timing smoke accounting/dataset
check `d56257/0`, timing and exact restoration `84770/38c0ab/0`, and independent
final audit `16800/8faaea/0`. Audit SHA-256 is
`9d87ba394620f4bdca9c8ce5a86efb7f0615b88bc3123ff5f5ffc1c3b59b7a36`.
The separate pooled extraction passes `8970e2/0` and binds all 60 inputs,
including the actual audit terminal. It reuses the established histogram
semantics without replaying workloads or decoding payloads again.

The [capacity continuation](cross-voter-multicohort-retention-v1/continuation/completion/README.md)
completed 40 plan cohorts and reached 85.305 GB available before this screen.
Fresh pre-smoke checks pass at 85,302,591,488 available bytes; after smoke,
79,448,543,232 bytes exceed the inherited remaining timing scenario of
73,462,044,626 bytes. All live storage guards remain unchanged. Independent
decoding covers **73,743,331,446 original logical bytes** across all 24 cohorts;
combined physical retention is **52,000,653,312 bytes**. No retention codec
overlaps measured work. Post-audit availability is 33,286,901,760 bytes before
later publication. Future work still requires fresh capacity checks.

[Original measurement, audit, source and summary records](write-receipt-tail-performance-v3/README.md)
make the result reviewable. The metadata package excludes large retained WAL
objects and executables and cannot replay the entire local retention audit.
Original inputs remain local. An initial preflight reporting reader expected
an inventory field named `rows` instead of `files` (`bb46f2/1`); the corrected
fresh read passes `5a7af6/0` before any workload starts. Frozen benchmark sources,
the workload and earlier failed records remain unchanged.

## Next write experiment

Measure receipt-queue lengths and ages, actual checked-lookup/fallback use,
entries resolved per apply, and their relationship to Ready/group-commit sizes
in a bounded diagnostic capture. Quantify instrumentation overhead separately.
The present throughput differences alone do not prove a short-queue cost or
justify another lookup implementation. Use those observations to select one
change, then retain the same proof, recovery, Chaos and complete performance
requirements. Do not combine the held FNV, directory and receipt candidates
before their interaction has evidence.

Read optimization remains held. Bounded dynamic multi-Raft, routing,
recoverable membership and automatic range splits follow the write phase.
All execution and CI were local. No original industrial work-package checkbox
closes from this short performance screen.
