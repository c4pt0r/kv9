# Read-group-credit matched diagnostic readout

Independent recording audit accepted all 24 cohorts. This is not full candidate promotion.

All 28,847,557 issued logical calls succeeded; refusals, unknown writes, read failures, client rejections, non-success reasons and dropped slots are zero. Both repeats show c64 throughput gains with lower mean/p99; c1 point GET throughput is lower and p99 is higher in both repeats.

Each row preserves one original repeat. Old = accepted `5ee897a`; candidate = `57ff6851`. Means and p99 intervals are whole logical calls in microseconds. Percentiles are not averaged.

| Workers | Repeat | API | Old QPS | Candidate QPS | Change | Old mean µs | Candidate mean µs | Old p99 µs | Candidate p99 µs |
| ---: | ---: | --- | ---: | ---: | ---: | ---: | ---: | --- | --- |
| 1 | 0 | point_get | 26,307.099 | 26,285.278 | -0.083% | 37.887936 | 37.925782 | 58.368–58.879 | 58.880–59.391 |
| 1 | 0 | batch_get | 26,098.525 | 26,144.417 | +0.176% | 38.187940 | 38.122582 | 59.904–60.415 | 60.416–60.927 |
| 64 | 0 | point_get | 344,151.651 | 371,789.279 | +8.031% | 185.830342 | 172.009856 | 360.448–364.543 | 303.104–307.199 |
| 64 | 0 | batch_get | 338,390.532 | 367,082.390 | +8.479% | 188.965439 | 174.183908 | 364.544–368.639 | 315.392–319.487 |
| 1 | 1 | point_get | 26,230.593 | 26,080.302 | -0.573% | 37.998212 | 38.217480 | 52.736–53.247 | 55.808–56.319 |
| 1 | 1 | batch_get | 26,220.398 | 26,245.102 | +0.094% | 38.009762 | 37.969843 | 53.248–53.759 | 52.224–52.735 |
| 64 | 1 | point_get | 345,670.967 | 374,086.279 | +8.220% | 185.017513 | 170.955770 | 352.256–356.351 | 299.008–303.103 |
| 64 | 1 | batch_get | 339,745.693 | 366,649.844 | +7.919% | 188.210804 | 174.394002 | 360.448–364.543 | 307.200–311.295 |

Redis references use the fixed `b8ec` client. Native measurement uses fixed `03c1`. GET and MGET(1) remain separate controls.

| Workers | Repeat | Redis API | QPS | Mean µs | p99 µs |
| ---: | ---: | --- | ---: | ---: | --- |
| 1 | 0 | mget | 169,324.329 | 5.824623 | 8.320–8.447 |
| 1 | 0 | get | 171,657.186 | 5.749963 | 8.128–8.191 |
| 64 | 0 | mget | 506,618.724 | 126.205567 | 229.376–231.423 |
| 64 | 0 | get | 509,004.271 | 125.620033 | 229.376–231.423 |
| 64 | 1 | get | 506,965.770 | 126.123910 | 229.376–231.423 |
| 64 | 1 | mget | 505,328.145 | 126.532200 | 231.424–233.471 |
| 1 | 1 | get | 172,487.926 | 5.721928 | 7.744–7.807 |
| 1 | 1 | mget | 170,754.458 | 5.774295 | 7.296–7.359 |

The unchanged auditor verified 80 exited owned lifetimes, 48 qualifying drains, 48 writer/listener bindings, 2,350 resource samples, 2,337 role/source checks, and 528 retained files / 88,755,243 bytes. All three owned container cpusets were exactly restored to configured/effective `0-31`; historical namespace identities were preserved.

Short shared-host diagnostic only: three-voter volatile tmpfs WAL with normal sync calls versus standalone memory Redis. No sustained-capacity, equal-durability or c1 no-regression claim. Exact-source Chaos and broader promotion remain open.

Accepted audit SHA-256: `227e49b64205a4c79982fdee024cd09da0a44b428d660caa45845b32d262340a`. Statistics SHA-256: `cbad8f86590a32ef0f3814e3e758f5fbe5ace1ce03e7e39d9aa5b64af81a547d`.
