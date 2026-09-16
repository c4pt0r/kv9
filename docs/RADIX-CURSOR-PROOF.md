# Radix cursor, range and predecessor model proofs

Updated: 2026-09-16 UTC. Tracking: [#51](https://github.com/c4pt0r/kv9/issues/51),
parent [#9](https://github.com/c4pt0r/kv9/issues/9).

This checkpoint adds **80 Lean theorems**, with **287 checked together**, for
the radix prototype's ordered cursor and range algorithms. A new set of
**25 rejecting controls** covers changed cursor/range models, invalid proofs
and source substitutions. The earlier
[point-operation](RADIX-POINT-PROOF.md) and
[cardinality/local-bound](RADIX-CARDINALITY-PROOF.md) checkpoints are preserved;
their 155 and 52 theorems are included in 287, not added again.

The model proves complete inclusive/exclusive forward/reverse seek, pending
stack traversal, double-ended crossover and arbitrary mixed-direction read
histories. Returned entries retain their values. Every finite history returns
each key at most once; enough calls return exactly the selected entries. The
inclusive predecessor is the greatest stored key no greater than its query.

**These are reviewed operational-model proofs, not verified Rust extraction.**
The complete gate still needs explicit Rust loop/vector/frame correspondence,
Arc/COW heap ownership and reclamation, and exact-source differential execution.
The resident-addressability premise of the prior word-counter proof is still
open. No timing, runtime promotion or new database QPS follows from this phase.

## Proof chain

1. `KeyOrder.lean` derives strict full-key bytewise order from routing validity
   and sorted byte edges. It proves prefix/edge comparison rules and the
   inclusive/exclusive acceptance rules independently of the cursor stack.
2. `CursorTraversal.lean` models a pending stack containing node and entry
   tasks. A top-first list represents the reverse of Rust's Vec. Branch
   expansion uses the actual direction-dependent terminal/child order. A
   decreasing pending-work measure proves the `next_entry` loop terminates;
   each result exposes exactly the head of an independently flattened entry
   sequence and retains exactly the rest. No arbitrary traversal fuel is used.
3. `SeekBounds.lean` proves whole-subtree pruning on a compressed-prefix
   mismatch, including an exhausted query, and orders terminals and siblings
   relative to the requested byte. It does not assume that all descendants
   happen to lie on the selected side.
4. `CursorSeek.lean` models every seek branch: leaf comparison, proper prefix
   mismatch, query exhaustion, matching edge and missing-edge partition.
   `seek_root_rows` proves its pending stack represents precisely the filtered
   full-entry list, reversed for a descending cursor.
5. `RangeStep.lean` models both loaded endpoints and pending cursors, including
   crossed endpoints, equal-key final emission, and advancing only the chosen
   side. `range_history_refines` proves every finite sequence of forward/back
   requests agrees with taking entries from the corresponding ends of one
   list. This includes mixed schedules, not only complete forward or reverse
   scans.
6. `RangeInit.lean` connects both real seek models to the exact intersection of
   lower and upper bounds. Empty intersections produce missing or crossed
   endpoints. An independent guard theorem matches the source's rejection of
   descending bounds and equal bounds excluded at both ends. Invalid input is
   modeled as refusal, not an empty successful range.
7. `RangeProperties.lean` proves conservation of full entries, remaining
   length, no repeated keys, complete exhaustion, fused empty behavior and
   inclusive predecessor maximality/absence. These conclusions compose from
   actual modeled traversal and seek transitions, not a specification-only
   list iterator substituted for the cursor.

The key exported results are `seek_root_rows`, `root_range_history`,
`root_range_no_duplicate_keys`, `root_range_exhaustive`, `range_fused`,
`range_bounds_match_source_guard` and `predecessor_refines`.

## Source correspondence and limits

| Rust component | Current modeled correspondence | Remaining implementation obligation |
| --- | --- | --- |
| Vec pending tasks and `next_entry` | Top-first stack, exact child/terminal order, decreasing work and flattened-sequence refinement | Vec push/extend/pop and borrowed-node correspondence |
| `Cursor::seek_slice` | All prefix, terminal, byte-hit/miss and direction cases; complete bound filtering | Relate actual depth/slice arithmetic and binary-search indices to the modeled path and sorted forest partition |
| `Range::next` / `next_back` | Exact missing/crossed/equal endpoint guards and chosen-side advance; arbitrary mixed histories | Connect the Rust fields and cursor borrows to the operational state |
| `range` bounds checks | Independent equivalence to both source assertions; refusal is separate from successful empty selection | Standard byte-vector comparison and panic behavior premises |
| `seek_le` | Actual inclusive reverse seek followed by one traversal result; greatest-key or absence result | Borrowed returned key/value and source/library correspondence |
| Snapshots during iteration | Cursor theorems quantify a fixed valid immutable root | Arc/COW ownership, lifetimes, concurrent reclamation and the pending addressability relation |

The forest search is a recursive abstraction of an ordered vector partition;
it does not certify the standard library's binary-search implementation. The
model uses UInt8/list byte ordering. Compiler, Vec/slice, comparison and shared
pointer contracts remain explicit implementation premises. Existing local
offset/index bounds will be used in the remaining actual-loop simulation.

## Verification and evidence

The [cursor contract](../proofs/lean/radix/cursor-contract.json) binds all
nineteen modules, prior contracts/verifiers and the exact unchanged Rust
prototype/tests. Pinned Lean 4.33.1 compiles everything in fresh directories
with warnings denied and `autoImplicit` disabled. Every theorem's dependency
inventory permits only `propext`, `Classical.choice` and `Quot.sound`.

Nineteen semantic controls break terminal/child traversal, pending retention,
prefix pruning, exclusive or reverse seek, matching/missing-edge handling,
crossed/equal endpoint behavior, chosen-side advancement, public bounds
refusal or inclusive predecessor. They fail substantive proofs. A proof hole
and custom false axiom are rejected separately. Four modified Rust sources
fail the same hash-binding gate used by the verifier; those controls do not
constitute semantic verification of the substituted Rust.

The [manifest](radix-cursor-proof-v1/manifest.json),
[result](radix-cursor-proof-v1/result.json), [audit](radix-cursor-proof-v1/audit.json)
and [archive](radix-cursor-proof-v1/evidence.tar.gz) retain the proof commands,
controls, exact sources and original authoring diagnostics. The prior twelve
proof modules and both prior contracts are unchanged. Candidate Rust and
production crates are unchanged; the earlier 213-test prototype qualification
remains applicable and is not counted as new execution here.

Next finish actual iterative insertion/deletion/seek frame and vector
correspondence, then the Arc/COW heap relation and differential execution.
Only after the complete gate should the predeclared matched performance
screen start. Recovery, actual Chaos Mesh and database/Redis throughput and
latency remain required for promotion. All checks in this phase are local;
hosted CI remains manual-only.
