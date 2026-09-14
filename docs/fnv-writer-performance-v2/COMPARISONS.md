Positive rate change means higher throughput; negative mean change means lower latency. Percentiles retain interval directions.

| Scope | API | c | Repeat | Calls/s change % | Mean change % | p99 direction | Healthy |
| --- | --- | ---: | --- | ---: | ---: | --- | --- |
| per_repeat | point | 1 | 0 | 1.6583026452458238 | -1.6316959231532868 | new_lower_interval | True |
| per_repeat | point | 1 | 1 | -0.7487387508445242 | 0.722913012341686 | same_bucket | True |
| pooled | point | 1 | None | 0.4436333430889672 | -0.4574489502511492 | new_lower_interval | True |
| per_repeat | point | 64 | 0 | 1.4899461618475884 | -1.4684820001193977 | new_lower_interval | True |
| per_repeat | point | 64 | 1 | -0.7820778855969435 | 0.7859214038512308 | new_higher_interval | True |
| pooled | point | 64 | None | 0.35134936312395393 | -0.3514736784098971 | new_higher_interval | True |
| per_repeat | batch64 | 1 | 0 | -1.6319004373640422 | 1.6606961061962355 | new_higher_interval | True |
| per_repeat | batch64 | 1 | 1 | 1.5284094100281198 | -1.5073596894184416 | new_lower_interval | True |
| pooled | batch64 | 1 | None | -0.06032139678381343 | 0.06021062070160621 | new_higher_interval | True |
| per_repeat | batch64 | 64 | 0 | 1.3651617382693715 | -1.3543075140359018 | new_higher_interval | True |
| per_repeat | batch64 | 64 | 1 | 6.864063076647087 | -6.4232897196216605 | new_lower_interval | True |
| pooled | batch64 | 64 | None | 4.130867941633398 | -3.9707225910959654 | new_higher_interval | True |
