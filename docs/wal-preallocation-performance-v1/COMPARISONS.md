Positive rate change means higher throughput; negative mean change means lower latency. Percentiles retain interval directions.

| Scope | API | c | Repeat | Calls/s change % | Mean change % | p99 direction | Healthy |
| --- | --- | ---: | --- | ---: | ---: | --- | --- |
| per_repeat | point | 1 | 0 | 1.4112468781494414 | -1.393278317852853 | new_lower_interval | True |
| per_repeat | point | 1 | 1 | 1.3266139633911322 | -1.315581408923172 | same_bucket | True |
| pooled | point | 1 | None | 1.3690848965974611 | -1.3545963870818967 | new_lower_interval | True |
| per_repeat | point | 64 | 0 | 0.43718189309500666 | -0.4367666327500519 | new_lower_interval | True |
| per_repeat | point | 64 | 1 | 0.6540605202838057 | -0.6515418539778772 | new_lower_interval | True |
| pooled | point | 64 | None | 0.5454200628807637 | -0.544071097962906 | same_bucket | True |
| per_repeat | batch64 | 1 | 0 | 1.023621157960597 | -1.0112884007551504 | new_lower_interval | True |
| per_repeat | batch64 | 1 | 1 | -0.3043176171408257 | 0.3092368058661199 | new_higher_interval | True |
| pooled | batch64 | 1 | None | 0.35779792682248956 | -0.3535533782458744 | same_bucket | True |
| per_repeat | batch64 | 64 | 0 | -0.14262182696632708 | 0.13487633545237188 | new_higher_interval | True |
| per_repeat | batch64 | 64 | 1 | 3.0659530842418015 | -2.9743725216086814 | new_lower_interval | True |
| pooled | batch64 | 64 | None | 1.4743622046261118 | -1.4566608326298969 | same_bucket | True |
