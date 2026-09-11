# Longer c1 comparison: predeclared protocol

This distinct study resolves the small low-concurrency point-GET regression
observed in the completed `kv9-read-group-credit-c1-c64-v1` screen. That first
recording, its c64 gains and its c1 regressions remain retained. No recording
will be discarded to obtain a preferred verdict.

Use exactly the same old `5ee897a` and candidate `57ff685` default releases,
native `03c1c776` client and Redis `b8ec38f` client. Public admission, quorum,
apply, durability, payload, connection behavior and resource placement are
unchanged. Only measurement duration and the repeated c1 schedule differ.

Protocol `kv9-read-credit-c1-long-v1` contains 24 cohorts: c1 only, the six
existing GET/BatchGet(1)/Redis GET/MGET(1) arms, four 30-second repetitions,
forward/reverse/reverse/forward. Run IDs remain six characters. The ten-million
call cap, 128 configured warmup calls, 4,096 keys plus sentinel, 128-byte values,
1,500-ms deadlines and one outstanding command per worker remain unchanged.
The original 90-second per-client observation bound covers each 30-second
measurement without modification. Redis does not pipeline.

Report all four paired QPS, mean and p99 observations, all outcomes and both
Redis controls. Do not average percentiles or invent a post-hoc tolerance
threshold. Longer measurements do not by themselves prove a negligible effect,
statistical significance, no regression or sustained service capacity. General
selection must account for the observed c1 cost, c64 benefits and fault gates.

The new driver and independent auditor preserve all runtime outcome, histogram,
source/build, memory/resource, lifetime, retention, drain and exact isolation
restoration predicates. Six driver and twelve independent auditor contracts
pass. Static comparison matches the actual driver, independent auditor, test
fixture and frozen protocol across all 24 descriptors. Source preflight checks
584 control, 594 candidate, 579 native-client and 580 Redis-client files.

The prior twelve-cell correctness smoke passed on these exact releases. No
additional runtime smoke is required for a duration/schedule-only study after
pure configuration validation. The optional smoke path remains correctness
only and cannot pass the timing auditor. No build, test, profile, fault or
independent audit may overlap timing. Wait for the delegated exact-source
Chaos task to be terminal and its fixture workloads/faults to be cleaned.

Root timing command:

```sh
env PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-15,22-31 python3 /tmp/kv9-read-credit-c1-long-preparation/isolate-and-run.py --driver /tmp/kv9-read-credit-c1-long-preparation/matched-driver.py --driver-arguments-file /tmp/kv9-read-credit-c1-long-preparation/driver-arguments.json --output /tmp/kv9-read-credit-c1-long-diagnostic-attempt1
```

After that exact root session is terminal exit 0, substitute its observed
session ID for `ROOT_TERMINAL_TIMING_SESSION`:

```sh
env PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-15,22-31 python3 /tmp/kv9-read-credit-c1-long-preparation/audit.py --run /tmp/kv9-read-credit-c1-long-diagnostic-attempt1/cohorts --output /tmp/kv9-read-credit-c1-long-preparation/results-first --expected-driver-sha256 a626622a706901bf4ccdab4f09257b4a9c787fcc0391945e94c99e136994409f --root-session ROOT_TERMINAL_TIMING_SESSION --root-exit-code 0 --successful-arms 24
```

All output paths must be fresh. `AUDITOR-PREPARATION.json` preserves complete
auditor/test diffs and the scope rationale. Root driver/test diffs and original
input hashes are separate files. The driver contract command passed in root
tool chunk `80a78b`; its metadata explicitly records that no redirected log
was created. No test was repeated to manufacture one. All historical inputs
and failed attempts remain available in their original directories.
