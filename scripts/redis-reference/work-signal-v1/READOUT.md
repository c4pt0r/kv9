# Work-signal coalescing matched diagnostic readout

The unchanged auditor accepted all 48 cohorts from timing session 58205 (exit 0). All 118,432,733 issued calls and attempts succeeded; no unknown writes, refusals, read failures, client rejections, retries, or dropped slots were recorded.

Four-repeat 10-second c1/c64 shared-host tmpfs diagnostic; no sustained-capacity or equal-durability claim. This compares candidate 1b70dba with control 57ff685; it does not establish statistical significance, sustained capacity, or absence of regression. Clients are the fixed native 03c1 and Redis b8ec artifacts. Native quorum WALs are on tmpfs; Redis is in memory, so durability is not equivalent. The host remains shared.

Four repeats use forward/reverse/reverse/forward order. All original outcomes, sums, elapsed times, report hashes and per-cohort p50/p95/p99 intervals are retained in statistics-first.json. QPS is completed calls divided by cohort elapsed time, including terminal drain; mean is whole-call latency sum divided by count. No percentiles are averaged.

| Repeat | Workers | API | Control calls/s | Candidate calls/s | Change | Control mean µs | Candidate mean µs | Control p99 µs | Candidate p99 µs |
|---:|---:|---|---:|---:|---:|---:|---:|---|---|
| 0 | 1 | point_get | 26636.859 | 26475.375 | -0.606% | 37.410203 | 37.640206 | [51.200, 51.711] | [52.224, 52.735] |
| 0 | 1 | batch_get | 26247.764 | 26384.166 | +0.520% | 37.959701 | 37.758034 | [52.224, 52.735] | [52.736, 53.247] |
| 0 | 64 | point_get | 375174.485 | 377977.480 | +0.747% | 170.466194 | 169.201321 | [294.912, 299.007] | [294.912, 299.007] |
| 0 | 64 | batch_get | 368308.643 | 370965.838 | +0.721% | 173.609836 | 172.365171 | [307.200, 311.295] | [303.104, 307.199] |
| 1 | 1 | point_get | 26543.829 | 26429.053 | -0.432% | 37.543003 | 37.720416 | [51.712, 52.223] | [52.224, 52.735] |
| 1 | 1 | batch_get | 26211.458 | 26234.559 | +0.088% | 38.025487 | 37.993715 | [52.736, 53.247] | [53.248, 53.759] |
| 1 | 64 | point_get | 375377.959 | 378494.643 | +0.830% | 170.370141 | 168.967732 | [303.104, 307.199] | [290.816, 294.911] |
| 1 | 64 | batch_get | 369046.771 | 370052.966 | +0.273% | 173.260437 | 172.788666 | [307.200, 311.295] | [299.008, 303.103] |
| 2 | 1 | point_get | 26515.239 | 26443.944 | -0.269% | 37.587147 | 37.694850 | [52.224, 52.735] | [52.224, 52.735] |
| 2 | 1 | batch_get | 26417.550 | 26137.390 | -1.061% | 37.726517 | 38.131631 | [51.712, 52.223] | [52.736, 53.247] |
| 2 | 64 | point_get | 373483.989 | 377353.033 | +1.036% | 171.239042 | 169.477852 | [299.008, 303.103] | [299.008, 303.103] |
| 2 | 64 | batch_get | 368778.563 | 368803.543 | +0.007% | 173.388400 | 173.375472 | [303.104, 307.199] | [311.296, 315.391] |
| 3 | 1 | point_get | 26450.092 | 26489.796 | +0.150% | 37.682103 | 37.627154 | [52.736, 53.247] | [52.224, 52.735] |
| 3 | 1 | batch_get | 26196.909 | 26307.727 | +0.423% | 38.032524 | 37.885833 | [53.248, 53.759] | [52.736, 53.247] |
| 3 | 64 | point_get | 375213.693 | 377393.696 | +0.581% | 170.443877 | 169.458339 | [299.008, 303.103] | [299.008, 303.103] |
| 3 | 64 | batch_get | 369780.501 | 369785.144 | +0.001% | 172.917325 | 172.914705 | [299.008, 303.103] | [303.104, 307.199] |

Redis controls: GET pairs with point GET; MGET(1) pairs with BatchGet(1).

| Repeat | Workers | API | Calls/s | Mean µs | p99 µs |
|---:|---:|---|---:|---:|---|
| 0 | 1 | mget | 171460.505 | 5.752646 | [7.552, 7.615] |
| 0 | 1 | get | 171488.164 | 5.755209 | [7.872, 7.935] |
| 0 | 64 | mget | 503798.415 | 126.920382 | [229.376, 231.423] |
| 0 | 64 | get | 513472.359 | 124.530825 | [229.376, 231.423] |
| 1 | 64 | get | 508241.863 | 125.810937 | [229.376, 231.423] |
| 1 | 64 | mget | 510509.471 | 125.247842 | [229.376, 231.423] |
| 1 | 1 | get | 172755.633 | 5.713044 | [7.552, 7.615] |
| 1 | 1 | mget | 171492.376 | 5.751716 | [7.680, 7.743] |
| 2 | 64 | get | 515512.986 | 124.037881 | [229.376, 231.423] |
| 2 | 64 | mget | 507298.367 | 126.039865 | [229.376, 231.423] |
| 2 | 1 | get | 173053.770 | 5.703152 | [7.552, 7.615] |
| 2 | 1 | mget | 171565.943 | 5.749380 | [7.424, 7.487] |
| 3 | 1 | mget | 171278.085 | 5.758300 | [7.488, 7.551] |
| 3 | 1 | get | 173140.407 | 5.700368 | [7.424, 7.487] |
| 3 | 64 | mget | 508551.781 | 125.732464 | [229.376, 231.423] |
| 3 | 64 | get | 511349.344 | 125.051417 | [231.424, 233.471] |

Pooled values retain all four repeats: total calls / total cohort elapsed, and total whole-call latency / total count.

| Workers | Arm | Calls | Pooled calls/s | Pooled mean µs |
|---:|---|---:|---:|---:|
| 1 | old-point | 1,061,462 | 26536.505 | 37.555366 |
| 1 | old-batch1 | 1,050,739 | 26268.420 | 37.935641 |
| 1 | new-point | 1,058,383 | 26459.542 | 37.670622 |
| 1 | new-batch1 | 1,050,641 | 26265.960 | 37.941827 |
| 1 | redis-mget1 | 6,857,971 | 171449.227 | 5.753009 |
| 1 | redis-get1 | 6,904,382 | 172609.493 | 5.717858 |
| 64 | old-point | 14,992,729 | 374812.531 | 170.629087 |
| 64 | old-batch1 | 14,759,350 | 368978.621 | 173.293636 |
| 64 | new-point | 15,112,403 | 377804.713 | 169.276052 |
| 64 | new-batch1 | 14,796,290 | 369901.873 | 172.860253 |
| 64 | redis-mget1 | 20,302,105 | 507539.503 | 125.982205 |
| 64 | redis-get1 | 20,486,278 | 512144.140 | 124.854313 |

Pooled candidate/control changes: c1 point_get QPS -0.290026%, mean +0.306895%; c1 batch_get QPS -0.009364%, mean +0.016306%; c64 point_get QPS +0.798314%, mean -0.792968%; c64 batch_get QPS +0.250219%, mean -0.250086%.

Readback verified 160 exited owned lifetimes, 96 qualifying fresh drains, 96 voter/writer/listener bindings, 9,386 resource samples, and 2,348 role source-file checks. It retained 1,056 files / 177,511,414 bytes. Source, executable, client, deterministic final data, nonce-zero read-only state, resource placement and retained-file predicates passed. All three owned containers restored configured/effective CPUs to 0–31; historical namespace identities were preserved. Original outer restoration remains successful with no cleanup errors.

Accepted audit SHA-256: `28a80e747f46cdc0fee27fb0b4bfa91dbb1e4d70cacb406355b7cdb370171a77`.
Statistics SHA-256: `edf3d12164eaea608607cd62fa2d1d6f0b7f237dda2f889f80f41f84a73b80c0`.
