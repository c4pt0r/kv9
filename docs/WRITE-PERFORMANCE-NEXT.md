# Write performance against three-copy Redis

Updated: 2026-09-16. The current priority is write throughput and latency,
targeting Redis with one primary and two replicas. Read optimization is held at
the selected ThinLTO/Safe ReadIndex baseline. The experimental lease work and
its remaining clock/Chaos gates are retained; read parity is not claimed and is
no longer a prerequisite for this write phase.

The latest [same-source WAL preallocation comparison](WAL-PREALLOCATION-PERFORMANCE.md)
passes all eight smokes, sixteen timed cohorts and independent acceptance:
7,200,959 successful calls / 63,451,454 items. Current default `86aa6fc` reaches
137,873.776 Put/s and 1,022,750.059 BatchPut(64) items/s at c64. The candidate
changes these by +0.545% / +1.474%, with unchanged loaded pooled p99 and batch
throughput/mean/p99 reversing direction between orders. Keep it default-off;
this does not establish a material general improvement or Redis parity.
The [exact Raw apply attribution](RAW-APPLY-ATTRIBUTION.md) now joins 1,564
original Raft commands to all 106 retained engine groups. Decode/lower/fence
means are 19.722 / 14.649 / 2.336 us per group. A separate direct-append prototype
reduces lowering mean 10.896% and p99 in both orders, but saves only 1.548 us per
group. Production remains unchanged. Next measure lowering together with real
MemEngine insertion/overwrite and retained-snapshot workloads before runtime
integration. Preserve mutation/fence order and snapshot/publication semantics;
do not repeat completed screens or rejected clone-removal, coalescing and
borrowed-upsert variants. These component results are not database QPS.

The earlier [complete write-observer capture](WRITE-OBSERVER-CAPTURE.md) measures
queue/group distributions and diagnostic overhead across all 16 planned cells.
Loaded Put repeatedly inspects pending receipts: 86.00–86.11% of lookups miss,
averaging about 1,022 logical comparisons. The separately proved upper-bound
candidate now has actual path counts and a
[complete ten-second matched result](WRITE-RECEIPT-UPPER-BOUND-PERFORMANCE.md).
Loaded Put improves 3.169% with better mean/p99 in both orders. Loaded batch
loses 1.419% pooled and its p99 worsens to 8.651–8.782 ms; direction changes
between orders. Keep CRC selected and hold promotion. Analyze the retained
batch-tail, group/writer and queue evidence before another rewrite; introduce
bounded timestamped observation only for intervals the existing data cannot
resolve. The two-second observer sweep is not a performance-selection gate.

The [completed offline batch review](write-batch-tail-review-v1/README.md)
confirms that observed apply-group maxima stay below the existing count/byte
limits. Increasing those limits has no support in these captures. Per-command
apply timers omit group decoding/lock acquisition and overlap within groups;
endpoint aggregates cannot locate one slow request. The new
[default-off write-stage trace](WRITE-STAGE-TRACE.md) records bounded exact
term/index joins across preparation, locks, apply, receipt insertion and terminal
inspection. The corrected-source capture below now qualifies retained coverage
and observer overhead. Use that evidence before selecting another writer or
scheduling change; it does not promote the held candidate.

The first current-source release pair now exists, but its
[actual capture stopped on five lost trace recordings](WRITE-STAGE-CAPTURE.md).
The default row completed; the instrumented row remains failed and the reverse
order never ran. Export now tries the existing pump gate and releases both
guards before allocation. That failed attempt supplies no observer-overhead
comparison; the corrected-source qualification is separate below.

That subsequent [corrected-source comparison now passes](WRITE-STAGE-CAPTURE-RESULTS.md):
all four rows, 131,737 calls and 8,431,168 items, with zero trace loss and exact
cleanup/restoration. Combined diagnostics cost 3.666% / 1.852% throughput in the
two short orders (2.770% pooled). The two leader tails locate a larger cost
inside state-machine apply: deduplicated group means 549.018 / 546.521 us,
against about 47–49 us before apply and 4 us to publish receipts. Next attribute
Raw batch lowering, WAL encoding/checksum and persistent-map publication before
choosing a change. Existing metrics do not show direct tmpfs sync as the dominant
cost. Preserve rejected owned-buffer and scheduling experiments; these internal
tails are not complete client histories or a new performance-selection result.

The next [resident-index experiments](RESIDENT-INDEX-EXPERIMENTS.md) reject
sorting/coalescing and two-pass value reuse. A preselected retained segment has
14.434% overwritten mutations within batches, but sorting costs more than it
saves. A one-traversal borrowed upsert passes 53 dependency tests and preserves
snapshots in differential checks. Under jemalloc, its prepopulated overwrite
index time falls 12.269%, while pure insertion grows 12.121%; it remains held.
All changes are isolated experiments, with reproducible source patches and the
exact corpus. No runtime integration, new QPS, proof/Chaos acceptance or repeated
full matrix is claimed. Static follow-up found no justified fix for the new-key
regression; stop advancing that variant and retain its original evidence.

The [WAL payload preallocation candidate](WAL-PAYLOAD-PREALLOCATION.md) reuses
already validated size with the same byte emitter and unchanged durability
ordering. It is off by default. Source-bound capacity/CRC proofs and both local
851-test workspace/Clippy configurations pass. Mean encoder time drops 27.339%
on the original batch corpus; six small cases improve mean and both-order p99.
[Matching release and ordinary recovery](WAL-PREALLOCATION-RUNTIME.md) now pass:
four complete histories / 728 operations (666 OK, 62 unknown), twelve fresh
drains and fourteen exited lifetimes. The feature remains off by default.
[Actual candidate Chaos Mesh](WAL-PREALLOCATION-CHAOS.md) now passes all 21
windows, 9,241 complete operations, four final drains, 31 server lifetimes and
25 exited containers, with independent audit/archive/cleanup. The
[same-source end-to-end comparison](WAL-PREALLOCATION-PERFORMANCE.md) above now
completes runtime selection: keep the feature default-off for its small and
order-dependent gains. Encoder microbenchmarks and correctness runs alone do
not establish database QPS. Keep CRC selected and avoid replaying completed
index/capture/full matrices.

During the earlier full-screen capacity constraint, the C04
[checkpoint publication increment](CHECKPOINT-PUBLICATION.md) strengthens local
restart authority: an actual atomic winning manifest apply must match the exact
committed command; a committed CAS loser cannot certify recovery. This consumes
the historical configuration-at-cut provider without adding work to online
apply or changing the selected write candidate. Complete portable authority and
retention integration remain storage prerequisites.

The subsequent [historical base identity increment](CHECKPOINT-BASE-IDENTITY.md)
derives upload scope from the exact frozen image and checks its certified root,
schema and owner epoch before startup WAL replay. Valid newer tail epochs remain
recoverable. The [initial anchor envelope](RECOVERY-ANCHOR-ENVELOPE.md) now
composes this observation with the committed configuration and actual winning
publication. Its bounded description and private local recovery observation pass
684 library tests, strict binding proofs and new actual three-voter/Chaos Mesh
recovery. The [tracking-only replicated ledger](RETENTION-LEDGER.md) now adds
whole-closure ownership transitions, strict committed-state proofs and actual
Chaos Mesh leader-failure acceptance. [Automatic checkpoint ownership](CHECKPOINT-OWNERS.md)
now connects the current worker's durable pre-upload plan to Pending/Version
owners and excludes retention bookkeeping from checkpoint scheduling. That
filter adds a prefix predicate during applied-batch scheduling; it has no new
throughput/latency result. Next validate the pre-upload crash cut, retain typed
negative history, backfill/fence references and bind transition evidence, then
implement destination admission and atomic installation.

New bulk output and proof tools now use `/mnt/data/kv9-work`; latency-sensitive
test data keeps its original NVMe location. [Fresh capacity and input checks](LOCAL-ARTIFACTS.md)
show enough space for the unchanged 79,455,850,496-byte full screen. The old fixed
v3 benchmark client is missing after an external cleanup. One source/toolchain-
matched rebuild produced a different ELF. Its [explicit qualification](WRITE-CLIENT-REQUALIFICATION.md)
now passes source tests and all eight actual smokes with independent dataset,
report and lifetime checks. Both server candidates use that same new client;
historical measurements are not pooled with it. All sixteen timed cohorts and
the enclosing audit now pass: 7,154,151 calls / 62,959,110 items, with no failed
or uncertain measured call, hidden data retry or dropped slot. This completes
the original comparison, not candidate promotion. Preserve the batch tradeoff
and original input failures; do not repeat the unchanged matrix.

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

## Latest writer checkpoint (2026-09-15 UTC)

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
lifetimes exited. The separate [rotation supplement](WRITE-PUBLISHED-DIRECTORY-ROTATION.md)
now passes positive default-threshold rotations before and after an actual
leader kill, same-store acknowledged-value recovery, five fresh drains,
independent full-history audit and exact cleanup. Both earlier fixture failures
remain preserved. [Local capacity recovery](LOCAL-CAPACITY-RECOVERY.md) reaches
the empirical storage-v3 launch budget without deleting retained logical
bytes. The [complete eight-smoke/sixteen-timed screen](WRITE-PUBLISHED-DIRECTORY-PERFORMANCE.md)
and independent audit now pass. Loaded batch throughput improves 1.941% to
1,076,849.048 items/s and pooled p99 improves to 6.554–6.619 ms; loaded point
throughput regresses 0.518% to 138,734.094 calls/s. Both orders retain batch
gains and loaded point regression. Keep CRC selected, preserve the directory
candidate as a batch improvement. The separate receipt screen below is now
complete. Do not combine candidates or infer a general gain before new evidence. Queue age and
Ready/checksum group diagnostics remain secondary if this does not explain the
cost. Preserve every quorum, publication and acknowledgment fence, required file
and parent sync, and the original first-order FNV tail.

Prepared separately from the FNV timing campaign, the
[validated receipt tail-hint candidate](WRITE-RECEIPT-TAIL-HINT.md), `a6ac335`,
now passes 18 parameterized theorem statements / 158 fresh obligations and
793 workspace tests/doctests (23 existing ignored), formatting and Clippy.
It tests a constant-time checked slot lookup for consecutive indexes, with
ordered-search and original first-match fallbacks. Its exact default release
and ordinary three-voter recovery now also pass independent checks: 359 complete
operations (329 OK / 30 unknown), six fresh drains and seven exited lifetimes.
[Original release/recovery evidence](receipt-tail-recovery-v1/README.md) is
retained with complete byte readback. The [actual 21-window Chaos campaign](WRITE-RECEIPT-TAIL-CHAOS.md)
now passes as well: 9,360 complete operations, four final drains, complete local
archive/readback and 32 observed server lifetimes exited. Its
[complete matched write screen](WRITE-RECEIPT-TAIL-PERFORMANCE.md) now passes:
all eight smokes, sixteen timed cohorts and independent final acceptance.
Loaded Put improves **2.971% to 142,202.415 calls/s**, with pooled p99
**720.896–729.087 us**, and both orders improve. Low concurrency BatchPut(64)
regresses **0.698%**; loaded batch gains **1.711%** pooled but reverses direction
and worsens p99 in the new-first order. **Keep CRC selected and hold receipt-tail
promotion.** The timed population contains 7,296,894 successful single-attempt
calls / 64,701,927 input items, with zero non-success outcomes or drops.
Next capture actual receipt-queue length/age, checked-lookup/fallback counts,
entries resolved per apply and Ready/group-commit sizes in a bounded diagnostic.
Measure observer overhead separately. These measurements must establish the
next causal target before another lookup change or combination experiment.
The [default-off write diagnostics](WRITE-PATH-DIAGNOSTICS.md) now implement
the baseline observations and pass local default/diagnostic checks. They record
actual linear scan lengths, queue ages, application groups and Ready sizes.
The next runtime work is release qualification and bounded capture with a
separate observer-overhead comparison. Candidate hint/fallback counters still
need a separately bound instrumented receipt build; no new performance gain
or explanation of the batch regression is claimed.

The [receipt matched-screen preparation](receipt-tail-performance-preparation-v1/README.md)
also passes its 16 focused controls and actual three-role source/binary/Cargo
binding. Its complete eight-smoke/sixteen-timed workload and storage-v3 policy
are unchanged. About 25 GB was available at preparation, versus an estimated
80–100 GB launch budget for a complete new campaign. The historical capacity
work below enabled the now-completed screen; preparation itself was not
performance evidence.
The [bounded WAL patch pilot](cross-voter-retention-pilot-v1/README.md) passes
six full original-byte reconstructions and eight refusal controls. It supports
testing a lossless retention migration, but has reclaimed no bytes. The follow-up
also reproduces all six old compressed-object identities. The subsequent
[whole-cohort transaction](cross-voter-cohort-retention-v1/README.md) now passes
19 controls, 80 exact original-object reconstructions, original-path restoration
and unchanged-reader acceptance of all 150 objects. Its final COLD state recovers
924,991,488 allocated bytes after transaction overhead, with all 870 fresh codec
lifetimes exited. Extend this qualified migration in bounded groups until actual
capacity covers the receipt screen; preserve original historical checks while
separately reviewing live readback-floor changes for older readers.

The [bounded expansion](cross-voter-multicohort-retention-v1/README.md) now passes
12 focused controls and its first larger cohort: 226 exact restores and complete
readback of all 369 objects, with 2,403 codec lifetimes exited. Net recovery adds
2,360,422,400 bytes after transaction/shared preparation/dispatcher allocation;
available space is about 27.71 GB before publication overhead. Only one of the
fixed 96 cohorts had run at that stage. Continue the remaining serialized migrations with
actual result validation and capacity accounting, then run the prepared receipt
screen unchanged. No additional QPS or general candidate promotion is established.

The [continuous controller](cross-voter-multicohort-retention-v1/controller/README.md)
now passes 15 controls and independent review, and its first actual iteration
completes exact restoration and full readback of 355 frame-buffer objects. The
[latest progress snapshot](cross-voter-multicohort-retention-v1/campaign-progress-20260915.json)
records 13 of the 96 planned cohorts COLD, conservative net recovery of
29.876 GB including controller allocation, and 55.201 GB actual available space
at the completed boundary. The controller subsequently failed during ordinal013
because its reader confused a reused historical PID with a live producer.
The [qualified identity repair and actual reconciliation](retention-pid-identity-v1/README.md)
preserve that failure and complete a new 344-object readback and final COLD,
extending the prefix to 14 cohorts and observing 57.387 GB available. The
[qualified continuation](cross-voter-multicohort-retention-v1/continuation/README.md)
executed from ordinal014 with a prospective 85 GB capacity stop; its 11
distinct focused controls and independent source review pass. Its first actual
cohort completes corrected full-CRC readback of 303 objects and final COLD,
extending the accepted prefix to 15 cohorts: 34.087 GB conservative net recovery
and 59.401 GB available at that boundary. The original plan and benchmark
guards stay intact.
The [later completed-boundary snapshot](cross-voter-multicohort-retention-v1/continuation/progress-20260915.json)
records 19 completed plan cohorts, 41.339 GB conservative net recovery and
66.648 GB available at that boundary. Ordinal019 was executing finish at capture.
That snapshot remained below the unchanged full-screen launch budget.
The [actual terminal capacity result](cross-voter-multicohort-retention-v1/continuation/completion/README.md)
subsequently records successful exit (`85630/b96c6f/0`), **40 completed plan
cohorts**, **59,995,123,712 bytes** conservative net recovery and
**85,304,766,464 bytes available** at the final boundary, before subsequent
reporting costs. No incomplete or restored cohorts remain. The raw controller
counter is 39 because bootstrap000 is separately bound; accounting and the
next ordinal both identify the complete 40-cohort prefix. The original failed
controller and its ordinal013 failure remain unchanged. Capacity work is
complete at its prospective target; benchmark acceptance remains separate.
Actual restoration/readback
now covers a [first cohort from all four reader families](cross-voter-multicohort-retention-v1/README.md#actual-coverage-of-all-four-reader-families),
including independent full-CRC metadata review.
The [PID-corrected derivatives now also have actual coverage across all four families](retention-pid-identity-v1/family-completion/README.md):
FNV013, full-CRC014, directory020 and frame021, with 947 successful current
readback decoder receipts. New directory/frame metadata and lifetime validation
passes independently; prior FNV/CRC evidence is reused without payload replay.
Fresh source/capacity/process checks subsequently pass, followed by all eight
smokes (`18934/899209/0`), smoke accounting/dataset checks (`d56257/0`), all
sixteen timed cohorts with exact outer restoration (`84770/38c0ab/0`), and final
independent acceptance (`16800/8faaea/0`). All 73,743,331,446 retained logical
bytes across smoke and timing pass independent decoding. The earlier 16
focused controls and role binder were not replayed. Timing had no overlap with
migration/codecs; the original workload and limits remained unchanged.

An [offline analysis of retained counters](write-amortization-v1/README.md)
separately quantifies existing write amortization. It preserves the distinction
between command events, WAL metric events, syscall counts and overlapping
per-command timers. It provides no new QPS, group distribution or justification
to repeat rejected owned-buffer/worker changes.

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
   Completed: published-directory reuse (`483b8c3`), with conditional proof,
   workspace/syscall checks, default release, ordinary recovery, actual full21
   Chaos and positive rotations before/after same-store leader recovery. Its
   complete paired write screen improves loaded batch throughput 1.941% and
   p99, while loaded point throughput regresses 0.518%. Keep CRC selected and
   retain the directory candidate as a batch improvement. The receipt tail hint's
   default release, recovery, actual full21 Chaos and complete matched screen
   now pass. Loaded Put gains 2.971% with better p99 in both orders; c1 batch
   regresses 0.698%, and the loaded batch result is order-sensitive. Hold its
   promotion and keep CRC selected. The [complete observer sweep](WRITE-OBSERVER-CAPTURE.md)
   now records actual queue, age, linear lookup and apply/Ready populations with
   matched diagnostic overhead. Loaded Put has 86.00–86.11% lookup misses and
   about 1,022 comparisons per inspection, despite mean nonempty service queues
   of 27 requests. Next bind actual candidate hint/fallback counts and prove
   whether redundant pending inspections can be skipped until relevant apply
   state changes. Preserve deadlines, eviction, replacement and fatal-state
   handling, followed by ordinary recovery, actual Chaos and the original full
   throughput/latency screen. The observer does not establish candidate speedup.
   Persistent-map work remains secondary;
   do not repeat rejected owned-buffer or worker/transport sweeps without new
   causal evidence.
   DPDK requires cross-host/NIC
   evidence. A real-disk panel must retain every sync and acknowledgment rule.
6. After the write phase, return to bounded dynamic multi-Raft (#22), epoch routing
   (#23), recoverable membership (#24), automatic splits (#25) and placement
   (#27), with their existing storage/recovery/proof prerequisites. Metadata and
   scheduling must be replicated or safely replaceable. Only object storage may
   be a service-critical singleton.

CI remains local. GitHub CI is reserved for releases or explicitly selected key
milestones. These write checkpoints close no original industrial work package.
