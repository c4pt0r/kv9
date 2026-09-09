# Latency observation validation

Implementation contract: [LATENCY-OBSERVABILITY.md](LATENCY-OBSERVABILITY.md).
Acceptance and exact-revision hosted artifact links are tracked on
[#39](https://github.com/c4pt0r/kv9/issues/39) and
[#9](https://github.com/c4pt0r/kv9/issues/9). A local result is not a substitute
for the required hosted checks, and this increment does not complete C03.

## Local source checks

The implementation-local workspace run passed 455 ordinary tests and 20 doc
tests. The 21 ignored MinIO tests are exercised by the separate hosted MinIO
gates. Formatting, warning-denying Clippy, actionlint and shell/Python parsing
also passed. The unchanged Lean inventory checked 24 declarations and rejected
all nine invalid controls.

Three new isolated Rust source controls each retained a passing baseline, the
intended failing assertion, and a passing restored source: queue time included
in backend duration, a replaced receipt reported as a successful apply, and a
missing actual-fsync observation. The three admission source controls also
passed all nine baseline/mutant/restored runs after adding cancellation timing
assertions. Source hashes were checked against the final source files.

New and extended behavioral tests cover all bucket boundaries and quantile
ranks; independent overflow flags; observer poisoning and result/unwind
preservation; preparation, queue, execution, unsubmitted release and live/queued
RPC cancellation; exact receipt outcomes; stable apply-lag observations; real
engine write and fsync errors; the Raft before/after write/sync EIO/ENOSPC matrix
and short-write path; WAL replacement observer lifetime; fixed export size; and
successful driver progress after metrics publication failure.

## Actual local Chaos Mesh

The complete run `/tmp/kv9-chaos-e2e.zFVZpe` passed the existing 13 fault/history
windows, all six Raft write EIO/ENOSPC cells, all 42 retained routing probe
attempts, and the new metric checks. The unchanged independent checker accepted
the complete 1,896-operation history: 1,680 successful, 11 proven refused, and
205 unknown. The witness was independently replayed; unknown operations were
retained as unknown. The first search strategy was inconclusive; the existing
unrestricted guided strategy found a legal witness, using 4,443 total search
states in about 2.11 seconds.

During the actual leader partition, the persistent-channel pressure fixture
issued 3,969 reads: 481 successful missing-key reads, 3,487 count refusals and one
oversized-request refusal. Admission peaked at eight requests and 352 encoded
bytes and drained. The serving node's backend success histogram increased by
494 samples, covering the 481 fixture successes plus overlapping traffic. Queue,
backend and ReadIndex durations increased in the retained snapshots. Each of the
six stopped I/O-fault processes recorded exactly one failed Raft record write
and no fabricated record fsync failure; fresh recovered processes reset counters
and recorded successful durability. Three independently invalid metric evidence
controls were rejected.

This was a three-replica, single-host Kind deployment with actual Chaos Mesh,
including the existing FUSE/no-statx compatibility fixture. It establishes the
specified fault effects and observations, not cross-machine throughput or
independence from a shared physical host failure.

## Failed local experiment retained

The earlier `/tmp/kv9-chaos-e2e.JjYjYc` run failed after the first I/O recovery.
Bash dynamic scope let `wait_until`'s descriptive `label` shadow the fault-cell
label, so the helper saved the valid recovered snapshot under the wrong filename.
The original artifacts were retained. A separate diagnostic copy of that raw
snapshot passed the recovery check; the incomplete matrix was not accepted.
The helper now receives the cell explicitly, with a regression executing it
under a shadowed wait label. The second run above reran the entire matrix.

The earlier export timestamp names were also clarified before the complete run:
exporter construction is not OS process start, and initialization can record WAL
samples before the exporter exists.

## Release experiment and hosted acceptance

`scripts/observer-overhead.py` retains 42 release trials, exact commit/source
hashes, compiler/profile, CPU/affinity, raw measurements and variation. Its output
is observer overhead, not server QPS or client latency. The CI workflow publishes
this evidence together with the three latency source controls. The Correctness
workflow additionally requires the metric fault marker and retains every raw
Chaos snapshot beside the independent history. Hosted acceptance must audit
these artifacts, real MinIO coverage, and the unchanged TLA+/TLAPS/Lean gates
against the pushed revision before closing #39.
