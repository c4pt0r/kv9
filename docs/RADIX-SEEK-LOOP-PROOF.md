# Radix indexed seek, native-word loop and physical cursor correspondence

Updated: 2026-09-16 UTC. Tracking: [#51](https://github.com/c4pt0r/kv9/issues/51),
parent [#9](https://github.com/c4pt0r/kv9/issues/9).

This checkpoint adds **41 Lean theorems**, with **408 checked together**, for
the radix prototype's indexed seek loop, native-word offsets, physical pending
Vec and double-ended range consumption. **35 new rejecting controls** cover
changed algorithms, invalid proofs and substituted Rust sources. The earlier
point, cardinality, recursive cursor and insertion/deletion proofs are unchanged.

For valid ordered roots and representable query lengths, the checked seek
machine terminates and produces the same pending tasks as the earlier cursor
model, in exactly reversed physical storage order. Its actual append/pop model
then yields the same entries. Two initialized cursors support arbitrary mixed
forward/backward histories with the earlier bound filtering, crossover and
no-duplicate-key guarantees.

**Full Rust verification remains open.** These are reviewed source-correspondence
models, not verified Rust extraction or an Arc/COW heap proof. Physical ownership,
saved roots, reclamation, storage representability and exact-source differential
execution still precede timing. This checkpoint has no new performance result.

## Proof chain

1. `PendingVector.lean` represents the pending Vec in bottom-first order:
   append pushes and the last element pops. It proves exact correspondence
   with the earlier top-first stack, including branch terminal/child ordering,
   expansion termination and entry/exhaustion results.
2. `SeekPartition.lean` connects the sorted search result to the exact
   `[..index]` and `[index + found..]` partitions. Hit and miss paths retain
   different child sets. Endpoint pushes and mismatch byte accesses agree
   with the earlier inclusive/exclusive forward/reverse seek model.
3. `SeekStep.lean` exposes suffix, edge-index and slice failures rather than
   assuming them away. Each valid transition excludes failure. Every descent
   selects the indexed child, preserves its valid ordered path and query
   prefix, computes the exact next depth and strictly decreases subtree work.
4. `SeekLoop.lean` composes these transitions while retaining physical pending
   order. It proves exact root initialization, nonfailure, bound-filtered rows
   and subsequent entry/exhaustion results without a fuel limit.
5. `SeekWordStep.lean` and `SeekWordLoop.lean` separately perform the source's
   native-word common-prefix conversion, byte accesses, depth additions,
   index conversion and hit-bit addition. Query-length bounds and the proven
   maximum of 256 distinct child labels exclude wraparound. The word loop
   refines the same complete seek result; arithmetic is not silently replaced
   by unbounded natural-number operations.
6. `VectorRange.lean` connects physical Vec cursors and cached endpoints to the
   earlier two-frontier range state. Each concrete step and arbitrary mixed
   direction history refines that model. Initialization uses the word seek
   loop. No-duplicate-key results and invalid-bound refusal carry through.

Principal results include `next_vector_refines`, `step_seek_refines`,
`seek_loop_refines`, `step_word_seek_refines`, `seek_word_loop_refines`,
`vector_range_run_refines` and `vector_root_range_history`.

## Source correspondence and limits

The unchanged candidate source is
[`scripts/resident-radix/src/lib.rs`](../scripts/resident-radix/src/lib.rs),
SHA-256 `00235871d6f168b3549da377793429dd4db281549425dbab38b1a66c37103fcc`.

| Source operation | Checked value-level correspondence | Remaining premise |
| --- | --- | --- |
| `Cursor::seek_slice` suffix/common-prefix decisions | All stop/descent choices; exact depth, accesses, pushes and termination | Query slice is representable; common-prefix and byte comparison contracts |
| `binary_search_by_key`, before/after slices and child selection | Unique ordered-search contract; exact hit/miss partitions; safe bounds and native-word hit increment | Standard-library search/slice/Vec behavior, not a proof of their implementations |
| `pending.extend`, `push`, `pop` and `next_entry` | Actual bottom-first order and branch expansion; next result and exhaustion | Heap storage, references, allocation and Rust/compiler contracts |
| Cached first/last values and mixed range iteration | Initialization, crossover, equal endpoints, frontier updates and arbitrary histories | Borrow validity and root lifetime through physical Arc ownership |

The word model requires `SeekBoundFits`, which states that a bounded query's
length fits the machine word. Descendant offsets are derived from that bound;
search indices are derived from the ordered byte labels. The library/heap bridge
must connect the length premise to the Rust slice. This proof does not establish
the separate resident-count premise used by insertion, immutable saved-root
ownership, allocator behavior or concurrent reclamation. Successful operations
are modeled; allocation failure and whole-compiler verification remain outside
this scope.

## Local qualification and retained evidence

The [seek contract](../proofs/lean/radix/seek-contract.json) binds all 37 modules,
the earlier proof contracts/verifiers and the unchanged candidate Rust/tests.
Pinned Lean 4.33.1 compiles fresh copies with warnings denied and implicit
theorem parameters disabled. All 408 theorem dependency inventories allow only
`propext`, `Classical.choice` and `Quot.sound`.

Thirty semantic controls change pop/push ordering, inclusive endpoints,
hit/miss partitions, mismatch ordering, bounds, child indices, depth arithmetic,
pending accumulation, word arithmetic or range frontier behavior. They must
fail substantive proof obligations. Separate controls reject a proof hole and
a custom false axiom. Three Rust substitutions test reviewed hash binding only;
they are explicitly not semantic Rust proofs.

The [result](radix-seek-proof-v1/result.json),
[audit](radix-seek-proof-v1/audit.json),
[manifest](radix-seek-proof-v1/manifest.json) and
[archive](radix-seek-proof-v1/evidence.tar.gz) retain exact copies, compiler
commands, controls and authoring diagnostics. Initial contract preparation
refused a pop anchor that appeared in both a definition and its proof; the
definition-only anchor was made unique before qualification.
The first full run rejected the custom-axiom sample at the wrong gate: removing
its original proof left an unused premise, which warnings-as-errors caught
before the axiom audit. The corrected sample explicitly consumes that premise
and still depends on the injected false axiom. A targeted preflight checks that
it compiles and exposes the dependency; the complete qualification then reruns.
Both the failed run's diagnostic and the corrected probe are retained.

No candidate Rust or production crate changed. The earlier 213-test prototype
qualification, including 25 existing ignored tests, remains prior evidence.
No new database QPS, Redis parity, ordinary recovery, Chaos Mesh acceptance or
runtime promotion is claimed. All checks are local; hosted CI is not triggered.

## Next gate

Establish a concrete Arc/COW heap relation for mutable places, immutable saved
roots and reclamation. Connect routing-buffer/query-length and resident-result
count representability to actual addressable storage. Differentially execute the
exact Rust candidate against the executable models. Only after the complete
proof gate, run predeclared matched ordinary-release throughput/mean/p99 and
separate allocation comparisons. Runtime promotion still requires material
database benefit, recovery, actual Chaos Mesh and three-copy Redis comparison.
The executable comparison must pin the Rust compiler/library and match its
machine-word width to Lean's `USize` model.
