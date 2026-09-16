# Inline short-key qualification

Date: 2026-09-16 UTC. Source base: `14f35ecb772e060b58b6898e648630a72033bf8c`.

The isolated candidate stores keys of at most **40 bytes** inside the persistent
map entry and retains a Vec fallback for arbitrary longer keys. Source/model/
proof qualification passes. Nonempty short-key insertion removes **one allocator
request**, but the key object grows from **24 to 48 bytes**. There is no engine
timing or database-QPS result yet, and production remains unchanged.

The [candidate and tools](../scripts/inline-key/README.md) use selected original
archery 1.2.3, rpds 1.2.1 and triomphe 0.1.16. Their installed compilation sources
match Cargo.lock-pinned upstream archives. Neither held callback extraction is
included. The [full packet](inline-key-qualification-v1/README.md) retains actual
sources, compiler inputs, test/proof outputs, allocator observations and audit.

## Representation and engine integration

The private key uses a safe Rust enum: an inline length/40-byte array or a Vec.
Construction establishes the length bound and copies the exact bytes; short
inputs are not borrowed. Eq, Ord and Borrow explicitly use `as_slice()`. Padding
and enum discriminants never determine ordering. Empty, binary and long keys
retain their ordinary byte semantics. No unsafe code is added to the candidate.

The copied engine changes its private map key, owned range bounds, borrowed
delete arguments and owned byte-key conversion on output. Persistent rpds roots,
values, input mutation order, lock boundaries, applied-position refusal and
revision logic stay unchanged. Range iterators still own their bounds and may
outlive the caller's bound buffers. Public and stored keys remain Vec bytes;
wire/storage formats, Raft, WAL and read authorization do not change.

## Correctness and conditional proof

Both engine/common workspaces pass Clippy with warnings denied. Their complete
local target suites pass **200 baseline / 203 candidate tests**. Each arm has
**25 existing ignored entries**: 23 external MinIO-related tests and two WAL
microbenchmark entries. Those ignored tests were not newly executed here.

The same independent BTreeMap-based model runs against both engine arms. Each
arm checks **240 generated live prefixes / 1,080 retained old views**, with all
column families, owned/resident GET, bounded scans, ascending/descending streams,
reverse seek, exact position/revision and refused nonadvancing applications.
Additional cases check checksum/range deletion, empty and repeated mutations,
metadata-only revision rules, stream-bound ownership and empty/prefix/binary keys
at lengths 1, 2, 27, 35, 39, 40, 41, 42, 64, 128 and 1,024. Candidate unit tests
also cover every length 0–2,048, changed source buffers, poisoned unused padding,
and byte ordering/equality across inline and heap representations.

The [Lean proof](../proofs/lean/inline-key/README.md) checks **19 theorems / nine
rejecting controls**. It proves encode/decode identity, padding independence,
the u8 length cast, equality/order/borrow coherence, lookup/update/delete and
range/reverse/limit projection, and arbitrary ordered mutation histories.

This is conditional representation refinement with reviewed source mapping.
Rust/compiler/Vec/Clone and rpds map/persistent-snapshot contracts remain explicit
premises. The association-list mutation model describes contents; sorted
traversal is an rpds premise, not a proved property of its abstract prepend-based
put. No verified Rust extraction, allocation/lock/panic proof or new Raft proof
is claimed. Source substitutions, exposed padding, length-only comparisons,
retained deleted keys, reversed histories, proof holes and custom axioms reject.

## Actual layout and allocator requests

A separate ordinary ThinLTO release probe compiles the exact key source and
original registry rpds using the existing jemalloc request counter. Its **33
observation rows** measure construction/clone plus single-entry insertion and
overwrite at 11 lengths. It emits **no elapsed time**. Inputs/values are prepared
outside the window; insertion clones both inside, matching MemEngine ownership.
Validation and output drop are outside the measured allocation window.

On this x86-64 build, the key is 48 bytes, aligned to eight bytes; Vec is 24 bytes.
The key/value tuple grows from 48 to 72 bytes. This tuple observation is not a
claim about the layout of rpds' private Entry. Actual map allocation requests
independently show an extra 24 bytes per allocated entry before subtracting the
removed short-key buffer. Constructor disassembly branches to the heap path at
length 41; the short path contains only a memcpy call and no allocator call.

With 128-byte values, the recorded single-entry request totals are:

| Key length | Insert allocations, baseline → candidate | Insert bytes, baseline → candidate | Overwrite allocations, baseline → candidate | Overwrite bytes, baseline → candidate |
| --- | ---: | ---: | ---: | ---: |
| 0 | 3 → 3 | 240 → 264 | 2 → 2 | 184 → 208 |
| 27 | 4 → 3 | 267 → 264 | 3 → 2 | 211 → 208 |
| 35 | 4 → 3 | 275 → 264 | 3 → 2 | 219 → 208 |
| 40 | 4 → 3 | 280 → 264 | 3 → 2 | 224 → 208 |
| 41 | 4 → 4 | 281 → 305 | 3 → 3 | 225 → 249 |
| 128 | 4 → 4 | 368 → 392 | 3 → 3 | 312 → 336 |

Short construction/clone performs zero requests; longer keys perform one request
for their actual length. Empty keys already needed no Vec buffer, so they gain
24 requested bytes without saving a request. Long keys retain their buffer and
also add 24 bytes. These are instrumented Rust allocation requests, not jemalloc
size-class footprint, RSS or proof of allocation behavior in a future timing ELF.
Single-entry observations do not establish pinned-snapshot or large-map cost.

## Next gate and retained limits

The [declared next screen](inline-key-qualification-v1/next-performance-plan.json)
first builds and prepares the actual ordinary-release engine pair, then measures
eight write cases: original and padded-long datasets, overwrite/unique insertion,
with/without an old snapshot. Separate allocation-only runs precede ABBA timing.
If the write gates fail, stop and retain the failure. Only a passing write screen
proceeds to owned/resident, snapshot and affected range-interface panels. All
timer scopes, order directions, raw samples, long-key costs and exact state
checks remain explicit. No timing stage has been implemented or executed yet.

The first preparation stopped on an incorrect strict patch-anchor count (six
expected ranges, five present), before modifying the candidate or running tests.
The corrected v2 workspace is retained separately. Initial proof-authoring errors
remain in development logs. An initial allocation probe preceded a formatting
check; the formatted wrapper was freshly rebuilt and its 33 rows rechecked.
Only that final probe is selected. No failed or superseded evidence is pooled.

Promotion still requires material matched database benefit, full local
correctness, ordinary recovery, actual Chaos Mesh and matched three-copy Redis
throughput/latency. Latest database results remain **137,873.776 Put calls/s /
1,022,750.059 BatchPut(64) items/s**, source `86aa6fc`, c64, 128-byte values,
three Raft replicas and the shared-host volatile tmpfs fixture. No new MinIO,
distributed recovery, actual Chaos, Redis parity or industrial checkbox closure
is claimed. CI stays local; bulk output stays under `/mnt/data/kv9-work`.
