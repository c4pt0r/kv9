# Predeclared c1 long-window readout

The first unchanged independent audit accepted all 24 cohorts. Four repeats used F/R/R/F order, six unchanged arms and 30-second windows. This is a separate predeclared study; the earlier successful 5-second c1/c64 screen and its regressions remain retained.

All 53,783,436 issued logical calls succeeded in 53,783,436 attempts. Refusals, unknown writes, read failures, client rejections, non-success reasons, connection failures and dropped slots are zero. No source, workload or predicate was changed during this readback.

Old = accepted `5ee897a`; candidate = `57ff6851`. Native measurement client = `03c1`; Redis reference client = `b8ec`. All latency values are whole logical calls in microseconds. p99 values preserve original histogram intervals; percentiles are never averaged.

| Repeat | API | Old calls | Candidate calls | Old QPS | Candidate QPS | QPS change | Old mean µs | Candidate mean µs | Old p99 µs | Candidate p99 µs |
| ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- |
| 0 | point_get | 793,882 | 794,624 | 26,462.704 | 26,487.456 | +0.094% | 37.668170 | 37.623194 | 51.712–52.223 | 52.224–52.735 |
| 0 | batch_get | 788,467 | 788,570 | 26,282.217 | 26,285.631 | +0.013% | 37.919173 | 37.916208 | 52.224–52.735 | 52.224–52.735 |
| 1 | point_get | 791,482 | 791,619 | 26,382.713 | 26,387.281 | +0.017% | 37.781453 | 37.773374 | 52.224–52.735 | 51.712–52.223 |
| 1 | batch_get | 784,705 | 797,266 | 26,156.824 | 26,575.532 | +1.601% | 38.103438 | 37.487607 | 53.248–53.759 | 50.176–50.687 |
| 2 | point_get | 786,141 | 794,898 | 26,204.691 | 26,496.583 | +1.114% | 38.035440 | 37.605633 | 52.736–53.247 | 51.712–52.223 |
| 2 | batch_get | 781,778 | 789,351 | 26,059.263 | 26,311.673 | +0.969% | 38.251243 | 37.879086 | 52.736–53.247 | 52.224–52.735 |
| 3 | point_get | 788,780 | 792,540 | 26,292.640 | 26,417.996 | +0.477% | 37.909484 | 37.720573 | 52.224–52.735 | 53.248–53.759 |
| 3 | batch_get | 783,569 | 785,002 | 26,118.957 | 26,166.714 | +0.183% | 38.140714 | 38.072301 | 53.248–53.759 | 52.736–53.247 |

All Redis controls:

| Repeat | Redis API | Calls | QPS | Mean µs | p99 µs |
| ---: | --- | ---: | ---: | ---: | --- |
| 0 | get | 5,166,471 | 172,215.675 | 5.731020 | 7.360–7.423 |
| 0 | mget | 5,105,768 | 170,192.245 | 5.795692 | 7.616–7.679 |
| 1 | get | 5,175,314 | 172,510.460 | 5.721309 | 7.360–7.423 |
| 1 | mget | 5,106,353 | 170,211.740 | 5.795696 | 7.488–7.551 |
| 2 | get | 5,177,712 | 172,590.393 | 5.718579 | 7.424–7.487 |
| 2 | mget | 5,112,404 | 170,413.445 | 5.788251 | 7.360–7.423 |
| 3 | get | 5,181,587 | 172,719.567 | 5.714098 | 7.360–7.423 |
| 3 | mget | 5,125,153 | 170,838.413 | 5.774017 | 7.424–7.487 |

Pooled QPS = total calls / total observed cohort elapsed. Pooled mean = total whole-call latency / total whole-call count. These are weighted by their denominators; no pooled p99 is inferred from per-repeat quantiles.

| Arm | Total calls | Total cohort seconds | Pooled QPS | Pooled mean µs |
| --- | ---: | ---: | ---: | ---: |
| old-point | 3,160,285 | 120.000097357 | 26,335.687 | 37.848132 |
| old-batch1 | 3,138,519 | 120.000044413 | 26,154.315 | 38.103270 |
| new-point | 3,173,681 | 120.000057591 | 26,447.329 | 37.680573 |
| new-batch1 | 3,160,189 | 120.000096122 | 26,334.887 | 37.837581 |
| redis-mget1 | 20,449,678 | 120.000015902 | 170,413.961 | 5.788401 |
| redis-get1 | 20,701,084 | 120.000006719 | 172,509.024 | 5.721245 |

Pooled point_get candidate change: QPS +0.423919%; mean latency -0.442713%.

Pooled batch_get candidate change: QPS +0.690410%; mean latency -0.697287%.

Resource/identity readback accepted 80 exited owned lifetimes, 48 qualifying drains, 48 writer/listener bindings, 14,096 resource samples and 2,337 role/source checks. All 528 retained files / 88,754,298 bytes matched. All three owned container cpusets were exactly restored to configured/effective `0-31`; historical namespace identities were preserved.

Shared-host, one-client-worker, volatile tmpfs-WAL three-voter diagnostic against standalone memory Redis. No significance test, sustained-capacity claim, equal-durability claim, c1 no-regression declaration or full promotion is inferred. Keep all four pairs and the earlier screen when assessing the candidate.

Accepted audit SHA-256: `420b680b147f3a9a2bc69a7840d5550e20c45d21a848b02b04ce43a35a87f660`. Statistics SHA-256: `fb5c6c45219c8281a32dbc86387a4a35883014bfe07141990391f91048a163ea`.
