# Indexed-receipt write-only screen: first accepted recording

Two 10-second forward/reverse c1/c64 point PUT/BatchPut(64) write-only selection repeats on a shared host; three-voter KV9 quorum/sync with tmpfs WAL versus standalone Redis with persistence and pipelining disabled. Not full-workload acceptance. No equal-durability, sustained-capacity, significance or new Chaos claim.

| Concurrency | Workload | Role | Calls/s | Keys/s | Mean us | p95 us | p99 us | Client cores | Server cores |
| ---: | --- | --- | ---: | ---: | ---: | --- | --- | ---: | ---: |
| 1 | point-r000 | old | 16574.303 | 16574.303 | 60.234 | [71.680, 72.703] | [87.040, 88.063] | 0.131 | 2.537 |
| 1 | point-r000 | new | 16779.199 | 16779.199 | 59.496 | [70.656, 71.679] | [86.016, 87.039] | 0.134 | 2.523 |
| 1 | point-r000 | redis | 170402.377 | 170402.377 | 5.749 | [6.016, 6.079] | [8.064, 8.127] | 0.602 | 0.535 |
| 1 | batch64-r000 | old | 5391.824 | 345076.740 | 185.359 | [210.944, 212.991] | [262.144, 266.239] | 0.134 | 1.908 |
| 1 | batch64-r000 | new | 5437.081 | 347973.198 | 183.814 | [206.848, 208.895] | [262.144, 266.239] | 0.134 | 1.912 |
| 1 | batch64-r000 | redis | 42194.241 | 2700431.399 | 22.562 | [23.552, 23.807] | [32.768, 33.279] | 0.613 | 0.420 |
| 64 | point-r000 | old | 123845.621 | 123845.621 | 516.636 | [704.512, 712.703] | [835.584, 843.775] | 0.392 | 3.555 |
| 64 | point-r000 | new | 128566.031 | 128566.031 | 497.664 | [679.936, 688.127] | [802.816, 811.007] | 0.398 | 3.526 |
| 64 | point-r000 | redis | 494304.200 | 494304.200 | 129.324 | [165.888, 167.935] | [233.472, 235.519] | 1.445 | 0.995 |
| 64 | batch64-r000 | old | 13495.387 | 863704.745 | 4741.068 | [6815.744, 6881.279] | [8912.896, 9043.967] | 0.408 | 3.353 |
| 64 | batch64-r000 | new | 13351.742 | 854511.497 | 4792.237 | [7077.888, 7143.423] | [8912.896, 9043.967] | 0.411 | 3.345 |
| 64 | batch64-r000 | redis | 94548.593 | 6051109.941 | 672.101 | [958.464, 966.655] | [1015.808, 1023.999] | 1.919 | 0.905 |

Quantiles are bucket intervals; batch latency covers the whole call.

| Concurrency | Workload | Repeat | QPS change | Mean change | Old p99 ns | New p99 ns |
| ---: | --- | ---: | ---: | ---: | --- | --- |
| 1 | point-r000 | 0 | +2.052% | -2.015% | {'lower_ns': 87040, 'upper_ns': 88063} | {'lower_ns': 86016, 'upper_ns': 87039} |
| 1 | point-r000 | 1 | +0.425% | -0.424% | {'lower_ns': 86016, 'upper_ns': 87039} | {'lower_ns': 87040, 'upper_ns': 88063} |
| 1 | batch64-r000 | 0 | +0.333% | -0.331% | {'lower_ns': 262144, 'upper_ns': 266239} | {'lower_ns': 260096, 'upper_ns': 262143} |
| 1 | batch64-r000 | 1 | +1.348% | -1.332% | {'lower_ns': 266240, 'upper_ns': 270335} | {'lower_ns': 262144, 'upper_ns': 266239} |
| 64 | point-r000 | 0 | +4.513% | -4.318% | {'lower_ns': 843776, 'upper_ns': 851967} | {'lower_ns': 802816, 'upper_ns': 811007} |
| 64 | point-r000 | 1 | +3.123% | -3.030% | {'lower_ns': 827392, 'upper_ns': 835583} | {'lower_ns': 802816, 'upper_ns': 811007} |
| 64 | batch64-r000 | 0 | +2.264% | -2.208% | {'lower_ns': 8781824, 'upper_ns': 8912895} | {'lower_ns': 8323072, 'upper_ns': 8388607} |
| 64 | batch64-r000 | 1 | -4.361% | +4.560% | {'lower_ns': 9043968, 'upper_ns': 9175039} | {'lower_ns': 9568256, 'upper_ns': 9699327} |

All measured calls: 22,498,455; input keys: 242,280,129; outcomes: {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 22498455, "unknown_write": 0}.

All phases, original per-operation populations and CPU sample scopes remain in statistics-first.json. Promotion requires further workload and recovery acceptance.
