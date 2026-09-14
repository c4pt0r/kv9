# Four-lane FNV writer: matched write results

Measured 2026-09-14 UTC. **Keep CRC main selected.** The bounded FNV writer
improves pooled c64 BatchPut(64) throughput **4.131%**, but its pooled p99 is
**9.830–9.961 ms**, against **7.602–7.668 ms** for the paired CRC control.
Point Put changes only **+0.351%**, reverses direction between orders and has
a slightly worse pooled p99. Candidate `12f44d3` remains experimental.

All eight two-second smoke cohorts and sixteen ten-second timed cohorts pass
independent acceptance. Every one of the **7,283,648 measured calls** succeeds
on one attempt, covering **65,259,587 input items**, with zero errors, unknown
writes or dropped slots. No cohort was rerun, shortened, omitted or replaced.
The separate smokes contain 825,500 successful calls and 8,411,078 items.

## Throughput and whole-call latency

Rows pool the two opposite complete orders. Throughput divides summed successful
calls/items by summed elapsed time. Means use the integer latency sums and
population counts; p99 intervals come from merged raw histograms. Batch latency
is for the complete 64-item call. Compare the two roles within this campaign;
do not pool these samples with an earlier storage policy or report.

| c | API | Role | Calls/s | Items/s | Mean us | p99 us |
| ---: | --- | --- | ---: | ---: | ---: | --- |
| 1 | Put | CRC main | 19,405.104 | 19,405.104 | 51.439 | 66.560–67.583 |
| 1 | Put | FNV writer | 19,491.192 | 19,491.192 | 51.203 | 65.536–66.559 |
| 64 | Put | CRC main | 139,388.631 | 139,388.631 | 459.024 | 737.280–745.471 |
| 64 | Put | FNV writer | 139,878.372 | 139,878.372 | 457.411 | 745.472–753.663 |
| 1 | BatchPut(64) | CRC main | 6,211.259 | 397,520.560 | 160.892 | 202.752–204.799 |
| 1 | BatchPut(64) | FNV writer | 6,207.512 | 397,280.771 | 160.988 | 204.800–206.847 |
| 64 | BatchPut(64) | CRC main | 16,453.896 | 1,053,049.319 | 3,888.719 | 7,602.176–7,667.711 |
| 64 | BatchPut(64) | FNV writer | 17,133.584 | 1,096,549.396 | 3,734.309 | 9,830.400–9,961.471 |

Loaded batch throughput improves **+1.365% / +6.864%** in the forward/reverse
orders. The first FNV batch p99 worsens to **12.059–12.190 ms** from
**7.864–7.930 ms**; the second improves to **6.685–6.750 ms** from
**7.340–7.406 ms**. Both complete observations remain in the result. The
pooled mean improves 3.971%; that does not remove the first-order tail.

Loaded point throughput changes **+1.490% / -0.782%** with opposite latency
directions. At c1, point pools to +0.444%, batch to -0.060%, with throughput
direction reversals in both cases. Two short runs on a shared host do not
establish statistical significance, sustained performance or the cause of
the tail variation. The first tail is not classified as an invalid outlier.

Three-voter sampled CPU is 3.563 versus 3.550 cores for loaded point Put and
3.344 versus 3.314 for loaded BatchPut(64). Client rates are 0.431/0.436 and
0.480/0.497 cores. These are observed process rates, not additive request-stage
latency or proof that one code path caused the variation.

## Configuration and correctness scope

- CRC main source: `bd42e60f84657e22e36e34924a5c80a08eac623a`; server SHA256
  `106f517c84fabe790ea139f3c18661e14447abd9e257fdd6ba82bfabe6796eb9`.
- FNV writer source: `12f44d35590ede5f89337fe731dd950162865154`; server SHA256
  `d84b0ec8e466a9dc3f4953751605d91c90a90a66df1b03b57a3412e43e42603e`.
- Fixed native v3 client: `0be806d9671e2c50701a64aa7889c8859b7648ba`; SHA256
  `8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957`.

Both roles use their clean default release, ThinLTO, opt-level 3 and one
codegen unit. Three voters share CPUs 2–5 and the client uses 0–1. Helpers and
owned background containers use 6–15,22–31. Workloads use 4,096 mutable keys
plus sentinel, 128-byte values, seed 71, 128 warmup calls and closed-loop
concurrency 1 or 64. The second repetition reverses the complete eight-row
order. No build, profiler or unrelated codec overlaps timing.

Raft quorum, WAL synchronization, durable apply and response fences remain
enabled on **volatile tmpfs WAL**. This is a same-host loopback CPU/protocol
diagnostic, not a physical-disk, power-loss, cross-host or equal-durability
Redis measurement. Redis was not rerun. Its [three-copy reference](WRITE-REDIS3-BASELINE.md)
retains separate WAIT 1/2 and persistence semantics. The present report does
not replace the earlier full read/mixed regression panel.

The exact [candidate source qualification](WRITE-FNV-WRITER.md) passes 15 writer
plus six kernel proof statements, 797 tests/doctests (23 existing ignored),
formatting and Clippy. Its [default release and recovery](WRITE-FNV-WRITER-RECOVERY.md),
[actual 21-window Chaos Mesh campaign](WRITE-FNV-WRITER-CHAOS.md) and
[eleven-window client-link/quorum-loss campaign](WRITE-FNV-WRITER-LINK-CHAOS.md)
pass separately. This performance screen adds no fault-injection history and
does not by itself establish full-history linearizability or complete Rust
refinement. The standalone checksum speedup is not the database speedup.

## Independent acceptance and retention

The separately versioned [storage policy v2](WRITE-FNV-STORAGE-POLICY.md)
passes 71 environment controls; reporting passes five inherited arithmetic
controls. The original v1 preparation remains unchanged and unexecuted.
Smoke `36365/df736d/0`, independent smoke readback `d3d06f/0`, timing
`61012/61e0b7/0`, full audit `89236/321c49/0` and reporting `2fe68b/0` pass.
The audit verifies 64 timed and 32 smoke process lifetimes exited, 48 fresh
timed drains and writer/listener bindings, 2,327 source-file checks, 3,073
resource samples and exact restoration of owned CPU settings and namespace maps.

All **74,608,085,855 logical WAL bytes** are independently decoded and
hash-checked: 66,063,018,782 timed plus 8,545,067,073 smoke. Combined retained
allocation is **52,612,304,896 bytes**. Compression follows each cohort's writer
shutdown and finishes before the next measured cohort. All original byte
coverage and finite retention limits remain. Post-audit available space is
74,389,651,456 bytes, above the 68,727,865,344 bytes required at that instant
for the declared maximum 16 GiB fresh restore plus its 48 GiB floor and 8 MiB
metadata allowance. This observation does not reserve space or claim that
every retained runtime cohort was materialized in a separate restore tree.

The [portable results and complete original metadata](https://github.com/c4pt0r/kv9/blob/fd2e18bd9471ad4f4e8aab6e3d0cd6f8c145e0ce/docs/fnv-writer-performance-v2/README.md)
contain 2,472 files / 319,208,049 decoded bytes in 19 parts totaling
38,598,684 compressed bytes. Packaging `13753/8c9728/0` and independent
portable verification `d4fbb5/0` pass.
The original WAL objects and executables remain local with explicit hash
references. The first publisher refused before compression because it counted
preserved synthetic refusal-test catalogs as runtime catalogs. The corrected
selector keeps those original control files and applies the unchanged 24-catalog
runtime requirement only to the exact smoke/timing roots. No workload or
acceptance audit was repeated.

## Next decision

Keep CRC main selected. Do not launch a full read/mixed promotion campaign
for this small point change and unstable batch tail. The [all-16 retained sample
analysis](WRITE-FNV-TAIL-ANALYSIS.md) now shows elevated global IO pressure in the
worst-tail FNV batch cohort, without a corresponding sampling-gap, CPU, RSS or
thread-count anomaly. This does not establish per-call causality. Source
inspection found repeated ancestor-directory fsync during every WAL rotation.
A separate [published-directory candidate](WRITE-PUBLISHED-DIRECTORY.md) now
passes conditional proof, 793 tests and actual syscall-result fault checks.
Next qualify its default release, ordinary recovery and actual Chaos Mesh,
then run a matched throughput/latency screen with every Raft fence retained.
Queue age and Ready/checksum group diagnostics remain secondary if needed.
Preserve the original first-order tail instead of rerunning until it disappears.
The [receipt tail-hint candidate](WRITE-RECEIPT-TAIL-HINT.md) remains a separate
source-qualified experiment, with release/recovery/Chaos/timing still pending.
Dynamic multi-Raft and automatic splits retain their subsequent place in the
[development route](WRITE-PERFORMANCE-NEXT.md). All work ran locally; no hosted
CI was dispatched and no original industrial work-package checkbox closes.
