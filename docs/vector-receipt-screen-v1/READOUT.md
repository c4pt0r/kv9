# Vector-receipt point-read/mixed screen

24 point-read/mixed c1/c64 diagnostic cohorts; two 10-second forward/reverse repetitions. Not full72, significance, equal durability or sustained capacity.

Both repeats are retained. CRC means the accepted byte-table server; vector-receipt is the clean vector-receipt candidate. Redis uses actual GET/SET with the same v3 dataset/configuration. KV9 retains quorum/sync calls on tmpfs WAL; Redis is standalone memory. The workloads do not have equal durability.

All quantiles below are merged raw histogram bucket intervals. Mixed combined means/quantiles weight actual read/write counts; the separate-operation table pools GET and PUT/SET independently. Operation QPS uses complete cohort elapsed time, not only time spent in that operation.

## Pooled over both repetitions

| c | Mix | Role | QPS | Mean µs | p50 µs | p95 µs | p99 µs |
|---:|---|---|---:|---:|---|---|---|
| 1 | 50% | CRC | 19050.290 | 52.384 | [51.712, 52.223] | [65.536, 66.559] | [73.728, 74.751] |
| 1 | 50% | vector-receipt | 18896.133 | 52.813 | [52.224, 52.735] | [66.560, 67.583] | [73.728, 74.751] |
| 1 | 50% | Redis | 173217.333 | 5.695 | [5.568, 5.631] | [5.888, 5.951] | [7.424, 7.487] |
| 1 | 100% | CRC | 26536.898 | 37.571 | [36.352, 36.863] | [44.032, 44.543] | [50.688, 51.199] |
| 1 | 100% | vector-receipt | 26478.504 | 37.650 | [36.352, 36.863] | [43.520, 44.031] | [49.664, 50.175] |
| 1 | 100% | Redis | 173698.971 | 5.680 | [5.568, 5.631] | [5.888, 5.951] | [7.296, 7.359] |
| 64 | 50% | CRC | 171677.317 | 372.651 | [356.352, 360.447] | [516.096, 520.191] | [614.400, 622.591] |
| 64 | 50% | vector-receipt | 176121.320 | 363.248 | [348.160, 352.255] | [503.808, 507.903] | [589.824, 598.015] |
| 64 | 50% | Redis | 502276.056 | 127.286 | [119.808, 120.831] | [163.840, 165.887] | [233.472, 235.519] |
| 64 | 100% | CRC | 345613.340 | 185.051 | [172.032, 174.079] | [278.528, 282.623] | [348.160, 352.255] |
| 64 | 100% | vector-receipt | 344406.245 | 185.702 | [172.032, 174.079] | [278.528, 282.623] | [352.256, 356.351] |
| 64 | 100% | Redis | 508211.693 | 125.820 | [118.784, 119.807] | [161.792, 163.839] | [231.424, 233.471] |

## Mixed operations, separately pooled

| c | Mix | Role | API | QPS | Mean µs | p50 µs | p95 µs | p99 µs |
|---:|---|---|---|---:|---:|---|---|---|
| 1 | 50% | CRC | get | 9539.270 | 46.947 | [46.080, 46.591] | [56.832, 57.343] | [62.976, 63.487] |
| 1 | 50% | CRC | put | 9511.020 | 57.838 | [56.832, 57.343] | [68.608, 69.631] | [76.800, 77.823] |
| 1 | 50% | vector-receipt | get | 9462.641 | 47.324 | [46.592, 47.103] | [57.856, 58.367] | [63.488, 63.999] |
| 1 | 50% | vector-receipt | put | 9433.491 | 58.319 | [57.344, 57.855] | [69.632, 70.655] | [76.800, 77.823] |
| 1 | 50% | Redis | get | 86522.866 | 5.677 | [5.568, 5.631] | [5.888, 5.951] | [7.424, 7.487] |
| 1 | 50% | Redis | set | 86694.466 | 5.713 | [5.632, 5.695] | [5.888, 5.951] | [7.424, 7.487] |
| 64 | 50% | CRC | get | 85754.660 | 384.797 | [368.640, 372.735] | [532.480, 540.671] | [622.592, 630.783] |
| 64 | 50% | CRC | put | 85922.656 | 360.528 | [344.064, 348.159] | [499.712, 503.807] | [589.824, 598.015] |
| 64 | 50% | vector-receipt | get | 87964.593 | 374.367 | [360.448, 364.543] | [516.096, 520.191] | [606.208, 614.399] |
| 64 | 50% | vector-receipt | put | 88156.727 | 352.153 | [335.872, 339.967] | [483.328, 487.423] | [573.440, 581.631] |
| 64 | 50% | Redis | get | 251037.356 | 127.309 | [119.808, 120.831] | [163.840, 165.887] | [233.472, 235.519] |
| 64 | 50% | Redis | set | 251238.701 | 127.263 | [119.808, 120.831] | [163.840, 165.887] | [233.472, 235.519] |

## Candidate changes versus CRC and Redis

| c | Mix | QPS vs CRC | Mean vs CRC | QPS vs Redis | Mean vs Redis |
|---:|---|---:|---:|---:|---:|
| 1 | point-r050 | -0.809% | +0.818% | -89.091% | +827.408% |
| 1 | point-r100 | -0.220% | +0.212% | -84.756% | +562.814% |
| 64 | point-r050 | +2.589% | -2.523% | -64.935% | +185.379% |
| 64 | point-r100 | -0.349% | +0.352% | -32.232% | +47.593% |

Measured totals: **49,724,855 calls / 49,724,855 issued / 49,724,855 attempts**; success=49,724,855, dropped slots=0. Every outcome population is retained in summary.json. Full phase accounting also retains initialization routing attempts rather than applying measurement-only single-attempt claims to setup.

Per-repeat combined and mixed-operation rows are in PER-REPEAT.md and summary.json. Pooled and per-repeat candidate-vs-CRC/Redis comparisons also preserve both p95/p99 intervals and per-operation deltas. No percentile is averaged.

The accepted audit records 80 exited lifetimes, 48 qualifying drains, 48 voter writer/listener bindings, 4677 resource samples, and 638 retained files / 4,614,768,209 bytes. Exact configured/effective CPU restoration and namespace preservation passed for all three owned containers. These original acceptance checks were not rerun.

The derivative auditor and this summary cover only point read50/read100. Original accepted audit bytes are retained unchanged.

The arithmetic core is copied byte-identically from the accepted v3/de37 statistics preparation; the bounded reader binds all 24 raw reports and their retained resource coverage to the accepted input inventory, and checks each original combined/per-operation histogram against the accepted audit before pooling. No runtime, build, test or profiling campaign is part of this derivation. Native SDK attempt-reason maps cover native calls only; Redis command attempts remain included in aggregate attempt counts.
