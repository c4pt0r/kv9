# Two-context point-read/mixed screen

24 point-read/mixed c1/c64 diagnostic cohorts; two 10-second forward/reverse repetitions. Not full72, significance, equal durability or sustained capacity.

Both repeats are retained. CRC means the accepted byte-table server; window2 is the clean two-context candidate. Redis uses actual GET/SET with the same v3 dataset/configuration. KV9 retains quorum/sync calls on tmpfs WAL; Redis is standalone memory. The workloads do not have equal durability.

All quantiles below are merged raw histogram bucket intervals. Mixed combined means/quantiles weight actual read/write counts; the separate-operation table pools GET and PUT/SET independently. Operation QPS uses complete cohort elapsed time, not only time spent in that operation.

## Pooled over both repetitions

| c | Mix | Role | QPS | Mean µs | p50 µs | p95 µs | p99 µs |
|---:|---|---|---:|---:|---|---|---|
| 1 | 50% | CRC | 18847.677 | 52.949 | [52.224, 52.735] | [66.560, 67.583] | [74.752, 75.775] |
| 1 | 50% | window2 | 18998.048 | 52.529 | [51.712, 52.223] | [66.560, 67.583] | [74.752, 75.775] |
| 1 | 50% | Redis | 171910.123 | 5.738 | [5.632, 5.695] | [5.952, 6.015] | [7.360, 7.423] |
| 1 | 100% | CRC | 26637.627 | 37.424 | [36.352, 36.863] | [43.520, 44.031] | [49.664, 50.175] |
| 1 | 100% | window2 | 26381.243 | 37.785 | [36.864, 37.375] | [44.032, 44.543] | [50.176, 50.687] |
| 1 | 100% | Redis | 174163.618 | 5.665 | [5.568, 5.631] | [5.888, 5.951] | [7.296, 7.359] |
| 64 | 50% | CRC | 172235.945 | 371.442 | [356.352, 360.447] | [516.096, 520.191] | [606.208, 614.399] |
| 64 | 50% | window2 | 174274.553 | 367.095 | [348.160, 352.255] | [520.192, 524.287] | [614.400, 622.591] |
| 64 | 50% | Redis | 500337.707 | 127.778 | [120.832, 121.855] | [165.888, 167.935] | [233.472, 235.519] |
| 64 | 100% | CRC | 345322.532 | 185.210 | [172.032, 174.079] | [278.528, 282.623] | [352.256, 356.351] |
| 64 | 100% | window2 | 367127.110 | 174.199 | [167.936, 169.983] | [247.808, 249.855] | [311.296, 315.391] |
| 64 | 100% | Redis | 511324.927 | 125.051 | [117.760, 118.783] | [161.792, 163.839] | [229.376, 231.423] |

## Mixed operations, separately pooled

| c | Mix | Role | API | QPS | Mean µs | p50 µs | p95 µs | p99 µs |
|---:|---|---|---|---:|---:|---|---|---|
| 1 | 50% | CRC | get | 9439.138 | 47.360 | [46.592, 47.103] | [57.856, 58.367] | [63.488, 63.999] |
| 1 | 50% | CRC | put | 9408.539 | 58.557 | [57.344, 57.855] | [69.632, 70.655] | [77.824, 78.847] |
| 1 | 50% | window2 | get | 9513.424 | 47.016 | [46.592, 47.103] | [57.344, 57.855] | [62.976, 63.487] |
| 1 | 50% | window2 | put | 9484.624 | 58.059 | [56.832, 57.343] | [68.608, 69.631] | [77.824, 78.847] |
| 1 | 50% | Redis | get | 85873.336 | 5.726 | [5.632, 5.695] | [5.952, 6.015] | [7.424, 7.487] |
| 1 | 50% | Redis | set | 86036.786 | 5.749 | [5.632, 5.695] | [5.888, 5.951] | [7.360, 7.423] |
| 64 | 50% | CRC | get | 86033.699 | 383.676 | [368.640, 372.735] | [532.480, 540.671] | [622.592, 630.783] |
| 64 | 50% | CRC | put | 86202.246 | 359.232 | [344.064, 348.159] | [495.616, 499.711] | [589.824, 598.015] |
| 64 | 50% | window2 | get | 87041.629 | 395.217 | [380.928, 385.023] | [548.864, 557.055] | [638.976, 647.167] |
| 64 | 50% | window2 | put | 87232.924 | 339.035 | [323.584, 327.679] | [471.040, 475.135] | [557.056, 565.247] |
| 64 | 50% | Redis | get | 250064.481 | 127.786 | [120.832, 121.855] | [165.888, 167.935] | [233.472, 235.519] |
| 64 | 50% | Redis | set | 250273.226 | 127.771 | [120.832, 121.855] | [165.888, 167.935] | [233.472, 235.519] |

## Candidate changes versus CRC and Redis

| c | Mix | QPS vs CRC | Mean vs CRC | QPS vs Redis | Mean vs Redis |
|---:|---|---:|---:|---:|---:|
| 1 | point-r050 | +0.798% | -0.794% | -88.949% | +815.525% |
| 1 | point-r100 | -0.962% | +0.966% | -84.853% | +567.006% |
| 64 | point-r050 | +1.184% | -1.170% | -65.169% | +187.291% |
| 64 | point-r100 | +6.314% | -5.945% | -28.201% | +39.302% |

Measured totals: **50,152,139 calls = 50,152,139 issued = 50,152,139 attempts**, all success. Zero dropped slots and all non-success populations are retained in summary.json. Full phase accounting also retains initialization routing attempts rather than applying measurement-only single-attempt claims to setup.

Per-repeat combined and mixed-operation rows are in PER-REPEAT.md and summary.json. Pooled and per-repeat candidate-vs-CRC/Redis comparisons also preserve both p95/p99 intervals and per-operation deltas. No percentile is averaged.

The accepted audit records 80 exited lifetimes, 48 qualifying drains, 48 voter writer/listener bindings, 4677 resource samples, and 636 retained files / 4,598,354,509 bytes. Exact configured/effective CPU restoration and namespace preservation passed for all three owned containers. These original acceptance checks were not rerun.

The accepted audit retains inherited point/batch64 read/write scope text. Its actual protocol/descriptors and this summary cover only point read50/100; original audit bytes are unchanged.

The arithmetic core is copied byte-identically from the accepted v3/de37 statistics preparation; the bounded reader binds all 24 raw reports and their retained resource coverage to the accepted input inventory, and checks each original combined/per-operation histogram against the accepted audit before pooling. No runtime, build, test or profiling campaign is part of this derivation.
