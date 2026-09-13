# Vectored WAL matched write comparison

Updated 2026-09-12 (America/Los_Angeles). The exact vectored-WAL release now
has an executable matched write comparison and **28 passing local controls**.
All eight smoke and sixteen timed cohorts remain unrun. No vectored-WAL speedup
or default promotion is claimed.

The [original preparation and control evidence](wal-segment-vectored-ab-plan-v1/README.md)
binds the following roles:

| Role | Source | Original executable SHA-256 |
| --- | --- | --- |
| Selected server | `11113f68f6a5df77da1ffb4fcec850953716ffa3` | `dd028cb2f61633dda133b05d814a6d173a79a83145d8ced2f81dc0cf8e0f33bc` |
| Vectored server | `cfd9c927f8ecd33974100f696e6b08b227d25a41` | `c88b79b53f10f76f4bb87c4293e1dd0de0a753e13dc1cce27bd0b44d45694581` |
| Fixed native v3 client | `0be806d9671e2c50701a64aa7889c8859b7648ba` | `8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957` |

Both server roles use their original default ThinLTO releases. The timing client
is identical between roles; the candidate's correctness workload binary is a
separate prerequisite. The driver retains complete source, executable, Cargo
and feature checks before and after runtime.

## Complete protocol and acceptance

The matrix preserves the accepted CRC/frame write comparison: point Put and
BatchPut(64), concurrency 1/64, 128-byte values, 4,096 keys plus a sentinel,
seed 71, 128 warmup calls and closed-loop `tonic_stream`. Each smoke lasts two
seconds; each timed cohort lasts ten seconds. Two complete opposite orders
produce sixteen timed cohorts. The original call cap, retry policy and 1,500 ms
deadline remain unchanged. Unknown writes are not replayed.

Timed clients use CPUs 0–1, and all three voters share CPUs 2–5. Smokes use
clients 6–7 and voters 8–15,22–31. Builds, tests, proofs, profilers, faults and
codecs must finish before timing. Preserve Raft commit, synchronization, durable
apply and response fences. Tmpfs WAL remains explicitly volatile.

Every outcome, attempt, dropped call and whole-call latency histogram remains
available. Healthy one-attempt suitability is checked separately from complete
accounting. Reports must retain successful calls/items per second, latency
percentiles, per-order comparisons and source-bound client/all-voter CPU.
Fresh final replica drains, complete dataset/sentinel/nonce checks, process
lifetimes and retained-byte audit remain mandatory.

The three local suites pass **8 driver + 15 auditor + 5 smoke-schema controls**
at original terminal `88a107/0`. They include actual read-only checks of all
three source/build roles, configuration validation, invalid outcome and
retention-evidence rejection, and exact protocol/order checks. No server,
benchmark, codec or fault was launched by these controls. A separate static
comparison confirms that the accepted frame harness differs only in candidate
identity, paths and dependent hashes; descriptors, accounting predicates,
CPU placements and resource limits are unchanged.

The [source/release/recovery](WRITE-SEGMENT-VECTORED-RECOVERY.md) and
[actual Chaos Mesh](WRITE-SEGMENT-VECTORED-CHAOS.md) gates already pass for this
exact candidate. Their 352 ordinary recovery operations and 9,636 Chaos
operations are separate populations and supply no performance measurement.

## Capacity and execution order

The retained same-shape CRC reference occupies 48,819,441,664 bytes, including
compressed objects and other resident files. Together with the unchanged
96 GiB retention floor, 16 GiB single-cohort restore reserve and 1 GiB metadata
margin, it yields an empirical requirement of **170,152,267,776 available bytes**.
The preparation observed 118,335,848,448 bytes available. This is a planning
scenario, not a bound on the unrun candidate or a runtime release. The earlier
object-only estimate is retained and superseded by whole resident accounting.

Keep the original 8 GiB member, 16 GiB cohort and 128 GiB combined logical/physical
campaign limits; 65,536 files; 600-second codec and 1,800-second cohort deadlines;
64 MiB decode memory; and 8 MiB retention metadata limit. No earlier cleanup
credit is reused. Additional storage still requires writable-capacity and
environment qualification before any workload starts.

Next qualify storage and exact runtime inputs, run all eight fresh smokes and
their independent readback, then run the complete sixteen-cohort comparison and
independent audit. Publish throughput and tail latency together. Full point/batch
and mixed-read regressions remain required before promotion. The prepared full
CRC regression and frame comparison retain their separate complete matrices.

The latest measured write results remain the [CRC comparison](WRITE-CRC-PERFORMANCE.md):
1,063,493.134 loaded batch items/s with p99 6.947–7.012 ms, and 139,402.831 loaded
point writes/s. These results do not transfer to the vectored candidate.
Cross-host and power-loss behavior, Redis parity and industrial acceptance
remain open. CI remains local.
