# CRC32 table: broad version 3 workload preparation

Protocol: `kv9-crc-broad-workloads-c1-c64-v1`. This refreshes actual point GET/PUT and
64-key BatchGet/BatchPut at 0/50/100 percent reads. It compares the unchanged
general baseline `5ee897a`, CRC table candidate `ca0002c7` and standalone
Redis. Both native and Redis measurement clients are clean `0be806d9` releases.
No server or client implementation is changed for this recording.

The exact protocol and independent descriptors specify c1/c64, 4,096 keys plus
sentinel, 128-byte values, seed 71, 128 warmup calls, 1,500-ms deadline,
ten-million-call cap and closed-loop traffic. SDK max attempts stays six;
all outcome and attempt populations are retained. Healthy one-attempt results
are distinguished from accounting-valid cohorts with failures.

The first correctness smoke covers one forward list of all 36 cells, each for
two seconds. It is excluded from performance selection. After reviewing smoke,
the timed matrix covers the full 36-cell list followed by its exact reverse,
ten seconds per cohort. There is no per-cell retry or discarded unfavorable
result. If a validation or resource guard fails, preserve the first partial
recording and diagnose before deciding a new protocol.

Timed clients use CPUs 0-1; all three KV9 voters, or the Redis server, share
2-5. Helpers and three exact owned background containers use 6-15,22-31.
Smoke clients use 6-7 and servers 8-15,22-31. The unchanged outer wrapper
restores the owned containers' original configured/effective 0-31 masks and
preserves historical namespace and fault identities. The host is shared and
unrelated services remain unconstrained. No build, test, fault, profile or
independent audit may overlap timed execution.

KV9 retains normal quorum and sync calls on tmpfs; Redis has no replicas,
save/AOF disabled, one I/O thread and one outstanding command per connection.
No pipeline, equivalent durability, real-NIC capacity, sustained-load or
production-readiness claim follows from this diagnostic.

The preserved lifecycle, source/build, PID/start/boot, listener, allocator,
fresh-drain, complete raw-retention and resource checks remain in the driver
and independent auditor. Exact selected v3 APIs and deterministic final
value/write-key membership replace the previous pure-read nonce-zero scope.
The actual issued nonce set and full linearizable histories are not recorded;
final scans are not substituted for those independent correctness gates.

Batch results report calls/s and input keys/s separately. Whole-call means and
p50/p95/p99 are never divided by batch size. Raw histograms preserve each
operation and outcome, including inactive populations. Pooled means are
weighted by calls; quantiles use merged counts, not averaged percentiles.

Space preflight requires 32 GiB free tmpfs and 96 GiB free retention storage.
The common resource sampler records both filesystems for every arm and stops
on observation failure or free space below 16/64 GiB. The inherited tmpfs
fixture's 32-GiB startup and 8-GiB snapshot checks also remain. These guards
do not prove a worst-case resource bound from the ten-million-call cap.
Smoke WAL growth informs operational release of the fixed timed recording;
extrapolation must remain explicitly conditional and cannot certify capacity.

This directory owns a pin/path derivative of the accepted v3 helper set. The
old control remains `5ee897a`; the only new server role is clean `ca0002c7`.
`ancestry-first.json` and each `*.diff` preserve the exact predecessor bytes
and changes. All 20 auditor functions are byte-identical. Of 27 driver
functions, only `main` changes its candidate source/build path defaults.
The wrapper is byte-identical. The original seven driver and fifteen auditor
contracts passed on their first attempts, with PID, exit and log evidence.
`static-readback-first.json` records independent descriptor agreement and
actual source/build identity checks (584/590/581/581 source files).

Root owns all process, cluster, fault, build and timing execution. No fixture
or actual audit has run from this directory. Preparation readiness does not
complete the separate exact-source correctness, recovery or Chaos gates.
Smoke remains separate from timing; no cohort is discarded or rerun to green.

`input-layout.json` describes the unchanged raw and audit layout. The
independent audit writes the original absolute-path input-inventory map and
binds all 288 existing native endpoint snapshots. Endpoint semantics are a
separate subsequent readback, not an additive latency partition here.
Initialization may have safe routing retries. Preserve actual phase counts;
do not impose a new all-phase single-attempt freeze check. The predecessor's
retained ancillary orchestration failure remains in its original directory;
none of those old outputs is copied or relabeled as new acceptance.

## Root commands after review

All outputs must be absent before their first launch. `commands.json` retains
these exact argv arrays. First run only the correctness smoke:

```sh
env PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-15,22-31 /usr/bin/python3 /tmp/kv9-crc-broad-workloads-preparation/matched-driver.py --output /tmp/kv9-crc-broad-workloads-smoke-first --smoke --client-source /tmp/kv9-point-write-measurement-v3 --client-build /tmp/kv9-point-write-v3-release-first/native --expected-client-revision 0be806d9671e2c50701a64aa7889c8859b7648ba --old-server-source /tmp/kv9-rpc-worker-pair --old-server-build /tmp/kv9-rpc-pair-release-first --new-server-source /tmp/kv9-wal-crc32-table --new-server-build /tmp/kv9-wal-crc32-release-first --redis-client-source /tmp/kv9-point-write-measurement-v3 --redis-client-build /tmp/kv9-point-write-v3-release-first/redis --storage tmpfs
```

After the smoke, source/recovery gates and quiet-window release, root may run
the predeclared timed recording through the unchanged isolation wrapper:

```sh
env PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-15,22-31 /usr/bin/python3 /tmp/kv9-crc-broad-workloads-preparation/isolate-and-run.py --driver /tmp/kv9-crc-broad-workloads-preparation/matched-driver.py --output /tmp/kv9-crc-broad-workloads-timing-first --driver-arguments-file /tmp/kv9-crc-broad-workloads-preparation/driver-arguments.json
```

Only after root confirms the actual timing session is terminal exit 0, replace
`ROOT_TERMINAL_TIMING_SESSION` with that real session and run the frozen auditor:

```sh
env PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-15,22-31 /usr/bin/python3 /tmp/kv9-crc-broad-workloads-preparation/audit.py --run /tmp/kv9-crc-broad-workloads-timing-first/cohorts --output /tmp/kv9-crc-broad-workloads-preparation/results-first --expected-driver-sha256 64c4df98a1191c6bd1605783dc6a206571cd5a590951296f5fc56e39aba654ca --root-session ROOT_TERMINAL_TIMING_SESSION --root-exit-code 0 --successful-arms 72
```

The audit rejects smoke and partial/wrong schedules. Preserve any first
failure and original output; no helper/predicate adjustment during a run.
