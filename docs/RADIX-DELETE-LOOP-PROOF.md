# Radix indexed-vector and deletion-loop correspondence

Updated: 2026-09-16 UTC. Tracking: [#51](https://github.com/c4pt0r/kv9/issues/51),
parent [#9](https://github.com/c4pt0r/kv9/issues/9).

This checkpoint adds **52 Lean theorems**, with **339 checked together**, for
indexed edge operations and the radix prototype's iterative deletion path.
It includes a separate machine-word execution model and the public stored-count
and return-flag wrapper. **24 new rejecting controls** cover changed algorithms,
invalid proofs and source substitutions. Earlier point, cardinality and cursor
proofs remain unchanged and are included in 339, not added again.

The main result connects the deletion loop's actual depth, binary-search index,
detached parent and unwind state to the already proved finite-map deletion.
Under the stated input/library premises, lookup changes only for the removed
key, the resulting root preserves its invariants, absent deletion is unchanged,
and successful deletion decrements the word count exactly once.

**The full Rust index proof remains open.** This is reviewed value-level source
correspondence, not verified extraction, Arc/COW heap verification or a new
performance result. Insertion and seek loop correspondence, heap ownership and
exact-source differential execution remain before the planned timing screen.

## Proof chain

1. `EdgeVector.lean` gives an invertible correspondence between a forest and
   ordered `(byte, child)` vector contents. Indexed get, insert, remove,
   take/drop and replacement agree with list positions. A successful removal
   reduces length by one and leaves the saved index safe for reinsertion,
   including insertion at the new vector's end.
2. `EdgeSearch.lean` states an independent library contract: a hit addresses
   the equal label; a miss lies within bounds and partitions strictly smaller
   and strictly larger labels. On a strictly ordered forest, every result
   satisfying this contract equals the reference search result. The reference
   search is a linear rank model; it does not purport to implement or verify
   Rust's binary-search algorithm.
3. `EdgeMutation.lean` connects indexed changes to the earlier recursive
   forest algorithms. Insertion at a missing label, update at a hit, indexed
   deletion and reinsertion into a detached parent all refine the same model.
   This does not yet establish the entire mutable insertion loop.
4. `DeleteLoop.lean` models the source's depth-based descent and saved frames.
   Rust checks presence once, then adds each compressed prefix length without
   checking the prefix again. The proof derives that this is safe: the prefix
   matches, the next byte is in bounds, search hits an existing child, and that
   child still contains the target. Each frame records a parent with one edge
   removed, its original index and byte. A top-first list represents the
   reverse of Rust's Vec. Unwinding restores the edge if a child remains, then
   normalizes the parent. Checked insertion bounds hold for every reachable
   frame. No arbitrary recursion fuel or maximum key length is imposed.
5. `DeleteWordLoop.lean` executes both depth additions using `USize`, then
   proves equivalence with the natural-number loop for a query length below
   `USize.size`. No addition wraps on that reachable path. The public wrapper
   performs one presence guard, takes the root, runs the loop and subtracts
   one from the stored word count only on success. It refines the previous
   `wordErase` state and flag; the prior count theorem supplies the
   non-underflow and exact-cardinality result.

The principal results are `edge_search_contract_unique`, `edge_delete_hit`,
`delete_loop_refines`, `erase_loop_no_assertion_failure`,
`delete_word_loop_refines`, `erase_word_loop_lookup` and
`erase_word_state_refines`. The assertion-failure outcome is distinct from
successfully deleting the final entry, so an empty result cannot hide failure.

## Source mapping and limits

The unchanged source is
[`scripts/resident-radix/src/lib.rs`](../scripts/resident-radix/src/lib.rs),
SHA-256 `00235871d6f168b3549da377793429dd4db281549425dbab38b1a66c37103fcc`.

| Source operation | Checked value-level correspondence | Remaining premise or work |
| --- | --- | --- |
| `binary_search_by_key` and `edges[index]` | Unique sorted-search result, hit bounds and child selection | Standard-library search/access contracts; no verified stdlib extraction |
| `edges.remove(index)` and saved `(parent, index, byte)` | Exact vector removal, preserved index and detached-parent contents | Vec storage and Arc ownership |
| `depth += prefix.len()` and `depth += 1` | Presence invariant, exact suffix, word arithmetic and safe query access | Matching Rust/Lean word widths and valid representable slice lengths |
| Leaf/terminal removal, frame pop, edge restore and normalization | Complete terminating descent/unwind, root and lookup refinement | Actual heap mutation, `branch_mut`, unique/shared Arc cases and reclamation |
| Root publication, count decrement and returned bool | Public state/flag equality with counted finite-map deletion | Physical root ownership; allocation failure behavior is outside the successful-operation model |
| Insertion and seek | Indexed edge helpers are available | Full action/split/depth/mutable-slot and cursor Vec simulation remains open |

The model does not obtain safety from source hashes. Hashes prevent a reviewed
source or proof file from being silently substituted. Rust language/compiler,
byte comparisons, Vec/slice, search and shared-pointer contracts still require
the remaining explicit implementation argument. No whole-Rust verification is
claimed. The resident-entry addressability premise needed for insertion count
representability remains open; a deletion-only result does not discharge it.

## Local verification

The [deletion contract](../proofs/lean/radix/delete-contract.json) binds all
twenty-four modules, the prior contracts/verifiers and unchanged Rust source
and tests. Pinned Lean 4.33.1 compiles fresh copies with warnings denied and
implicit theorem parameters disabled. The dependency inventory for every
theorem permits only `propext`, `Classical.choice` and `Quot.sound`.

Nineteen semantic controls change vector removal/insertion/replacement, search
rank/partition, parent restoration, unwind, insertion-at-end bounds, leaf or
terminal removal, prefix/edge offsets, ancestor retention, absent deletion,
word offsets, count decrement or success flags. They must fail substantive
proofs. A proof hole and custom false axiom are rejected separately. Three
changed Rust files fail reviewed source binding only, not a semantic Rust
verification gate.

The [result](radix-delete-proof-v1/result.json),
[audit](radix-delete-proof-v1/audit.json),
[manifest](radix-delete-proof-v1/manifest.json) and
[archive](radix-delete-proof-v1/evidence.tar.gz) retain exact commands, compiler
logs, changed controls and original authoring diagnostics. The authoring helper
itself returns zero even after a module error; only per-module logs/results and
the complete qualification runner determine acceptance. Earlier unsuccessful
proof attempts are retained rather than replaced with the final pass.

The first qualification stopped at a verifier diagnostic-classification gap:
Lean rejected the wrong restoration index with `Application type mismatch`,
but the verifier recognized only other proof-error spellings. The classifier
was corrected, and the complete qualification was rerun in a fresh directory.
The original rejected model, compiler log and failed qualification are retained
in the archive; this was not acceptance of an otherwise passing faulty model.

Candidate Rust and production crates did not change. The earlier 213-test Rust
prototype qualification, including 25 existing ignored tests, is prior evidence
and was not counted as a new execution. No new QPS, Redis comparison, recovery
or Chaos Mesh acceptance follows. All checks are local; hosted CI stays manual.

Next finish insertion and indexed seek correspondence, then Arc/COW ownership,
saved roots, reclamation and executable differential evaluation. After the full
gate, run the predeclared matched write/read-range timing and allocation screens.
Runtime promotion still requires a material database benefit, local correctness,
ordinary recovery, actual Chaos Mesh and three-copy Redis throughput/latency
with separate durability scopes.
