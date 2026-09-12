# Bounded owner poll point-read/mixed screen

24 point-read/mixed c1/c64 diagnostic cohorts; two 10-second forward/reverse repetitions. Not full72, significance, equal durability or sustained capacity.

Both repeats are retained. Control11113 means the selected uninstrumented ThinLTO server; bounded-owner-poll is the separately bound fixed-budget candidate. Redis uses actual GET/SET with the same v3 dataset/configuration. KV9 retains quorum/sync calls on tmpfs WAL; Redis is standalone memory. The workloads do not have equal durability.

All quantiles below are merged raw histogram bucket intervals. Mixed combined means/quantiles weight actual read/write counts; the separate-operation table pools GET and PUT/SET independently. Operation QPS uses complete cohort elapsed time, not only time spent in that operation.

## Pooled over both repetitions

| c | Mix | Role | QPS | Mean µs | p50 µs | p95 µs | p99 µs |
|---:|---|---|---:|---:|---|---|---|
| 1 | 50% | Control11113 | 21450.361 | 46.510 | [46.080, 46.591] | [57.856, 58.367] | [65.536, 66.559] |
| 1 | 50% | bounded-owner-poll | 17583.188 | 56.763 | [49.664, 50.175] | [97.280, 98.303] | [121.856, 122.879] |
| 1 | 50% | Redis | 171553.680 | 5.750 | [5.632, 5.695] | [5.952, 6.015] | [7.744, 7.807] |
| 1 | 100% | Control11113 | 28252.671 | 35.280 | [34.304, 34.815] | [39.936, 40.447] | [47.104, 47.615] |
| 1 | 100% | bounded-owner-poll | 22895.144 | 43.561 | [39.936, 40.447] | [70.656, 71.679] | [81.920, 82.943] |
| 1 | 100% | Redis | 173925.251 | 5.673 | [5.568, 5.631] | [5.888, 5.951] | [7.488, 7.551] |
| 64 | 50% | Control11113 | 190236.125 | 336.286 | [319.488, 323.583] | [466.944, 471.039] | [557.056, 565.247] |
| 64 | 50% | bounded-owner-poll | 167369.568 | 382.247 | [360.448, 364.543] | [557.056, 565.247] | [679.936, 688.127] |
| 64 | 50% | Redis | 499044.195 | 128.107 | [120.832, 121.855] | [165.888, 167.935] | [233.472, 235.519] |
| 64 | 100% | Control11113 | 374812.758 | 170.629 | [157.696, 159.743] | [260.096, 262.143] | [323.584, 327.679] |
| 64 | 100% | bounded-owner-poll | 280167.048 | 228.306 | [221.184, 223.231] | [319.488, 323.583] | [368.640, 372.735] |
| 64 | 100% | Redis | 511910.764 | 124.910 | [117.760, 118.783] | [163.840, 165.887] | [229.376, 231.423] |

## Mixed operations, separately pooled

| c | Mix | Role | API | QPS | Mean µs | p50 µs | p95 µs | p99 µs |
|---:|---|---|---|---:|---:|---|---|---|
| 1 | 50% | Control11113 | get | 10754.281 | 41.996 | [40.960, 41.471] | [50.176, 50.687] | [56.320, 56.831] |
| 1 | 50% | Control11113 | put | 10696.081 | 51.048 | [49.664, 50.175] | [60.416, 60.927] | [68.608, 69.631] |
| 1 | 50% | bounded-owner-poll | get | 8799.669 | 49.074 | [43.520, 44.031] | [81.920, 82.943] | [107.520, 108.543] |
| 1 | 50% | bounded-owner-poll | put | 8783.519 | 64.467 | [57.856, 58.367] | [106.496, 107.519] | [130.048, 131.071] |
| 1 | 50% | Redis | get | 85695.140 | 5.736 | [5.632, 5.695] | [5.952, 6.015] | [7.744, 7.807] |
| 1 | 50% | Redis | set | 85858.540 | 5.764 | [5.632, 5.695] | [5.952, 6.015] | [7.744, 7.807] |
| 64 | 50% | Control11113 | get | 95024.690 | 347.650 | [331.776, 335.871] | [479.232, 483.327] | [565.248, 573.439] |
| 64 | 50% | Control11113 | put | 95211.436 | 324.945 | [311.296, 315.391] | [450.560, 454.655] | [540.672, 548.863] |
| 64 | 50% | bounded-owner-poll | get | 83591.186 | 394.553 | [372.736, 376.831] | [573.440, 581.631] | [688.128, 696.319] |
| 64 | 50% | bounded-owner-poll | put | 83778.382 | 369.967 | [348.160, 352.255] | [540.672, 548.863] | [671.744, 679.935] |
| 64 | 50% | Redis | get | 249414.550 | 128.103 | [120.832, 121.855] | [165.888, 167.935] | [233.472, 235.519] |
| 64 | 50% | Redis | set | 249629.645 | 128.112 | [120.832, 121.855] | [165.888, 167.935] | [235.520, 237.567] |

## Candidate changes versus Control11113 and Redis

| c | Mix | QPS vs Control11113 | Mean vs Control11113 | QPS vs Redis | Mean vs Redis |
|---:|---|---:|---:|---:|---:|
| 1 | point-r050 | -18.028% | +22.046% | -89.751% | +887.185% |
| 1 | point-r100 | -18.963% | +23.473% | -86.836% | +667.859% |
| 64 | point-r050 | -12.020% | +13.667% | -66.462% | +198.380% |
| 64 | point-r100 | -25.251% | +33.802% | -45.270% | +82.776% |

Measured totals: **49,184,881 calls / 49,184,881 issued / 49,184,881 attempts**; success=49,184,881, dropped slots=0. Every outcome population is retained in summary.json. Full phase accounting also retains initialization routing attempts rather than applying measurement-only single-attempt claims to setup.

Per-repeat combined and mixed-operation rows are in PER-REPEAT.md and summary.json. Pooled and per-repeat candidate-vs-Control11113/Redis comparisons also preserve both p95/p99 intervals and per-operation deltas. No percentile is averaged.

The accepted audit records 80 exited lifetimes, 48 qualifying drains, 48 voter writer/listener bindings, 4680 resource samples, and 642 retained files / 4,739,356,586 bytes. Exact configured/effective CPU restoration and namespace preservation passed for all three owned containers. These original acceptance checks were not rerun.

The derivative auditor and this summary cover only point read50/read100. Original accepted audit bytes are retained unchanged.

The arithmetic core is copied byte-identically from the accepted v3/de37 statistics preparation; the bounded reader binds all 24 raw reports and their retained resource coverage to the accepted input inventory, and checks each original combined/per-operation histogram against the accepted audit before pooling. No runtime, build, test or profiling campaign is part of this derivation. Native SDK attempt-reason maps cover native calls only; Redis command attempts remain included in aggregate attempt counts.
