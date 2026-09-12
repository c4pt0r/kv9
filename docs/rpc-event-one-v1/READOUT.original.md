# Inbox vector reuse point-read/mixed screen

24 point-read/mixed c1/c64 diagnostic cohorts; two 10-second forward/reverse repetitions. Not full72, significance, equal durability or sustained capacity.

Both repeats are retained. CRC means the accepted byte-table server; rpc-event-one is the clean rpc-event-one candidate. Redis uses actual GET/SET with the same v3 dataset/configuration. KV9 retains quorum/sync calls on tmpfs WAL; Redis is standalone memory. The workloads do not have equal durability.

All quantiles below are merged raw histogram bucket intervals. Mixed combined means/quantiles weight actual read/write counts; the separate-operation table pools GET and PUT/SET independently. Operation QPS uses complete cohort elapsed time, not only time spent in that operation.

## Pooled over both repetitions

| c | Mix | Role | QPS | Mean µs | p50 µs | p95 µs | p99 µs |
|---:|---|---|---:|---:|---|---|---|
| 1 | 50% | CRC | 18990.556 | 52.549 | [51.712, 52.223] | [66.560, 67.583] | [73.728, 74.751] |
| 1 | 50% | rpc-event-one | 18171.514 | 54.923 | [54.272, 54.783] | [68.608, 69.631] | [75.776, 76.799] |
| 1 | 50% | Redis | 171962.869 | 5.735 | [5.632, 5.695] | [5.952, 6.015] | [7.360, 7.423] |
| 1 | 100% | CRC | 26618.801 | 37.449 | [36.352, 36.863] | [43.520, 44.031] | [49.664, 50.175] |
| 1 | 100% | rpc-event-one | 24935.181 | 39.987 | [38.912, 39.423] | [45.568, 46.079] | [51.200, 51.711] |
| 1 | 100% | Redis | 173073.396 | 5.701 | [5.568, 5.631] | [5.952, 6.015] | [7.552, 7.615] |
| 64 | 50% | CRC | 171793.616 | 372.398 | [356.352, 360.447] | [516.096, 520.191] | [614.400, 622.591] |
| 64 | 50% | rpc-event-one | 162541.915 | 393.603 | [376.832, 380.927] | [548.864, 557.055] | [655.360, 663.551] |
| 64 | 50% | Redis | 502676.242 | 127.185 | [119.808, 120.831] | [163.840, 165.887] | [233.472, 235.519] |
| 64 | 100% | CRC | 345535.084 | 185.097 | [172.032, 174.079] | [278.528, 282.623] | [352.256, 356.351] |
| 64 | 100% | rpc-event-one | 300320.897 | 212.978 | [198.656, 200.703] | [323.584, 327.679] | [413.696, 417.791] |
| 64 | 100% | Redis | 510848.551 | 125.172 | [117.760, 118.783] | [163.840, 165.887] | [231.424, 233.471] |

## Mixed operations, separately pooled

| c | Mix | Role | API | QPS | Mean µs | p50 µs | p95 µs | p99 µs |
|---:|---|---|---|---:|---:|---|---|---|
| 1 | 50% | CRC | get | 9509.753 | 47.046 | [46.592, 47.103] | [57.344, 57.855] | [62.976, 63.487] |
| 1 | 50% | CRC | put | 9480.803 | 58.068 | [56.832, 57.343] | [69.632, 70.655] | [76.800, 77.823] |
| 1 | 50% | rpc-event-one | get | 9094.332 | 49.432 | [48.640, 49.151] | [59.904, 60.415] | [65.536, 66.559] |
| 1 | 50% | rpc-event-one | put | 9077.182 | 60.425 | [59.392, 59.903] | [71.680, 72.703] | [79.872, 80.895] |
| 1 | 50% | Redis | get | 85896.684 | 5.720 | [5.632, 5.695] | [5.952, 6.015] | [7.296, 7.359] |
| 1 | 50% | Redis | set | 86066.184 | 5.750 | [5.632, 5.695] | [5.888, 5.951] | [7.360, 7.423] |
| 64 | 50% | CRC | get | 85814.810 | 384.665 | [368.640, 372.735] | [532.480, 540.671] | [622.592, 630.783] |
| 64 | 50% | CRC | put | 85978.806 | 360.155 | [344.064, 348.159] | [499.712, 503.807] | [589.824, 598.015] |
| 64 | 50% | rpc-event-one | get | 81154.211 | 408.502 | [393.216, 397.311] | [565.248, 573.439] | [671.744, 679.935] |
| 64 | 50% | rpc-event-one | put | 81387.704 | 378.746 | [360.448, 364.543] | [524.288, 532.479] | [630.784, 638.975] |
| 64 | 50% | Redis | get | 251240.448 | 127.179 | [119.808, 120.831] | [163.840, 165.887] | [233.472, 235.519] |
| 64 | 50% | Redis | set | 251435.794 | 127.190 | [119.808, 120.831] | [163.840, 165.887] | [233.472, 235.519] |

## Candidate changes versus CRC and Redis

| c | Mix | QPS vs CRC | Mean vs CRC | QPS vs Redis | Mean vs Redis |
|---:|---|---:|---:|---:|---:|
| 1 | point-r050 | -4.313% | +4.519% | -89.433% | +857.717% |
| 1 | point-r100 | -6.325% | +6.777% | -85.593% | +601.466% |
| 64 | point-r050 | -5.385% | +5.694% | -67.665% | +209.473% |
| 64 | point-r100 | -13.085% | +15.063% | -41.211% | +70.148% |

Measured totals: **48,550,288 calls / 48,550,288 issued / 48,550,288 attempts**; success=48,550,288, dropped slots=0. Every outcome population is retained in summary.json. Full phase accounting also retains initialization routing attempts rather than applying measurement-only single-attempt claims to setup.

Per-repeat combined and mixed-operation rows are in PER-REPEAT.md and summary.json. Pooled and per-repeat candidate-vs-CRC/Redis comparisons also preserve both p95/p99 intervals and per-operation deltas. No percentile is averaged.

The accepted audit records 80 exited lifetimes, 48 qualifying drains, 48 voter writer/listener bindings, 4679 resource samples, and 636 retained files / 4,447,964,001 bytes. Exact configured/effective CPU restoration and namespace preservation passed for all three owned containers. These original acceptance checks were not rerun.

The derivative auditor and this summary cover only point read50/read100. Original accepted audit bytes are retained unchanged.

The arithmetic core is copied byte-identically from the accepted v3/de37 statistics preparation; the bounded reader binds all 24 raw reports and their retained resource coverage to the accepted input inventory, and checks each original combined/per-operation histogram against the accepted audit before pooling. No runtime, build, test or profiling campaign is part of this derivation. Native SDK attempt-reason maps cover native calls only; Redis command attempts remain included in aggregate attempt counts.
