# Raft-Append-Payload point-read/mixed screen

24 point-read/mixed c1/c64 diagnostic cohorts; two 10-second forward/reverse repetitions. Not full72, significance, equal durability or sustained capacity.

Both repeats are retained. CRC means the accepted byte-table server; raft-append-payload is the clean raft-append-payload candidate. Redis uses actual GET/SET with the same v3 dataset/configuration. KV9 retains quorum/sync calls on tmpfs WAL; Redis is standalone memory. The workloads do not have equal durability.

All quantiles below are merged raw histogram bucket intervals. Mixed combined means/quantiles weight actual read/write counts; the separate-operation table pools GET and PUT/SET independently. Operation QPS uses complete cohort elapsed time, not only time spent in that operation.

## Pooled over both repetitions

| c | Mix | Role | QPS | Mean µs | p50 µs | p95 µs | p99 µs |
|---:|---|---|---:|---:|---|---|---|
| 1 | 50% | CRC | 18919.954 | 52.745 | [52.224, 52.735] | [66.560, 67.583] | [74.752, 75.775] |
| 1 | 50% | raft-append-payload | 18864.575 | 52.900 | [52.224, 52.735] | [66.560, 67.583] | [74.752, 75.775] |
| 1 | 50% | Redis | 172608.746 | 5.714 | [5.632, 5.695] | [5.888, 5.951] | [7.488, 7.551] |
| 1 | 100% | CRC | 26443.097 | 37.701 | [36.864, 37.375] | [44.032, 44.543] | [50.176, 50.687] |
| 1 | 100% | raft-append-payload | 26491.680 | 37.625 | [36.352, 36.863] | [44.032, 44.543] | [50.176, 50.687] |
| 1 | 100% | Redis | 174085.899 | 5.667 | [5.568, 5.631] | [5.888, 5.951] | [7.232, 7.295] |
| 64 | 50% | CRC | 171765.607 | 372.458 | [356.352, 360.447] | [516.096, 520.191] | [614.400, 622.591] |
| 64 | 50% | raft-append-payload | 171818.179 | 372.346 | [356.352, 360.447] | [516.096, 520.191] | [606.208, 614.399] |
| 64 | 50% | Redis | 502854.446 | 127.137 | [119.808, 120.831] | [163.840, 165.887] | [233.472, 235.519] |
| 64 | 100% | CRC | 343972.123 | 185.934 | [174.080, 176.127] | [278.528, 282.623] | [352.256, 356.351] |
| 64 | 100% | raft-append-payload | 346690.465 | 184.477 | [172.032, 174.079] | [278.528, 282.623] | [352.256, 356.351] |
| 64 | 100% | Redis | 509202.075 | 125.576 | [118.784, 119.807] | [161.792, 163.839] | [231.424, 233.471] |

## Mixed operations, separately pooled

| c | Mix | Role | API | QPS | Mean µs | p50 µs | p95 µs | p99 µs |
|---:|---|---|---|---:|---:|---|---|---|
| 1 | 50% | CRC | get | 9474.727 | 47.205 | [46.592, 47.103] | [57.344, 57.855] | [63.488, 63.999] |
| 1 | 50% | CRC | put | 9445.227 | 58.302 | [57.344, 57.855] | [69.632, 70.655] | [77.824, 78.847] |
| 1 | 50% | raft-append-payload | get | 9446.963 | 47.304 | [46.592, 47.103] | [57.344, 57.855] | [63.488, 63.999] |
| 1 | 50% | raft-append-payload | put | 9417.613 | 58.514 | [57.344, 57.855] | [69.632, 70.655] | [77.824, 78.847] |
| 1 | 50% | Redis | get | 86222.673 | 5.703 | [5.568, 5.631] | [5.888, 5.951] | [7.488, 7.551] |
| 1 | 50% | Redis | set | 86386.073 | 5.725 | [5.632, 5.695] | [5.888, 5.951] | [7.552, 7.615] |
| 64 | 50% | CRC | get | 85799.631 | 384.575 | [368.640, 372.735] | [532.480, 540.671] | [622.592, 630.783] |
| 64 | 50% | CRC | put | 85965.977 | 360.364 | [344.064, 348.159] | [499.712, 503.807] | [589.824, 598.015] |
| 64 | 50% | raft-append-payload | get | 85826.017 | 384.376 | [368.640, 372.735] | [532.480, 540.671] | [622.592, 630.783] |
| 64 | 50% | raft-append-payload | put | 85992.162 | 360.339 | [344.064, 348.159] | [499.712, 503.807] | [589.824, 598.015] |
| 64 | 50% | Redis | get | 251328.301 | 127.140 | [119.808, 120.831] | [163.840, 165.887] | [233.472, 235.519] |
| 64 | 50% | Redis | set | 251526.145 | 127.133 | [119.808, 120.831] | [163.840, 165.887] | [233.472, 235.519] |

## Candidate changes versus CRC and Redis

| c | Mix | QPS vs CRC | Mean vs CRC | QPS vs Redis | Mean vs Redis |
|---:|---|---:|---:|---:|---:|
| 1 | point-r050 | -0.293% | +0.294% | -89.071% | +825.806% |
| 1 | point-r100 | +0.184% | -0.203% | -84.782% | +563.912% |
| 64 | point-r050 | +0.031% | -0.030% | -65.831% | +192.871% |
| 64 | point-r100 | +0.790% | -0.784% | -31.915% | +46.905% |

Measured totals: **49,675,313 calls / 49,675,313 issued / 49,675,313 attempts**; success=49,675,313, dropped slots=0. Every outcome population is retained in summary.json. Full phase accounting also retains initialization routing attempts rather than applying measurement-only single-attempt claims to setup.

Per-repeat combined and mixed-operation rows are in PER-REPEAT.md and summary.json. Pooled and per-repeat candidate-vs-CRC/Redis comparisons also preserve both p95/p99 intervals and per-operation deltas. No percentile is averaged.

The accepted audit records 80 exited lifetimes, 48 qualifying drains, 48 voter writer/listener bindings, 4678 resource samples, and 636 retained files / 4,562,369,306 bytes. Exact configured/effective CPU restoration and namespace preservation passed for all three owned containers. These original acceptance checks were not rerun.

The derivative auditor and this summary cover only point read50/read100. Original accepted audit bytes are retained unchanged.

The arithmetic core is copied byte-identically from the accepted v3/de37 statistics preparation; the bounded reader binds all 24 raw reports and their retained resource coverage to the accepted input inventory, and checks each original combined/per-operation histogram against the accepted audit before pooling. No runtime, build, test or profiling campaign is part of this derivation. Native SDK attempt-reason maps cover native calls only; Redis command attempts remain included in aggregate attempt counts.
