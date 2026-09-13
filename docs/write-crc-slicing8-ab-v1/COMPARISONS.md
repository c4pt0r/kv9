Positive rate change means higher throughput; negative mean change means lower latency. Percentiles retain interval directions.

| Scope | API | c | Repeat | Calls/s change % | Mean change % | p99 direction | Healthy |
| --- | --- | ---: | --- | ---: | ---: | --- | --- |
| per_repeat | point | 1 | 0 | 0.5856423468870497 | -0.5849241532456806 | same_bucket | True |
| per_repeat | point | 1 | 1 | 1.683322117220154 | -1.651547124308339 | new_lower_interval | True |
| pooled | point | 1 | None | 1.1326027973953323 | -1.1192923491049611 | new_lower_interval | True |
| per_repeat | point | 64 | 0 | 2.655379325102336 | -2.587088060434828 | new_lower_interval | True |
| per_repeat | point | 64 | 1 | 2.915976803865483 | -2.8337386825359467 | new_lower_interval | True |
| pooled | point | 64 | None | 2.78552295082628 | -2.7104228885548753 | new_lower_interval | True |
| per_repeat | batch64 | 1 | 0 | 7.024699036836735 | -6.564578070954285 | new_lower_interval | True |
| per_repeat | batch64 | 1 | 1 | 7.888185675096038 | -7.31484981742997 | new_lower_interval | True |
| pooled | batch64 | 1 | None | 7.454476327974313 | -6.939518543330814 | new_lower_interval | True |
| per_repeat | batch64 | 64 | 0 | 20.04323497748768 | -16.694991320954443 | new_lower_interval | True |
| per_repeat | batch64 | 64 | 1 | 17.615144698183904 | -14.976496879598155 | new_lower_interval | True |
| pooled | batch64 | 64 | None | 18.80669883774635 | -15.828591506258672 | new_lower_interval | True |
