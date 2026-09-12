# Every original rate and order

Whole-call and scheduled-to-completion histograms cover terminal issued calls only; dropped slots have no latency sample. All integer sums/counts and p50/p95/p99 intervals are in summary.json. Pooling merges original buckets and sums; no averaged percentile. Batch latency is not divided by64. CPU weights use resource-coverage sample_seconds, not exclusively measurement CPU. Three voters/quorum/sync on volatile tmpfs; no real-disk/equal-Redis-durability claim. Initial/warmup/verification outcomes remain separate. No automatic promotion.

| Rate | Role | Repeat | Completed / offered | Drops / post-cutoff | Success calls/s | Mean whole µs | p99 whole µs | Mean scheduled µs | p99 scheduled µs | Mean lateness µs | p99 lateness µs |
|---:|---|---:|---:|---:|---:|---:|---|---:|---|---:|---|
| 8000 | old | 0 | 79985 / 80000 | 15 / 0 | 7998.500 | 901.220 | [4063.232, 4095.999] | 1849.090 | [5242.880, 5308.415] | 947.870 | [1982.464, 1998.847] |
| 8000 | new | 0 | 79987 / 80000 | 13 / 0 | 7998.700 | 901.410 | [5308.416, 5373.951] | 1858.616 | [6553.600, 6619.135] | 957.205 | [1998.848, 2015.231] |
| 12000 | old | 0 | 119771 / 120000 | 229 / 12 | 11976.721 | 1563.770 | [6488.064, 6553.599] | 2390.329 | [8257.536, 8323.071] | 826.559 | [2752.512, 2785.279] |
| 12000 | new | 0 | 119621 / 120000 | 379 / 12 | 11961.473 | 1553.383 | [6750.208, 6815.743] | 2385.548 | [8912.896, 9043.967] | 832.164 | [3178.496, 3211.263] |
| 12000 | new | 1 | 119408 / 120000 | 592 / 0 | 11940.800 | 1622.897 | [7405.568, 7471.103] | 2461.359 | [9437.184, 9568.255] | 838.462 | [3538.944, 3571.711] |
| 12000 | old | 1 | 119199 / 120000 | 801 / 24 | 11918.709 | 1553.409 | [7012.352, 7077.887] | 2330.274 | [9306.112, 9437.183] | 776.865 | [3178.496, 3211.263] |
| 8000 | new | 1 | 79991 / 80000 | 9 / 8 | 7998.816 | 912.296 | [5570.560, 5636.095] | 1871.583 | [6815.744, 6881.279] | 959.288 | [1998.848, 2015.231] |
| 8000 | old | 1 | 79994 / 80000 | 6 / 0 | 7999.400 | 945.084 | [5373.952, 5439.487] | 1844.041 | [6553.600, 6619.135] | 898.957 | [1949.696, 1966.079] |
