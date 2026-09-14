Positive rate change means higher throughput; negative mean change means lower latency. Percentiles retain interval directions.

| Scope | API | c | Repeat | Calls/s change % | Mean change % | p99 direction | Healthy |
| --- | --- | ---: | --- | ---: | ---: | --- | --- |
| per_repeat | point | 1 | 0 | -0.4865447698340075 | 0.49617682210518765 | new_higher_interval | True |
| per_repeat | point | 1 | 1 | 1.162120032429259 | -1.1532058137176282 | new_lower_interval | True |
| pooled | point | 1 | None | 0.3347841999820522 | -0.3323066457502799 | same_bucket | True |
| per_repeat | point | 64 | 0 | -0.37340985606127974 | 0.3749173265838346 | new_higher_interval | True |
| per_repeat | point | 64 | 1 | -0.8533309195415284 | 0.8603269712367911 | new_higher_interval | True |
| pooled | point | 64 | None | -0.6120132682573054 | 0.6156622565395953 | new_higher_interval | True |
| per_repeat | batch64 | 1 | 0 | 0.5801232111840582 | -0.5786876405176833 | new_lower_interval | True |
| per_repeat | batch64 | 1 | 1 | -0.34809978261006735 | 0.35151678890259497 | same_bucket | True |
| pooled | batch64 | 1 | None | 0.11588525042713904 | -0.11561536449569143 | new_lower_interval | True |
| per_repeat | batch64 | 64 | 0 | -0.12304289960463066 | 0.12457240344907472 | new_lower_interval | True |
| per_repeat | batch64 | 64 | 1 | -1.9116886432893443 | 1.9470697698658501 | new_higher_interval | True |
| pooled | batch64 | 64 | None | -1.020433454841052 | 1.0307187454540179 | new_higher_interval | True |
