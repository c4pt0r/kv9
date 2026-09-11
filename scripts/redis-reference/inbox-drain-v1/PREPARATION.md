# Inbox-vector reuse: predeclared matched screen

Protocol `kv9-inbox-drain-c1-c64-v1` compares direct parent `57ff6851` with
candidate `c3131800`. The general baseline remains `5ee897a`; calling the
parent the experiment's control does not promote it. Fixed measurement clients
remain native `03c1c776` and Redis `b8ec38f`, with their retained standalone
release binaries. The candidate's build workload is not used for timing.

The six existing arms are old/new point GET and BatchGet(1), Redis GET and
MGET(1). Each repetition visits both c1 and c64. Four repetitions use
forward/reverse/reverse/forward order over the entire twelve-cell list, with
10,000-ms read-only closed-loop windows: exactly 48 timed cohorts. Report all
sixteen old/new QPS/mean/p99 pairs and all sixteen Redis controls. Do not average
percentiles, invent a post-hoc tolerance, discard an unfavorable repetition or
infer statistical significance/no regression from pooled point estimates.

This study changes source/build pins, paths and protocol identity from the
read-credit screen; measurement duration, repeated schedule and their derived
exact population checks change as declared here. All original outcome,
histogram, attempt, resource, CPU, source, lifetime, retention, drain, writer
binding and exact isolation-restoration predicates remain. The expected full
inventory is 160 exited owned lifetimes and 96 qualifying drains/writer bindings.
The existing 90-second observation bound is unchanged.

The twelve-cell correctness-only smoke remains one forward pass at 5,000 ms,
on background CPUs, and cannot pass the timing auditor. No smoke result enters
the performance table. Final helper hashes and source preflight are recorded
before timing; pure driver and independent auditor contracts must pass first.

The workload remains 4,096 keys plus a sentinel, 128-byte values, batch size
one, 128 configured warmup calls, a ten-million-call cap, seed 71 and 1,500-ms
deadlines. Public admission is 64 requests / 16 MiB, asynchronous reads 128,
and SDK in-flight capacity equals concurrency. Configured retry capacity is
unchanged, while timing acceptance requires one actual attempt per call.

Redis 7.0.15 remains standalone, save/AOF off, one I/O thread, one preconnected
connection per worker and one outstanding command per connection, no pipeline.
KV9 retains three voters, normal sync calls and volatile tmpfs WAL. Clients
use CPUs 0-1, voters share 2-5, and observers plus the three owned background
containers use 6-15,22-31. Unrelated host services remain unconstrained. This
shared-host diagnostic establishes neither equal durability nor sustained
capacity, real-NIC behavior or the production fault matrix.

No builds, tests, faults, profiles or independent audits may overlap timing.
All preparatory commands and the smoke must be terminal before launch. Root
owns the timing session and must observe its terminal result; the independent
auditor runs only after exact root exit 0. Output directories must be new and
any failure is retained without a rerun merely to obtain acceptance.

Root timing command:

```sh
env PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-15,22-31 python3 /tmp/kv9-inbox-drain-comparison-preparation/isolate-and-run.py --driver /tmp/kv9-inbox-drain-comparison-preparation/matched-driver.py --driver-arguments-file /tmp/kv9-inbox-drain-comparison-preparation/driver-arguments.json --output /tmp/kv9-inbox-drain-matched-diagnostic-attempt1
```

The final audit invocation substitutes the observed terminal root session ID
in the recorded command and requests exactly 48 successful arms. Source gates
already passed 218 Raft tests and Clippy; independently checked process E2E
passed 364 operations. Neither that process gate nor this timing screen is
candidate-specific Chaos acceptance. No hosted CI is triggered.
