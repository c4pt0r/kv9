Frame buffer on CRC: matched write A/B screen

Healthy comparison eligible: true. No automatic promotion.

Whole-call latency includes every completed outcome. Quantiles are histogram bucket bounds. CPU is sampled process usage, not per-request service time. Complete per-voter CPU, attempts, cutoff counts, outcomes and directional old/new comparisons are retained in summary.json.

| API | c | Role | Success calls/s | Success items/s | Mean us | p50 us bounds | p95 us bounds | p99 us bounds | Voter1 CPU | Voter2 CPU | Voter3 CPU | Server CPU sum | Client CPU | Outcomes |
| --- | ---: | --- | ---: | ---: | ---: | --- | --- | --- | ---: | ---: | ---: | ---: | ---: | --- |
| put | 1 | old | 19,463.837 | 19,463.837 | 51.277 | 50.176–50.687 | 58.880–59.391 | 65.536–66.559 | 0.643 | 1.236 | 0.641 | 2.520 | 0.151 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 389278, "unknown_write": 0} |
| put | 1 | new | 19,424.073 | 19,424.073 | 51.382 | 50.176–50.687 | 58.880–59.391 | 65.536–66.559 | 0.641 | 1.237 | 0.640 | 2.518 | 0.151 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 388483, "unknown_write": 0} |
| put | 64 | old | 139,188.639 | 139,188.639 | 459.683 | 438.272–442.367 | 630.784–638.975 | 737.280–745.471 | 0.595 | 2.382 | 0.585 | 3.562 | 0.431 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 2783842, "unknown_write": 0} |
| put | 64 | new | 138,469.020 | 138,469.020 | 462.072 | 442.368–446.463 | 630.784–638.975 | 745.472–753.663 | 0.598 | 2.376 | 0.584 | 3.558 | 0.429 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 2769477, "unknown_write": 0} |
| batch_put | 1 | old | 6,217.406 | 397,914.000 | 160.734 | 155.648–157.695 | 178.176–180.223 | 204.800–206.847 | 0.415 | 0.928 | 0.414 | 1.757 | 0.150 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 124349, "unknown_write": 0} |
| batch_put | 1 | new | 6,252.179 | 400,139.454 | 159.832 | 153.600–155.647 | 176.128–178.175 | 202.752–204.799 | 0.412 | 0.928 | 0.412 | 1.752 | 0.152 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 125044, "unknown_write": 0} |
| batch_put | 64 | old | 16,651.252 | 1,065,680.142 | 3842.594 | 3670.016–3702.783 | 5570.560–5636.095 | 6946.816–7012.351 | 0.719 | 1.924 | 0.719 | 3.361 | 0.481 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 333091, "unknown_write": 0} |
| batch_put | 64 | new | 16,714.393 | 1,069,721.142 | 3827.746 | 3637.248–3670.015 | 5636.096–5701.631 | 7143.424–7208.959 | 0.720 | 1.926 | 0.714 | 3.359 | 0.481 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 334390, "unknown_write": 0} |
