Native segmented-WAL vectored A/B screen

Healthy comparison eligible: true. No automatic promotion.

Whole-call latency includes every completed outcome. Quantiles are histogram bucket bounds. CPU is sampled process usage, not per-request service time. Complete per-voter CPU, attempts, cutoff counts, outcomes and directional old/new comparisons are retained in summary.json.

| API | c | Role | Success calls/s | Success items/s | Mean us | p50 us bounds | p95 us bounds | p99 us bounds | Voter1 CPU | Voter2 CPU | Voter3 CPU | Server CPU sum | Client CPU | Outcomes |
| --- | ---: | --- | ---: | ---: | ---: | --- | --- | --- | ---: | ---: | ---: | ---: | ---: | --- |
| put | 1 | old | 19,225.607 | 19,225.607 | 51.911 | 50.688–51.199 | 59.904–60.415 | 68.608–69.631 | 0.637 | 1.237 | 0.645 | 2.519 | 0.150 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 384513, "unknown_write": 0} |
| put | 1 | new | 19,289.972 | 19,289.972 | 51.738 | 50.688–51.199 | 59.904–60.415 | 68.608–69.631 | 0.643 | 1.230 | 0.639 | 2.512 | 0.150 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 385800, "unknown_write": 0} |
| put | 64 | old | 135,426.616 | 135,426.616 | 472.454 | 450.560–454.655 | 647.168–655.359 | 761.856–770.047 | 0.609 | 2.347 | 0.597 | 3.554 | 0.419 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 2708626, "unknown_write": 0} |
| put | 64 | new | 134,597.788 | 134,597.788 | 475.363 | 454.656–458.751 | 647.168–655.359 | 770.048–778.239 | 0.603 | 2.358 | 0.593 | 3.554 | 0.420 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 2692056, "unknown_write": 0} |
| batch_put | 1 | old | 5,791.344 | 370,646.003 | 172.559 | 165.888–167.935 | 194.560–196.607 | 225.280–227.327 | 0.460 | 0.939 | 0.457 | 1.855 | 0.142 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 115828, "unknown_write": 0} |
| batch_put | 1 | new | 5,798.055 | 371,075.527 | 172.360 | 165.888–167.935 | 194.560–196.607 | 223.232–225.279 | 0.463 | 0.936 | 0.459 | 1.857 | 0.142 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 115962, "unknown_write": 0} |
| batch_put | 64 | old | 14,037.009 | 898,368.574 | 4558.186 | 4325.376–4390.911 | 6619.136–6684.671 | 8781.824–8912.895 | 0.776 | 1.795 | 0.778 | 3.349 | 0.416 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 280834, "unknown_write": 0} |
| batch_put | 64 | new | 13,893.771 | 889,201.320 | 4605.168 | 4325.376–4390.911 | 6815.744–6881.279 | 9043.968–9175.039 | 0.767 | 1.772 | 0.766 | 3.305 | 0.409 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 277939, "unknown_write": 0} |
