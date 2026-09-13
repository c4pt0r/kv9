# CRC slicing-by-eight write performance

Measured 2026-09-12. The isolated CRC candidate improves loaded BatchPut(64)
throughput by **18.807%**, from 895,145.766 to **1,063,493.134 input items/s**.
Whole-call mean falls from 4.575 to 3.850 ms, and p99 falls from
**9.306–9.437 ms to 6.947–7.012 ms**. Loaded point writes improve **2.786%**,
from 135,624.966 to **139,402.831 calls/s**. Both run orders improve throughput
and mean in all four workload cells. This accepts the initial write screen;
default promotion still requires full read and mixed-workload regressions.

Eight two-second smokes and sixteen ten-second timed cohorts pass independent
acceptance. All **7,126,939 measured calls** succeed with one data-command
attempt, covering **60,849,685 input items**. There are no measured errors,
unknown writes or dropped slots. The smokes separately account for 762,210
successful calls and 7,443,486 input items; their rates are not timing results.

## Configuration and scope

The control is selected ThinLTO `11113f6`; candidate `e748620` changes only the
engine WAL checksum implementation to slicing by eight. The polynomial, frame
bytes, Raft protocol, sync order, durable apply and response fences are unchanged.
The fixed native v3 client comes from `0be806d`. Each target has three voters
sharing CPUs 2–5; clients share CPUs 0–1. Background containers and helpers use
CPUs 6–15 and 22–31. Both complete target orders are retained.

Each run uses 4,096 mutable keys plus an unchanged sentinel, 128-byte values,
seed 71, 128 warmup calls, concurrency 1 or 64 and a ten-million-call cap.
Point requests use Put; each atomic BatchPut contains 64 input items. Connections
have one logical call in flight. Uncertain writes are never retried.

Both releases execute normal synchronization calls on **volatile tmpfs WAL**,
on one shared host using loopback. This is not real-disk, power-loss,
independent-host, sustained-capacity or equal-durability evidence. It contains
no fault injection or full-history linearizability test. Separate exact-source
[proof and recovery qualification](write-reference-qualification-v1/README.md)
and [21 actual Chaos Mesh windows](WRITE-CRC-CHAOS.md) retain their own scope.

## Throughput, latency and CPU

Rates divide the sum of successful work by the sum of cohort durations.
Latency merges original integer histograms; percentiles are bucket bounds,
not averages of per-run percentiles. CPU is an estimated core count from
in-window process tick samples, with all three voters included. Process sample
intervals differ and can cross client interval edges; these are sampled rates,
not per-request CPU service times. Boundary sample gaps are at most 150 ms.

| Workload | c | Server | Calls/s | Items/s | Mean us | p50 us | p95 us | p99 us | Server CPU | Client CPU |
| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| point | 1 | selected | 19,228.119 | 19,228.119 | 51.897 | 50.688–51.199 | 59.904–60.415 | 67.584–68.607 | 2.515 | 0.148 |
| point | 1 | CRC | 19,445.897 | 19,445.897 | 51.316 | 50.176–50.687 | 58.880–59.391 | 65.536–66.559 | 2.511 | 0.151 |
| point | 64 | selected | 135,624.966 | 135,624.966 | 471.765 | 450.560–454.655 | 638.976–647.167 | 753.664–761.855 | 3.551 | 0.420 |
| point | 64 | CRC | 139,402.831 | 139,402.831 | 458.978 | 438.272–442.367 | 630.784–638.975 | 737.280–745.471 | 3.554 | 0.434 |
| batch64 | 1 | selected | 5,797.115 | 371,015.362 | 172.382 | 165.888–167.935 | 192.512–194.559 | 217.088–219.135 | 1.852 | 0.141 |
| batch64 | 1 | CRC | 6,229.260 | 398,672.614 | 160.419 | 155.648–157.695 | 178.176–180.223 | 202.752–204.799 | 1.755 | 0.153 |
| batch64 | 64 | selected | 13,986.653 | 895,145.766 | 4,574.505 | 4,325.376–4,390.911 | 6,619.136–6,684.671 | 9,306.112–9,437.183 | 3.351 | 0.414 |
| batch64 | 64 | CRC | 16,617.080 | 1,063,493.134 | 3,850.425 | 3,670.016–3,702.783 | 5,636.096–5,701.631 | 6,946.816–7,012.351 | 3.362 | 0.481 |

Loaded batch throughput improves 20.043% in the forward order and 17.615%
in reverse; its mean falls 16.695% and 14.976%. All eight per-order comparisons
have lower p99 intervals except forward c1 point writes, whose p99 remains in
the same bucket. Pooled p99 improves in all four cells. Two short repetitions
do not establish statistical significance, especially for the 1.133% pooled
c1 point gain. The [complete summary](https://github.com/c4pt0r/kv9/blob/5d31f9eeecfe78adb765ae0725dd6749cb649968/docs/write-crc-slicing8-ab-v1/summary.json)
and [individual repetitions](https://github.com/c4pt0r/kv9/blob/5d31f9eeecfe78adb765ae0725dd6749cb649968/docs/write-crc-slicing8-ab-v1/PER-REPEAT.md)
retain all outcomes, attempts, latency bounds and per-process CPU estimates.

Loaded server CPU stays near 3.55 cores for point writes and 3.35–3.36 for
batches. More batch work completes at similar CPU usage. This measures the
candidate's overall effect; it does not assign saved time to individual Raft,
network or storage stages.

The [earlier Redis three-copy reference](WRITE-REDIS3-BASELINE.md) recorded
229,760.166 / 232,465.084 point writes/s with WAIT 1 / WAIT 2, and about four
million batch items/s. Redis was not rerun in this A/B. Against those historical
figures, the remaining throughput gap is roughly 1.65–1.67 times for loaded
point writes and 3.76–3.77 times for batches. This is context across campaigns,
not a newly matched three-way result. Redis WAIT also does not provide Raft
consistency or equivalent fsync durability.

## Acceptance and retained failures

The timed root session is **97690**, terminal `5063f4`, exit 0; independent
audit session **40042**, terminal `1e3734`, exit 0. Summary arithmetic completes
in direct tool receipt `50f90b`, exit 0. It verifies 64 timed process lifetimes,
48 fresh voter drains, 48 writer/listener bindings and 3,071 resource samples.
Another 32 smoke lifetimes exited. Source, executable, build and CPU bindings
pass, and original container CPU assignments and namespace maps are restored.

All 69,247,125,269 logical WAL bytes are independently decoded and hash-checked:
61,670,158,132 timed bytes plus 7,576,967,137 smoke bytes. Combined retained
physical allocation is 48,819,441,664 bytes. Compression runs only after writer
reaping and before the next cohort; those gaps can still affect cache/thermal
state. Original 32/96-GiB tmpfs/retained preflights, 16/64-GiB runtime floors,
the additional 96-GiB compression floor and every retention cap remain intact.

An initial supervisor attempt (`69d85e/1`) failed before driver output or any
cohort launch because nested sudo lost the source-owner identity used by Git.
Running the same frozen child command from the normal user fixed the invocation;
no global Git trust setting changed. Successful smokes are session 97839,
terminal `6ff43c/0`. The smoke checker first failed at a nonexistent validator
import path (`9784b7/1`); its corrected path is bound to the exact v3 source and
passes (`35910d/0`). No cohort was rerun. An intervening receipt write failed
on a root-owned directory (`198b43/1`); the receipt was then written with the
appropriate identity. The failures and narrow repairs remain retained.

Capacity qualification finished before timing. The archival and accepted-WAL
tranches preserve 5,696 originals in verified compressed storage. Their final
readbacks report conservative recovery of 30,673,645,568 and 29,103,132,672
bytes. Original reader failures and corrections remain retained. A conditional
cleanup of eight first-party dev-cache packages, using the existing BuildCache
lock, increased observed free space by 4,754,497,536 bytes; retained release
binaries were hash-checked unchanged. Fresh capacity releases charge completed
smoke retention and reserve the remaining timed scenario without weakening
any guard. These are empirical reservations, not worst-case fit guarantees or
a claim that the separate 64-GiB planning recovery target was achieved.

## Attribution and next steps

| Role | Source revision | Executable SHA-256 |
| --- | --- | --- |
| Selected server | `11113f68f6a5df77da1ffb4fcec850953716ffa3` | `dd028cb2f61633dda133b05d814a6d173a79a83145d8ced2f81dc0cf8e0f33bc` |
| CRC server | `e748620a7b0ba326ac5f4fa8e3a6b1ff48553c6a` | `616eed1b823dfaa192a22b2b03a3e7d778bf71efdad7d2b875a49c4bfc94e476` |
| Native v3 client | `0be806d9671e2c50701a64aa7889c8859b7648ba` | `8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957` |

The [portable evidence](https://github.com/c4pt0r/kv9/blob/5d31f9eeecfe78adb765ae0725dd6749cb649968/docs/write-crc-slicing8-ab-v1/README.md)
is published separately from the measured source. It preserves original
reports and acceptance receipts, with large local WAL objects/executables
separately hash-bound. Portable byte verification and summary arithmetic do
not rerun the live process, source, WAL or database acceptance.

Next, qualify CRC against the complete point/batch read, write and mixed
regression matrix with the same throughput, tail-latency and correctness gates.
The [prepared coverage and capacity plan](write-crc-full-regression-plan-v1/README.md)
specifies 24 smokes and 48 timed cohorts. Its empirical reserve needs about
80.88 GB more verified free space at the preparation snapshot; it is not yet
released for execution. Existing guards remain unchanged.
Keep the single-buffer Raft WAL experiment `01d128f` separate until its exact
release, recovery, actual Chaos and matched performance pass. The write path
still has a substantial gap to Redis; subsequent work should use measured
allocation and replication costs while preserving every acknowledgment fence.
Dynamic multi-Raft and automatic range splits follow the write phase with their
storage, ownership and proof prerequisites. Selected runtime stays `11113f6`;
no original industrial checklist item closes and no hosted CI was dispatched.
