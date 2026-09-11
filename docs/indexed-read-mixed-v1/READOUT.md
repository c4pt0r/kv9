# Indexed-receipt point-read/mixed screen

24 point-read/mixed c1/c64 diagnostic cohorts; two 10-second forward/reverse repetitions. Not full72, significance, equal durability or sustained capacity.

Both repeats are retained. CRC means the accepted byte-table server; indexed-receipt is the clean indexed-receipt candidate. Redis uses actual GET/SET with the same v3 dataset/configuration. KV9 retains quorum/sync calls on tmpfs WAL; Redis is standalone memory. The workloads do not have equal durability.

All quantiles below are merged raw histogram bucket intervals. Mixed combined means/quantiles weight actual read/write counts; the separate-operation table pools GET and PUT/SET independently. Operation QPS uses complete cohort elapsed time, not only time spent in that operation.

## Pooled over both repetitions

| c | Mix | Role | QPS | Mean µs | p50 µs | p95 µs | p99 µs |
|---:|---|---|---:|---:|---|---|---|
| 1 | 50% | CRC | 18831.573 | 52.995 | [52.224, 52.735] | [66.560, 67.583] | [74.752, 75.775] |
| 1 | 50% | indexed-receipt | 19208.302 | 51.956 | [51.200, 51.711] | [65.024, 65.535] | [71.680, 72.703] |
| 1 | 50% | Redis | 172850.645 | 5.706 | [5.632, 5.695] | [5.888, 5.951] | [7.296, 7.359] |
| 1 | 100% | CRC | 26408.027 | 37.754 | [36.352, 36.863] | [44.032, 44.543] | [50.688, 51.199] |
| 1 | 100% | indexed-receipt | 26403.726 | 37.762 | [36.864, 37.375] | [44.032, 44.543] | [50.176, 50.687] |
| 1 | 100% | Redis | 174355.745 | 5.658 | [5.568, 5.631] | [5.888, 5.951] | [7.360, 7.423] |
| 64 | 50% | CRC | 172071.183 | 371.799 | [356.352, 360.447] | [516.096, 520.191] | [614.400, 622.591] |
| 64 | 50% | indexed-receipt | 176846.975 | 361.754 | [344.064, 348.159] | [499.712, 503.807] | [589.824, 598.015] |
| 64 | 50% | Redis | 499646.640 | 127.954 | [119.808, 120.831] | [163.840, 165.887] | [233.472, 235.519] |
| 64 | 100% | CRC | 346077.961 | 184.806 | [172.032, 174.079] | [278.528, 282.623] | [352.256, 356.351] |
| 64 | 100% | indexed-receipt | 344656.570 | 185.566 | [172.032, 174.079] | [282.624, 286.719] | [356.352, 360.447] |
| 64 | 100% | Redis | 513878.693 | 124.434 | [117.760, 118.783] | [159.744, 161.791] | [229.376, 231.423] |

## Mixed operations, separately pooled

| c | Mix | Role | API | QPS | Mean µs | p50 µs | p95 µs | p99 µs |
|---:|---|---|---|---:|---:|---|---|---|
| 1 | 50% | CRC | get | 9430.687 | 47.443 | [46.592, 47.103] | [57.856, 58.367] | [63.488, 63.999] |
| 1 | 50% | CRC | put | 9400.887 | 58.565 | [57.344, 57.855] | [69.632, 70.655] | [77.824, 78.847] |
| 1 | 50% | indexed-receipt | get | 9617.476 | 46.857 | [46.080, 46.591] | [56.832, 57.343] | [62.464, 62.975] |
| 1 | 50% | indexed-receipt | put | 9590.826 | 57.070 | [56.320, 56.831] | [67.584, 68.607] | [74.752, 75.775] |
| 1 | 50% | Redis | get | 86343.222 | 5.685 | [5.568, 5.631] | [5.888, 5.951] | [7.232, 7.295] |
| 1 | 50% | Redis | set | 86507.422 | 5.728 | [5.632, 5.695] | [5.888, 5.951] | [7.296, 7.359] |
| 64 | 50% | CRC | get | 85953.494 | 383.691 | [368.640, 372.735] | [532.480, 540.671] | [622.592, 630.783] |
| 64 | 50% | CRC | put | 86117.689 | 359.929 | [344.064, 348.159] | [499.712, 503.807] | [598.016, 606.207] |
| 64 | 50% | indexed-receipt | get | 88325.790 | 372.473 | [356.352, 360.447] | [516.096, 520.191] | [606.208, 614.399] |
| 64 | 50% | indexed-receipt | put | 88521.186 | 351.059 | [335.872, 339.967] | [483.328, 487.423] | [573.440, 581.631] |
| 64 | 50% | Redis | get | 249716.973 | 127.963 | [120.832, 121.855] | [163.840, 165.887] | [233.472, 235.519] |
| 64 | 50% | Redis | set | 249929.668 | 127.944 | [119.808, 120.831] | [163.840, 165.887] | [233.472, 235.519] |

## Candidate changes versus CRC and Redis

| c | Mix | QPS vs CRC | Mean vs CRC | QPS vs Redis | Mean vs Redis |
|---:|---|---:|---:|---:|---:|
| 1 | point-r050 | +2.001% | -1.961% | -88.887% | +810.488% |
| 1 | point-r100 | -0.016% | +0.022% | -84.856% | +567.388% |
| 64 | point-r050 | +2.775% | -2.702% | -64.606% | +182.722% |
| 64 | point-r100 | -0.411% | +0.411% | -32.930% | +49.128% |

Measured totals: **49,825,632 calls / 49,825,632 issued / 49,825,632 attempts**; success=49,825,632, dropped slots=0. Every outcome population is retained in summary.json. Full phase accounting also retains initialization routing attempts rather than applying measurement-only single-attempt claims to setup.

Per-repeat combined and mixed-operation rows are in PER-REPEAT.md and summary.json. Pooled and per-repeat candidate-vs-CRC/Redis comparisons also preserve both p95/p99 intervals and per-operation deltas. No percentile is averaged.

The accepted audit records 80 exited lifetimes, 48 qualifying drains, 48 voter writer/listener bindings, 4677 resource samples, and 640 retained files / 4,629,571,887 bytes. Exact configured/effective CPU restoration and namespace preservation passed for all three owned containers. These original acceptance checks were not rerun.

The derivative auditor and this summary cover only point read50/read100. Original accepted audit bytes are retained unchanged.

The arithmetic core is copied byte-identically from the accepted v3/de37 statistics preparation; the bounded reader binds all 24 raw reports and their retained resource coverage to the accepted input inventory, and checks each original combined/per-operation histogram against the accepted audit before pooling. No runtime, build, test or profiling campaign is part of this derivation. Native SDK attempt-reason maps cover native calls only; Redis command attempts remain included in aggregate attempt counts.
