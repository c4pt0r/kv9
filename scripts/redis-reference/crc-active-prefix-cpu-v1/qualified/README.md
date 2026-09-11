# CRC write profiles with an active prefix — preparation only

This is a new, unexecuted derivative of `/tmp/kv9-crc-write-profile-first`. That original attempt remains intact: runtime exited 0, Point PUT accepted 3,324 selected samples, and BatchPut(64) failed because its first sample was 6.426334 ms after measurement start. No result from that failure is relabelled.

`analyze.py` is byte-identical to the original, SHA256 `c188fd8c648100dee311923bb042dcff488098592eda8230ed5929bcbe958df4`. It retains strict nominal start/end containment, wall/monotonic anchor bounds, one-millisecond exclusions, sample-count and aggregate 32-bin/edge checks, same-PID scope, cap, lost-sample and all original workload/data/cleanup checks. The recorder remains 20 seconds/128 MiB, cpu-clock 199 Hz with DWARF 16 KiB. The start is never trimmed to the first sample.

## Concrete active prefix

After the existing perf process check and 250 ms delay, `active_prefix.py` performs **128 paced CLI RawGet invocations over one second**, at most eight concurrent. The CLI executable is the exact retained CRC server binary; every request addresses the already-created native keyspace and an absent key `__profile_prefix__:<run_id>` outside the measured dataset prefix. The fixed CLI calls `RawClient::get` once through unary gRPC, with no wrapper replay. Success requires the exact miss response `found=false`; a hit, error, refusal, output overflow, late completion or missing dispatch is retained and aborts measurement launch.

Every invocation retains argv, scheduled/start/deadline/completion times, stdout/stderr bytes and hashes, exact PID/start/boot, executable device/inode and placement, outcome and terminal absence. The binary bytes are hashed before and after; every observed `/proc/PID/exe` inode must match that retained immutable binary. The same three perf-target voter lifetimes are bound before/after. Calls have a 1,500 ms external process deadline, one outstanding RPC per CLI, and no retries; the successful prefix must fit four seconds. These external process deadlines are not fabricated gRPC wire deadlines. The CLI API has no deadline flag, and a timed-out read remains a failed priming attempt.

The event loop owns each child before executable inspection can fail. Its `finally` terminates/reaps all remaining owned CLI processes and records errors. Failure cleanup is separately bounded per child (250 ms TERM grace then one-second KILL wait, maximum eight outstanding children); it can exceed the successful four-second prefix bound. It does not signal voters or unrelated processes. An interrupted or failed prefix aborts this attempt and keeps the existing outer fixture/profiler cleanup active. No measurement begins after a prefix failure.

After successful CLI exit, the adapter calls the **existing fresh two-export empty Serving drain** with label `post-prefix`. It then checks perf is still alive and that less than 12 seconds of its unchanged 20-second recording budget have elapsed. Original nominal coverage remains mandatory after recording. An active read phase, CPU counters or a live perf process alone are not proof of a recorded prefix sample; only the unchanged decoder's actual sample bounds establish it. A new failure is retained without retries or relaxed predicates.

The priming clients/harness stay on CPUs 6–15,22–31. Measured clients remain 0–1; measured voters remain 2–5. No background container isolation or exclusive-host claim is added. Priming uses actual requests on the existing leader; it does not require idle followers to have dense sample coverage.

## Fixed measurement and disclosed bias

The server remains CRC `ca0002c7f8e9ee6f595efcc9f4151085ccce87cb`, retained server SHA256 `b33b5d302f901aac5cf6a95449b9bb5dd68473747e011220dbed7aed2e591d13`. The measurement client remains `0be806d9671e2c50701a64aa7889c8859b7648ba`, SHA256 `8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957`.

Both original measurement descriptors are unchanged: point PUT/BatchPut64, 64 workers, 4,096 keys plus sentinel, 128-byte values, seed 71, 128 warmup calls, five seconds, 10M cap, 1,500 ms SDK deadline and six configured maximum attempts. Fresh dataset initialization, deterministic values, reports and final readback remain with the original measurement client. Priming never performs a Put, Delete, keyspace creation or data mutation.

The extra read traffic changes metadata/cache/allocation and CPU state. It is separately retained and excluded from the measured profile interval, but its prior effects can persist. These are instrumented diagnostic profiles, not directly comparable throughput samples, timing acceptance or causal attribution. Full fixture endpoint deltas may include priming and cannot be labelled measurement-only. The original before/after fixture cleanup evidence still covers its four managed lifetimes; additional CLI lifetimes are explicitly retained in `active-prefix/summary.json`, rather than silently folded into that old count.

## Review and later commands

Only source reading, file authoring, exact copying and static diff/hash preparation have occurred. **No control, profiler, decoder, workload, model, Cargo or acceptance audit has run.** `test_prefix_contract.py` contains seven pure contract tests for reply/opcode/key scope, call conservation, deadlines, concurrency, replay, process/binary ownership and cleanup failures. They have not been executed. Runtime exception/kill paths also require later actual qualification; pure controls are not process evidence.

Root may later release the pure controls:

```sh
env PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-15,22-31 /usr/bin/python3 /tmp/kv9-crc-profile-active-prefix-preparation/test_prefix_contract.py
```

Only after review, controls and a new runtime coordination release:

```sh
env PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-15,22-31 /usr/bin/python3 /tmp/kv9-crc-profile-active-prefix-preparation/profile.py
```

Only after that exact runtime is terminal:

```sh
env PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-15,22-31 /usr/bin/python3 /tmp/kv9-crc-profile-active-prefix-preparation/analyze.py
```

These parameterless helpers put new runtime artifacts beside their source files in this new directory. `protocol.preview.json` is only a static design preview; the real protocol and all PID/timing/outcome fields must come from actual execution. Each command needs a first exclusive log and actual invocation/terminal receipt. Nothing here authorizes launch during root's current timing, creates a new release, or overwrites an earlier attempt.
