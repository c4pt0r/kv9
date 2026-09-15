Receipt-tail write screen: keep CRC bd42 as the baseline; hold candidate promotion.

The accepted two-order screen shows repeatable point-write gains, but BatchPut64 c1 is slightly slower in both orders and the c64 batch gain is order-sensitive. This supports further investigation of receipt-tail, not an unconditional all-write speedup or baseline replacement.

Final audit 16800/8faaea/0 and timing 84770/38c0ab/0 passed. All 16 cohorts contain 7,296,894 successful single-attempt calls and 64,701,927 input items, with zero drops or non-success outcomes. Audit acceptance includes 64 exited timed lifetimes, 48 fresh drains/listener bindings and outer restoration. The extraction reads existing metadata only.

Pooled rates divide total successful work by total actual cohort elapsed time. Whole-call mean uses summed latency nanoseconds / summed calls; p99 uses merged integer histogram counts and remains a bucket interval. Batch call latency is never divided by64.

| API | c | CRC rate | Receipt rate | Rate delta | CRC mean us | Receipt mean us | CRC p99 us bounds | Receipt p99 us bounds |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- |
| Put (calls/s) | 1 | 19,388.881 | 19,584.972 | +1.011% | 51.474 | 50.956 | 66.560–67.583 | 65.024–65.535 |
| Put (calls/s) | 64 | 138,099.031 | 142,202.415 | +2.971% | 463.312 | 449.937 | 745.472–753.663 | 720.896–729.087 |
| BatchPut64 (items/s) | 1 | 399,984.491 | 397,191.431 | -0.698% | 159.899 | 161.018 | 206.848–208.895 | 204.800–206.847 |
| BatchPut64 (items/s) | 64 | 1,050,119.752 | 1,068,082.311 | +1.711% | 3899.430 | 3833.884 | 7864.320–7929.855 | 7012.352–7077.887 |

Each opposite target order is retained below. Positive throughput is better; positive mean latency is worse.

| API | c | Order | Rate delta | Mean delta | CRC p99 us bounds | Receipt p99 us bounds |
| --- | ---: | --- | ---: | ---: | --- | --- |
| Put | 1 | old-first | +1.313% | -1.298% | 66.560–67.583 | 65.024–65.535 |
| Put | 1 | new-first | +0.711% | -0.711% | 65.536–66.559 | 65.024–65.535 |
| Put | 64 | old-first | +2.611% | -2.545% | 745.472–753.663 | 720.896–729.087 |
| Put | 64 | new-first | +3.334% | -3.228% | 753.664–761.855 | 720.896–729.087 |
| BatchPut64 | 1 | old-first | -0.410% | +0.403% | 202.752–204.799 | 202.752–204.799 |
| BatchPut64 | 1 | new-first | -0.986% | +0.996% | 208.896–210.943 | 204.800–206.847 |
| BatchPut64 | 64 | old-first | +3.572% | -3.450% | 8388.608–8519.679 | 6881.280–6946.815 |
| BatchPut64 | 64 | new-first | -0.089% | +0.092% | 7077.888–7143.423 | 7208.960–7274.495 |

The material tail caution is c64 BatchPut64 in the new-first order: receipt-tail p99 is7.209–7.274 ms versus CRC7.078–7.143 ms, while rate is−0.089% and mean is+0.092%. Its old-first result is stronger (+3.572% rate with a lower p99), so the pooled+1.711% result should not hide the sign reversal. With only two ten-second orders on a shared host, these observations do not establish statistical significance or causality.

Memory remains visibly workload-dependent. Pooled duration-weighted sums of sampled voter mean RSS change by+0.343% (Put c1),+2.741% (Put c64),−0.970% (Batch64 c1), and+0.794% (Batch64 c64). c64 batch mean voter RSS sums span3.316–3.358 GiB across the four runs; sums of separate voter sampled maxima span6.332–6.561 GiB. The old-first batch candidate maximum sum rises3.626%; the reverse order is−0.124%. These are coarse sample summaries, not a demonstrated leak or simultaneous host peak. Per-voter original measurements and lifetime HWM are retained in summary.json; HWM includes setup.

Scope: CRC bd42e60 versus receipt-tail a6ac335, fixed native v3 client0be806d, three voters sharing CPUs2–5, client CPUs0–1, shared host and tmpfs WAL with normal sync/quorum semantics. This is a short write-only diagnostic; it provides no read/mixed, sustained-capacity, physical-disk or production-host result. No selection was changed.

Reproducibility: extract.py imports only the unchanged latency/histogram helpers from the accepted prior summarize.py. input-hashes.json pins every input by original absolute path, length and SHA256; those paths are historical local locations. summary.json binds the final audit SHA9d87ba394620f4bdca9c8ce5a86efb7f0615b88bc3123ff5f5ffc1c3b59b7a36 and actual audit-terminal.json. No old workload results are pooled.
