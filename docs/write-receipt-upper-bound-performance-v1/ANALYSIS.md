Upper-bound versus CRC: matched write A/B with one requalified client

Healthy comparison eligible: true. No automatic promotion.

Only the sixteen 10-second timed cohorts contribute rates/latencies; no smoke performance values. One requalified client ELF (1b8060eb) is shared by CRC bd42 and upper-bound e2e. Runtime storage is volatile tmpfs; outputs are retained on the data volume. No physical-power-loss, historical-ELF equivalence, automatic promotion or statistical-significance claim. Whole-call latency includes every completed outcome. Quantiles are histogram bucket bounds. CPU is sampled process usage, not per-request service time. Complete per-voter CPU, attempts, cutoff counts, outcomes and directional old/new comparisons are retained in summary.json.

| API | c | Role | Success calls/s | Success items/s | Mean us | p50 us bounds | p95 us bounds | p99 us bounds | Voter1 CPU | Voter2 CPU | Voter3 CPU | Server CPU sum | Client CPU | Outcomes |
| --- | ---: | --- | ---: | ---: | ---: | --- | --- | --- | ---: | ---: | ---: | ---: | ---: | --- |
| put | 1 | old | 18,960.518 | 18,960.518 | 52.641 | 51.712–52.223 | 61.440–61.951 | 69.632–70.655 | 0.633 | 1.222 | 0.634 | 2.489 | 0.161 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 379212, "unknown_write": 0} |
| put | 1 | new | 19,027.483 | 19,027.483 | 52.453 | 51.200–51.711 | 61.440–61.951 | 68.608–69.631 | 0.633 | 1.213 | 0.631 | 2.478 | 0.159 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 380550, "unknown_write": 0} |
| put | 64 | old | 135,562.425 | 135,562.425 | 471.982 | 450.560–454.655 | 655.360–663.551 | 778.240–786.431 | 0.594 | 2.341 | 0.582 | 3.517 | 0.434 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 2711307, "unknown_write": 0} |
| put | 64 | new | 139,858.145 | 139,858.145 | 457.484 | 434.176–438.271 | 622.592–630.783 | 745.472–753.663 | 0.605 | 2.308 | 0.595 | 3.508 | 0.441 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 2797289, "unknown_write": 0} |
| batch_put | 1 | old | 6,151.395 | 393,689.312 | 162.464 | 157.696–159.743 | 180.224–182.271 | 204.800–206.847 | 0.414 | 0.924 | 0.412 | 1.751 | 0.156 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 123029, "unknown_write": 0} |
| batch_put | 1 | new | 6,149.347 | 393,558.224 | 162.514 | 157.696–159.743 | 180.224–182.271 | 202.752–204.799 | 0.416 | 0.915 | 0.418 | 1.749 | 0.158 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 122988, "unknown_write": 0} |
| batch_put | 64 | old | 16,105.143 | 1,030,729.153 | 3972.655 | 3768.320–3801.087 | 5963.776–6029.311 | 7798.784–7864.319 | 0.699 | 1.859 | 0.694 | 3.251 | 0.448 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 322197, "unknown_write": 0} |
| batch_put | 64 | new | 15,876.665 | 1,016,106.575 | 4030.153 | 3801.088–3833.855 | 6160.384–6225.919 | 8650.752–8781.823 | 0.693 | 1.822 | 0.692 | 3.207 | 0.460 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 317579, "unknown_write": 0} |
