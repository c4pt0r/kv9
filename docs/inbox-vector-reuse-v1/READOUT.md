# Inbox vector reuse point-read/mixed screen

24 point-read/mixed c1/c64 diagnostic cohorts; two 10-second forward/reverse repetitions. Not full72, significance, equal durability or sustained capacity.

Both repeats are retained. CRC means the accepted byte-table server; inbox-vector-reuse is the clean inbox-vector-reuse candidate. Redis uses actual GET/SET with the same v3 dataset/configuration. KV9 retains quorum/sync calls on tmpfs WAL; Redis is standalone memory. The workloads do not have equal durability.

All quantiles below are merged raw histogram bucket intervals. Mixed combined means/quantiles weight actual read/write counts; the separate-operation table pools GET and PUT/SET independently. Operation QPS uses complete cohort elapsed time, not only time spent in that operation.

## Pooled over both repetitions

| c | Mix | Role | QPS | Mean µs | p50 µs | p95 µs | p99 µs |
|---:|---|---|---:|---:|---|---|---|
| 1 | 50% | CRC | 18910.606 | 52.775 | [52.224, 52.735] | [66.560, 67.583] | [73.728, 74.751] |
| 1 | 50% | inbox-vector-reuse | 18936.496 | 52.699 | [52.224, 52.735] | [66.560, 67.583] | [73.728, 74.751] |
| 1 | 50% | Redis | 172401.423 | 5.722 | [5.632, 5.695] | [5.888, 5.951] | [7.360, 7.423] |
| 1 | 100% | CRC | 26724.653 | 37.305 | [36.352, 36.863] | [43.520, 44.031] | [49.664, 50.175] |
| 1 | 100% | inbox-vector-reuse | 26592.545 | 37.489 | [36.352, 36.863] | [43.008, 43.519] | [49.152, 49.663] |
| 1 | 100% | Redis | 174388.117 | 5.659 | [5.568, 5.631] | [5.888, 5.951] | [7.360, 7.423] |
| 64 | 50% | CRC | 171845.445 | 372.287 | [356.352, 360.447] | [516.096, 520.191] | [614.400, 622.591] |
| 64 | 50% | inbox-vector-reuse | 172567.435 | 370.729 | [356.352, 360.447] | [516.096, 520.191] | [606.208, 614.399] |
| 64 | 50% | Redis | 505972.388 | 126.355 | [119.808, 120.831] | [163.840, 165.887] | [231.424, 233.471] |
| 64 | 100% | CRC | 345439.371 | 185.144 | [172.032, 174.079] | [278.528, 282.623] | [352.256, 356.351] |
| 64 | 100% | inbox-vector-reuse | 345339.377 | 185.202 | [172.032, 174.079] | [282.624, 286.719] | [356.352, 360.447] |
| 64 | 100% | Redis | 513869.587 | 124.437 | [117.760, 118.783] | [161.792, 163.839] | [229.376, 231.423] |

## Mixed operations, separately pooled

| c | Mix | Role | API | QPS | Mean µs | p50 µs | p95 µs | p99 µs |
|---:|---|---|---|---:|---:|---|---|---|
| 1 | 50% | CRC | get | 9470.128 | 47.228 | [46.592, 47.103] | [57.344, 57.855] | [62.976, 63.487] |
| 1 | 50% | CRC | put | 9440.478 | 58.340 | [57.344, 57.855] | [69.632, 70.655] | [77.824, 78.847] |
| 1 | 50% | inbox-vector-reuse | get | 9482.873 | 47.171 | [46.592, 47.103] | [57.344, 57.855] | [62.976, 63.487] |
| 1 | 50% | inbox-vector-reuse | put | 9453.623 | 58.243 | [57.344, 57.855] | [69.632, 70.655] | [77.824, 78.847] |
| 1 | 50% | Redis | get | 86120.136 | 5.709 | [5.632, 5.695] | [5.888, 5.951] | [7.424, 7.487] |
| 1 | 50% | Redis | set | 86281.286 | 5.735 | [5.632, 5.695] | [5.888, 5.951] | [7.360, 7.423] |
| 64 | 50% | CRC | get | 85838.550 | 384.352 | [368.640, 372.735] | [532.480, 540.671] | [622.592, 630.783] |
| 64 | 50% | CRC | put | 86006.895 | 360.245 | [344.064, 348.159] | [499.712, 503.807] | [589.824, 598.015] |
| 64 | 50% | inbox-vector-reuse | get | 86200.970 | 382.613 | [364.544, 368.639] | [524.288, 532.479] | [622.592, 630.783] |
| 64 | 50% | inbox-vector-reuse | put | 86366.465 | 358.867 | [344.064, 348.159] | [495.616, 499.711] | [589.824, 598.015] |
| 64 | 50% | Redis | get | 252891.771 | 126.373 | [119.808, 120.831] | [163.840, 165.887] | [231.424, 233.471] |
| 64 | 50% | Redis | set | 253080.617 | 126.338 | [119.808, 120.831] | [161.792, 163.839] | [231.424, 233.471] |

## Candidate changes versus CRC and Redis

| c | Mix | QPS vs CRC | Mean vs CRC | QPS vs Redis | Mean vs Redis |
|---:|---|---:|---:|---:|---:|
| 1 | point-r050 | +0.137% | -0.145% | -89.016% | +821.030% |
| 1 | point-r100 | -0.494% | +0.491% | -84.751% | +562.464% |
| 64 | point-r050 | +0.420% | -0.418% | -65.894% | +193.402% |
| 64 | point-r100 | -0.029% | +0.031% | -32.796% | +48.832% |

Measured totals: **49,860,618 calls / 49,860,618 issued / 49,860,618 attempts**; success=49,860,618, dropped slots=0. Every outcome population is retained in summary.json. Full phase accounting also retains initialization routing attempts rather than applying measurement-only single-attempt claims to setup.

Per-repeat combined and mixed-operation rows are in PER-REPEAT.md and summary.json. Pooled and per-repeat candidate-vs-CRC/Redis comparisons also preserve both p95/p99 intervals and per-operation deltas. No percentile is averaged.

The accepted audit records 80 exited lifetimes, 48 qualifying drains, 48 voter writer/listener bindings, 4675 resource samples, and 636 retained files / 4,573,041,910 bytes. Exact configured/effective CPU restoration and namespace preservation passed for all three owned containers. These original acceptance checks were not rerun.

The derivative auditor and this summary cover only point read50/read100. Original accepted audit bytes are retained unchanged.

The arithmetic core is copied byte-identically from the accepted v3/de37 statistics preparation; the bounded reader binds all 24 raw reports and their retained resource coverage to the accepted input inventory, and checks each original combined/per-operation histogram against the accepted audit before pooling. No runtime, build, test or profiling campaign is part of this derivation. Native SDK attempt-reason maps cover native calls only; Redis command attempts remain included in aggregate attempt counts.
