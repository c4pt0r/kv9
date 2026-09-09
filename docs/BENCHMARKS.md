# Reproducible single-group measurement protocol

The benchmark runs the actual persistent workload executable against three
independent database processes, and separately against an authenticated in-memory
gRPC calibration endpoint. It changes no database read barrier, acknowledgement,
WAL sync or checkpoint algorithm. Existing protocol proofs and real Chaos Mesh
acceptance remain mandatory; this experiment itself does not inject faults.

The first clean release matrix, raw population extract and measured limitations
are recorded in [BENCHMARK-VALIDATION.md](BENCHMARK-VALIDATION.md).

## Build and reproduce

Build all retained executables from one unchanged source inventory:

```sh
python3 scripts/build-benchmark.py --output /tmp/kv9-benchmark-build
python3 scripts/benchmark.py --build /tmp/kv9-benchmark-build \
  --output /tmp/kv9-benchmark-measurement
python3 scripts/check-benchmark.py /tmp/kv9-benchmark-measurement \
  --build /tmp/kv9-benchmark-build --output /tmp/kv9-benchmark-checked.json
```

The default build uses `--release`. The builder retains the production `kv9`
binary, the real `kv9-workload` binary and the explicitly named
`workload-loopback` example. Database and example builds are separate commands;
the database build never enables test-only features. The manifests bind binary
hashes, compiler/build commands and the complete source inventory. The harness
also hashes `/proc/PID/exe` for each actual database, client and calibration
process and records its identity and CPU affinity. These are trusted-builder and
trusted-host observations, not compiler attestation.

Use `--expected-revision COMMIT` on the runner/checker to require an exact clean
revision. Every build/run/checker-output directory must be new. Failed attempts
remain in their directories; the runner never replaces them with a retry. A
partial run or missing trial cannot produce an accepted matrix.

Linux, Docker, the existing Rust build dependencies, `lscpu`, `lsblk` and
`findmnt` are required. The runner creates uniquely named MinIO containers and
owns all spawned child processes. It removes only its own containers/processes;
no Kubernetes context or existing database is used. MinIO credentials are generated
per fixture, passed through a temporary mode-0600 file/environment, removed from
the filesystem and excluded from command/report contents.

## Fixed version-1 matrix

| Dimension | Required values |
| --- | --- |
| Targets | Three voters using local WALs; three voters using real pinned MinIO; separate in-memory loopback calibration |
| Workloads | 100% Get; 100% Put; 50% Get / 40% Put / 10% Delete |
| Logical concurrency | 1 and 4, closed loop, no pacing sleep |
| Dataset | 8 mutable keys plus immutable sentinel, 128-byte values, uniform deterministic generated keys |
| Warmup | 32 acknowledged operations, outside measurement |
| Measurement | 5 seconds of admission, then drain every issued call and verify |
| Repetitions | 3, seeds 40/41/42; reverse target/cell order on alternate repetitions |
| Client | Two runtime threads, 1.5-second logical deadline, at most 6 attempts, 5-ms retry backoff |
| Server settings | Public admission limit 64 requests / 16 MiB; MinIO flush interval 100 ms; unchanged durable writes/quorum reads |
| CPU placement | Two allowed CPUs for clients; up to four different allowed CPUs shared by the three voters and MinIO or the calibration endpoint |

A full run needs at least four allowed CPUs. The affinity masks are applied
before child execution and retained from the actual processes; MinIO receives
the same server CPU mask through Docker. This separates the experiment's client
and server CPU sets. Other host activity remains possible, and the host is not
claimed to be exclusively reserved. The selected CPU count, model, memory,
filesystem/block devices, pressure files, kernel, Docker version and loopback
network topology are retained in `host.json`.

Each target/repetition starts a fresh fixture. Each database trial creates a new
Raw keyspace and retains its acknowledged creation position under the existing
CLI names `proposed_term` / `proposed_index`. Dataset size does not grow with the
number of measured operations. Across the eight runs in a fixture, the dataset
remains far below the current checkpoint size boundary. This is not a test of
large-dataset storage capacity, cold-cache reads or region splits.

Each fixture brackets its six performance trials with complete-history mixed
workloads. The full matrix therefore has **54 performance trials and 18 complete
history guards**. Guards initialize through public requests, preserve all unknown
outcomes, verify the sentinel and must pass the independent sequential model and
witness replay. Performance trials retain complete bounded counts/histograms but
not a full history; passing the surrounding guards does not turn their absent
histories into a correctness proof.

The performance operation limit is one million, including setup/warmup/final
reads. Reaching that limit before the configured duration fails measurement
acceptance. Uncertain writes are never replayed. Failed, refused, unknown and
successful outcomes remain in the recorded populations and denominators. A
workload/process failure, missing report, incomplete matrix or inconclusive
history check fails the run and retains the failure in `index.json`.

## Client calibration and interpretation

The loopback example supports only bounded point operations in keyspace 1. It
holds at most 4,096 keys, 128-byte keys and 8,192-byte values, under one in-memory
mutex. It uses two runtime threads and emits **synthetic** term/index positions;
there is no Raft, WAL, quorum, object store or crash durability. It is not a
production service or database dependency.

Calibration uses the same retained workload executable, payload/mix/concurrency,
client CPU set and recorder modes. Its final actual request and connection counts
must agree with the client evidence and demonstrate connection reuse. The report
shows its rates next to the corresponding database rates, including the ratio
of the minimum calibration rate to the maximum database rate. This is an
observed lower bound on the combined client/fixture path under this placement,
not the client's isolated maximum capacity. The fixture can also bottleneck.
It has one configured endpoint; the real database has three and performs quorum
reads, durable writes, routing and background work. Do not subtract their
latencies or declare server saturation from the ratio alone.

## Raw evidence and independent report

Every trial retains requested/actual configuration, build manifest, client
report, any complete history, log, verification result and actual process
identity. Database trials additionally retain fresh server metrics and status
from all three voters before and after the complete client lifecycle. Those
snapshots must bracket the trial, agree on process identity, preserve admission
settings and account for at least the acknowledged read/write backend samples.
MinIO fixtures retain actual checkpoint descriptor bytes and hashes from all
three voters, alongside image/version/endpoint/CPU configuration.

The independent verifier reconstructs all expected cells and repetitions,
checks every workload report, replays each full guard history, verifies live
process/build/CPU identities, and recomputes throughput, histogram deltas and
between-trial min/median/max rates. Percentile values remain their bucket bounds;
percentiles from different trials are never averaged. Logical and attempt latency
populations stay separate, as do Get/Put/Delete and success/error/refusal outcomes.
The stored matrix summary must equal independent recomputation.

Client cohort throughput includes completions during drain and excludes setup,
warmup and final reads. Server metric deltas and CPU/I/O samples cover the
**whole trial envelope**, including those stages and export waits, and are
labeled accordingly. Their percentile intervals are computed from cumulative
histogram bucket differences; minima/maxima cannot be subtracted. Kernel database
process `write_bytes` is retained, but it excludes MinIO process I/O and is not
a total object-store or storage write-amplification measurement.

RSS and high-water RSS are the raw approximate `/proc` samples. Linux documents
asynchronous RSS accounting and inaccurate `VmRSS`/`VmHWM` values; a retained
initial experiment observed a 20-KiB decrease in reported `VmHWM` for the same
server PID/start time. The verifier preserves and flags that observation rather
than treating it as a conserved counter. CPU tick counters and process identity
remain checked. These samples cannot prove an exact memory ceiling.
See the [kernel `/proc` documentation](https://www.kernel.org/doc/html/latest/filesystems/proc.html)
and [Linux status-file manual](https://www.man7.org/linux/man-pages/man5/proc_pid_status.5.html).

## CI smoke and negative controls

```sh
python3 scripts/build-benchmark.py --debug --output /tmp/kv9-benchmark-smoke-build
python3 scripts/benchmark.py --smoke --build /tmp/kv9-benchmark-smoke-build \
  --output /tmp/kv9-benchmark-smoke
python3 scripts/check-benchmark-controls.py --evidence /tmp/kv9-benchmark-smoke \
  --build /tmp/kv9-benchmark-smoke-build --output /tmp/kv9-benchmark-controls
```

The smoke preset uses two repetitions and 500-ms performance trials: **36 trials
and 12 full-history guards**. It may share client/server CPUs on a host with fewer
than four allowed CPUs; the actual placement is explicit. It verifies machinery
and cannot be labeled a measured baseline. CI retains raw inputs/reports,
executables, independent results and corrupted evidence; node WAL directories
are excluded from the benchmark artifact because they are not verifier inputs.

Eleven isolated corruptions must fail for their specified reason: missing trial,
missing calibration fixture, premature stop, false throughput, stale server
metrics, incorrect calibration counts, false durability, false aggregate,
regressed CPU ticks, incorrect client CPU placement, and writes falsely counted
as part of a read-only trial. The original full matrix
is revalidated after every rejection. Existing workload corruption controls,
client stop/drain/unknown-write controls, protocol proofs and Chaos Mesh gates
remain required.

This first protocol produces short, reproducible single-host measurements. #13
remains open for larger datasets, cold/warm cache and hotspot matrices, complete
remote I/O accounting, recovery measurements, longer trials and multi-host fault
domains. Group commit, batching and storage-layout changes require separate
algorithm proofs and crash/Chaos acceptance informed by these measurements.
