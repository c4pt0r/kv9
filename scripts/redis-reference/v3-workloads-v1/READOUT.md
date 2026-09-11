# V3 point and batch64 workload diagnostic readout

The first frozen independent audit accepted all 72 cohorts from timing session 87861 (exit 0). Measurement retained 79,909,933 calls / 787,052,110 input keys, all successful with one attempt each; all refusals, unknown writes, read failures, client rejections and dropped slots were zero. This is accounting and environment acceptance, not a promotion decision.

Control is 5ee897a; candidate is 57ff685. Both share the frozen native/Redis v3 clients at 0be806d. Two repeats run the entire 36-case order forward then reverse. Each timed cell is 10 seconds, c1 or c64, point size 1 or batch size 64, read percentage 0/50/100, 4,096 keys plus sentinel, 128-byte values, seed 71, 128 warmup calls, 10M cap and 1,500 ms deadline. Configured native maximum attempts remains six; actual phase counts below preserve any routing attempts.

All original per-repeat combined and per-operation call/key rates, means, p50/p95/p99 intervals and outcome populations are in `cohorts-first.csv` (216 rows) and `statistics-first.json`. `pooled-first.csv` contains 108 corresponding pooled rows. Pooled rates divide summed calls/keys by summed cohort elapsed time; means divide summed whole-call latency by summed count. Pooled quantiles merge original raw buckets, never average percentiles. Mixed combined latency weights every read/write call by its actual population; latency is never divided by batch size.

| Workers | Workload | Role | Pooled calls/s | Pooled keys/s | Mean µs | p95 µs | p99 µs | Client CPU cores | Server CPU cores |
|---:|---|---|---:|---:|---:|---|---|---:|---:|
| 1 | point-r000 | old | 16129.448 | 16129.448 | 61.897 | [74.752, 75.775] | [83.968, 84.991] | 0.127 | 2.540 |
| 1 | point-r000 | new | 16208.148 | 16208.148 | 61.593 | [73.728, 74.751] | [82.944, 83.967] | 0.128 | 2.545 |
| 1 | point-r000 | redis | 171962.317 | 171962.317 | 5.696 | [5.888, 5.951] | [7.360, 7.423] | 0.604 | 0.534 |
| 1 | point-r050 | old | 18403.639 | 18403.639 | 54.227 | [69.632, 70.655] | [78.848, 79.871] | 0.141 | 2.167 |
| 1 | point-r050 | new | 18401.778 | 18401.778 | 54.233 | [69.632, 70.655] | [79.872, 80.895] | 0.142 | 2.169 |
| 1 | point-r050 | redis | 172608.265 | 172608.265 | 5.713 | [5.888, 5.951] | [7.488, 7.551] | 0.609 | 0.529 |
| 1 | point-r100 | old | 26345.068 | 26345.068 | 37.837 | [44.544, 45.055] | [51.712, 52.223] | 0.192 | 1.642 |
| 1 | point-r100 | new | 26314.750 | 26314.750 | 37.879 | [44.544, 45.055] | [52.736, 53.247] | 0.192 | 1.639 |
| 1 | point-r100 | redis | 173059.405 | 173059.405 | 5.699 | [5.952, 6.015] | [7.680, 7.743] | 0.615 | 0.526 |
| 1 | batch64-r000 | old | 4626.233 | 296078.899 | 216.050 | [239.616, 241.663] | [278.528, 282.623] | 0.114 | 2.057 |
| 1 | batch64-r000 | new | 4605.321 | 294740.542 | 217.025 | [241.664, 243.711] | [274.432, 278.527] | 0.113 | 2.055 |
| 1 | batch64-r000 | redis | 42344.487 | 2710047.156 | 22.480 | [23.296, 23.551] | [30.976, 31.231] | 0.617 | 0.418 |
| 1 | batch64-r050 | old | 5840.309 | 373779.807 | 170.718 | [235.520, 237.567] | [270.336, 274.431] | 0.148 | 1.735 |
| 1 | batch64-r050 | new | 5866.556 | 375459.568 | 169.943 | [233.472, 235.519] | [266.240, 270.335] | 0.149 | 1.736 |
| 1 | batch64-r050 | redis | 40370.817 | 2583732.276 | 23.623 | [25.344, 25.599] | [30.464, 30.719] | 0.691 | 0.340 |
| 1 | batch64-r100 | old | 9476.984 | 606526.981 | 104.617 | [117.760, 118.783] | [137.216, 139.263] | 0.235 | 1.095 |
| 1 | batch64-r100 | new | 9511.103 | 608710.586 | 104.247 | [116.736, 117.759] | [137.216, 139.263] | 0.236 | 1.096 |
| 1 | batch64-r100 | redis | 39067.979 | 2500350.636 | 24.445 | [25.344, 25.599] | [30.720, 30.975] | 0.766 | 0.265 |
| 64 | point-r000 | old | 117212.843 | 117212.843 | 545.885 | [745.472, 753.663] | [868.352, 876.543] | 0.365 | 3.533 |
| 64 | point-r000 | new | 117729.005 | 117729.005 | 543.494 | [737.280, 745.471] | [876.544, 884.735] | 0.367 | 3.538 |
| 64 | point-r000 | redis | 493395.868 | 493395.868 | 129.557 | [163.840, 165.887] | [235.520, 237.567] | 1.447 | 0.997 |
| 64 | point-r050 | old | 165103.624 | 165103.624 | 387.492 | [540.672, 548.863] | [638.976, 647.167] | 0.540 | 3.492 |
| 64 | point-r050 | new | 167810.019 | 167810.019 | 381.244 | [548.864, 557.055] | [647.168, 655.359] | 0.553 | 3.476 |
| 64 | point-r050 | redis | 501638.887 | 501638.887 | 127.447 | [163.840, 165.887] | [235.520, 237.567] | 1.460 | 0.998 |
| 64 | point-r100 | old | 345784.984 | 345784.984 | 184.961 | [278.528, 282.623] | [352.256, 356.351] | 1.076 | 3.024 |
| 64 | point-r100 | new | 373909.148 | 373909.148 | 171.042 | [237.568, 239.615] | [294.912, 299.007] | 1.175 | 2.805 |
| 64 | point-r100 | redis | 512190.581 | 512190.581 | 124.845 | [161.792, 163.839] | [231.424, 233.471] | 1.458 | 0.996 |
| 64 | batch64-r000 | old | 9796.912 | 627002.373 | 6530.813 | [9175.040, 9306.111] | [12189.696, 12320.767] | 0.308 | 3.304 |
| 64 | batch64-r000 | new | 9814.101 | 628102.475 | 6519.435 | [9175.040, 9306.111] | [12320.768, 12451.839] | 0.305 | 3.308 |
| 64 | batch64-r000 | redis | 94782.511 | 6066080.684 | 670.455 | [958.464, 966.655] | [1007.616, 1015.807] | 1.922 | 0.902 |
| 64 | batch64-r050 | old | 15474.602 | 990374.533 | 4132.106 | [6291.456, 6356.991] | [7798.784, 7864.319] | 0.455 | 3.064 |
| 64 | batch64-r050 | new | 15351.896 | 982521.315 | 4165.011 | [6422.528, 6488.063] | [8323.072, 8388.607] | 0.451 | 3.048 |
| 64 | batch64-r050 | redis | 90182.291 | 5771666.625 | 705.724 | [1015.808, 1023.999] | [1081.344, 1097.727] | 1.954 | 0.748 |
| 64 | batch64-r100 | old | 35087.325 | 2245588.781 | 1818.721 | [2621.440, 2654.207] | [3047.424, 3080.191] | 0.976 | 2.205 |
| 64 | batch64-r100 | new | 36239.771 | 2319345.345 | 1760.753 | [2555.904, 2588.671] | [2949.120, 2981.887] | 1.005 | 2.152 |
| 64 | batch64-r100 | redis | 92743.014 | 5935552.876 | 688.070 | [991.232, 999.423] | [1040.384, 1048.575] | 1.993 | 0.615 |

Both original candidate/control repeats remain visible below; complete per-operation comparisons are in the JSON and CSV artifacts.

| Workers | Workload | Repeat 0 calls/s change | Repeat 1 calls/s change | Pooled calls/s change | Pooled mean change |
|---:|---|---:|---:|---:|---:|
| 1 | point-r000 | +1.353% | -0.371% | +0.488% | -0.491% |
| 1 | point-r050 | -0.779% | +0.767% | -0.010% | +0.010% |
| 1 | point-r100 | -0.101% | -0.129% | -0.115% | +0.112% |
| 1 | batch64-r000 | -0.965% | +0.066% | -0.452% | +0.451% |
| 1 | batch64-r050 | +0.108% | +0.793% | +0.449% | -0.454% |
| 1 | batch64-r100 | +0.006% | +0.710% | +0.360% | -0.354% |
| 64 | point-r000 | +0.376% | +0.505% | +0.440% | -0.438% |
| 64 | point-r050 | +1.355% | +1.923% | +1.639% | -1.613% |
| 64 | point-r100 | +8.240% | +8.027% | +8.133% | -7.525% |
| 64 | batch64-r000 | +0.320% | +0.032% | +0.175% | -0.174% |
| 64 | batch64-r050 | +0.137% | -1.718% | -0.793% | +0.796% |
| 64 | batch64-r100 | +3.161% | +3.408% | +3.285% | -3.187% |

All-phase accounting (includes initialization and final verification, outside measured comparisons):

| Phase | Calls | Attempts | Extra attempts | Non-success terminal outcomes |
|---|---:|---:|---:|---:|
| initialization | 299,664 | 299,712 | 48 | 0 |
| warmup | 9,216 | 9,216 | 0 | 0 |
| measurement | 79,909,933 | 79,909,933 | 0 | 0 |
| verification | 149,832 | 149,832 | 0 | 0 |

Preserved orchestration deviation: root’s ancillary freeze-summary script failed at an unsupported all-smoke-phases single-attempt assertion (tool 23f5ec, exit 1), but root then started this first timing run in the same orchestration sequence. The 24 native smoke initialization reads each had one extra routing attempt; smoke warmup/measurement/verification were single-attempt. The driver, protocol and auditor were already frozen and independently reviewed, and no acceptance predicate or source changed. The final root inventory was written after launch and is not presented as a prelaunch inventory. The original failure record and launch chronology remain unchanged; this audit does not erase that process deviation.

The audit checked 240 exited owned lifetimes, 144 fresh drain stages, 144 writer/listener bindings, 13,990 resource samples, 2,340 role source-file checks and 3,736 retained files / 74,627,673,540 bytes. Exact original/effective CPU restoration and historical namespace identities passed. All 288 already-recorded endpoint envelopes are bound in the accepted inventory; root’s stage analysis remains separate and its stage means must not be treated as an additive latency partition.

Observed free-space minima were 60,019,949,568 bytes for tmpfs and 526,960,189,440 bytes for retention storage. Preflight and runtime storage guards passed. They are operational observations, not a proof that the call cap bounds future disk growth.

Scope: shared host; native three-voter quorum WAL on tmpfs versus standalone memory Redis, with unequal durability. Final deterministic values, sentinel preservation, configured nonce budgets and write-key membership passed. The aggregate reports do not record the exact issued nonce set or full concurrent histories. No whole-history, linearizability, Chaos, sustained-capacity, statistical-significance or no-regression claim is made. Both repeats and all original outcomes remain retained.

CPU core equivalents are taken from the already accepted resource-coverage summaries. Per-process estimates are weighted by their observed sample seconds; server CPU is the sum of those estimates (three native voters or one Redis process). They are not an additive partition of request latency.

The first statistics-only reader failed on the valid empty-bucket representation for inactive operations, before producing statistics or readout files. Its original script/log are retained; this separately named second reader supports only the validator-permitted zero-count/zero-sum empty vector. The frozen audit and runtime were not changed or rerun.

Accepted audit SHA-256: `f875cc8712ba179c38885a5c598a80757f4fd9f759bb740a8e9425dba020224e`.
Statistics SHA-256: `a612948b8f4571a09cea6dbd328b89edcb9e789cc47eef32b2babf0bf9f2023c`.
Original orchestration-deviation SHA-256: `20f706d77922d9b2cb1b5b8198afd769a7dcab36b9fd34aae1162de2ca7dd3f8`.
