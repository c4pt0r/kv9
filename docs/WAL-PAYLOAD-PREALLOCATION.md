# WAL payload preallocation

Updated 2026-09-16. The new `wal-payload-preallocation` feature is **off by
default**. It reuses the segmented writer's already validated record length to
reserve the payload buffer before encoding. Both configurations execute the
same byte emitter. This removes a source of buffer growth without changing the
record format, accepted batch limit, CRC, fsync or Raft acknowledgment rules.

The candidate passes source-bound capacity/CRC proofs and local regressions.
Encoder microbenchmarks improve across the retained large-batch corpus and six
small synthetic cases. **There is no new database QPS or Redis comparison.**
[Matching releases and ordinary recovery](WAL-PREALLOCATION-RUNTIME.md) now
pass: four complete histories, 728 operations including 62 unknowns, twelve
fresh drains and fourteen exited lifetimes. [Actual 21-window Chaos Mesh](WAL-PREALLOCATION-CHAOS.md)
now also passes: 9,241 complete operations, four final drains and 31 exited
server lifetimes. Keep the selected CRC runtime until end-to-end throughput
plus latency qualification establishes a useful gain.

## Implementation and safety

`WalSegment::append` still checks failed-writer state, position ordering and
`encoded_size` before encoding or writing. With the feature enabled, it reserves
`frame_size - 32 - 4` bytes. The existing size calculation limits payloads to
64 MiB; checked conversion to `usize` precedes reservation. The unchanged
post-encoding length check also remains. Legacy WAL callers and the default
configuration pass zero initial capacity.

The existing emitter still appends count, ordered Put/Delete tags, column family,
key length/key bytes and value length/value bytes. Header and payload checksums,
three `write_all` calls, synchronization, failed-writer fencing and summary
publication after successful synchronization are unchanged. The candidate keeps
both existing size-validation scans in the stream/segment path; it does not
combine this experiment with a validation or durability change.

The [capacity proof](../proofs/lean/wal-preallocation/README.md) checks **11
universal theorems and seven rejection controls**. It proves byte independence
from initial capacity/growth policy, exact payload size and sufficient capacity
for every append prefix, plus frame subtraction and record bounds. The checker
pins full Rust source and verifies the change against `0fc2753`. The reviewed
Rust/stdlib mapping is explicit; this is not a mechanized proof of the compiler,
allocator, physical storage or whole Raft implementation. Allocation failure
liveness and a physical allocator call count are outside the model.

CRC declarations are identical to the selected implementation. Its updated
whole-WAL-file hash passes the original fresh strict checker: **47 theorems,
256 + 2,048 compiled table entries and eight rejected controls**. The actual
Lean toolchain is the restored, repository-pinned 4.33.1 installation.

Both default and feature configurations pass the full local workspace checks:
**851 tests pass, 28 are ignored in each**, and workspace/all-targets Clippy
with warnings denied passes. These are overlapping populations, not 1,702
distinct tests. The ignored cases include external MinIO acceptance, the two
explicit microbenchmarks and a separately verified response-loss fixture; they
are not counted as passing. An earlier isolated jemalloc feature executable
passes 145 engine tests with two ignored tests, before the small benchmark was
added. Existing engine tests exercise torn tails, corruption, actual Linux
write/sync failures, writer fencing, segment rotation and recovery.

Three new tests cover golden bytes and mutation order, mixed binary inputs and
growing values, and the exact 64 MiB boundary. An oversized append leaves the
fresh segment at its header with zero records and does not poison the healthy
writer; a later valid append succeeds. The new boundary test removes its own
fresh directory. Existing fixtures retain their NVMe placement.

## Encoder measurements

All measurements use the actual Linux jemalloc 0.6.1 global allocator, default
features disabled, matching the server selection. They run one
baseline/candidate/candidate/baseline sequence on CPU 4 of a shared host. Both
arms include existing size validation, encoding and buffer destruction. There
is no CRC, WAL I/O, Raft, RPC or client queue in these timings.

The preselected corpus is reused unchanged from the
[resident-index experiment](RESIDENT-INDEX-EXPERIMENTS.md): **106 batches /
100,096 mutations**, 64–2,688 mutations per batch. Each arm makes 128 passes,
or 13,568 batch measurements and 2,114,081,792 encoded payload bytes. Its source
is one retained interior WAL segment, not a complete client history.

| Corpus order | Baseline mean | Reserved mean | Baseline p99 | Reserved p99 |
| --- | ---: | ---: | ---: | ---: |
| Baseline then candidate | 8.044 µs | 5.474 µs | 21.160 µs | 15.028 µs |
| Candidate then baseline | 6.819 µs | 5.326 µs | 17.533 µs | 14.648 µs |

Pooled mean encoder time is **7.431 → 5.400 µs (−27.339%)**. Baseline movement
between orders remains visible; this is a short shared-host microbenchmark.
Do not subtract it from sampled apply-group timings, which have a different
population, or extrapolate the percentage into whole-database throughput.

The later small-case check uses 32-byte keys, fixed hot inputs and 20,000 samples
per arm. Nanosecond values include timer overhead. Pooled means are below;
per-arm p99 remains separate in the packet and improves in both orders for all
six cases.

| Mutations | Value bytes | Baseline mean | Reserved mean | Mean change |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 0 | 93.243 ns | 31.910 ns | −65.777% |
| 1 | 256 | 112.905 ns | 32.032 ns | −71.629% |
| 1 | 4,096 | 144.314 ns | 64.955 ns | −54.991% |
| 64 | 0 | 603.753 ns | 353.750 ns | −41.408% |
| 64 | 256 | 727.110 ns | 334.677 ns | −53.972% |
| 64 | 4,096 | 4,101.180 ns | 2,547.396 ns | −37.886% |

The [packet](wal-payload-preallocation-v1/README.md) retains original commands,
logs, exact source/executable hashes, proof inputs/results and all measurements.
The [preparation helper](../scripts/wal-preallocation/README.md) reproduces the
isolated allocator configuration and verifies the original corpus. The original
large-corpus and later small-case executables remain separately identified.
No completed comparison was repeated after adding the small-case harness.

## Next qualification

1. Matching current-source default/feature releases and ordinary three-voter
   recovery are complete. Preserve their original evidence and the fixed,
   previously requalified performance client; do not rerun them unchanged.
2. Actual candidate Chaos Mesh acceptance is complete, including independent
   full histories, recovery, archive/readback and cleanup. Preserve the original
   evidence. The existing C04 pre-upload
   acceptance remains separate and incomplete; this increment does not close it.
3. The [same-source end-to-end comparison](WAL-PREALLOCATION-PERFORMANCE.md)
   now passes all eight smokes and sixteen timed cohorts. Loaded Put gains
   0.545%; loaded batch gains 1.474% pooled but changes direction by order,
   with unchanged loaded pooled p99. Keep default-off and do not repeat the
   unchanged screen. Redis was not rerun; its earlier reference retains
   separate acknowledgment and durability semantics.
4. Keep the industrial roadmap, write goal and split/multi-Raft prerequisites
   open. Do not restart the completed resident-index experiments: their static
   follow-up supplied no justified correction for the insertion regression.

All new bulk outputs and proof tools are under `/mnt/data/kv9-work`. The reused
compiler caches and declared active test WAL fixtures remain on NVMe. CI stays
local; no hosted workflow is needed for this experimental source checkpoint.
