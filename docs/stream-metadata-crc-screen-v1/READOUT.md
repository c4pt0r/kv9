# Stream-metadata-CRC point-read/mixed screen

24 point-read/mixed c1/c64 diagnostic cohorts; two 10-second forward/reverse repetitions. Not full72, significance, equal durability or sustained capacity.

Both repeats are retained. CRC means the accepted byte-table server; stream-metadata-crc is the clean stream-metadata-crc candidate. Redis uses actual GET/SET with the same v3 dataset/configuration. KV9 retains quorum/sync calls on tmpfs WAL; Redis is standalone memory. The workloads do not have equal durability.

All quantiles below are merged raw histogram bucket intervals. Mixed combined means/quantiles weight actual read/write counts; the separate-operation table pools GET and PUT/SET independently. Operation QPS uses complete cohort elapsed time, not only time spent in that operation.

## Pooled over both repetitions

| c | Mix | Role | QPS | Mean µs | p50 µs | p95 µs | p99 µs |
|---:|---|---|---:|---:|---|---|---|
| 1 | 50% | CRC | 18928.381 | 52.719 | [52.224, 52.735] | [66.560, 67.583] | [74.752, 75.775] |
| 1 | 50% | stream-metadata-crc | 18844.438 | 52.957 | [52.224, 52.735] | [66.560, 67.583] | [74.752, 75.775] |
| 1 | 50% | Redis | 173049.933 | 5.701 | [5.632, 5.695] | [5.888, 5.951] | [7.296, 7.359] |
| 1 | 100% | CRC | 26478.403 | 37.647 | [36.352, 36.863] | [44.032, 44.543] | [50.688, 51.199] |
| 1 | 100% | stream-metadata-crc | 26494.836 | 37.626 | [36.352, 36.863] | [44.032, 44.543] | [50.176, 50.687] |
| 1 | 100% | Redis | 174124.768 | 5.668 | [5.568, 5.631] | [5.888, 5.951] | [7.424, 7.487] |
| 64 | 50% | CRC | 171751.565 | 372.493 | [356.352, 360.447] | [516.096, 520.191] | [614.400, 622.591] |
| 64 | 50% | stream-metadata-crc | 170973.047 | 374.190 | [356.352, 360.447] | [520.192, 524.287] | [614.400, 622.591] |
| 64 | 50% | Redis | 501858.958 | 127.392 | [120.832, 121.855] | [163.840, 165.887] | [233.472, 235.519] |
| 64 | 100% | CRC | 345420.507 | 185.155 | [172.032, 174.079] | [278.528, 282.623] | [352.256, 356.351] |
| 64 | 100% | stream-metadata-crc | 345456.303 | 185.136 | [172.032, 174.079] | [278.528, 282.623] | [356.352, 360.447] |
| 64 | 100% | Redis | 511909.266 | 124.916 | [117.760, 118.783] | [161.792, 163.839] | [229.376, 231.423] |

## Mixed operations, separately pooled

| c | Mix | Role | API | QPS | Mean µs | p50 µs | p95 µs | p99 µs |
|---:|---|---|---|---:|---:|---|---|---|
| 1 | 50% | CRC | get | 9479.590 | 47.134 | [46.592, 47.103] | [57.344, 57.855] | [63.488, 63.999] |
| 1 | 50% | CRC | put | 9448.790 | 58.322 | [57.344, 57.855] | [69.632, 70.655] | [77.824, 78.847] |
| 1 | 50% | stream-metadata-crc | get | 9437.019 | 47.342 | [46.592, 47.103] | [57.856, 58.367] | [63.488, 63.999] |
| 1 | 50% | stream-metadata-crc | put | 9407.419 | 58.589 | [57.344, 57.855] | [69.632, 70.655] | [77.824, 78.847] |
| 1 | 50% | Redis | get | 86434.667 | 5.680 | [5.568, 5.631] | [5.888, 5.951] | [7.296, 7.359] |
| 1 | 50% | Redis | set | 86615.267 | 5.722 | [5.632, 5.695] | [5.888, 5.951] | [7.360, 7.423] |
| 64 | 50% | CRC | get | 85790.785 | 384.598 | [368.640, 372.735] | [532.480, 540.671] | [622.592, 630.783] |
| 64 | 50% | CRC | put | 85960.780 | 360.412 | [344.064, 348.159] | [499.712, 503.807] | [589.824, 598.015] |
| 64 | 50% | stream-metadata-crc | get | 85401.977 | 386.367 | [368.640, 372.735] | [532.480, 540.671] | [622.592, 630.783] |
| 64 | 50% | stream-metadata-crc | put | 85571.071 | 362.036 | [348.160, 352.255] | [499.712, 503.807] | [589.824, 598.015] |
| 64 | 50% | Redis | get | 250829.756 | 127.400 | [120.832, 121.855] | [163.840, 165.887] | [233.472, 235.519] |
| 64 | 50% | Redis | set | 251029.202 | 127.384 | [120.832, 121.855] | [163.840, 165.887] | [233.472, 235.519] |

## Candidate changes versus CRC and Redis

| c | Mix | QPS vs CRC | Mean vs CRC | QPS vs Redis | Mean vs Redis |
|---:|---|---:|---:|---:|---:|
| 1 | point-r050 | -0.443% | +0.452% | -89.110% | +828.906% |
| 1 | point-r100 | +0.062% | -0.055% | -84.784% | +563.884% |
| 64 | point-r050 | -0.453% | +0.456% | -65.932% | +193.731% |
| 64 | point-r100 | +0.010% | -0.010% | -32.516% | +48.209% |

Measured totals: **49,706,706 calls / 49,706,706 issued / 49,706,706 attempts**; success=49,706,706, dropped slots=0. Every outcome population is retained in summary.json. Full phase accounting also retains initialization routing attempts rather than applying measurement-only single-attempt claims to setup.

Per-repeat combined and mixed-operation rows are in PER-REPEAT.md and summary.json. Pooled and per-repeat candidate-vs-CRC/Redis comparisons also preserve both p95/p99 intervals and per-operation deltas. No percentile is averaged.

The accepted audit records 80 exited lifetimes, 48 qualifying drains, 48 voter writer/listener bindings, 4678 resource samples, and 636 retained files / 4,552,528,169 bytes. Exact configured/effective CPU restoration and namespace preservation passed for all three owned containers. These original acceptance checks were not rerun.

The derivative auditor and this summary cover only point read50/read100. Original accepted audit bytes are retained unchanged.

The arithmetic core is copied byte-identically from the accepted v3/de37 statistics preparation; the bounded reader binds all 24 raw reports and their retained resource coverage to the accepted input inventory, and checks each original combined/per-operation histogram against the accepted audit before pooling. No runtime, build, test or profiling campaign is part of this derivation. Native SDK attempt-reason maps cover native calls only; Redis command attempts remain included in aggregate attempt counts.
