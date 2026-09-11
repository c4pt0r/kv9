# Direct peer body matched readout

The one frozen audit exited 0 and accepted all 24 original cohorts. Candidate 6707 has lower successful throughput and higher mean latency in every old 5ee → new 6707 pair. Seven p99 intervals improve; c1 point repeat 0 worsens. These are five-second shared-host tmpfs diagnostics, not sustained-capacity or equal-durability evidence.

All 27,724,506 measured calls were issued, completed and successful with one recorded attempt each: old 7,304,571; new 7,049,750; Redis 13,370,185. Every reported refusal, read failure, unknown write, client rejection and dropped slot count is zero.

Old→new values below. QPS is successful whole calls/s; latency display is microseconds, derived from original nanoseconds. p99 is the histogram bucket interval, not an exact percentile.

| c | Repeat | API | QPS old→new | Δ QPS | Mean µs old→new | p99 µs old→new |
|---|---|---|---:|---:|---:|---|
| 1 | 0 | point_get | 26182.9→25111.1 | -4.093% | 38.067→39.692 | 58.368–58.879→60.416–60.927 |
| 1 | 0 | batch_get | 26250.8→24809.1 | -5.492% | 37.946→40.169 | 59.904–60.415→59.392–59.903 |
| 1 | 1 | point_get | 26093.9→25112.7 | -3.760% | 38.201→39.698 | 60.416–60.927→57.344–57.855 |
| 1 | 1 | batch_get | 25292.0→24872.6 | -1.658% | 39.404→40.063 | 64.000–64.511→58.368–58.879 |
| 64 | 0 | point_get | 343685.9→331809.6 | -3.456% | 186.084→192.747 | 360.448–364.543→344.064–348.159 |
| 64 | 0 | batch_get | 337382.4→324271.7 | -3.886% | 189.531→197.200 | 364.544–368.639→356.352–360.447 |
| 64 | 1 | point_get | 336722.6→329012.2 | -2.290% | 189.936→194.392 | 364.544–368.639→352.256–356.351 |
| 64 | 1 | batch_get | 339257.2→324900.2 | -4.232% | 188.481→196.819 | 360.448–364.543→352.256–356.351 |

| c | Repeat | Redis API | QPS | Mean µs | p99 µs |
|---|---|---|---:|---:|---|
| 1 | 0 | get | 170252.4 | 5.798 | 8.064–8.127 |
| 1 | 0 | mget | 167647.1 | 5.885 | 8.576–8.703 |
| 1 | 1 | get | 170294.4 | 5.796 | 8.448–8.575 |
| 1 | 1 | mget | 168848.7 | 5.843 | 8.448–8.575 |
| 64 | 0 | get | 507254.6 | 126.055 | 231.424–233.471 |
| 64 | 0 | mget | 507986.7 | 125.873 | 229.376–231.423 |
| 64 | 1 | get | 502881.4 | 127.150 | 233.472–235.519 |
| 64 | 1 | mget | 478779.8 | 133.547 | 243.712–245.759 |

Acceptance retained 80 exited lifetimes, 48 serial fresh-drain documents, 48 voter/listener bindings, 2,330 role source-file checks, 2,345 in-window resource samples, and 528 retained files totaling 88,755,769 bytes. The input inventory contains 1,177 records. All three owned containers restored configured/effective CPUs 0–31 after isolation to 6–15,22–31; historical namespace maps remained unchanged. Root session 11757 and independent session 52985 both exited 0. No auditor source alteration, retry or extra sampling occurred.

Audit SHA-256: `fcb7c01b52cf83870449aa98e6e43517cec9600666eb364b715ccffb38735f90`.
Input inventory SHA-256: `7cd704c590edaaa33672435bcfdd436a9c68541dd5a0a95a09f5f362a78de6cb`.
Statistics SHA-256: `f537a4a2041bdecbde61fe52eb6ec8aa7f23129a740e56208b6838013735b2a6`.
