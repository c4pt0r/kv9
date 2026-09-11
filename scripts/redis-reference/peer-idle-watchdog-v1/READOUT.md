# Peer idle watchdog: complete matched readout

Candidate f62c08e is not selected. Independent recording validation passes.
All values are successful logical calls; all measured calls succeeded.
Repeats below retain raw numbering 0 and 1. p99 values are original bucket intervals.

| Concurrency | Repeat | Arm | Calls | QPS | Mean us | p99 interval us |
| ---: | ---: | --- | ---: | ---: | ---: | --- |
| 1 | 0 | old-point | 131,288 | 26,257.164 | 37.961782 | 59.392-59.903 |
| 1 | 0 | old-batch1 | 130,254 | 26,050.749 | 38.253870 | 59.392-59.903 |
| 1 | 0 | new-point | 130,180 | 26,035.867 | 38.276230 | 57.856-58.367 |
| 1 | 0 | new-batch1 | 127,207 | 25,441.333 | 39.173111 | 58.880-59.391 |
| 1 | 0 | redis-mget1 | 841,521 | 168,304.123 | 5.861427 | 8.448-8.575 |
| 1 | 0 | redis-get1 | 853,163 | 170,632.455 | 5.784847 | 8.064-8.127 |
| 64 | 0 | old-point | 1,672,988 | 334,590.354 | 191.145669 | 368.640-372.735 |
| 64 | 0 | old-batch1 | 1,686,656 | 337,318.675 | 189.567531 | 364.544-368.639 |
| 64 | 0 | new-point | 1,662,653 | 332,523.065 | 192.335439 | 344.064-348.159 |
| 64 | 0 | new-batch1 | 1,632,853 | 326,561.289 | 195.815725 | 352.256-356.351 |
| 64 | 0 | redis-mget1 | 2,476,503 | 495,280.845 | 129.098464 | 231.424-233.471 |
| 64 | 0 | redis-get1 | 2,546,727 | 509,322.463 | 125.546288 | 231.424-233.471 |
| 64 | 1 | redis-get1 | 2,532,006 | 506,378.434 | 126.276605 | 233.472-235.519 |
| 64 | 1 | redis-mget1 | 2,485,036 | 496,986.001 | 128.652766 | 231.424-233.471 |
| 64 | 1 | new-batch1 | 1,632,789 | 326,548.373 | 195.823710 | 352.256-356.351 |
| 64 | 1 | new-point | 1,660,779 | 332,147.228 | 192.554613 | 348.160-352.255 |
| 64 | 1 | old-batch1 | 1,691,272 | 338,242.128 | 189.049471 | 368.640-372.735 |
| 64 | 1 | old-point | 1,710,670 | 342,121.030 | 186.936509 | 356.352-360.447 |
| 1 | 1 | redis-get1 | 853,773 | 170,754.591 | 5.780305 | 8.064-8.127 |
| 1 | 1 | redis-mget1 | 839,997 | 167,999.327 | 5.870956 | 8.320-8.447 |
| 1 | 1 | new-batch1 | 130,011 | 26,002.055 | 38.335391 | 57.344-57.855 |
| 1 | 1 | new-point | 131,261 | 26,252.111 | 37.969759 | 56.832-57.343 |
| 1 | 1 | old-batch1 | 131,129 | 26,225.659 | 38.010334 | 60.416-60.927 |
| 1 | 1 | old-point | 130,997 | 26,199.279 | 38.035970 | 60.416-60.927 |

All 27,821,713 calls succeeded with one observed attempt per call. No dropped
slots or non-success population was removed. Same-run Redis GET and MGET(1)
are separate rows; the Redis server has no replicas, persistence or pipeline.

See statistics-first.json for independent comparison arithmetic and
root-statistics-pre-audit.json for the scalar full-cohort readback. All eight
paired QPS and mean changes were independently cross-checked.
