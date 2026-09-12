# Coalesced notification point-read/mixed screen

24 point-read/mixed c1/c64 diagnostic cohorts; two 10-second forward/reverse repetitions. Not full72, significance, equal durability or sustained capacity.

Both repeats are retained. CRC means the accepted byte-table server; coalesced-notification is the clean coalesced-notification candidate. Redis uses actual GET/SET with the same v3 dataset/configuration. KV9 retains quorum/sync calls on tmpfs WAL; Redis is standalone memory. The workloads do not have equal durability.

All quantiles below are merged raw histogram bucket intervals. Mixed combined means/quantiles weight actual read/write counts; the separate-operation table pools GET and PUT/SET independently. Operation QPS uses complete cohort elapsed time, not only time spent in that operation.

## Pooled over both repetitions

| c | Mix | Role | QPS | Mean µs | p50 µs | p95 µs | p99 µs |
|---:|---|---|---:|---:|---|---|---|
| 1 | 50% | CRC | 18915.512 | 52.759 | [52.224, 52.735] | [66.560, 67.583] | [74.752, 75.775] |
| 1 | 50% | coalesced-notification | 18971.023 | 52.604 | [52.224, 52.735] | [65.536, 66.559] | [73.728, 74.751] |
| 1 | 50% | Redis | 172064.978 | 5.732 | [5.632, 5.695] | [5.952, 6.015] | [7.360, 7.423] |
| 1 | 100% | CRC | 26461.551 | 37.674 | [36.352, 36.863] | [44.032, 44.543] | [50.176, 50.687] |
| 1 | 100% | coalesced-notification | 26395.976 | 37.772 | [36.864, 37.375] | [43.520, 44.031] | [50.176, 50.687] |
| 1 | 100% | Redis | 174159.224 | 5.665 | [5.568, 5.631] | [5.888, 5.951] | [7.360, 7.423] |
| 64 | 50% | CRC | 171594.529 | 372.830 | [356.352, 360.447] | [516.096, 520.191] | [614.400, 622.591] |
| 64 | 50% | coalesced-notification | 176278.501 | 362.919 | [348.160, 352.255] | [503.808, 507.903] | [598.016, 606.207] |
| 64 | 50% | Redis | 503716.521 | 126.921 | [119.808, 120.831] | [161.792, 163.839] | [233.472, 235.519] |
| 64 | 100% | CRC | 345626.302 | 185.046 | [172.032, 174.079] | [282.624, 286.719] | [352.256, 356.351] |
| 64 | 100% | coalesced-notification | 349507.003 | 182.990 | [172.032, 174.079] | [266.240, 270.335] | [331.776, 335.871] |
| 64 | 100% | Redis | 513504.422 | 124.523 | [117.760, 118.783] | [159.744, 161.791] | [229.376, 231.423] |

## Mixed operations, separately pooled

| c | Mix | Role | API | QPS | Mean µs | p50 µs | p95 µs | p99 µs |
|---:|---|---|---|---:|---:|---|---|---|
| 1 | 50% | CRC | get | 9472.531 | 47.197 | [46.592, 47.103] | [57.344, 57.855] | [62.976, 63.487] |
| 1 | 50% | CRC | put | 9442.981 | 58.339 | [57.344, 57.855] | [69.632, 70.655] | [77.824, 78.847] |
| 1 | 50% | coalesced-notification | get | 9500.087 | 47.118 | [46.592, 47.103] | [57.344, 57.855] | [62.976, 63.487] |
| 1 | 50% | coalesced-notification | put | 9470.937 | 58.107 | [56.832, 57.343] | [68.608, 69.631] | [77.824, 78.847] |
| 1 | 50% | Redis | get | 85949.889 | 5.718 | [5.632, 5.695] | [5.952, 6.015] | [7.360, 7.423] |
| 1 | 50% | Redis | set | 86115.089 | 5.747 | [5.632, 5.695] | [5.888, 5.951] | [7.296, 7.359] |
| 64 | 50% | CRC | get | 85713.193 | 384.744 | [368.640, 372.735] | [532.480, 540.671] | [622.592, 630.783] |
| 64 | 50% | CRC | put | 85881.336 | 360.940 | [344.064, 348.159] | [499.712, 503.807] | [589.824, 598.015] |
| 64 | 50% | coalesced-notification | get | 88043.578 | 375.026 | [360.448, 364.543] | [516.096, 520.191] | [606.208, 614.399] |
| 64 | 50% | coalesced-notification | put | 88234.923 | 350.838 | [335.872, 339.967] | [483.328, 487.423] | [581.632, 589.823] |
| 64 | 50% | Redis | get | 251764.463 | 126.920 | [119.808, 120.831] | [161.792, 163.839] | [233.472, 235.519] |
| 64 | 50% | Redis | set | 251952.058 | 126.921 | [119.808, 120.831] | [161.792, 163.839] | [233.472, 235.519] |

## Candidate changes versus CRC and Redis

| c | Mix | QPS vs CRC | Mean vs CRC | QPS vs Redis | Mean vs Redis |
|---:|---|---:|---:|---:|---:|
| 1 | point-r050 | +0.293% | -0.293% | -88.975% | +817.664% |
| 1 | point-r100 | -0.248% | +0.260% | -84.844% | +566.781% |
| 64 | point-r050 | +2.730% | -2.658% | -65.004% | +185.941% |
| 64 | point-r100 | +1.123% | -1.111% | -31.937% | +46.953% |

Measured totals: **49,944,895 calls / 49,944,895 issued / 49,944,895 attempts**; success=49,944,895, dropped slots=0. Every outcome population is retained in summary.json. Full phase accounting also retains initialization routing attempts rather than applying measurement-only single-attempt claims to setup.

Per-repeat combined and mixed-operation rows are in PER-REPEAT.md and summary.json. Pooled and per-repeat candidate-vs-CRC/Redis comparisons also preserve both p95/p99 intervals and per-operation deltas. No percentile is averaged.

The accepted audit records 80 exited lifetimes, 48 qualifying drains, 48 voter writer/listener bindings, 4679 resource samples, and 638 retained files / 4,612,673,388 bytes. Exact configured/effective CPU restoration and namespace preservation passed for all three owned containers. These original acceptance checks were not rerun.

The derivative auditor and this summary cover only point read50/read100. Original accepted audit bytes are retained unchanged.

The arithmetic core is copied byte-identically from the accepted v3/de37 statistics preparation; the bounded reader binds all 24 raw reports and their retained resource coverage to the accepted input inventory, and checks each original combined/per-operation histogram against the accepted audit before pooling. No runtime, build, test or profiling campaign is part of this derivation. Native SDK attempt-reason maps cover native calls only; Redis command attempts remain included in aggregate attempt counts.
