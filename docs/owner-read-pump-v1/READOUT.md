# Owner-read-pump point-read/mixed screen

24 point-read/mixed c1/c64 diagnostic cohorts; two 10-second forward/reverse repetitions. Not full72, significance, equal durability or sustained capacity.

Both repeats are retained. CRC means the accepted byte-table server; owner-pump is the clean owner-read-pump candidate. Redis uses actual GET/SET with the same v3 dataset/configuration. KV9 retains quorum/sync calls on tmpfs WAL; Redis is standalone memory. The workloads do not have equal durability.

All quantiles below are merged raw histogram bucket intervals. Mixed combined means/quantiles weight actual read/write counts; the separate-operation table pools GET and PUT/SET independently. Operation QPS uses complete cohort elapsed time, not only time spent in that operation.

## Pooled over both repetitions

| c | Mix | Role | QPS | Mean µs | p50 µs | p95 µs | p99 µs |
|---:|---|---|---:|---:|---|---|---|
| 1 | 50% | CRC | 18936.173 | 52.702 | [52.224, 52.735] | [66.560, 67.583] | [74.752, 75.775] |
| 1 | 50% | owner-pump | 18905.871 | 52.786 | [52.224, 52.735] | [66.560, 67.583] | [74.752, 75.775] |
| 1 | 50% | Redis | 173245.772 | 5.694 | [5.568, 5.631] | [5.888, 5.951] | [7.552, 7.615] |
| 1 | 100% | CRC | 26518.453 | 37.590 | [36.352, 36.863] | [44.032, 44.543] | [50.176, 50.687] |
| 1 | 100% | owner-pump | 26640.845 | 37.425 | [36.352, 36.863] | [43.520, 44.031] | [49.664, 50.175] |
| 1 | 100% | Redis | 174419.231 | 5.656 | [5.568, 5.631] | [5.888, 5.951] | [7.232, 7.295] |
| 64 | 50% | CRC | 172056.321 | 371.831 | [356.352, 360.447] | [516.096, 520.191] | [606.208, 614.399] |
| 64 | 50% | owner-pump | 171638.135 | 372.736 | [356.352, 360.447] | [516.096, 520.191] | [614.400, 622.591] |
| 64 | 50% | Redis | 503257.463 | 127.038 | [119.808, 120.831] | [163.840, 165.887] | [233.472, 235.519] |
| 64 | 100% | CRC | 345145.331 | 185.303 | [172.032, 174.079] | [278.528, 282.623] | [352.256, 356.351] |
| 64 | 100% | owner-pump | 348189.728 | 183.681 | [172.032, 174.079] | [278.528, 282.623] | [352.256, 356.351] |
| 64 | 100% | Redis | 514041.171 | 124.394 | [117.760, 118.783] | [159.744, 161.791] | [229.376, 231.423] |

## Mixed operations, separately pooled

| c | Mix | Role | API | QPS | Mean µs | p50 µs | p95 µs | p99 µs |
|---:|---|---|---|---:|---:|---|---|---|
| 1 | 50% | CRC | get | 9483.111 | 47.156 | [46.592, 47.103] | [57.344, 57.855] | [63.488, 63.999] |
| 1 | 50% | CRC | put | 9453.062 | 58.266 | [57.344, 57.855] | [69.632, 70.655] | [77.824, 78.847] |
| 1 | 50% | owner-pump | get | 9468.010 | 47.181 | [46.592, 47.103] | [57.344, 57.855] | [62.976, 63.487] |
| 1 | 50% | owner-pump | put | 9437.861 | 58.408 | [57.344, 57.855] | [69.632, 70.655] | [77.824, 78.847] |
| 1 | 50% | Redis | get | 86533.736 | 5.676 | [5.568, 5.631] | [5.888, 5.951] | [7.488, 7.551] |
| 1 | 50% | Redis | set | 86712.036 | 5.711 | [5.632, 5.695] | [5.888, 5.951] | [7.552, 7.615] |
| 64 | 50% | CRC | get | 85945.463 | 384.041 | [368.640, 372.735] | [524.288, 532.479] | [622.592, 630.783] |
| 64 | 50% | CRC | put | 86110.858 | 359.644 | [344.064, 348.159] | [499.712, 503.807] | [589.824, 598.015] |
| 64 | 50% | owner-pump | get | 85735.669 | 384.781 | [368.640, 372.735] | [532.480, 540.671] | [622.592, 630.783] |
| 64 | 50% | owner-pump | put | 85902.466 | 360.715 | [344.064, 348.159] | [499.712, 503.807] | [589.824, 598.015] |
| 64 | 50% | Redis | get | 251532.958 | 127.061 | [119.808, 120.831] | [163.840, 165.887] | [233.472, 235.519] |
| 64 | 50% | Redis | set | 251724.505 | 127.015 | [119.808, 120.831] | [163.840, 165.887] | [233.472, 235.519] |

## Candidate changes versus CRC and Redis

| c | Mix | QPS vs CRC | Mean vs CRC | QPS vs Redis | Mean vs Redis |
|---:|---|---:|---:|---:|---:|
| 1 | point-r050 | -0.160% | +0.158% | -89.087% | +827.077% |
| 1 | point-r100 | +0.462% | -0.437% | -84.726% | +561.644% |
| 64 | point-r050 | -0.243% | +0.243% | -65.895% | +193.406% |
| 64 | point-r100 | +0.882% | -0.875% | -32.264% | +47.661% |

Measured totals: **49,860,741 calls / 49,860,741 issued / 49,860,741 attempts**; success=49,860,741, dropped slots=0. Every outcome population is retained in summary.json. Full phase accounting also retains initialization routing attempts rather than applying measurement-only single-attempt claims to setup.

Per-repeat combined and mixed-operation rows are in PER-REPEAT.md and summary.json. Pooled and per-repeat candidate-vs-CRC/Redis comparisons also preserve both p95/p99 intervals and per-operation deltas. No percentile is averaged.

The accepted audit records 80 exited lifetimes, 48 qualifying drains, 48 voter writer/listener bindings, 4678 resource samples, and 636 retained files / 4,564,431,479 bytes. Exact configured/effective CPU restoration and namespace preservation passed for all three owned containers. These original acceptance checks were not rerun.

The derivative auditor and this summary cover only point read50/read100. Original accepted audit bytes are retained unchanged.

The arithmetic core is copied byte-identically from the accepted v3/de37 statistics preparation; the bounded reader binds all 24 raw reports and their retained resource coverage to the accepted input inventory, and checks each original combined/per-operation histogram against the accepted audit before pooling. No runtime, build, test or profiling campaign is part of this derivation. Native SDK attempt-reason maps cover native calls only; Redis command attempts remain included in aggregate attempt counts.
