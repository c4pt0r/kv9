# Owned apply buffer: matched workload preparation

Protocol `kv9-owned-batch-broad-workloads-c1-c64-v1` compares accepted CRC
`ca0002c7f8e9ee6f595efcc9f4151085ccce87cb` (old) with owned-buffer candidate
`9be0c1963515ff974426faadef80482b847b13a5` (new). Both are the original clean,
standalone default Cargo release builds. Native and Redis measurement clients
remain the exact `0be806d9671e2c50701a64aa7889c8859b7648ba` releases.

The accepted CRC broad driver/auditor and statistics reader are preserved in
their original directories. Per-file diffs and `ancestry-first.json` disclose
all substitutions. Only control/candidate pins, owned paths and protocol
identity change; statistics additionally changes its descriptive title.
All 20 auditor functions and the statistics arithmetic are byte-identical.
Of 27 driver functions, only `main` changes source/build path defaults.
The isolation wrapper is byte-identical. No acceptance predicate changes.

There are 36 correctness-only smoke cells at 2 seconds, followed separately
by 72 timed cohorts at 10 seconds: point size 1 / batch size 64, 0/50/100 percent
reads, c1/c64, old/new/Redis, one whole forward and one whole reverse repeat.
Both actual inventories equal the independent descriptor oracle and the
accepted CRC predecessor. Every run keeps 4,096 keys plus sentinel, 128-byte
values, seed 71, 128 warmup calls, closed loop, 10M cap and 1,500-ms deadline.
SDK maximum attempts stays six. Original outcomes and attempts are retained;
healthy single-attempt suitability is separate from accounting validity.
Initialization may have safe routing retries; do not impose all-phase
single-attempt checks. No cell is discarded or retried to obtain a pass.

Timed clients use CPUs 0-1 and servers 2-5; helpers and three exact owned
background containers use 6-15,22-31. Smoke clients use 6-7 and servers
8-15,22-31. The inherited wrapper requires exact configured/effective 0-31
restoration and preserves historical namespaces/fault identities. Root owns
runtime execution, storage release and quiet-window coordination. No build,
test, audit, profile or fault may overlap timing. Other shared-host services
remain unconstrained.

The unchanged storage preflight requires free tmpfs/retention of 32/96 GiB;
observed runtime thresholds are 16/64 GiB, with the inherited 8-GiB snapshot
guard. Observation failure or a guard trip retains the failed partial run.
Neither the call cap nor smoke growth proves a worst-case storage bound.

All source/Cargo/binary, PID/start/boot/listener, CPU/resource/allocator,
fresh empty drain, deterministic data/sentinel and complete retention/cleanup
checks remain. Timed acceptance expects 240 managed lifetimes, 144 qualifying
voter drains/listener bindings and 288 existing endpoint snapshots. The audit
keeps the original absolute-path -> {bytes, sha256} input inventory. Endpoint
stage semantics are independently validated later, not an additive partition.
`input-layout.json` records the unchanged evidence layout.

The statistics reader retains all phases, per-operation and combined
whole-call histograms, both repeats, all outcomes, input-key rates and sampled
CPU scope. Pooled rates divide summed counts by summed cohort elapsed time;
means use summed latency/count; quantiles merge raw buckets, never averaged
percentiles or batch latency divided by keys. The statistics core is exactly
`827110ec99b2ae82f042e40fc81844263eefe4c484680ef6d8ab54653515b17f`.

This is preparation only. The original seven driver and fifteen auditor
contracts passed once on background CPUs; actual role inventories verified
590/592/581/581 source files. No workload, Cargo, fault, actual audit or
statistics derivation ran here. Candidate correctness/recovery/Chaos gates
are separate. This shared-host tmpfs quorum-WAL versus standalone memory
Redis diagnostic does not establish equal durability, sustained capacity,
statistical significance or a speedup. Dataset checks cover deterministic
bytes, configured nonce bounds and write-key membership, not a full issued
nonce ledger or linearizable history.

## Root commands after review

All output paths must be absent. `commands.json` retains exact argv arrays.
Run the correctness-only smoke first:

```sh
env PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-15,22-31 /usr/bin/python3 /tmp/kv9-owned-batch-broad-workloads-preparation/matched-driver.py --output /tmp/kv9-owned-batch-broad-workloads-smoke-first --smoke --client-source /tmp/kv9-point-write-measurement-v3 --client-build /tmp/kv9-point-write-v3-release-first/native --expected-client-revision 0be806d9671e2c50701a64aa7889c8859b7648ba --old-server-source /tmp/kv9-wal-crc32-table --old-server-build /tmp/kv9-wal-crc32-release-first --new-server-source /tmp/kv9-owned-apply-batch --new-server-build /tmp/kv9-owned-batch-release-first --redis-client-source /tmp/kv9-point-write-measurement-v3 --redis-client-build /tmp/kv9-point-write-v3-release-first/redis --storage tmpfs
```

After reviewing smoke and separate correctness/recovery/Chaos completion,
release the quiet window and run the predeclared timed matrix:

```sh
env PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-15,22-31 /usr/bin/python3 /tmp/kv9-owned-batch-broad-workloads-preparation/isolate-and-run.py --driver /tmp/kv9-owned-batch-broad-workloads-preparation/matched-driver.py --output /tmp/kv9-owned-batch-broad-workloads-timing-first --driver-arguments-file /tmp/kv9-owned-batch-broad-workloads-preparation/driver-arguments.json
```

After root confirms that exact timing session is terminal exit 0, replace the
placeholder with the actual session and run the frozen independent auditor:

```sh
env PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-15,22-31 /usr/bin/python3 /tmp/kv9-owned-batch-broad-workloads-preparation/audit.py --run /tmp/kv9-owned-batch-broad-workloads-timing-first/cohorts --output /tmp/kv9-owned-batch-broad-workloads-preparation/results-first --expected-driver-sha256 ec2203614200eabfb3aa4f423b66cd511c36fc664854fb767bf82e8293527e77 --root-session ROOT_TERMINAL_TIMING_SESSION --root-exit-code 0 --successful-arms 72
```

After its accepted result, root retains the real audit terminal receipt at
`/tmp/kv9-owned-batch-broad-root-preparation/audit-terminal-first.json`, with
`terminal`, `exit_code`, `timing_session`, `audit_session`, and result hash.
No such receipt has been fabricated by this preparation. The reader requires
successful terminal state and the same timing session as the accepted audit.
Then derive statistics into this preparation's fresh `statistics/` outputs:

```sh
env PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-15,22-31 /usr/bin/python3 /tmp/kv9-owned-batch-broad-workloads-preparation/statistics/derive.py
```

First failures and original outputs remain; no automatic rerun or relaxation.
