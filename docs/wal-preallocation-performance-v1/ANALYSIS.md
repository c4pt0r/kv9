WAL preallocation versus default: same-source write A/B with one requalified client

Healthy comparison eligible: true. No automatic promotion.

Only the sixteen 10-second timed cohorts contribute rates/latencies; no smoke performance values. One requalified client ELF (1b8060eb) is shared by default (old) and WAL preallocation (new), both at source 86aa6fc. Runtime storage is volatile tmpfs; outputs are retained on the data volume. No physical-power-loss, historical-ELF equivalence, automatic promotion or statistical-significance claim. Whole-call latency includes every completed outcome. Quantiles are histogram bucket bounds. CPU is sampled process usage, not per-request service time. Complete per-voter CPU, attempts, cutoff counts, outcomes and directional old/new comparisons are retained in summary.json.

| API | c | Role | Success calls/s | Success items/s | Mean us | p50 us bounds | p95 us bounds | p99 us bounds | Voter1 CPU | Voter2 CPU | Voter3 CPU | Server CPU sum | Client CPU | Outcomes |
| --- | ---: | --- | ---: | ---: | ---: | --- | --- | --- | ---: | ---: | ---: | ---: | ---: | --- |
| put | 1 | old | 19,316.845 | 19,316.845 | 51.673 | 50.688–51.199 | 59.904–60.415 | 68.608–69.631 | 0.643 | 1.232 | 0.639 | 2.514 | 0.151 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 386338, "unknown_write": 0} |
| put | 1 | new | 19,581.309 | 19,581.309 | 50.973 | 50.176–50.687 | 58.880–59.391 | 67.584–68.607 | 0.637 | 1.237 | 0.638 | 2.512 | 0.153 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 391627, "unknown_write": 0} |
| put | 64 | old | 137,873.776 | 137,873.776 | 464.072 | 442.368–446.463 | 630.784–638.975 | 745.472–753.663 | 0.597 | 2.369 | 0.586 | 3.552 | 0.430 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 2757523, "unknown_write": 0} |
| put | 64 | new | 138,625.767 | 138,625.767 | 461.547 | 438.272–442.367 | 630.784–638.975 | 745.472–753.663 | 0.596 | 2.379 | 0.585 | 3.559 | 0.433 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 2772606, "unknown_write": 0} |
| batch_put | 1 | old | 6,208.243 | 397,327.528 | 160.959 | 155.648–157.695 | 178.176–180.223 | 208.896–210.943 | 0.416 | 0.929 | 0.415 | 1.760 | 0.153 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 124166, "unknown_write": 0} |
| batch_put | 1 | new | 6,230.456 | 398,749.157 | 160.390 | 155.648–157.695 | 178.176–180.223 | 208.896–210.943 | 0.414 | 0.927 | 0.411 | 1.752 | 0.153 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 124610, "unknown_write": 0} |
| batch_put | 64 | old | 15,980.470 | 1,022,750.059 | 4003.778 | 3735.552–3768.319 | 6029.312–6094.847 | 9306.112–9437.183 | 0.715 | 1.890 | 0.714 | 3.319 | 0.468 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 319684, "unknown_write": 0} |
| batch_put | 64 | new | 16,216.080 | 1,037,829.100 | 3945.456 | 3670.016–3702.783 | 5898.240–5963.775 | 9306.112–9437.183 | 0.716 | 1.885 | 0.720 | 3.321 | 0.471 | {"client_rejected": 0, "read_failure": 0, "refused": 0, "success": 324405, "unknown_write": 0} |
