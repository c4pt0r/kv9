Positive rate change means higher throughput; negative mean change means lower latency. Percentiles retain interval directions.

| Scope | API | c | Repeat | Calls/s change % | Mean change % | p99 direction | Healthy |
| --- | --- | ---: | --- | ---: | ---: | --- | --- |
| per_repeat | point | 1 | 0 | -0.1236239748686696 | 0.12320260189147003 | same_bucket | True |
| per_repeat | point | 1 | 1 | -0.2853142997984115 | 0.2877319022821734 | same_bucket | True |
| pooled | point | 1 | None | -0.2042959384680909 | 0.20522670254052855 | same_bucket | True |
| per_repeat | point | 64 | 0 | -1.798679475239906 | 1.8315539663776637 | new_higher_interval | True |
| per_repeat | point | 64 | 1 | 0.7860939603907102 | -0.7797569517465797 | new_lower_interval | True |
| pooled | point | 64 | None | -0.5170100814393064 | 0.5197663721815848 | new_higher_interval | True |
| per_repeat | batch64 | 1 | 0 | 0.4286008146580311 | -0.43089088443076795 | new_lower_interval | True |
| per_repeat | batch64 | 1 | 1 | 0.6917182784097475 | -0.6926884351869944 | new_lower_interval | True |
| pooled | batch64 | 1 | None | 0.5592801038154294 | -0.5610912783262423 | new_lower_interval | True |
| per_repeat | batch64 | 64 | 0 | 2.320855966206925 | -2.266463239040717 | new_lower_interval | True |
| per_repeat | batch64 | 64 | 1 | -1.5438073338655989 | 1.5485662639987074 | new_higher_interval | True |
| pooled | batch64 | 64 | None | 0.3791944597556274 | -0.38640802695900645 | new_higher_interval | True |
