# Single key/value buffer qualification

Date: 2026-09-16 UTC. Source base: `e9ebdc42f2171fd5ab62f3fc2d2d010d88e55d22`.

The new isolated engine representation passes source/model/proof/layout
qualification. It combines key and value bytes into one private allocation.
When both are nonempty, insertion and overwrite remove one allocator request;
actual map entry requests shrink by 24 bytes, including for long keys. There is
no engine timing or new database-QPS result. Production remains unchanged.

The previous [inline40 screen](INLINE-KEY-PERFORMANCE.md) stays failed. Its
long-key entry overhead motivated this distinct representation; neither the
held callback variants nor owned-buffer consumption is included. Selected
archery 1.2.3, rpds 1.2.1 and triomphe 0.1.16 compilation sources match all 50
source/manifest members of their Cargo.lock-pinned upstream archives.

## Representation and source mapping

The [private type](../scripts/entry-buffer/entry_buffer.rs) owns a `Box<[u8]>`
containing `key || value` plus `key_len`. The key prefix alone determines Eq,
Ord and Borrow. The engine's private map becomes `CfMap<EntryBuffer, ()>`;
`get_key_value` exposes the stored key object and its value suffix. Equal-key
rpds insertion replaces the whole Entry, so overwriting a key publishes the new
value even though ordering ignores that value. Retained persistent roots own
the old Entry. This relies on the inspected, pinned rpds implementation.

All public output conversion copies the separate key and value bytes. Resident
GET borrows the payload from the captured view. Range bounds and delete-range
temporary keys contain key bytes only. Checksums still visit each key followed
by its value. The candidate uses no unsafe code. The private map is not exposed
through generic equality, serialization or mutable unit-value access: those
operations would not implement the engine's payload semantics.

`checked_add` and an `isize::MAX` bound guard the combined allocation length.
Both write entry points preflight every Put before mutating any column family;
the replicated path retains applied-position refusal before this new preflight.
This adds an explicit combined-buffer representability condition. Individual
public key/value limits and formats are unchanged. An unrepresentable sum
returns an engine error before batch mutation. Scalar boundary tests cover
overflow without attempting huge allocations; real maximum-sized input batches
were not allocated. Allocation failure retains ordinary Vec/Box behavior and
is not proved recoverable here. The preflight's scan cost must remain inside
the later measured `write_applied` window.

## Correctness and conditional proof

The complete local engine/common target suites pass **202 baseline / 206
candidate tests**, with Clippy warnings denied in both arms. Each arm has 25
existing ignored entries: 23 external MinIO-related tests and two WAL
microbenchmarks. Those entries were not newly executed here.

The unchanged independent BTreeMap model validates **240 generated live
prefixes and 1,080 retained old views per arm**, across all column families,
owned/resident GET, scan/forward/reverse iteration/seek, position/revision,
refusal, checksums, range deletion and bound/output lifetimes. Two additional
tests in both arms retain seven payload versions of the same keys, vary values
from empty through 65,536 bytes, and distinguish identical combined bytes with
different key boundaries. Four private-type tests cover every key length
0–2,048 with varied values, input mutation, clone ownership and binary ordering.

The [Lean proof](../proofs/lean/entry-buffer/README.md) checks **23 theorems and
13 rejecting controls**. It establishes split identity, checked-length bounds,
key-only comparisons, lookup/replacement/deletion, range projection, arbitrary
ordered mutation histories and preflight refusal. This is conditional
representation refinement, not verified Rust extraction. Safe Rust/compiler and
rpds order/replacement/persistence are explicit premises; association-list
updates model contents, not balanced-tree traversal. No new Raft proof,
allocation-failure, lock or panic-recovery proof is claimed.

## Allocation observations

An ordinary ThinLTO release allocation probe uses the exact qualified type,
original rpds and existing jemalloc request counter. It records **198 rows**:
11 key lengths × six value lengths × constructor/clone, insert and overwrite.
Key lengths are 0, 1, 27, 35, 39, 40, 41, 128, 136, 1,024 and 4,096; values are
0, 1, 7, 128, 4,096 and 65,536 bytes. It records no elapsed time.

On this x86-64 build, EntryBuffer is 24 bytes, aligned to eight. Its key/unit
tuple is 24 bytes versus the original Vec/Vec tuple's 48 bytes. Tuple size is
not a claim about the layout of the private rpds Entry. Actual map requests
independently establish the 24-byte reduction for every observed size.

With 128-byte values:

| Key bytes | Insert requests, baseline → candidate | Insert bytes, baseline → candidate | Overwrite requests, baseline → candidate | Overwrite bytes, baseline → candidate |
| --- | ---: | ---: | ---: | ---: |
| 0 | 3 → 3 | 240 → 216 | 2 → 2 | 184 → 160 |
| 27 | 4 → 3 | 267 → 243 | 3 → 2 | 211 → 187 |
| 35 | 4 → 3 | 275 → 251 | 3 → 2 | 219 → 195 |
| 40 | 4 → 3 | 280 → 256 | 3 → 2 | 224 → 200 |
| 41 | 4 → 3 | 281 → 257 | 3 → 2 | 225 → 201 |
| 128 | 4 → 3 | 368 → 344 | 3 → 2 | 312 → 288 |

Construction/clone requests exactly one combined buffer when its length is
nonzero, zero when both inputs are empty, and no reallocation in all observed
cases. When either input is empty, the baseline already needs at most one data
buffer; the candidate still saves 24 entry bytes but no buffer request.
Overwrite allocates the replacement before freeing the old entry, with zero
net live-byte growth. Each row's six allocator fields, live and peak requested
bytes were independently recalculated. Release disassembly checks total and
zero length before the constructor's first call and retains the Vec-to-Box
conversion; observed counters, not an assumed compiler optimization, establish
the lack of reallocation. These are allocator requests, not jemalloc size-class
footprint or RSS. Large maps and pinned-path costs still need measurement.

## Next gate

The [evidence packet](entry-buffer-qualification-v1/README.md) retains exact
sources, compiler/test/proof records, allocator rows, disassembly and independent
audit. The [next timing plan](entry-buffer-qualification-v1/next-performance-plan.json)
first qualifies fresh ordinary-release engine paths, then measures eight write
cases over original and padded-long inputs, overwrite/unique insertion, and
with/without old views. Separate counters precede ABBA timing. Material original
write gains, long-key mean/p99 bounds and exact state checks are required before
read/snapshot/range/delete panels run. That screen is not yet executed.

No new distributed recovery, MinIO, actual Chaos Mesh or Redis comparison is
included. Database promotion still requires material matched benefit, complete
local correctness, ordinary recovery, actual Chaos Mesh and three-copy Redis
throughput/latency with durability scopes stated. No industrial roadmap
checkbox closes from this component qualification. Bulk output stays under
`/mnt/data/kv9-work`, the compiler target is reused, and CI stays local.
