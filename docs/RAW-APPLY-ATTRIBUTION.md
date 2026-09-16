# Raw apply attribution and direct lowering experiment

Recorded: 2026-09-16 UTC. This is an offline component experiment, not a new
database throughput result. Production code and the selected CRC runtime are
unchanged. The [latest end-to-end write results](WAL-PREALLOCATION-PERFORMANCE.md)
remain 137,873.776 Put calls/s and 1,022,750.059 BatchPut(64) items/s at c64
for their recorded source, client and volatile three-voter fixture.

The subsequent [composed MemEngine screen](RAW-LOWERING-COMPOSED.md) is now
complete. It holds the prototype: overwrite gains shrink to 1.134% / 0.452%
without/with snapshots, and initial-fill unpinned p99 worsens 3.309%. The original
component evidence and pre-composition reasoning below remain separately scoped.

Directly appending each command's mutations into the final batch removes
temporary mutation vectors. On exact retained commands, lowering mean falls
**10.896%**, from **14.208 to 12.660 us/group**, and pooled p99 falls from
**39.744 to 34.504 us/group**. Both measurement orders improve. The absolute
mean saving is only **1.548 us/group**: measure lowering together with real
MemEngine updates before considering runtime integration or a full campaign.

## Exact input and source

The input is the segment preselected for the
[resident-index experiments](RESIDENT-INDEX-EXPERIMENTS.md), not a segment
chosen after observing these results. Its 106 frames contain 100,096 mutations.
Engine payloads alone do not identify command boundaries, so the harness joins
them to the same node's original Raft log:

- Engine segment: 16,520,140 bytes; SHA-256
  `87aef6bfacdb58dd5e271db5ab96f3d16a03a4264c1594b7bf2503b6660b37fa`.
- Raft log: 360,581,738 bytes; SHA-256
  `e0bc3415aafb0f9eaaf19f5af2752471d012fb56e9f77976c71a1c54286765df`.
- Extracted group file: 16,519,240 bytes; SHA-256
  `d978ba9e49131f6277f975ecd333c33854845cc03bd14ee57a22fff915a08350`.

The extractor checks every engine frame length, position and CRC. The Rust
harness verifies the complete retained Raft stream's checksums and protobuf
records: one configuration record, 2,327 hard-state records and 33,991 entries,
with final term 1, committed index 33,991 and no overwritten suffix.

Replaying the first 14,221 entries reconstructs the catalog state for the
selected groups. Entries 14,222 through 15,785 supply exactly 1,564 original
commands, all fenced Raw writes. Each command re-encodes to its original wire
bytes. The actual catalog adjudicator accepts every selected fence; these
non-system Default-CF writes do not change that catalog. Concatenated production
lowering matches every byte of all 106 original engine payloads. Group counts
and encoded sizes satisfy the existing 128-command / 1 MiB bounds. Protocol
entries produce no user-data mutations here; this harness does not reconstruct
membership or establish recovery authority.

Dependencies use the retained clean source
`86aa6fc07d82f4d49b43fbfe309e0e5c20276da5`. The six pinned command, raw-group,
state-machine, fence, WriteBatch and MemEngine files match report-parent
`c058465c4cc317d6d6780dd769b150c555b4ae13`. Builds bind 165 dependency source
files and 269 unchanged registry identities/checksums before and after build.

## Measurement

Rust/Cargo 1.94, release ThinLTO, one codegen unit, jemalloc 0.6.1 and CPU 4 are
fixed. The host is shared. Each component has a warmup followed by 12 passes
over the 106 original groups. Attribution order is decode/lower/fence followed
by fence/lower/decode. Each reported component pools 2,544 group samples; p99
is ranked from the combined raw samples, not averaged from row percentiles.

Timing uses an ordinary jemalloc executable. A separate executable counts
allocator requests and supplies no latency result. File reads, replay, input
validation and setup occur outside timing. Destruction of final decoded or
lowered output is outside timing; internal temporary work in production
callees remains included. The fence call includes its returned temporaries.

| Component | Mean per group | Pooled p99 per group |
| --- | ---: | ---: |
| Production command decoding | 19.722 us | 56.707 us |
| Production lowering and batch append | 14.649 us | 40.385 us |
| Actual catalog fence evaluation | 2.336 us | 6.432 us |

These intervals exclude group eligibility/position checks, full apply,
MemEngine updates, WAL I/O, RPC, Raft and queues. They cannot be subtracted from
the earlier roughly 549 us runtime apply mean: the populations and boundaries
differ. This sample does not justify caching or skipping fence evaluation.

## Changed-variable comparison

The private direct-lowering prototype appends cloned keys/values into the final
WriteBatch instead of constructing and appending one temporary WriteBatch per
command. It retains mutation order, column-family conversion, all key/value
clones and ordinary final-vector growth. It introduces no reservation, sorting,
coalescing, fence cache or index change. Its fenced inputs have already passed
the original adjudicator; it is not a production fence-bypass API.

This comparison runs baseline/direct/direct/baseline independently of the
attribution rows above. Those older rows are not pooled into this comparison.

| Order | Baseline mean | Direct mean | Mean change | Baseline p99 | Direct p99 |
| --- | ---: | ---: | ---: | ---: | ---: |
| Baseline first | 14.433 us | 12.747 us | -11.680% | 41.177 us | 35.215 us |
| Direct first | 13.984 us | 12.573 us | -10.086% | 38.952 us | 34.184 us |
| Pooled | 14.208 us | 12.660 us | -10.896% | 39.744 us | 34.504 us |

Counts are identical across opposite orders. Each row covers 12 passes,
18,768 commands and 1,201,152 mutations:

| Allocator requests per row | Baseline | Direct |
| --- | ---: | ---: |
| Allocations | 2,422,344 | 2,403,576 |
| Requested allocation bytes | 194,941,440 | 186,463,488 |
| Reallocations | 79,812 | 9,828 |
| Requested reallocation bytes | 304,346,112 | 186,772,992 |

Reallocation calls fall 87.686%. These are requested layout bytes, not resident
memory or net live allocations. Reallocation requests count the new requested
size. Final output destruction is outside the counter interval as well.

Both implementations match every original group payload. A separate passing
differential test covers empty input/operations, empty and binary keys/values,
Put/Write/accepted FencedWrite, all three column families, deletions, repeated
overwrites and every command prefix. This is not a mechanized proof, complete
state-machine validation or new Chaos Mesh acceptance.

## Decision and next experiment

Keep this prototype outside production. The next bounded experiment should
combine lowering with actual MemEngine updates for prepopulated overwrites,
initial insertion and unique insertion, with and without retained snapshots.
Check final state and snapshot immutability against the baseline. That answers
whether the 1.548 us component saving survives actual index work before spending
on integration, source-bound proof, recovery, actual Chaos Mesh and matched
database timing. Do not repeat the completed component measurements or the
previously rejected index variants without a changed variable.

## Retained evidence

The [evidence packet](raw-apply-attribution-v1/README.md) contains the exact
harnesses, plans, source/build identities, raw timings and allocation counts,
command/group joins, terminal logs and independently recomputed statistics.
Both build/run pairs completed successfully. The original attribution build
retains its unused-import warning; the direct prototype fixes that import.
No build or measurement was repeated merely to clean up the original log.

Original large inputs and executables remain under `/mnt/data/kv9-work`.
The metadata archive has 81 members and 758,716 compressed bytes; full bounded
readback verifies every member, gzip EOF and tar termination. This publication
check does not rerun the benchmark, replay every parser independently or add
any new database-QPS, Redis-parity or industrial milestone claim. CI was local.
