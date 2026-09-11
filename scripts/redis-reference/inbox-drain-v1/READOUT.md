# Inbox-drain matched diagnostic readout

The first unchanged independent audit accepted all 48 timed cohorts. Four F/R/R/F passes cover the full c1/c64 six-arm list with 10-second windows. The separate 12-cell correctness smoke is excluded. No repeat is discarded.

Total logical calls: 117,886,444; issued: 117,886,444; attempts: 117,886,444; successes: 117,886,444; dropped slots: 0. Outcome and reason populations are retained in the JSON.
All non-success outcomes, call/attempt reasons and recorded connection failures are zero.

Parent control = `57ff6851`; candidate = `c3131800`; fixed native client = `03c1`; fixed Redis client = `b8ec`. Rates use original counts and elapsed time. Means use whole logical call sums/counts. p99 values are original histogram intervals in microseconds, never averaged.

| Workers | Repeat | API | Parent calls | Candidate calls | Parent QPS | Candidate QPS | QPS change | Parent mean µs | Candidate mean µs | Parent p99 µs | Candidate p99 µs |
| ---: | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- |
| 1 | 0 | point_get | 265,846 | 265,992 | 26,584.572 | 26,599.106 | +0.055% | 37.494463 | 37.471947 | 52.224–52.735 | 51.200–51.711 |
| 1 | 0 | batch_get | 262,676 | 262,733 | 26,267.588 | 26,273.247 | +0.022% | 37.935956 | 37.933388 | 52.736–53.247 | 52.736–53.247 |
| 64 | 0 | point_get | 3,681,877 | 3,734,852 | 368,180.499 | 373,479.882 | +1.439% | 173.699425 | 171.236701 | 303.104–307.199 | 299.008–303.103 |
| 64 | 0 | batch_get | 3,695,047 | 3,692,260 | 369,499.819 | 369,221.216 | -0.075% | 173.049176 | 173.180248 | 303.104–307.199 | 299.008–303.103 |
| 1 | 1 | point_get | 264,582 | 264,994 | 26,458.200 | 26,499.305 | +0.155% | 37.669212 | 37.605720 | 52.224–52.735 | 51.712–52.223 |
| 1 | 1 | batch_get | 261,197 | 262,776 | 26,119.621 | 26,277.572 | +0.605% | 38.157952 | 37.927976 | 52.736–53.247 | 52.224–52.735 |
| 64 | 1 | point_get | 3,719,234 | 3,742,382 | 371,917.147 | 374,233.520 | +0.623% | 171.956483 | 170.890814 | 307.200–311.295 | 294.912–299.007 |
| 64 | 1 | batch_get | 3,695,193 | 3,663,767 | 369,512.735 | 366,372.449 | -0.850% | 173.043540 | 174.526044 | 303.104–307.199 | 307.200–311.295 |
| 1 | 2 | point_get | 263,174 | 264,629 | 26,317.315 | 26,462.878 | +0.553% | 37.873577 | 37.662137 | 52.736–53.247 | 51.712–52.223 |
| 1 | 2 | batch_get | 263,494 | 263,448 | 26,349.333 | 26,344.711 | -0.018% | 37.812673 | 37.812233 | 52.224–52.735 | 52.736–53.247 |
| 64 | 2 | point_get | 3,747,167 | 3,735,746 | 374,712.633 | 373,569.622 | -0.305% | 170.671802 | 171.193421 | 299.008–303.103 | 299.008–303.103 |
| 64 | 2 | batch_get | 3,674,227 | 3,694,440 | 367,417.288 | 369,437.762 | +0.550% | 174.029742 | 173.078315 | 311.296–315.391 | 299.008–303.103 |
| 1 | 3 | point_get | 264,725 | 264,198 | 26,472.417 | 26,419.764 | -0.199% | 37.644600 | 37.719176 | 52.224–52.735 | 52.224–52.735 |
| 1 | 3 | batch_get | 262,811 | 262,211 | 26,281.032 | 26,221.053 | -0.228% | 37.903673 | 37.998913 | 52.736–53.247 | 52.224–52.735 |
| 64 | 3 | point_get | 3,716,332 | 3,714,372 | 371,628.422 | 371,431.613 | -0.053% | 172.089811 | 172.180095 | 299.008–303.103 | 303.104–307.199 |
| 64 | 3 | batch_get | 3,681,864 | 3,696,757 | 368,180.555 | 369,669.211 | +0.404% | 173.669148 | 172.969255 | 307.200–311.295 | 303.104–307.199 |

All sixteen Redis controls:

| Workers | Repeat | Redis API | Calls | QPS | Mean µs | p99 µs |
| ---: | ---: | --- | ---: | ---: | ---: | --- |
| 1 | 0 | get | 1,724,820 | 172,481.962 | 5.722071 | 7.552–7.615 |
| 1 | 0 | mget | 1,709,430 | 170,942.956 | 5.770285 | 7.488–7.551 |
| 1 | 1 | get | 1,725,993 | 172,599.239 | 5.718573 | 7.744–7.807 |
| 1 | 1 | mget | 1,708,930 | 170,892.929 | 5.771146 | 7.232–7.295 |
| 1 | 2 | get | 1,719,718 | 171,971.732 | 5.739399 | 7.744–7.807 |
| 1 | 2 | mget | 1,708,566 | 170,856.547 | 5.773229 | 7.232–7.295 |
| 1 | 3 | get | 1,727,270 | 172,726.947 | 5.713831 | 7.360–7.423 |
| 1 | 3 | mget | 1,712,693 | 171,269.203 | 5.759081 | 7.680–7.743 |
| 64 | 0 | get | 5,073,902 | 507,375.142 | 126.025642 | 231.424–233.471 |
| 64 | 0 | mget | 5,083,967 | 508,382.489 | 125.773964 | 231.424–233.471 |
| 64 | 1 | get | 5,082,253 | 508,213.850 | 125.821026 | 231.424–233.471 |
| 64 | 1 | mget | 5,066,720 | 506,658.467 | 126.200570 | 229.376–231.423 |
| 64 | 2 | get | 5,135,701 | 513,557.510 | 124.510396 | 229.376–231.423 |
| 64 | 2 | mget | 5,062,373 | 506,225.165 | 126.310391 | 229.376–231.423 |
| 64 | 3 | get | 5,059,365 | 505,924.502 | 126.384530 | 231.424–233.471 |
| 64 | 3 | mget | 5,079,740 | 507,959.500 | 125.874624 | 229.376–231.423 |

Pooled values use total calls / total cohort elapsed and total latency / total latency count for each concurrency/API/role. No pooled p99 is inferred from the four per-repeat quantiles.

| Workers | Arm | Total calls | Total cohort seconds | Pooled QPS | Pooled mean µs |
| ---: | --- | ---: | ---: | ---: | ---: |
| 1 | old-point | 1,058,327 | 40.000074182 | 26,458.126 | 37.669979 |
| 1 | old-batch1 | 1,050,178 | 40.000085849 | 26,254.394 | 37.952159 |
| 1 | new-point | 1,059,813 | 40.000093277 | 26,495.263 | 37.614516 |
| 1 | new-batch1 | 1,051,168 | 40.000082559 | 26,279.146 | 37.918016 |
| 1 | redis-mget1 | 6,839,619 | 40.000015500 | 170,990.409 | 5.768430 |
| 1 | redis-get1 | 6,897,801 | 40.000012798 | 172,444.970 | 5.723453 |
| 64 | old-point | 14,864,610 | 40.000600813 | 371,609.668 | 172.097683 |
| 64 | old-batch1 | 14,746,331 | 40.000615822 | 368,652.599 | 173.446878 |
| 64 | new-point | 14,927,352 | 40.000551097 | 373,178.659 | 171.373898 |
| 64 | new-batch1 | 14,747,224 | 40.000589975 | 368,675.162 | 173.436168 |
| 64 | redis-mget1 | 20,292,800 | 40.001071805 | 507,306.407 | 126.039497 |
| 64 | redis-get1 | 20,351,221 | 40.001004401 | 508,767.750 | 125.681387 |

Directional observations retain all repeats:

- c1 point_get: per-repeat QPS changes [+0.055%, +0.155%, +0.553%, -0.199%]; p99 directions [lower, lower, lower, same_or_overlapping]. Pooled QPS change +0.140362%; pooled mean change -0.147235%.

- c1 batch_get: per-repeat QPS changes [+0.022%, +0.605%, -0.018%, -0.228%]; p99 directions [same_or_overlapping, lower, higher, lower]. Pooled QPS change +0.094278%; pooled mean change -0.089963%.

- c64 point_get: per-repeat QPS changes [+1.439%, +0.623%, -0.305%, -0.053%]; p99 directions [lower, lower, same_or_overlapping, higher]. Pooled QPS change +0.422215%; pooled mean change -0.420566%.

- c64 batch_get: per-repeat QPS changes [-0.075%, -0.850%, +0.550%, +0.404%]; p99 directions [lower, higher, lower, lower]. Pooled QPS change +0.006120%; pooled mean change -0.006175%.

Identity/resource/retention checks accepted 160 exited owned lifetimes, 96 qualifying drains, 96 writer/listener bindings, 9,392 resource samples, 2,348 role/source checks, and 1,056 retained files / 177,511,111 bytes. All three owned container cpusets were exactly restored to configured/effective `0-31`; historical namespace identities were preserved.

This remains a shared-host volatile tmpfs-WAL three-voter diagnostic against standalone memory Redis. No significance, no-regression, equal-durability, sustained-capacity or full promotion claim follows. Mixed directional results remain visible in the complete tables.

Accepted audit SHA-256: `448528041ee2ccc42476b5faefa8279d42babc362b14c9a89441371dc839eafc`. Statistics SHA-256: `fa86ce3cda76fe858e6717a52fba632ac0fc7ccdd4a26faff519fc1709b861cd`.
