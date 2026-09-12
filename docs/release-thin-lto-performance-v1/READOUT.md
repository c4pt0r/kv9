# ThinLTO release point-read/mixed screen

24 point-read/mixed c1/c64 diagnostic cohorts; two 10-second forward/reverse repetitions. Not full72, significance, equal durability or sustained capacity.

Both repeats are retained. CRC means the accepted byte-table server; thin-lto is the clean thin-lto candidate. Redis uses actual GET/SET with the same v3 dataset/configuration. KV9 retains quorum/sync calls on tmpfs WAL; Redis is standalone memory. The workloads do not have equal durability.

All quantiles below are merged raw histogram bucket intervals. Mixed combined means/quantiles weight actual read/write counts; the separate-operation table pools GET and PUT/SET independently. Operation QPS uses complete cohort elapsed time, not only time spent in that operation.

## Pooled over both repetitions

| c | Mix | Role | QPS | Mean µs | p50 µs | p95 µs | p99 µs |
|---:|---|---|---:|---:|---|---|---|
| 1 | 50% | CRC | 18968.689 | 52.610 | [52.224, 52.735] | [66.560, 67.583] | [73.728, 74.751] |
| 1 | 50% | thin-lto | 21603.659 | 46.180 | [45.568, 46.079] | [56.832, 57.343] | [63.488, 63.999] |
| 1 | 50% | Redis | 172140.336 | 5.730 | [5.632, 5.695] | [5.888, 5.951] | [7.424, 7.487] |
| 1 | 100% | CRC | 26591.615 | 37.485 | [36.352, 36.863] | [43.520, 44.031] | [50.176, 50.687] |
| 1 | 100% | thin-lto | 28293.988 | 35.226 | [34.304, 34.815] | [39.424, 39.935] | [45.056, 45.567] |
| 1 | 100% | Redis | 174022.103 | 5.671 | [5.568, 5.631] | [5.888, 5.951] | [7.424, 7.487] |
| 64 | 50% | CRC | 170992.289 | 374.145 | [356.352, 360.447] | [520.192, 524.287] | [614.400, 622.591] |
| 64 | 50% | thin-lto | 188817.154 | 338.811 | [323.584, 327.679] | [471.040, 475.135] | [557.056, 565.247] |
| 64 | 50% | Redis | 500132.085 | 127.830 | [119.808, 120.831] | [165.888, 167.935] | [233.472, 235.519] |
| 64 | 100% | CRC | 346695.956 | 184.474 | [172.032, 174.079] | [278.528, 282.623] | [352.256, 356.351] |
| 64 | 100% | thin-lto | 376202.163 | 169.998 | [157.696, 159.743] | [260.096, 262.143] | [323.584, 327.679] |
| 64 | 100% | Redis | 510898.184 | 125.158 | [117.760, 118.783] | [161.792, 163.839] | [229.376, 231.423] |

## Mixed operations, separately pooled

| c | Mix | Role | API | QPS | Mean µs | p50 µs | p95 µs | p99 µs |
|---:|---|---|---|---:|---:|---|---|---|
| 1 | 50% | CRC | get | 9498.319 | 47.090 | [46.592, 47.103] | [57.344, 57.855] | [62.976, 63.487] |
| 1 | 50% | CRC | put | 9470.369 | 58.146 | [57.344, 57.855] | [69.632, 70.655] | [76.800, 77.823] |
| 1 | 50% | thin-lto | get | 10832.030 | 41.772 | [40.960, 41.471] | [49.664, 50.175] | [54.784, 55.295] |
| 1 | 50% | thin-lto | put | 10771.630 | 50.614 | [49.664, 50.175] | [59.392, 59.903] | [66.560, 67.583] |
| 1 | 50% | Redis | get | 85986.868 | 5.718 | [5.632, 5.695] | [5.952, 6.015] | [7.424, 7.487] |
| 1 | 50% | Redis | set | 86153.468 | 5.742 | [5.632, 5.695] | [5.888, 5.951] | [7.424, 7.487] |
| 64 | 50% | CRC | get | 85411.396 | 386.171 | [368.640, 372.735] | [532.480, 540.671] | [630.784, 638.975] |
| 64 | 50% | CRC | put | 85580.892 | 362.143 | [344.064, 348.159] | [499.712, 503.807] | [598.016, 606.207] |
| 64 | 50% | thin-lto | get | 94316.429 | 350.297 | [335.872, 339.967] | [483.328, 487.423] | [565.248, 573.439] |
| 64 | 50% | thin-lto | put | 94500.725 | 327.347 | [311.296, 315.391] | [450.560, 454.655] | [540.672, 548.863] |
| 64 | 50% | Redis | get | 249962.295 | 127.850 | [120.832, 121.855] | [165.888, 167.935] | [233.472, 235.519] |
| 64 | 50% | Redis | set | 250169.789 | 127.810 | [119.808, 120.831] | [165.888, 167.935] | [233.472, 235.519] |

## Candidate changes versus CRC and Redis

| c | Mix | QPS vs CRC | Mean vs CRC | QPS vs Redis | Mean vs Redis |
|---:|---|---:|---:|---:|---:|
| 1 | point-r050 | +13.891% | -12.221% | -87.450% | +705.948% |
| 1 | point-r100 | +6.402% | -6.025% | -83.741% | +521.185% |
| 64 | point-r050 | +10.424% | -9.444% | -62.247% | +165.049% |
| 64 | point-r100 | +8.511% | -7.847% | -26.365% | +35.826% |

Measured totals: **50,708,100 calls / 50,708,100 issued / 50,708,100 attempts**; success=50,708,100, dropped slots=0. Every outcome population is retained in summary.json. Full phase accounting also retains initialization routing attempts rather than applying measurement-only single-attempt claims to setup.

Per-repeat combined and mixed-operation rows are in PER-REPEAT.md and summary.json. Pooled and per-repeat candidate-vs-CRC/Redis comparisons also preserve both p95/p99 intervals and per-operation deltas. No percentile is averaged.

The accepted audit records 80 exited lifetimes, 48 qualifying drains, 48 voter writer/listener bindings, 4681 resource samples, and 642 retained files / 4,786,432,855 bytes. Exact configured/effective CPU restoration and namespace preservation passed for all three owned containers. These original acceptance checks were not rerun.

The derivative auditor and this summary cover only point read50/read100. Original accepted audit bytes are retained unchanged.

The arithmetic core is copied byte-identically from the accepted v3/de37 statistics preparation; the bounded reader binds all 24 raw reports and their retained resource coverage to the accepted input inventory, and checks each original combined/per-operation histogram against the accepted audit before pooling. No runtime, build, test or profiling campaign is part of this derivation. Native SDK attempt-reason maps cover native calls only; Redis command attempts remain included in aggregate attempt counts.
