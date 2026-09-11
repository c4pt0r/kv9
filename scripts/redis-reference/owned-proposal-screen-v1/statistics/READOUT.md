# Owned-proposal write-only screen: first accepted recording

Two 10-second forward/reverse c1/c64 point PUT/BatchPut(64) write-only selection repeats on a shared host; three-voter KV9 quorum/sync with tmpfs WAL versus standalone Redis with persistence and pipelining disabled. Not full-workload acceptance. No equal-durability, sustained-capacity, significance or new Chaos claim.

| Concurrency | Workload | Role | Calls/s | Keys/s | Mean us | p95 us | p99 us | Client cores | Server cores |
| ---: | --- | --- | ---: | ---: | ---: | --- | --- | ---: | ---: |
| 1 | point-r000 | old | 16815.047 | 16815.047 | 59.368 | [69.632, 70.655] | [81.920, 82.943] | 0.133 | 2.536 |
| 1 | point-r000 | new | 16667.810 | 16667.810 | 59.895 | [70.656, 71.679] | [82.944, 83.967] | 0.132 | 2.537 |
| 1 | point-r000 | redis | 170583.297 | 170583.297 | 5.742 | [6.144, 6.207] | [7.872, 7.935] | 0.604 | 0.532 |
| 1 | batch64-r000 | old | 5409.634 | 346216.582 | 184.743 | [206.848, 208.895] | [249.856, 251.903] | 0.132 | 1.902 |
| 1 | batch64-r000 | new | 5399.380 | 345560.317 | 185.094 | [208.896, 210.943] | [251.904, 253.951] | 0.132 | 1.902 |
| 1 | batch64-r000 | redis | 42380.941 | 2712380.240 | 22.458 | [23.296, 23.551] | [31.488, 31.743] | 0.615 | 0.420 |
| 64 | point-r000 | old | 123526.656 | 123526.656 | 517.971 | [704.512, 712.703] | [827.392, 835.583] | 0.389 | 3.534 |
| 64 | point-r000 | new | 122619.489 | 122619.489 | 521.805 | [704.512, 712.703] | [835.584, 843.775] | 0.386 | 3.530 |
| 64 | point-r000 | redis | 494237.917 | 494237.917 | 129.339 | [163.840, 165.887] | [235.520, 237.567] | 1.445 | 0.997 |
| 64 | batch64-r000 | old | 13480.611 | 862759.112 | 4746.206 | [6815.744, 6881.279] | [9306.112, 9437.183] | 0.406 | 3.346 |
| 64 | batch64-r000 | new | 12206.767 | 781233.072 | 5241.703 | [7274.496, 7340.031] | [25427.968, 25690.111] | 0.362 | 3.003 |
| 64 | batch64-r000 | redis | 95638.603 | 6120870.591 | 664.510 | [950.272, 958.463] | [999.424, 1007.615] | 1.925 | 0.908 |

Quantiles are bucket intervals; batch latency covers the whole call.

| Concurrency | Workload | Repeat | QPS change | Mean change | Old p99 ns | New p99 ns |
| ---: | --- | ---: | ---: | ---: | --- | --- |
| 1 | point-r000 | 0 | +0.082% | -0.078% | {'lower_ns': 82944, 'upper_ns': 83967} | {'lower_ns': 81920, 'upper_ns': 82943} |
| 1 | point-r000 | 1 | -1.828% | +1.863% | {'lower_ns': 80896, 'upper_ns': 81919} | {'lower_ns': 82944, 'upper_ns': 83967} |
| 1 | batch64-r000 | 0 | -0.255% | +0.254% | {'lower_ns': 247808, 'upper_ns': 249855} | {'lower_ns': 247808, 'upper_ns': 249855} |
| 1 | batch64-r000 | 1 | -0.124% | +0.126% | {'lower_ns': 253952, 'upper_ns': 255999} | {'lower_ns': 256000, 'upper_ns': 258047} |
| 64 | point-r000 | 0 | -0.452% | +0.454% | {'lower_ns': 827392, 'upper_ns': 835583} | {'lower_ns': 835584, 'upper_ns': 843775} |
| 64 | point-r000 | 1 | -1.019% | +1.030% | {'lower_ns': 827392, 'upper_ns': 835583} | {'lower_ns': 827392, 'upper_ns': 835583} |
| 64 | batch64-r000 | 0 | +1.125% | -1.107% | {'lower_ns': 9961472, 'upper_ns': 10092543} | {'lower_ns': 12320768, 'upper_ns': 12451839} |
| 64 | batch64-r000 | 1 | -19.863% | +24.789% | {'lower_ns': 8912896, 'upper_ns': 9043967} | {'lower_ns': 37224448, 'upper_ns': 37748735} |

All measured calls: 22,379,974; input keys: 242,284,183; outcomes: {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 22379974, "unknown_write": 0}.

All phases, original per-operation populations and CPU sample scopes remain in statistics-first.json. Promotion requires further workload and recovery acceptance.
