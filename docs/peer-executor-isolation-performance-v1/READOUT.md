# Peer executor isolation point-read/mixed screen

24 point-read/mixed c1/c64 diagnostic cohorts; two 10-second forward/reverse repetitions. Not full72, significance, equal durability or sustained capacity.

Both repeats are retained. CRC means the accepted byte-table server; peer-executor-isolation is the clean peer-executor-isolation candidate. Redis uses actual GET/SET with the same v3 dataset/configuration. KV9 retains quorum/sync calls on tmpfs WAL; Redis is standalone memory. The workloads do not have equal durability.

All quantiles below are merged raw histogram bucket intervals. Mixed combined means/quantiles weight actual read/write counts; the separate-operation table pools GET and PUT/SET independently. Operation QPS uses complete cohort elapsed time, not only time spent in that operation.

## Pooled over both repetitions

| c | Mix | Role | QPS | Mean µs | p50 µs | p95 µs | p99 µs |
|---:|---|---|---:|---:|---|---|---|
| 1 | 50% | CRC | 18923.392 | 52.738 | [52.224, 52.735] | [66.560, 67.583] | [74.752, 75.775] |
| 1 | 50% | peer-executor-isolation | 19210.232 | 51.949 | [51.712, 52.223] | [65.536, 66.559] | [73.728, 74.751] |
| 1 | 50% | Redis | 171812.067 | 5.741 | [5.632, 5.695] | [5.952, 6.015] | [7.424, 7.487] |
| 1 | 100% | CRC | 26750.692 | 37.265 | [36.352, 36.863] | [43.008, 43.519] | [49.664, 50.175] |
| 1 | 100% | peer-executor-isolation | 27863.230 | 35.774 | [34.816, 35.327] | [39.424, 39.935] | [47.616, 48.127] |
| 1 | 100% | Redis | 173282.670 | 5.693 | [5.568, 5.631] | [5.952, 6.015] | [7.680, 7.743] |
| 64 | 50% | CRC | 171768.495 | 372.453 | [356.352, 360.447] | [516.096, 520.191] | [614.400, 622.591] |
| 64 | 50% | peer-executor-isolation | 168220.418 | 380.306 | [360.448, 364.543] | [548.864, 557.055] | [663.552, 671.743] |
| 64 | 50% | Redis | 500947.685 | 127.622 | [120.832, 121.855] | [163.840, 165.887] | [233.472, 235.519] |
| 64 | 100% | CRC | 345507.734 | 185.108 | [172.032, 174.079] | [282.624, 286.719] | [352.256, 356.351] |
| 64 | 100% | peer-executor-isolation | 330252.289 | 193.665 | [180.224, 182.271] | [294.912, 299.007] | [364.544, 368.639] |
| 64 | 100% | Redis | 507232.724 | 126.062 | [117.760, 118.783] | [163.840, 165.887] | [229.376, 231.423] |

## Mixed operations, separately pooled

| c | Mix | Role | API | QPS | Mean µs | p50 µs | p95 µs | p99 µs |
|---:|---|---|---|---:|---:|---|---|---|
| 1 | 50% | CRC | get | 9476.021 | 47.226 | [46.592, 47.103] | [57.344, 57.855] | [63.488, 63.999] |
| 1 | 50% | CRC | put | 9447.371 | 58.266 | [57.344, 57.855] | [69.632, 70.655] | [77.824, 78.847] |
| 1 | 50% | peer-executor-isolation | get | 9618.941 | 45.214 | [44.544, 45.055] | [54.784, 55.295] | [60.416, 60.927] |
| 1 | 50% | peer-executor-isolation | put | 9591.291 | 58.703 | [57.344, 57.855] | [68.608, 69.631] | [77.824, 78.847] |
| 1 | 50% | Redis | get | 85822.783 | 5.728 | [5.632, 5.695] | [5.952, 6.015] | [7.424, 7.487] |
| 1 | 50% | Redis | set | 85989.283 | 5.754 | [5.632, 5.695] | [5.888, 5.951] | [7.424, 7.487] |
| 64 | 50% | CRC | get | 85802.750 | 384.481 | [368.640, 372.735] | [532.480, 540.671] | [622.592, 630.783] |
| 64 | 50% | CRC | put | 85965.745 | 360.448 | [344.064, 348.159] | [499.712, 503.807] | [589.824, 598.015] |
| 64 | 50% | peer-executor-isolation | get | 84013.287 | 395.922 | [376.832, 380.927] | [565.248, 573.439] | [679.936, 688.127] |
| 64 | 50% | peer-executor-isolation | put | 84207.131 | 364.725 | [344.064, 348.159] | [524.288, 532.479] | [638.976, 647.167] |
| 64 | 50% | Redis | get | 250372.745 | 127.628 | [120.832, 121.855] | [163.840, 165.887] | [233.472, 235.519] |
| 64 | 50% | Redis | set | 250574.940 | 127.615 | [120.832, 121.855] | [163.840, 165.887] | [233.472, 235.519] |

## Candidate changes versus CRC and Redis

| c | Mix | QPS vs CRC | Mean vs CRC | QPS vs Redis | Mean vs Redis |
|---:|---|---:|---:|---:|---:|
| 1 | point-r050 | +1.516% | -1.496% | -88.819% | +804.866% |
| 1 | point-r100 | +4.159% | -4.000% | -83.920% | +528.348% |
| 64 | point-r050 | -2.066% | +2.108% | -66.420% | +197.994% |
| 64 | point-r100 | -4.415% | +4.622% | -34.891% | +53.626% |

Measured totals: **49,236,254 calls / 49,236,254 issued / 49,236,254 attempts**; success=49,236,254, dropped slots=0. Every outcome population is retained in summary.json. Full phase accounting also retains initialization routing attempts rather than applying measurement-only single-attempt claims to setup.

Per-repeat combined and mixed-operation rows are in PER-REPEAT.md and summary.json. Pooled and per-repeat candidate-vs-CRC/Redis comparisons also preserve both p95/p99 intervals and per-operation deltas. No percentile is averaged.

The accepted audit records 80 exited lifetimes, 48 qualifying drains, 48 voter writer/listener bindings, 4675 resource samples, and 636 retained files / 4,525,699,688 bytes. Exact configured/effective CPU restoration and namespace preservation passed for all three owned containers. These original acceptance checks were not rerun.

The derivative auditor and this summary cover only point read50/read100. Original accepted audit bytes are retained unchanged.

The arithmetic core is copied byte-identically from the accepted v3/de37 statistics preparation; the bounded reader binds all 24 raw reports and their retained resource coverage to the accepted input inventory, and checks each original combined/per-operation histogram against the accepted audit before pooling. No runtime, build, test or profiling campaign is part of this derivation. Native SDK attempt-reason maps cover native calls only; Redis command attempts remain included in aggregate attempt counts.
