Positive rate change means higher throughput; negative mean change means lower latency. Percentiles retain interval directions.

| Scope | API | c | Repeat | Calls/s change % | Mean change % | p99 direction | Healthy |
| --- | --- | ---: | --- | ---: | ---: | --- | --- |
| per_repeat | point | 1 | 0 | -0.3540872343489654 | 0.35784271358512143 | new_higher_interval | True |
| per_repeat | point | 1 | 1 | 1.0401205665466096 | -1.0411125786492614 | new_lower_interval | True |
| pooled | point | 1 | None | 0.3531799311927708 | -0.3565865887217079 | new_lower_interval | True |
| per_repeat | point | 64 | 0 | 3.677304965367445 | -3.545338810853371 | new_lower_interval | True |
| per_repeat | point | 64 | 1 | 2.6849626210120103 | -2.616669027123697 | new_lower_interval | True |
| pooled | point | 64 | None | 3.168813106264934 | -3.071662727076352 | new_lower_interval | True |
| per_repeat | batch64 | 1 | 0 | -0.4782347476088078 | 0.48059763441010706 | same_bucket | True |
| per_repeat | batch64 | 1 | 1 | 0.4036128660165561 | -0.40701586876454865 | new_lower_interval | True |
| pooled | batch64 | 1 | None | -0.0332972707519974 | 0.030816977291703296 | new_lower_interval | True |
| per_repeat | batch64 | 64 | 0 | -3.437387561336225 | 3.5704052522819385 | new_higher_interval | True |
| per_repeat | batch64 | 64 | 1 | 0.6560412126054516 | -0.6457645824385128 | new_lower_interval | True |
| pooled | batch64 | 64 | None | -1.4186634304309442 | 1.4473611904915584 | new_higher_interval | True |
