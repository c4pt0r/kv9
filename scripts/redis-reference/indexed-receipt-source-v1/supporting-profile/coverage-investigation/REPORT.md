# First CRC write-profile coverage rejection

The original decoder correctly rejected **BatchPut(64) at the start boundary**. Its first retained CPU sample is **6.426334 ms after** the nominal measurement start. There are zero premeasurement samples. The original runtime remains exit 0; the original decoder remains exit 1, and no combined profile acceptance is claimed.

| Retained observation | Point PUT | BatchPut(64) |
| --- | ---: | ---: |
| Raw decoded headers | 3,585 | 3,201 |
| Samples before / within / after nominal window | 165 / 3,325 / 95 | 0 / 3,148 / 53 |
| First sample relative to nominal start | −460.720846 ms | +6.426334 ms |
| Last sample after nominal end | 13,849.700918 ms | 13,918.736322 ms |
| Wall/monotonic anchor-offset spread | 189 ns | 521 ns |
| Original perf byte count | 59,871,320 | 53,442,940 |
| Original selected-sample acceptance | 3,324 selected; passed | Not produced; rejected |

The within-window counts above are passive header counts, **not accepted selected populations**. No frame attribution, histogram acceptance or alternative coverage gate was run.

## Exact failed inequality

`analyze.py:75–79` subtracts the mean of two realtime-minus-monotonic anchors from the report's absolute start and requires a sample strictly before start and strictly after the five-second end.

For BatchPut, the retained anchors produce offset `1787711287576721910 ns` and nominal monotonic interval `[1435885282859187, 1435890282859187] ns`. The first header is at `1435885289285521 ns`, in `batch_put/perf-script.txt:1`, PID/TID `4152406/4152409`. Subtraction gives `1435885289285521 − 1435885282859187 = 6,426,334 ns`: the first conjunct fails. The last header is at `1435904201595509 ns`, line 18652, so the end conjunct passes by `13,918,736,322 ns`.

The 521 ns anchor spread and 270/1,773 ns anchor acquisition widths are far smaller than the 6.426 ms failure. Both retained perf-script/perf-report commands exited 0, both decoder stderr files are empty, and perf-report records `Total Lost Samples: 0`. The 53,442,940-byte recording is below the original 134,217,728-byte cap. These observations identify neither truncation nor a tail/clock-conversion failure.

## Recorder cause and its limit

`profile.py:124–142` starts `perf record --clockid mono -e cpu-clock -F 199 ... -- sleep 20`, observes a live perf process, sleeps 250 ms, then launches the benchmark client. This confirms a process lifetime, not a sample-bearing prefix. The separate permission probe is a different perf invocation and its activity is not part of this recording.

For BatchPut, the record command was issued 327.444941 ms before measurement and the profiler was observed 306.408314 ms before it. The before-client marker precedes measurement by only 56.323550 ms. The client's initialization lasts 27.782806 ms; warmup ends at 52.455089 ms and measurement starts at 52.511244 ms on its relative stage clock. Point PUT has a much longer 507.956180 ms initialization and measurement starts at 516.771111 ms, with 165 recorded prefix samples.

The fixed client writes `ready.json` and immediately starts measurement (`kv9-batch-benchmark.rs:444–450`); it does not wait for recorder acknowledgement. Because cpu-clock sampling requires on-CPU activity, the fixed sleep and short BatchPut setup did not establish the required prefix. The retained text cannot distinguish insufficient premeasurement CPU activity from event-enable latency; it does not prove that perf failed to run during that wall-clock interval.

## Future recorder correction, not applied or run

Keep the exact containment, one-millisecond edge exclusion, sample-count, aggregate-bin, edge-gap, source/PID, loss and cap predicates unchanged. Increasing only `sleep 20` cannot fix this missing start sample, and changing the accepted start to the first sample would weaken the contract.

In a new explicitly reviewed profiler adapter, establish an active prefix after attaching to the same three owned voter lifetimes and before launching the unchanged measurement client. Use a separately labelled, bounded read-only RPC priming interval against an absent scratch key: for example, a predeclared one-second/100,000-call ceiling with 16 workers and retained call outcomes. Keep it outside the client's initialization/warmup/measurement report and retain its exact monotonic start/end, PIDs and profiler liveness. Reuse existing fixture RPC and cleanup helpers; do not change the measured dataset, 128 warmup calls, 64 workers, five-second window or sampling frequency. The extra priming affects cache/allocation state and must be disclosed as profiling preparation, not an A/B performance result.

A future recorder can additionally bind perf's actual event-arm acknowledgement once the exact installed CLI is verified, but process existence or an acknowledgement alone still does not prove a CPU sample. The final original decoder must continue requiring actual sample headers on both sides; if a new prefix still lacks samples, retain that failure. No longer sleep, active priming, acknowledgement mechanism or second recording was executed here.

## Retention and scope

`diagnosis.json` retains exact arithmetic, header lines, configuration, clock anchors, command timing and first rejection. `input-inventory.json` binds 51 original text/JSON inputs and confirms their bytes unchanged after reading. Copies of the original decoder log/invocation/terminal and runtime terminal are retained here. Original perf binaries were neither decoded nor rehashed: their original recorded SHA256 values and present file sizes are reported distinctly.

The original fixture records retain complete client reports, 591,242 successful point calls and 65,420 successful batch calls, no other measured outcomes, four exited lifetimes per fixture, cleanup and tmpfs-retention completion. Those are readbacks of existing records, not a repeated full fixture/data audit. Point's original accepted profile remains intact. Batch's missing prefix and original decoder exit 1 remain authoritative; no whole-profile acceptance or new timing claim follows from this passive diagnosis.
