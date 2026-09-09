# First release benchmark validation

On 2026-09-09, the fixed [version-1 measurement protocol](BENCHMARKS.md) completed
all **54 performance trials and 18 complete-history guards** using the clean
release revision `56e8d7f75774d63ff0daff7540b61c6f8d025841`. This document is a later
publication of those measurements; it does not relabel the measured executable
as a subsequent documentation commit.

All 252 source hashes match that commit. The actual workload, production database
and explicitly non-durable calibration executables were retained and checked
against `/proc/PID/exe`. The independent checker accepted the complete matrix
again, and all 11 isolated corrupt-evidence controls were rejected for their
expected reasons; the original matrix was revalidated after every control.

The measured database population was **6,716 operations**: 3,594 using WAL and
3,122 using MinIO. The separate in-memory calibration completed 2,196,536 measured
operations. All three populations had zero unsuccessful measured outcomes in
this experiment. These are short healthy-load measurements, not fault-run error
rates. Full independent histories exist for the surrounding guards only.

## Configuration and retained evidence

- AMD Ryzen 9 9950X, 32 logical CPUs; 129,445,456 KiB host RAM reported by `/proc`.
- Linux `6.17.0-19-generic`, ext4 on a Samsung SSD 990 PRO 2TB; Docker `29.1.2`.
- Rust `1.94.0`, optimized release profile, default database features.
- Clients on logical CPUs 0–1; the three voters and MinIO share CPUs 2–5.
  The host was not reserved exclusively; placement is not physical-host isolation.
- Loopback networking, three independent voter processes, 8 mutable keys plus a
  sentinel, 128-byte values, 32 acknowledged warmup operations and 5-second
  measurement admission followed by drain. Three repetitions use seeds 40–42.
- Durable write acknowledgements, quorum reads, public admission limits and the
  100-ms MinIO flush interval are unchanged. The MinIO image is pinned by digest
  in the protocol; all three voters retained remote checkpoint descriptors.

The [per-trial logical population CSV](benchmarks/2026-09-09-56e8d7f.csv) publishes
all 54 trial rates and 90 nonempty operation/outcome latency populations, with
exact cohort/count/sum/extrema values, histogram percentile bounds and raw report
hashes. Repeated trial-level throughput columns describe the same trial; do not
sum them across its Get/Put/Delete rows. Empty outcomes remain in the full raw
reports. Attempt-level histograms and every server snapshot are also retained
there; the CSV is an extract, not a replacement verifier input.

Local evidence paths on the development host:

| Evidence | Path |
| --- | --- |
| Retained release build and complete source inventory | `/tmp/kv9-benchmark-build-release-1` |
| Every raw trial, history, checkpoint descriptor, host inventory and matrix report | `/tmp/kv9-benchmark-measurement-1` |
| Independent full-matrix result | `/tmp/kv9-benchmark-measurement-1-independent.json` |
| Eleven corrupted copies and original revalidation records | `/tmp/kv9-benchmark-measurement-controls-1` |

These local paths are not downloadable GitHub artifacts. Hosted CI separately
publishes the shorter debug smoke matrix and its raw evidence. Exact-revision
hosted acceptance links are maintained on #40 and #9.

| Artifact | SHA-256 |
| --- | --- |
| Build manifest | `8466e3c44c0f483a3f8bbb4e42441c8a464c582f9b7258d0199fb6f64484b9c3` |
| Protocol | `cdded1950f2b43bbdd8e91e2cceb6856d47b5e811d0f995afcaf559a231ff949` |
| Host inventory | `0d41f99a03ad8b41d6dbf17d0757e404d466276709262e2bdcad54c2c1606541` |
| Full matrix report | `3b889686cd04a00fe74288315d74994119d2f12a7c75cffa3dac45dce94a1a84` |
| Published population CSV | `e422c41b2240a3f2ca48f33540a2793d6cc1d3bc342ae82bdb14d9a4c7f7f980` |

## Observed successful operations per second

Every number below is one complete trial's successful cohort rate, including
its drain. The table does not average percentiles or present a maximum capacity.

| Target | Mix | Concurrency | Repeat 1 | Repeat 2 | Repeat 3 |
| --- | --- | ---: | ---: | ---: | ---: |
| wal | read | 1 | 24.98 | 24.95 | 24.95 |
| wal | read | 4 | 97.63 | 97.22 | 97.42 |
| wal | write | 1 | 17.43 | 16.28 | 15.27 |
| wal | write | 4 | 29.89 | 30.27 | 30.17 |
| wal | mixed | 1 | 19.74 | 20.07 | 19.52 |
| wal | mixed | 4 | 47.89 | 47.50 | 48.17 |
| minio | read | 1 | 24.95 | 24.95 | 24.95 |
| minio | read | 4 | 96.05 | 97.81 | 96.25 |
| minio | write | 1 | 10.90 | 10.40 | 10.50 |
| minio | write | 4 | 22.84 | 22.13 | 22.18 |
| minio | mixed | 1 | 15.76 | 15.77 | 15.98 |
| minio | mixed | 4 | 34.88 | 34.80 | 35.64 |
| loopback | read | 1 | 53219.91 | 52833.31 | 53196.37 |
| loopback | read | 4 | 12465.16 | 9821.14 | 10848.83 |
| loopback | write | 1 | 23368.86 | 22102.48 | 22257.52 |
| loopback | write | 4 | 9419.55 | 8556.37 | 8829.27 |
| loopback | mixed | 1 | 40506.72 | 41893.42 | 41398.24 |
| loopback | mixed | 4 | 8311.13 | 11107.37 | 8469.73 |

## Interpretation and next work

One-client database reads had measured logical means of 40.04–40.07 ms. Their
p50/p95/p99 histogram interval was 33.55–67.11 ms; the histogram cannot resolve an
exact percentile inside that interval. With four clients, read p99 was in
67.11–134.22 ms. WAL writes with four clients averaged 131.90–133.27 ms; MinIO
writes averaged 174.76–180.07 ms. Per-trial percentile intervals are in the CSV,
including the higher final MinIO write p99 bucket.

Fresh whole-lifecycle server snapshots locate most read time inside quorum-read
establishment. The current driver pumps inbound/Ready work and advances logical
time together, then sleeps 20 ms; outbound transport also batches for up to
2 ms. This makes scheduling a concrete hypothesis to test. The snapshots include
setup, warmup, drain and verification; their means must not be labeled as
measurement-only client latency. Fsync and background flush costs also remain
visible. These data do not yet isolate the causal contribution of each wait.

The calibration minimum divided by the corresponding database maximum was at
least 100.4 in every matrix cell. The client process consumed only 0.01–0.04 CPU
seconds over each complete database trial lifecycle. This supports observed
headroom in this configuration. It does not establish isolated client capacity:
the calibration has one endpoint and an in-memory mutex, and four-client
calibration was slower and more variable than one-client calibration. Shared
host scheduling, transport and recorder costs can all contribute.

[#41](https://github.com/c4pt0r/kv9/issues/41) defines the next development path:
measure enqueue-to-pump waits, specify bounded event-driven scheduling and
independent tick deadlines, prove wakeup/tick/failure invariants with an explicit
Rust refinement record, exercise deterministic race controls, then repeat real
Chaos Mesh and paired release measurements. Persistence ordering and quorum reads
remain mandatory. Group commit and storage-layout changes retain their separate
proof and crash-recovery requirements.

#13 remains open for larger datasets, cache/hotspot and recovery matrices,
complete remote I/O accounting, longer trials and independent physical failure
domains. These short, single-host results are not industrial capacity acceptance.

## Earlier failed experiments and regression checks

The first debug smoke completed its workload processes but failed matrix
validation because the new parser expected `applied_term/index` for a legacy
keyspace-creation receipt. The actual CLI uses `proposed_term/index`; the
validator now checks those acknowledged fields. That original run remains failed
at `/tmp/kv9-benchmark-smoke-1`.

Its diagnostic recheck also exposed an invalid assumption that Linux's reported
`VmHWM` must never decrease: one same-process sample fell by 20 KiB. The raw bytes
were retained. The validator now preserves both approximate memory samples and
flags a decrease; CPU counters and process identity remain strict. The kernel
accounting limitation and sources are documented in [BENCHMARKS.md](BENCHMARKS.md).
The subsequent debug smoke passed all 36 trials and 12 guards, followed by the
clean release experiment above; diagnostic copies are not counted as new trials.

The implementation also passed 496 workspace/doc tests, warning-denying Clippy,
formatting, workflow lint, and the existing 12 corrupt workload-report controls.
No core database or retry algorithm changed in this increment. Existing
metadata/Ready/client proof and actual Chaos Mesh gates remain required at the
pushed revision; a local performance pass does not close those gates.
