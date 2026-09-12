# Global queue interval eight point-read/mixed screen

24 point-read/mixed c1/c64 diagnostic cohorts; two 10-second forward/reverse repetitions. Not full72, significance, equal durability or sustained capacity.

Both repeats are retained. CRC means the accepted byte-table server; global-queue-interval-eight is the clean global-queue-interval-eight candidate. Redis uses actual GET/SET with the same v3 dataset/configuration. KV9 retains quorum/sync calls on tmpfs WAL; Redis is standalone memory. The workloads do not have equal durability.

All quantiles below are merged raw histogram bucket intervals. Mixed combined means/quantiles weight actual read/write counts; the separate-operation table pools GET and PUT/SET independently. Operation QPS uses complete cohort elapsed time, not only time spent in that operation.

## Pooled over both repetitions

| c | Mix | Role | QPS | Mean µs | p50 µs | p95 µs | p99 µs |
|---:|---|---|---:|---:|---|---|---|
| 1 | 50% | CRC | 18832.735 | 52.992 | [52.224, 52.735] | [66.560, 67.583] | [74.752, 75.775] |
| 1 | 50% | global-queue-interval-eight | 18791.621 | 53.107 | [52.736, 53.247] | [66.560, 67.583] | [74.752, 75.775] |
| 1 | 50% | Redis | 172507.698 | 5.718 | [5.632, 5.695] | [5.888, 5.951] | [7.360, 7.423] |
| 1 | 100% | CRC | 26542.953 | 37.562 | [36.352, 36.863] | [43.520, 44.031] | [49.664, 50.175] |
| 1 | 100% | global-queue-interval-eight | 26391.062 | 37.779 | [36.864, 37.375] | [44.032, 44.543] | [50.176, 50.687] |
| 1 | 100% | Redis | 174382.402 | 5.658 | [5.568, 5.631] | [5.888, 5.951] | [7.296, 7.359] |
| 64 | 50% | CRC | 170326.223 | 375.601 | [360.448, 364.543] | [520.192, 524.287] | [614.400, 622.591] |
| 64 | 50% | global-queue-interval-eight | 168640.129 | 379.356 | [364.544, 368.639] | [524.288, 532.479] | [622.592, 630.783] |
| 64 | 50% | Redis | 503781.883 | 126.904 | [119.808, 120.831] | [163.840, 165.887] | [233.472, 235.519] |
| 64 | 100% | CRC | 346069.116 | 184.807 | [172.032, 174.079] | [278.528, 282.623] | [352.256, 356.351] |
| 64 | 100% | global-queue-interval-eight | 337799.779 | 189.335 | [176.128, 178.175] | [286.720, 290.815] | [356.352, 360.447] |
| 64 | 100% | Redis | 511088.898 | 125.114 | [117.760, 118.783] | [161.792, 163.839] | [231.424, 233.471] |

## Mixed operations, separately pooled

| c | Mix | Role | API | QPS | Mean µs | p50 µs | p95 µs | p99 µs |
|---:|---|---|---|---:|---:|---|---|---|
| 1 | 50% | CRC | get | 9431.492 | 47.420 | [46.592, 47.103] | [57.856, 58.367] | [63.488, 63.999] |
| 1 | 50% | CRC | put | 9401.242 | 58.581 | [57.344, 57.855] | [69.632, 70.655] | [77.824, 78.847] |
| 1 | 50% | global-queue-interval-eight | get | 9410.261 | 47.483 | [46.592, 47.103] | [57.856, 58.367] | [64.000, 64.511] |
| 1 | 50% | global-queue-interval-eight | put | 9381.361 | 58.749 | [57.856, 58.367] | [69.632, 70.655] | [78.848, 79.871] |
| 1 | 50% | Redis | get | 86171.524 | 5.708 | [5.632, 5.695] | [5.888, 5.951] | [7.360, 7.423] |
| 1 | 50% | Redis | set | 86336.174 | 5.727 | [5.632, 5.695] | [5.888, 5.951] | [7.360, 7.423] |
| 64 | 50% | CRC | get | 85076.689 | 387.535 | [372.736, 376.831] | [532.480, 540.671] | [630.784, 638.975] |
| 64 | 50% | CRC | put | 85249.534 | 363.691 | [348.160, 352.255] | [503.808, 507.903] | [598.016, 606.207] |
| 64 | 50% | global-queue-interval-eight | get | 84226.692 | 390.292 | [372.736, 376.831] | [540.672, 548.863] | [630.784, 638.975] |
| 64 | 50% | global-queue-interval-eight | put | 84413.437 | 368.443 | [352.256, 356.351] | [507.904, 511.999] | [606.208, 614.399] |
| 64 | 50% | Redis | get | 251791.494 | 126.912 | [119.808, 120.831] | [163.840, 165.887] | [233.472, 235.519] |
| 64 | 50% | Redis | set | 251990.389 | 126.897 | [119.808, 120.831] | [163.840, 165.887] | [233.472, 235.519] |

## Candidate changes versus CRC and Redis

| c | Mix | QPS vs CRC | Mean vs CRC | QPS vs Redis | Mean vs Redis |
|---:|---|---:|---:|---:|---:|
| 1 | point-r050 | -0.218% | +0.218% | -89.107% | +828.827% |
| 1 | point-r100 | -0.572% | +0.578% | -84.866% | +567.760% |
| 64 | point-r050 | -0.990% | +1.000% | -66.525% | +198.931% |
| 64 | point-r100 | -2.390% | +2.450% | -33.906% | +51.330% |

Measured totals: **49,504,037 calls / 49,504,037 issued / 49,504,037 attempts**; success=49,504,037, dropped slots=0. Every outcome population is retained in summary.json. Full phase accounting also retains initialization routing attempts rather than applying measurement-only single-attempt claims to setup.

Per-repeat combined and mixed-operation rows are in PER-REPEAT.md and summary.json. Pooled and per-repeat candidate-vs-CRC/Redis comparisons also preserve both p95/p99 intervals and per-operation deltas. No percentile is averaged.

The accepted audit records 80 exited lifetimes, 48 qualifying drains, 48 voter writer/listener bindings, 4678 resource samples, and 636 retained files / 4,507,888,898 bytes. Exact configured/effective CPU restoration and namespace preservation passed for all three owned containers. These original acceptance checks were not rerun.

The derivative auditor and this summary cover only point read50/read100. Original accepted audit bytes are retained unchanged.

The arithmetic core is copied byte-identically from the accepted v3/de37 statistics preparation; the bounded reader binds all 24 raw reports and their retained resource coverage to the accepted input inventory, and checks each original combined/per-operation histogram against the accepted audit before pooling. No runtime, build, test or profiling campaign is part of this derivation. Native SDK attempt-reason maps cover native calls only; Redis command attempts remain included in aggregate attempt counts.
