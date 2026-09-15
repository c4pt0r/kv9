# Write performance against three-copy Redis

Updated: 2026-09-14. The current priority is write throughput and latency,
targeting Redis with one primary and two replicas. Read optimization is held at
the selected ThinLTO/Safe ReadIndex baseline. The experimental lease work and
its remaining clock/Chaos gates are retained; read parity is not claimed and is
no longer a prerequisite for this write phase.

## Local storage checkpoint (2026-09-14 UTC)

The [completed cache cleanup](write-storage-cache-cleanup-v1/result.json) increased
observed root-filesystem available space from 118,255,992,832 to 208,181,657,600
bytes: 89,925,664,768 bytes reclaimed. Only inactive Rust incremental compilation
caches and npm/Bun download caches were removed. Source, retained executables,
historical benchmark payloads and Chaos/recovery evidence remain available.
The initial Bun command required a package context; its successful retry is
recorded separately. No RAID mount, Docker prune or hosted CI was needed.

The vectored screen subsequently retained 45.49 GB. Before full CRC regressions,
native cleanup of unused default/ARM BuildKit caches reclaimed an additional
**46,951,264,256 bytes**, raising available space from 162,362,454,016 to
209,313,718,272 bytes. Images, containers, volumes, sources and historical
benchmark/Chaos/recovery data were preserved. The corrected CRC whole-resident
scenario required 203,271,221,248 available bytes; fresh smoke and timing
capacity checks passed with the original floors and restore reserves.

The [completed full CRC campaign](WRITE-CRC-FULL-REGRESSION-PERFORMANCE.md)
now retains 84.59 GB and leaves about 117 GiB free at the post-campaign check.
These observations supersede older available-space estimates below; they do
not reserve another campaign. CRC improves loaded batch writes 17.119% and
mixed batch throughput 16.898%, while the [vectored screen](WRITE-SEGMENT-VECTORED-PERFORMANCE.md)
found no write gain. The [exact-main CRC integration](CRC32-SLICING-INTEGRATION.md)
now passes source-bound proof, 789 tests/doctests, a clean default release,
ordinary recovery and all 21 actual Chaos windows with complete independent
histories and cleanup. The subsequent [exact-main/frame-buffer write screen](WRITE-FRAME-BUFFER-CRC-PERFORMANCE.md)
now passes eight smokes and sixteen timed cohorts. CRC main reaches 139,188.639
point Put/s and 1,065,680.142 BatchPut(64) items/s at c64. The combined frame-buffer
candidate is not selected: point throughput changes -0.517%, batch changes +0.379%
with worse pooled p99, and both loaded workloads reverse direction across orders.
All 7,247,954 measured calls succeed once. The source/proof/recovery/Chaos evidence
and complete write results remain published separately.

Before that screen, exact inactive Cargo-artifact cleanup reclaimed an observed
59,290,816,512 bytes while preserving source, application binaries and prior
evidence. The screen now retains 52,317,179,904 bytes; its smoke/timing capacity
checks passed with unchanged floors and restoration reserve. Historical free-space
observations do not reserve another campaign. The [exact current CRC main CPU
profile](WRITE-CRC-MAIN-CPU-PROFILE.md) now passes both instrumented workloads and
independent decode: 3,259 point and 3,106 batch samples, full interval coverage
and zero sample loss. Fresh disassembly attributes 11.751% of batch samples to
the legacy Raft WAL FNV loop and 5.094% of point samples to receipt linear search.
The [standalone four-lane FNV kernel](WRITE-FNV-INTERLEAVE-KERNEL.md) now passes
six source-bound Lean statements, four Rust equivalence tests in both debug and
optimized builds, and six rejected controls. Its fixed 108-row kernel matrix
improves equal four-body checksum throughput 3.342x at 206 bytes and 3.969x at
10,601 bytes, with consistent opposite orders. Skewed inputs gain little and
empty groups cost more. The subsequent writer integration below preserves
the original stream, sync/publication and failure semantics. The standalone
kernel experiment remains a separate measurement; no database QPS gain is claimed.

## Latest writer checkpoint (2026-09-14 UTC)

The [bounded four-lane FNV writer](WRITE-FNV-WRITER.md), experimental runtime
`12f44d3`, now passes 15 writer plus six kernel proof statements and the local
workspace gate: 797 tests/doctests, 23 existing ignored, formatting, Clippy and
explicit experimental-lease compilation. Five new persistence tests cover full
frame-stream/budget boundaries and 1,512 deterministic failure combinations.
Its [default release and ordinary recovery](WRITE-FNV-WRITER-RECOVERY.md) now
pass on exact tested source: 365 complete operations (337 OK / 28 unknown),
six fresh drains and seven exited lifetimes. The original evidence-inventory
preflight failure is retained; the tested source worktree passes unchanged
limits. Its [actual 21-window Chaos Mesh campaign](WRITE-FNV-WRITER-CHAOS.md)
also passes: 9,872 complete operations (9,300 OK / 541 unknown / 31 refused),
four fresh drains, full archive readback and all 31 server lifetimes exited.
The separate [eleven-window client-link/reset/quorum-loss campaign](WRITE-FNV-WRITER-LINK-CHAOS.md)
also passes, with 2,126 complete operations (1,835 OK / 63 unknown / 228 refused),
independent full-history and packet-effect checks, and owned cleanup.
The [cache cleanup and compression pilot](WRITE-FNV-CAPACITY.md) preserve
historical payloads; stronger compression saves only 1.6–1.8% and does not
justify bulk recompression. Review traces the inherited 96 GiB host floor to
an operational policy rather than a Raft or codec requirement. The separate
[storage policy v2](WRITE-FNV-STORAGE-POLICY.md) keeps the 64 GiB measured host
floor, uses a 48 GiB retention floor with the same serial restore allowance,
and passes 71 environment controls. Another inactive dev-cache cleanup releases
6,261,227,520 bytes. The original preparation remains immutable; every workload,
payload, synchronization fence and tmpfs guard remains. The complete
[eight-smoke/sixteen-timed comparison](WRITE-FNV-WRITER-PERFORMANCE.md) and
independent acceptance now pass: 7,283,648 successful one-attempt calls,
65,259,587 items, zero errors/unknown writes/drops. FNV reaches 139,878.372
point Put/s (+0.351%) and 1,096,549.396 batch items/s (+4.131%) at c64.
However, pooled batch p99 worsens from 7.602–7.668 ms to 9.830–9.961 ms,
including a 12.059–12.190 ms first-order candidate tail. Point throughput
reverses direction between orders. Keep CRC main selected; do not run a full
promotion matrix for this result. The [completed retained-sample analysis](WRITE-FNV-TAIL-ANALYSIS.md)
finds higher global IO pressure in the worst-tail FNV batch cohort, without
establishing individual-call causality. The separate [WAL directory-publication
candidate](WRITE-PUBLISHED-DIRECTORY.md), `483b8c3`, now passes 12 conditional
TLAPS statements / 24 fresh obligations, 793 tests/doctests, formatting, Clippy,
five actual syscall cases and four refusal controls. It retains full ancestor
sync during creation/recovery and avoids repeating it during normal rotation.
Its [default release and ordinary recovery](WRITE-PUBLISHED-DIRECTORY-RECOVERY.md)
now pass, including 364 complete operations and six fresh drained voters. The
[actual 21-window Chaos baseline](WRITE-PUBLISHED-DIRECTORY-CHAOS.md) also passes:
9,372 complete operations, four fresh drains and all 33 observed server
lifetimes exited. Next establish positive default-threshold segment rotation
and same-store leader-crash recovery before the complete matched
throughput/latency screen. The ordinary low-volume Chaos baseline does not
establish that fast-path coverage. Review benchmark storage headroom against
actual coexistence requirements while preserving all cohorts and retained
byte coverage. No performance gain is claimed yet. Queue age and
Ready/checksum group diagnostics remain secondary if this does not explain the
cost. Preserve every quorum, publication and acknowledgment fence, required file
and parent sync, and the original first-order FNV tail.

Prepared separately from the FNV timing campaign, the
[validated receipt tail-hint candidate](WRITE-RECEIPT-TAIL-HINT.md), `a6ac335`,
now passes 18 parameterized theorem statements / 158 fresh obligations and
793 workspace tests/doctests (23 existing ignored), formatting and Clippy.
It tests a constant-time checked slot lookup for consecutive indexes, with
ordered-search and original first-match fallbacks. Default release, recovery,
actual Chaos and database timing remain pending; it is not combined with FNV
or selected on main. This advances the secondary receipt target without
claiming that proof or a CPU percentage establishes performance gains.

## Comparison contract

| Panel | Successful call requires | Interpretation |
| --- | --- | --- |
| KV9, three voters | Existing Raft commit, durable apply and response fences | Preserve all current consistency and synchronization rules. |
| Redis, primary + two replicas, WAIT 1 | SET/MSET OK and at least one replica acknowledgment on that connection | Primary replication-confirmed reference; two acknowledged copies, not Raft semantics. |
| Redis, primary + two replicas, WAIT 2 | SET/MSET OK and both replica acknowledgments on that connection | Additional all-replica latency/throughput reference. |
| Redis asynchronous replication | SET/MSET OK only | Optional separately labeled reference; never substitute it for WAIT results. |

[WAIT](https://redis.io/docs/latest/commands/wait/) applies to preceding writes
on the same connection and returns the actual acknowledgment count. It does not
make Redis strongly consistent or provide replica fsync receipts. The installed
Redis 7.0.15 lacks [WAITAOF](https://redis.io/docs/latest/commands/waitaof/), which
requires Redis 7.2 or newer. A future fsync-confirmed panel needs a separately
qualified version/configuration; even WAITAOF does not establish Raft semantics.

The [first accepted performance panel](WRITE-REDIS3-BASELINE.md) uses KV9's
normal synchronization calls on explicitly volatile tmpfs WAL and Redis with
save disabled and appendonly=no.
It isolates replication/protocol/CPU cost on one shared host. It cannot establish
power-loss durability, independent host failure, cross-host capacity or equal
durability. Keep actual disk costs in a separate panel. Preserve all historical
standalone Redis measurements under their original configuration and hashes.

## Implemented reference client

`kv9-redis-batch-reference` version 4 adds mandatory, explicit
`write_confirmation` configuration. For example, a point-write run includes:

```json
{
  "version": 4,
  "read_api": "get",
  "write_api": "set",
  "batch_size": 1,
  "write_confirmation": {"kind": "wait", "replicas": 1, "timeout_ms": 100}
}
```

This fragment supplements the existing address, deadline, dataset, concurrency
and bounded-load fields. Batch writes use mget/mset and batch_size=64.
`{"kind":"async"}` selects explicit asynchronous acknowledgment. WAIT counts
must be 1 or 2; its nonzero timeout cannot exceed the original call deadline.
Null, missing v4 confirmation and unknown fields are rejected.

Each worker permits one logical call in flight on one persistent connection.
It sends the data command and WAIT together, consumes both responses and uses
one absolute deadline. There is no additional artificial client round trip,
no second connection for confirmation and no retry of an uncertain write.
A short acknowledgment count is `replication_shortfall` in the `unknown_write`
population. A timeout, lost response or malformed confirmation cannot turn the
preceding OK into a successful logical write. Reads issue no WAIT.

Reports retain whole-call/dispatch latency histograms, separate command and
confirmation attempts, actual replica acknowledgment counts and their combined
RESP command count. One SET+WAIT is one logical call and two RESP commands;
one MSET(64)+WAIT is one logical call and 64 input items. Attempt counters record
client send attempts, not guaranteed server execution. Version 1–3 input and
metric shapes stay compatible. The current tree also restores the exact v3
point GET/SET implementation from `0be806d9671e2c50701a64aa7889c8859b7648ba`,
which was already used by the retained matched point measurements.

[Local validation and original evidence](redis-replication-reference-v1/README.md)
cover real three-process SET/MSET confirmation, replica pause/shortfall controls,
unknown-write/deadline/framing tests and backward report compatibility. This
checkpoint provides no new QPS result and changes no KV9 runtime algorithm.
The [independent v4 reader and clean release](write-reference-qualification-v1/README.md)
now pass 44 Python tests, all four original client reports and 32 rejection
controls. The subsequent [12 smoke and 24 timed cohorts](WRITE-REDIS3-BASELINE.md)
now pass independent readback. All 18,993,624 measured calls succeed once;
original failed audit and schema repair remain retained without workload reruns.

## Executable development order

1. Completed: independent v4 accounting, clean releases and exact source/binary,
   Redis three-node configuration, CPU and finite-protocol binding. Preserve
   the current build lock, immutable inputs and original retention/disk guards.
2. Completed: SET/Put and MSET/BatchPut(64), c1/c64, 128-byte values, ten-second
   windows and two opposite orders, with 12 fresh smokes and 24 timed cohorts.
   The [report](WRITE-REDIS3-BASELINE.md) includes successful calls/items per second,
   mean/p50/p95/p99, all outcomes, drops and independently recomputed client/all
   three-server CPU. No build, profiler or codec overlaps timing. Use these as
   the selected write baseline; do not repeat runs to conceal failed attempts.
3. The existing [slicing-by-eight CRC candidate and proof](https://github.com/c4pt0r/kv9/blob/65511010e2fda8adba04efd831a39bcdca1979a4/docs/CRC32-SLICING-QUALIFICATION.md)
   is now reapplied to selected ThinLTO as experimental `e748620`.
   It preserves the checksum polynomial and WAL bytes; it already has a
   source-bound Lean equivalence proof and ordinary recovery evidence on its
   historical base. Fresh exact-source checks now pass 47 distinct Lean theorem
   statements, 710 workspace tests/doctests (23 existing ignored), Clippy, a clean
   release and ordinary recovery with 363 complete operations and 26 unknowns.
   [Original qualification evidence](write-reference-qualification-v1/README.md)
   preserves the separate populations. Historical kernel timings are not a database speedup.
   Existing engine and Raft Ready group commit must not be reimplemented.
4. Completed: the [selected-versus-CRC write screen](WRITE-CRC-PERFORMANCE.md)
   with the fixed native v3 client: point Put/BatchPut(64), c1/c64, two opposite
   orders, eight two-second smokes and sixteen ten-second timed cohorts.
   All 7,126,939 measured calls succeed once, without unknowns or drops.
   Loaded batch improves 18.807% to 1,063,493.134 items/s; p99 falls from
   9.306–9.437 ms to 6.947–7.012 ms. Loaded point writes improve 2.786%.
   Capacity, source/CPU bindings, 64 timed lifetimes, 48 drains/bindings and
   all retained bytes pass independent acceptance under the original guards.
   Its [actual 21-window Chaos histories](WRITE-CRC-CHAOS.md), independent audit
   and cleanup now pass: 9,833 complete operations, 600 unknowns and 28 refusals.
   Twenty-eight A/B environment controls and five summary arithmetic controls
   pass. The [full regression campaign](WRITE-CRC-FULL-REGRESSION-PERFORMANCE.md)
   now passes all 24 smokes and 48 timed cohorts, covering point/batch64,
   0/50/100% reads, c1/c64 and both complete orders. All 35,103,005 measured
   calls succeed once, without errors, unknowns or dropped slots. Loaded batch
   writes reach 1,062,522.902 items/s (+17.119%), with p99 7.406–7.471 ms
   versus 8.389–8.520 ms; point writes reach 139,532.275/s (+2.203%). Mixed
   batch throughput improves 16.898%, with better separate read/write p99 in
   both orders. Loaded pure GET changes -0.191%; no active operation has a
   worse p99 bucket in either order. The earlier write-only results remain
   separate. The [tools](WRITE-CRC-FULL-REGRESSION-TOOLS.md) retain 35 runtime
   and 13 reporting controls; full runtime, retained WAL and reporting acceptance
   now pass. The capacity blocker for this completed run is resolved.
   Completed: [CRC integration into main `bd42e60`](CRC32-SLICING-INTEGRATION.md),
   with 789 tests/doctests (23 existing ignored), 47 distinct Lean statements,
   clean default release and 353 ordinary-recovery operations (325 OK / 28 unknown).
   Its own 21-window Chaos campaign retains 11,316 operations: 10,683 OK /
   602 unknown / 31 refused. Independent full histories, four fresh final drains,
   all 31 observed server lifetime exits, full archive readback and scoped cleanup
   pass. Its new binary had not been timed at that checkpoint; the e748 results remain
   separate. Every Raft/sync/response fence and remaining industrial gate stays.

5. Continue with measured checksum, allocation, batching and replication costs.
   The isolated [single-buffer Raft WAL experiment `01d128f`](https://github.com/c4pt0r/kv9/blob/01d128fd771dfbf0e6826ee5b6821411afac1fec/docs/WRITE-RAFT-FRAME-BUFFER.md)
   removes a body allocation/copy without changing frame bytes, checksums, sync
   or response fences. Three universal SMT checks, three counterexample controls,
   710 workspace tests/doctests, formatting and Clippy pass. Its 23 ignored tests
   are existing; the new writer/replay compatibility case covers 3,840 frames.
   Its [exact release and ordinary recovery](WRITE-RAFT-FRAME-BUFFER-RECOVERY.md)
   now pass: 353 complete operations, 323 OK / 30 unknown, six fresh drains and
   seven exited lifetimes. Its [actual 21-window Chaos Mesh campaign](WRITE-RAFT-FRAME-BUFFER-CHAOS.md)
   and repaired independent audit now pass: 9,818 operations, 9,250 OK / 539 unknown /
   29 refused, four fresh drains and all 34 recorded server lifetimes exited.
   The original missing-receipt audit failure is retained; no workload was rerun.
   Its [matched write preparation](WRITE-RAFT-FRAME-BUFFER-PERFORMANCE-PLAN.md)
   passes all 28 local driver/auditor/smoke-schema controls. Eight smokes and
   sixteen timed cohorts remain unrun: the unchanged retention/restore scenario
   needs 170.15 GB available against 120.13 GB observed, a 50.02 GB gap.
   These historical reservations apply to that original standalone candidate;
   no frame-buffer speedup is established. The combined candidate `e9249f2` is
   now published and [qualified on the integrated CRC baseline](WRITE-FRAME-BUFFER-CRC-QUALIFICATION.md):
   790 tests/doctests pass (23 existing ignored), frame and CRC proofs pass,
   and its clean release and ordinary recovery retain 337 operations (309 OK /
   28 unknown). Actual Chaos Mesh passes 21 windows with 9,805 complete-history
   operations (9,170 OK / 613 unknown / 22 refused), four fresh final drains,
   all 33 server lifetime exits, archive readback and scoped cleanup.
   All eight fresh write smokes pass 810,022 calls and 32 lifetime exits. The
   [paired 16-cohort timing campaign](WRITE-FRAME-BUFFER-CRC-PERFORMANCE.md) passes
   7,247,954 one-attempt successful calls, full independent retained-byte checks,
   64 timed lifetime exits and 48 fresh drains/bindings. The candidate is not
   selected: loaded point changes -0.517%, loaded batch +0.379% with worse pooled
   p99, and both loaded throughput directions reverse between orders. The c1
   batch improvement does not establish general selection. Preserve this and
   the standalone experiment; do not pool their evidence.
   The separate [segmented-WAL vectored-write candidate `cfd9c92`](https://github.com/c4pt0r/kv9/blob/cfd9c927f8ecd33974100f696e6b08b227d25a41/docs/WRITE-SEGMENT-VECTORED.md)
   now preserves the frame stream through a short-write-aware vectored loop.
   Three SMT checks, three countermodels, 714 tests/doctests, formatting and
   Clippy pass; an actual file probe confirms one `writev` and following `fsync`
   per frame. Its [exact release and ordinary recovery](WRITE-SEGMENT-VECTORED-RECOVERY.md)
   now pass: 352 complete operations, 29 unknowns, six fresh drains and seven
   exited lifetimes. Its [actual 21-window Chaos Mesh qualification](WRITE-SEGMENT-VECTORED-CHAOS.md)
   now passes on the original attempt: 9,636 operations, 9,037 OK / 572 unknown /
   27 refused, four fresh drains and all 34 recorded server lifetimes exited.
   Independent complete-history checking, full archive readback and owned cleanup
   pass. A guarded cleanup of this completed source gate's first-party dev cache
   increased observed free space by 4.76 GB with all protected binaries unchanged;
   it does not resolve the larger performance reservations. The [matched vectored
   comparison preparation](WRITE-SEGMENT-VECTORED-PERFORMANCE-PLAN.md) now passes
   all 28 driver/auditor/smoke-schema controls. The [completed matched screen](WRITE-SEGMENT-VECTORED-PERFORMANCE.md)
   now passes eight smokes, sixteen timed cohorts and complete independent
   acceptance: 6,961,558 measured calls all succeed once. Loaded point throughput
   changes -0.612% and batch throughput -1.020%, with worse pooled p99 intervals.
   Keep the isolated candidate experimental; this screen establishes no write
   gain and does not justify default promotion. Full CRC regressions, main
   integration and the combined frame-buffer write screen now pass; the latter
   does not justify promotion. Preserve the original vectored experiment for
   a separately justified real-disk or combined-candidate study.
   Completed: [current CRC main CPU attribution](WRITE-CRC-MAIN-CPU-PROFILE.md)
   with the unchanged bounded point/batch protocol. Both workloads, original
   independent decoder, active-prefix/edge/32-bin/clock/zero-loss coverage and
   cleanup pass. Fresh exact-binary disassembly identifies the legacy Raft FNV
   loop at 365/3,106 batch samples (11.751%), and receipt linear search at
   166/3,259 point samples (5.094%). The old byte-table profile stays historical.
   Completed: [standalone four-lane FNV kernel proof and measurements](WRITE-FNV-INTERLEAVE-KERNEL.md).
   Six universal Lean statements and six rejection controls pass; four Rust
   equivalence tests pass in each of debug and optimized builds. The 108-row,
   opposite-order kernel matrix shows 3.342x/3.969x speedups for equal four-body
   206/10,601-byte groups, with much smaller skewed-input gains and higher
   empty-group overhead. This is not database QPS or proof of Ready-group
   frequency. The standalone prototype remains a separate experiment.
   Completed: bounded writer integration at `12f44d3`, with four-body/64 KiB
   staging, scalar fallback and no wait for future requests. Composition proof,
   byte/budget/failure tests, clean default release, ordinary recovery and
   actual 21-window Chaos acceptance pass. Original sync/publication and
   failure-poisoning boundaries remain unchanged. The paired write screen now
   passes but does not justify promotion: pooled batch throughput gains 4.131%
   with worse p99, while point throughput changes only 0.351%. Keep CRC selected.
   Completed next source checkpoint: published-directory reuse (`483b8c3`),
   with conditional proof, workspace tests and syscall-fault acceptance. Next
   run actual Chaos, then a paired write screen; exact default release and
   independently audited ordinary recovery now pass.
   Keep receipt and persistent-map ownership as
   secondary targets; do not repeat rejected worker/transport sweeps.
   DPDK requires cross-host/NIC
   evidence. A real-disk panel must retain every sync and acknowledgment rule.
6. After the write phase, return to bounded dynamic multi-Raft (#22), epoch routing
   (#23), recoverable membership (#24), automatic splits (#25) and placement
   (#27), with their existing storage/recovery/proof prerequisites. Metadata and
   scheduling must be replicated or safely replaceable. Only object storage may
   be a service-critical singleton.

CI remains local. GitHub CI is reserved for releases or explicitly selected key
milestones. These write checkpoints close no original industrial work package.
