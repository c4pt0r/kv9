# Persistent compressed radix prototype qualification

Updated: 2026-09-16 UTC. Tracking: [#51](https://github.com/c4pt0r/kv9/issues/51),
parent [#9](https://github.com/c4pt0r/kv9/issues/9).

Subsequent proof checkpoint: [155 point-operation model theorems and 17
rejecting controls](RADIX-POINT-PROOF.md) now pass for this exact Rust source.
The complete index proof remains open; no timing or production promotion follows.

The isolated safe Rust index now exists and passes its first implementation
qualification. The common/engine suite passes **213 tests, with 25 pre-existing
ignored tests**; this includes the 11 radix tests, rather than adding 11 to 213.
An independent BTreeMap model checks **6,770 mutation prefixes**, retained roots
and **818 valid bound pairs** with five mixed-direction iterator patterns.
Six deliberately faulty Rust variants compile and are rejected by the tests.
All six allocation-lifetime cases return to their starting requested-byte totals.

**The complete source-bound algorithm proof remains open.** No radix timing,
database QPS, Redis comparison, recovery/Chaos acceptance or production promotion
is claimed. The main workspace still selects rpds. This is implementation and
test progress on a new index, not evidence that it is faster.

## Implementation and invariants to prove

Source: [lib.rs](../scripts/resident-radix/src/lib.rs). A leaf owns separate full
key/value vectors. A branch owns a compressed prefix, an optional terminal entry
and a sorted vector of unique byte-labelled child pointers. The terminal is
separate from the 256 byte labels, including zero. Branches are boxed so that
leaf storage does not inherit the branch's size. There is one fixed child-vector
design; no fanout, alignment or representation sweep has been performed.

For an accumulated incoming path `P`, the intended representation invariant is:

1. A leaf's full key starts with `P`.
2. A branch with prefix `C` has a terminal only for the exact key `P ++ C`.
3. Every key below child byte `b` starts with `P ++ C ++ [b]`. Edges are strictly
   ascending and unique. A published branch has at least two alternatives,
   counting its terminal and children.
4. The root's interpretation contains exactly `size` unique entries. No node
   appears twice in one root; distinct snapshots may share subtrees.

The independent test validator reconstructs paths and checks these conditions
after mutations. This finite evidence is not an inductive proof of preservation.

Insertion selects exact replacement, leaf split, compressed-prefix split,
terminal update, new edge or descent. Only modified paths become unique through
`Arc::make_mut`; replacing a shared leaf avoids first copying buffers that will
be discarded. Root clone copies an Arc and a length. Unmodified subtrees remain
shared. Original input buffers are still separately owned.

Deletion first checks presence so misses preserve sharing without allocation.
It detaches the affected path into heap frames, removes the entry, then restores
parents iteratively. Local normalization converts a terminal-only branch to a
leaf and joins the parent prefix, edge byte and child prefix when collapsing a
nonterminal unary branch. Full-key leaves need no prefix adjustment.

Forward/reverse cursors use prefix comparisons and ordered edges to seek without
scanning unrelated subtrees. The two cursors stop at full-key crossover. Bounds
are needed only during seek, so an iterator never borrows temporary caller bounds.
Invalid bounds use BTreeMap's documented panic convention; the engine models
exercise the valid half-open intervals used by its APIs.

Node-level destruction detaches children into an explicit pending vector. It
uses `Arc::into_inner` and drains children before dropping each detached node.
This also covers replacements and temporary mutation frames. The concurrent
last-owner behavior follows the [Rust Arc contract](https://doc.rust-lang.org/std/sync/struct.Arc.html#method.into_inner);
the installed Rust 1.94.0 implementation is retained with the evidence. The
tests cover exact-root and partial-subtree sharing across eight dropping threads.

Formal refinement still needs to connect every actual split/collapse case,
route/length invariant, cursor order/crossover, mutation frame and snapshot
operation to the finite-map specification. It must state Rust/compiler/Arc and
allocation assumptions explicitly. Local key-order lemmas or a functional map
specification by themselves do not close this gate. Panic/allocation-failure
recovery is not an added API guarantee: the prototype currently follows ordinary
Vec/Arc allocation behavior, and its owned deletion frames do not promise rollback
after a panic. Production engine poisoning/recovery semantics remain unchanged.

## Local validation

| Evidence | Result and scope |
| --- | --- |
| Standalone prototype | 11 tests, zero failures/ignored; warnings-denied Clippy |
| Short binary/prefix histories | 720 mutation prefixes in six histories: three orders with and without retained roots |
| Every byte label | 2,050 mutation prefixes over 1,025 distinct keys, with a complete old root retained |
| Generated histories | 4,000 mutation prefixes over four deterministic seeds, checking up to six old roots after every mutation |
| Bounds | 818 valid bound pairs; forward, reverse and five mixed next/next_back patterns; fused exhaustion and temporary-bound lifetime checks |
| Long keys | Compressed-prefix split/collapse cases up to 16,384 bytes, plus a 1 MiB binary key and extension; no new public key-length cap |
| Deep paths | 4,096-layer prefix ladder: mutation, range/predecessor, Debug and complete deletion on a 64 KiB thread stack |
| Concurrent reclamation | Eight 64 KiB-stack threads releasing exact-root and partial-subtree snapshots of a 2,048-layer ladder; all retained Weak node observers expire |
| Isolated engine | 213 passed, 25 existing ignored: all CFs, batches, owned/resident snapshots, iterator lifetimes, checksum/range deletion, applied-position refusal and revision publication |
| Allocation lifetime | 54 observations across six datasets; O(1) snapshots and absent deletes allocate nothing; complete teardown returns every case to its requested-byte baseline |

The 25 ignored tests remain 23 external MinIO-related tests and two WAL
microbenchmarks. This run is not new MinIO, ordinary process-recovery or actual
Chaos Mesh acceptance. The isolated engine's `mem.rs` change is exactly its
map import and type alias. No lock, mutation ordering, position/revision or Raft
code is changed. The original engine models are reused unchanged.

The first Clippy attempt found three lint issues, corrected before accepted
qualification. A later negative control exposed a **test coverage gap**: dropping
an overwrite of a uniquely owned leaf passed the original short-key histories
because they retained old roots. The final histories cover both ownership modes.
The final standalone and engine suites were rerun with this extension; all six
controls then rejected. Earlier attempts are retained separately, never counted
as successful final controls.

| Deliberate fault | Final rejecting check |
| --- | --- |
| Discard uniquely owned leaf overwrite | Independent model history fails |
| Keep the consumed edge byte in a split prefix | Path/model history fails |
| Omit the edge byte during branch collapse | Path/model history fails |
| Skip an inclusive terminal bound | BTreeMap range comparison fails |
| Reverse a cursor's traversal direction | BTreeMap range comparison fails |
| Replace iterative destruction with recursive field destruction | The 64 KiB-stack test aborts with stack overflow; core dumps disabled |

## Actual layout and requested memory

On the recorded x86_64 Rust 1.94.0 build: map **16 B**, node **48 B**, boxed
branch payload **96 B**, edge **16 B**, entry **48 B**, Arc handle **8 B**.
These are `size_of` values, not total allocations or resident-memory estimates.

The ordinary ThinLTO release allocation probe uses the existing request counter
with System as its delegate. It records allocation, reallocation, deallocation,
net live and peak requested bytes. There is no elapsed-time measurement. All
input/model construction happens outside measured windows. Values are 128 bytes.

| Dataset | Keys | Build live requested bytes | Extra live bytes after overwriting one quarter with old roots retained |
| --- | ---: | ---: | ---: |
| Empty | 0 | 0 | 0 |
| All byte labels and terminals | 1,025 | 272,672 | 214,816 |
| Prefix ladder | 1,025 | 1,360,064 | 836,288 |
| Common prefix | 4,096 | 965,593 | 326,573 |
| Long common prefix | 4,096 | 1,457,473 | 449,573 |
| Injective varied prefix | 4,096 | 970,835 | 245,909 |

These six datasets are allocation/lifetime diagnostics, not the declared future
matched timing corpus. Requested live bytes include keys/values, nodes, branch
storage and retained Vec capacities. Compressed-prefix `drain` can retain unused
capacity, particularly in prefix ladders. Copying shared branch terminals can
copy full keys/values. The pending teardown vector can reallocate: this is measured,
not described as allocation-free destruction. Physical memory, allocator usable
sizes, path costs and comparison against the selected engine remain unmeasured.

## Evidence and next gate

The [manifest](persistent-radix-prototype-v1/manifest.json),
[qualification summary](persistent-radix-prototype-v1/qualification.json),
[source audit](persistent-radix-prototype-v1/source-audit.json) and
[evidence archive](persistent-radix-prototype-v1/evidence.tar.gz) retain source,
build/test logs, raw allocation observations and compiled-control outcomes.
ELFs and Cargo cache contents are excluded; their identities remain recorded.
Run commands are in the [prototype README](../scripts/resident-radix/README.md).

Next: finish the actual algorithm's source-bound formal refinement and rejecting
proof controls. Then predeclare ordinary-release and separate allocation screens
against the selected original engine, including original, long and injective
varied-prefix keys, both orders, mean/p99, material writes and read/range gates.
Keep the rejected single-buffer/accessor family stopped. Database promotion still
requires material matched benefit, local correctness, ordinary recovery, actual
Chaos Mesh and three-copy Redis throughput/latency. Raft, durability, read
authorization and the no-singleton requirement remain unchanged.
