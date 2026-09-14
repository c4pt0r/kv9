FNV writer on CRC: matched write A/B screen with storage policy v2

Healthy comparison eligible: true. No automatic promotion.

Whole-call latency includes every completed outcome. Quantiles are histogram bucket bounds. CPU is sampled process usage, not per-request service time. Complete per-voter CPU, attempts, cutoff counts, outcomes and directional old/new comparisons are retained in summary.json.

| API | c | Role | Success calls/s | Success items/s | Mean us | p50 us bounds | p95 us bounds | p99 us bounds | Voter1 CPU | Voter2 CPU | Voter3 CPU | Server CPU sum | Client CPU | Outcomes |
| --- | ---: | --- | ---: | ---: | ---: | --- | --- | --- | ---: | ---: | ---: | ---: | ---: | --- |
| put | 1 | old | 19,405.104 | 19,405.104 | 51.439 | 50.176–50.687 | 59.392–59.903 | 66.560–67.583 | 0.638 | 1.237 | 0.642 | 2.517 | 0.150 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 388103, "unknown_write": 0} |
| put | 1 | new | 19,491.192 | 19,491.192 | 51.203 | 50.176–50.687 | 58.880–59.391 | 65.536–66.559 | 0.641 | 1.234 | 0.639 | 2.514 | 0.151 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 389825, "unknown_write": 0} |
| put | 64 | old | 139,388.631 | 139,388.631 | 459.024 | 438.272–442.367 | 630.784–638.975 | 737.280–745.471 | 0.598 | 2.377 | 0.588 | 3.563 | 0.431 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 2787829, "unknown_write": 0} |
| put | 64 | new | 139,878.372 | 139,878.372 | 457.411 | 434.176–438.271 | 630.784–638.975 | 745.472–753.663 | 0.595 | 2.373 | 0.582 | 3.550 | 0.436 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 2797638, "unknown_write": 0} |
| batch_put | 1 | old | 6,211.259 | 397,520.560 | 160.892 | 155.648–157.695 | 178.176–180.223 | 202.752–204.799 | 0.418 | 0.924 | 0.417 | 1.759 | 0.151 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 124226, "unknown_write": 0} |
| batch_put | 1 | new | 6,207.512 | 397,280.771 | 160.988 | 155.648–157.695 | 178.176–180.223 | 204.800–206.847 | 0.415 | 0.934 | 0.415 | 1.764 | 0.149 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 124151, "unknown_write": 0} |
| batch_put | 64 | old | 16,453.896 | 1,053,049.319 | 3888.719 | 3702.784–3735.551 | 5767.168–5832.703 | 7602.176–7667.711 | 0.719 | 1.907 | 0.718 | 3.344 | 0.480 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 329134, "unknown_write": 0} |
| batch_put | 64 | new | 17,133.584 | 1,096,549.396 | 3734.309 | 3473.408–3506.175 | 5570.560–5636.095 | 9830.400–9961.471 | 0.682 | 1.950 | 0.682 | 3.314 | 0.497 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 342742, "unknown_write": 0} |
