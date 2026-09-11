# CRC write screen: first accepted recording

Two 10-second forward/reverse c64 write repeats on a shared host; three-voter KV9 quorum/sync with tmpfs WAL versus standalone Redis with persistence and pipelining disabled. No equal-durability, sustained-capacity, significance or new Chaos claim.

| Workload | Role | Calls/s | Keys/s | Mean us | p95 us | p99 us | Client cores | Server cores |
| --- | --- | ---: | ---: | ---: | --- | --- | ---: | ---: |
| point-r000 | old | 118888.863 | 118888.863 | 538.189 | [737.280, 745.471] | [860.160, 868.351] | 0.371 | 3.538 |
| point-r000 | new | 124210.707 | 124210.707 | 515.117 | [704.512, 712.703] | [827.392, 835.583] | 0.389 | 3.557 |
| point-r000 | redis | 493585.759 | 493585.759 | 129.511 | [163.840, 165.887] | [235.520, 237.567] | 1.439 | 0.996 |
| batch64-r000 | old | 9795.438 | 626908.052 | 6531.843 | [9043.968, 9175.039] | [11665.408, 11796.479] | 0.304 | 3.321 |
| batch64-r000 | new | 13565.331 | 868181.199 | 4716.673 | [6750.208, 6815.743] | [8912.896, 9043.967] | 0.398 | 3.357 |
| batch64-r000 | redis | 94638.541 | 6056866.634 | 671.487 | [958.464, 966.655] | [1007.616, 1015.807] | 1.922 | 0.905 |

Quantiles are bucket intervals; batch latency covers the whole call.

| Workload | Repeat | QPS change | Mean change | Old p99 ns | New p99 ns |
| --- | ---: | ---: | ---: | --- | --- |
| point-r000 | 0 | +4.853% | -4.630% | {'lower_ns': 851968, 'upper_ns': 860159} | {'lower_ns': 819200, 'upper_ns': 827391} |
| point-r000 | 1 | +4.099% | -3.941% | {'lower_ns': 860160, 'upper_ns': 868351} | {'lower_ns': 835584, 'upper_ns': 843775} |
| batch64-r000 | 0 | +40.267% | -28.710% | {'lower_ns': 12058624, 'upper_ns': 12189695} | {'lower_ns': 8323072, 'upper_ns': 8388607} |
| batch64-r000 | 1 | +36.721% | -26.854% | {'lower_ns': 11272192, 'upper_ns': 11403263} | {'lower_ns': 9830400, 'upper_ns': 9961471} |

All measured calls: 17,094,412; input keys: 165,788,461; outcomes: {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 17094412, "unknown_write": 0}.

All phases, original per-operation populations and CPU sample scopes remain in statistics-first.json. Promotion requires further workload and recovery acceptance.
