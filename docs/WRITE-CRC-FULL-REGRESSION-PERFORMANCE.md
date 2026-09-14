# CRC full read/write regression results

Measured 2026-09-14 UTC. Slicing-by-eight CRC preserves the loaded batch-write
gain across the full point/batch and read/write/mixed matrix. At concurrency 64,
BatchPut(64) reaches **1,062,522.902 items/s (+17.119%)**, with whole-call p99
falling from **8.389–8.520 ms to 7.406–7.471 ms**. Point Put reaches
**139,532.275 calls/s (+2.203%)**. Mixed batch throughput improves 16.898%,
with lower separate read and write p99 intervals in both run orders.

All **24 two-second smokes and 48 ten-second timed cohorts** pass independent
acceptance. All **35,103,005 measured calls** succeed in one attempt, covering
**285,605,495 input items**, with no measured errors, unknown writes or dropped
slots. The subsequent [main integration](CRC32-SLICING-INTEGRATION.md) now passes
its own proof, 789 tests, clean release, ordinary recovery and actual Chaos Mesh
qualification. The measurements below remain attributed to the original e748
binary; they are not a new main performance run.

## Same-campaign throughput and latency

The control is selected ThinLTO `11113f6`; the candidate is CRC `e748620`.
Rates divide summed successful calls/items by summed cohort elapsed time.
Means use total nanoseconds/count; percentile intervals come from merged
integer histograms. All latency values describe complete calls, including a
complete 64-item batch. No latency is divided by 64.

| c | Workload | Role | Calls/s | Items/s | Mean us | p99 us |
| ---: | --- | --- | ---: | ---: | ---: | --- |
| 1 | point-r000 | selected | 19,480.725 | 19,480.725 | 51.233 | 65.024–65.535 |
| 1 | point-r000 | CRC | 19,505.225 | 19,505.225 | 51.168 | 65.024–65.535 |
| 1 | point-r050 | selected | 21,881.156 | 21,881.156 | 45.593 | 61.952–62.463 |
| 1 | point-r050 | CRC | 21,905.102 | 21,905.102 | 45.545 | 61.440–61.951 |
| 1 | point-r100 | selected | 28,431.334 | 28,431.334 | 35.049 | 43.520–44.031 |
| 1 | point-r100 | CRC | 28,525.416 | 28,525.416 | 34.946 | 43.008–43.519 |
| 1 | batch64-r000 | selected | 5,863.974 | 375,294.325 | 170.423 | 215.040–217.087 |
| 1 | batch64-r000 | CRC | 6,263.942 | 400,892.290 | 159.533 | 202.752–204.799 |
| 1 | batch64-r050 | selected | 7,028.538 | 449,826.435 | 141.768 | 210.944–212.991 |
| 1 | batch64-r050 | CRC | 7,418.189 | 474,764.074 | 134.299 | 196.608–198.655 |
| 1 | batch64-r100 | selected | 10,057.279 | 643,665.875 | 98.536 | 124.928–125.951 |
| 1 | batch64-r100 | CRC | 10,191.896 | 652,281.367 | 97.227 | 122.880–123.903 |
| 64 | point-r000 | selected | 136,524.376 | 136,524.376 | 468.656 | 753.664–761.855 |
| 64 | point-r000 | CRC | 139,532.275 | 139,532.275 | 458.550 | 737.280–745.471 |
| 64 | point-r050 | selected | 190,770.817 | 190,770.817 | 335.341 | 548.864–557.055 |
| 64 | point-r050 | CRC | 193,273.331 | 193,273.331 | 330.997 | 540.672–548.863 |
| 64 | point-r100 | selected | 378,600.420 | 378,600.420 | 168.919 | 323.584–327.679 |
| 64 | point-r100 | CRC | 377,878.438 | 377,878.438 | 169.243 | 319.488–323.583 |
| 64 | batch64-r000 | selected | 14,175.287 | 907,218.386 | 4,513.761 | 8,388.608–8,519.679 |
| 64 | batch64-r000 | CRC | 16,601.920 | 1,062,522.902 | 3,853.926 | 7,405.568–7,471.103 |
| 64 | batch64-r050 | selected | 21,652.532 | 1,385,762.053 | 2,952.298 | 5,308.416–5,373.951 |
| 64 | batch64-r050 | CRC | 25,311.389 | 1,619,928.876 | 2,525.158 | 4,587.520–4,653.055 |
| 64 | batch64-r100 | selected | 36,909.800 | 2,362,227.222 | 1,728.811 | 2,850.816–2,883.583 |
| 64 | batch64-r100 | CRC | 37,308.588 | 2,387,749.630 | 1,710.266 | 2,850.816–2,883.583 |

`r000`, `r050` and `r100` mean configured read percentages 0, 50 and 100.
Mixed workloads retain their actual read/write counts; they are not forced to
an exact 50/50 split. Separate operation rates below use the full elapsed
interval for both operations.

## Separate mixed-operation results

| c | API | Role | Calls/s | Items/s | Mean us | p99 us |
| ---: | --- | --- | ---: | ---: | ---: | --- |
| 1 | get | selected | 10,973.378 | 10,973.378 | 41.279 | 53.248–53.759 |
| 1 | put | selected | 10,907.778 | 10,907.778 | 49.932 | 64.512–65.023 |
| 1 | get | CRC | 10,985.526 | 10,985.526 | 41.272 | 53.248–53.759 |
| 1 | put | CRC | 10,919.576 | 10,919.576 | 49.844 | 64.000–64.511 |
| 1 | batch_get | selected | 3,512.494 | 224,799.618 | 109.131 | 141.312–143.359 |
| 1 | batch_put | selected | 3,516.044 | 225,026.817 | 174.373 | 219.136–221.183 |
| 1 | batch_get | CRC | 3,706.219 | 237,198.038 | 107.400 | 137.216–139.263 |
| 1 | batch_put | CRC | 3,711.969 | 237,566.035 | 161.157 | 202.752–204.799 |
| 64 | get | selected | 95,292.611 | 95,292.611 | 346.683 | 565.248–573.439 |
| 64 | put | selected | 95,478.206 | 95,478.206 | 324.022 | 532.480–540.671 |
| 64 | get | CRC | 96,548.093 | 96,548.093 | 342.288 | 557.056–565.247 |
| 64 | put | CRC | 96,725.238 | 96,725.238 | 319.726 | 524.288–532.479 |
| 64 | batch_get | selected | 10,856.758 | 694,832.489 | 3,116.480 | 5,505.024–5,570.559 |
| 64 | batch_put | selected | 10,795.774 | 690,929.564 | 2,787.188 | 4,980.736–5,046.271 |
| 64 | batch_get | CRC | 12,687.039 | 811,970.496 | 2,660.048 | 4,718.592–4,784.127 |
| 64 | batch_put | CRC | 12,624.350 | 807,958.380 | 2,389.598 | 4,325.376–4,390.911 |

Loaded pure GET changes **-0.191%** to 377,878.438 calls/s; mean rises
0.192% while pooled p99 moves to a lower bucket. Its two order-specific
throughput changes are +0.089% and -0.469%. This is not evidence of a new read
speedup. Loaded BatchGet changes +1.080% with the same pooled p99 bucket.
No active API has a higher p99 bucket in either order, including mixed reads.

Loaded batch-write throughput gains are **+18.857% / +15.384%** in the two
orders; point-write gains are **+2.466% / +1.945%**. At concurrency 1, pure
batch writes improve 6.821%; point writes change only +0.126%, with opposite
signs across orders. The prior write-only campaign remains separately reported
in [WRITE-CRC-PERFORMANCE.md](WRITE-CRC-PERFORMANCE.md): its 1,063,493.134
batch items/s and 6.947–7.012 ms p99 are not pooled into this result. This new
campaign has similar absolute batch throughput and a higher absolute p99,
while both orders still improve against their own controls. Two short orders
on a shared host do not establish statistical significance or sustained capacity.

Sampled three-voter CPU for loaded batch writes is 3.343085 cores selected
and 3.348855 CRC; the client rises from 0.418440 to 0.483160 cores as more work
completes. The complete CPU table retains individual voters and every cell.
These samples are duration-weighted process CPU, not exclusive request service
time, per-operation attribution or proof of the next bottleneck.

## Source, configuration and correctness scope

- Selected source: `11113f68f6a5df77da1ffb4fcec850953716ffa3`; server SHA256
  `dd028cb2f61633dda133b05d814a6d173a79a83145d8ced2f81dc0cf8e0f33bc`.
- CRC source: `e748620a7b0ba326ac5f4fa8e3a6b1ff48553c6a`; server SHA256
  `616eed1b823dfaa192a22b2b03a3e7d778bf71efdad7d2b875a49c4bfc94e476`.
- Fixed native v3 client: `0be806d9671e2c50701a64aa7889c8859b7648ba`; SHA256
  `8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957`.

Three voters share CPUs 2–5; clients use 0–1; helpers and owned background
containers use 6–15 and 22–31. The full forward order and complete reverse
cover point/batch64, 0/50/100% reads and concurrency 1/64. Each cohort retains
4,096 keys plus sentinel, 128-byte values, seed 71, 128 warmup calls and a
ten-million-call cap. Smoke results are excluded from performance pooling.

Both roles retain normal Raft quorum, WAL synchronization, durable-apply and
response fences on **volatile tmpfs WAL**. Checksumming changes preserve the
IEEE polynomial and WAL byte format. Read authorization remains Safe ReadIndex.
Uncertain writes are never replayed. This shared-host loopback panel is not
a physical-disk, power-loss, cross-host or equal-durability Redis comparison.
Redis was not rerun. The [three-copy Redis reference](WRITE-REDIS3-BASELINE.md)
retains its own WAIT 1/2 configuration and persistence limitations.

The exact candidate already passed 47 distinct source-bound Lean theorem
statements, 710 workspace tests/doctests (23 existing ignored), formatting,
Clippy, clean release and ordinary recovery with 363 operations / 26 unknowns;
see [qualification](write-reference-qualification-v1/README.md). Its separate
[21-window Chaos Mesh campaign](WRITE-CRC-CHAOS.md) passed complete-history
checking with 9,833 operations, 600 unknowns and 28 refusals. Those are separate
populations and scopes. The CRC proof is checksum equivalence under its stated
Rust/compiler premises, not a proof of all Rust or Raft. This performance
campaign adds no new fault-injection history.

## Acceptance and storage

The frozen runtime tools retain 35 passing local controls; reporting retains
13. All 142 runtime preparation files and 26 reporting preparation files were
checked before release. Actual timing session 76372 ended at `3727d8/0`,
independent audit 12850 at `3b33fa/0`, and reporting at `faf0cf/0`. No cohort
was rerun or omitted. The audit verifies 192 timed / 96 smoke process lifetimes
exited, 144 timed voter drains and writer/listener bindings, 1,891 role/source
checks and 9,287 resource samples. Original container CPU assignments and
namespace maps were restored. Setup routing attempts are separate: 199,776
initialization calls made 199,824 attempts, including 48 safe not-leader replies.

The audit independently decodes all **119,471,576,199 logical WAL bytes**,
including 106,225,921,995 timed and 13,245,654,204 smoke bytes. Combined resident
allocation is **84,585,664,512 bytes**. Compression occurs after writer reaping
and outside timed windows. Original caps, retention floors and restore reserves
remain unchanged.

Before this campaign, native pruning of unused default and ARM BuildKit caches
increased available space from **162,362,454,016 to 209,313,718,272 bytes**:
an observed **46,951,264,256 bytes** reclaimed. Container identities, running
states, images and volume inventories were unchanged. No retained benchmark,
Chaos/recovery payload, executable or source was deleted. This is additional
to the earlier [89.93 GB cleanup](write-storage-cache-cleanup-v1/result.json).
The corrected whole-resident scenario required 203,271,221,248 available bytes.
After smoke, fresh free space of 199,804,317,696 exceeded the remaining timed
reservation of 194,085,216,256. The campaign then retained its own 84.59 GB;
a fresh post-campaign check reports about 117 GiB free. Future campaigns need
a fresh capacity check; the pre-run observation is not reusable free space.

## Evidence and development decision

[Portable original reporting evidence](https://github.com/c4pt0r/kv9/blob/425ae37f8f2dbf7dea22c63af2c4cccc9f38eb05/docs/write-crc-full-regression-v1/README.md)
preserves all 6,498 original metadata files: 874,334,238 decoded bytes in 52
parts (108,324,271 compressed bytes). Packaging and independent full byte
verification pass at `dde2c9/0` and `79fadc/0`. The index SHA256 is
`2422847486c6a5a56dcba53a4369938b2bc711e40fe69f4a186d792b51e0d212`;
the accepted audit SHA256 is
`99e3912ae4463aff73d038ae7b054aa70c2a8b322e9e29619aff3aed3e731d34`.
The exact 48-case/24-pooled-row summary, both order-specific comparisons,
separate operation histograms and CPU table are readable without extraction.
Large local WAL objects and executables remain bound by catalogs/manifests.
Portable-byte verification does not rerun original runtime/source acceptance.
Evidence stays on a separate branch; main's original archive budget is unchanged.

Next integrate only the proven CRC change into current main, run source/proof
checks and local build/default/recovery validation for that exact integration,
and preserve the already accepted experimental-source measurements under their
original binary hashes. Main has accumulated experimental lease changes since
the measured control; a newly built main binary must not inherit these QPS
numbers by assertion. No speculative combination with vectored/frame-buffer
changes is selected.

Then measure the isolated frame-buffer candidate after resolving its retained
campaign capacity. The [vectored screen](WRITE-SEGMENT-VECTORED-PERFORMANCE.md)
found no write benefit and stays experimental. Use measured CPU/allocation
costs to choose the next optimization. Dynamic multi-Raft and automatic splits
follow the write phase. No original industrial roadmap checkbox closes here.
All validation remains local; no hosted CI was dispatched.
