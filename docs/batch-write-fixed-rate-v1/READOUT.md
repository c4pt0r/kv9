# Fixed-rate batch-write diagnosis

Whole-call and scheduled-to-completion histograms cover terminal issued calls only; dropped slots have no latency sample. All integer sums/counts and p50/p95/p99 intervals are in summary.json. Pooling merges original buckets and sums; no averaged percentile. Batch latency is not divided by64. CPU weights use resource-coverage sample_seconds, not exclusively measurement CPU. Three voters/quorum/sync on volatile tmpfs; no real-disk/equal-Redis-durability claim. Initial/warmup/verification outcomes remain separate. No automatic promotion.

| Rate | Role | Repeat | Completed / offered | Drops / post-cutoff | Success calls/s | Mean whole µs | p99 whole µs | Mean scheduled µs | p99 scheduled µs | Mean lateness µs | p99 lateness µs |
|---:|---|---:|---:|---:|---:|---:|---|---:|---|---:|---|
| 8000 | old | pooled | 159979 / 160000 | 21 / 0 | 7998.950 | 923.153 | [4849.664, 4915.199] | 1846.565 | [5963.776, 6029.311] | 923.412 | [1966.080, 1982.463] |
| 8000 | new | pooled | 159978 / 160000 | 22 / 8 | 7998.758 | 906.853 | [5439.488, 5505.023] | 1865.100 | [6684.672, 6750.207] | 958.247 | [1998.848, 2015.231] |
| 12000 | old | pooled | 238970 / 240000 | 1030 / 36 | 11947.714 | 1558.602 | [6750.208, 6815.743] | 2360.374 | [8781.824, 8912.895] | 801.772 | [2949.120, 2981.887] |
| 12000 | new | pooled | 239029 / 240000 | 971 / 12 | 11951.137 | 1588.109 | [7143.424, 7208.959] | 2423.419 | [9175.040, 9306.111] | 835.310 | [3375.104, 3407.871] |

See PER-REPEAT.md and complete suitability flags in summary.json. Equal counts with drops do not establish identical slot/key membership.
