# Radix insertion action, split and word-loop correspondence

Updated: 2026-09-16 UTC. Tracking: [#51](https://github.com/c4pt0r/kv9/issues/51),
parent [#9](https://github.com/c4pt0r/kv9/issues/9).

This checkpoint adds **28 Lean theorems**, with **367 checked together**, for
the radix prototype's six insertion actions, absolute-offset split helpers,
nested mutable-slot descent and machine-word execution. **28 new rejecting
controls** cover changed algorithms, invalid proofs and source substitutions.
The earlier point, cardinality, cursor and deletion proofs are unchanged.

For valid inputs satisfying the explicit buffer/count premises, the resulting
root and inserted/replaced flag agree with the earlier finite-map insertion.
The word-state wrapper initializes an empty map with count one and adds exactly
the returned insertion bit for an existing root. Its arithmetic checks cannot
fail under those premises. Updates retain all other keys and values.

**Complete Rust verification remains open.** This is reviewed value-level
source correspondence, not verified Rust extraction, Arc/COW heap ownership or
a new performance result. Indexed seek/pending-Vec correspondence, heap
ownership and exact-source differential execution still precede timing.

## Proof chain

1. `InsertSplits.lean` models source byte reads at `depth + common`, the leaf
   prefix slice, and branch prefix removal through the first divergent byte.
   It preserves the leaf helper's both-exhausted assertion and branch helper's
   required indexed access. For a valid path and distinct full keys, leaf
   splitting cannot exhaust both suffixes. A proper branch split always has
   its old edge byte. Both helpers return exactly the earlier split trees.
2. `InsertStep.lean` keeps action selection separate from execution, as in
   Rust. It covers replacement, leaf split, branch split, terminal update,
   edge insertion and child descent. Selection checks the relevant suffix
   slice bounds; execution checks the node kind, split result, byte access and
   vector insertion/access bounds. A valid path establishes every premise.
   A descent records its actual source parent and child, strictly smaller
   subtree work, valid extended path, safe index and exact next depth.
3. `InsertLoop.lean` composes those transitions through any number of nested
   slots. Ghost frames describe the containing branch and selected child;
   their checked replacement preserves the rest of the parent. The decreasing
   tree-work measure proves termination without a fuel or key-length cap.
   Public root and lookup results refine the previously proved map insertion,
   including the inserted/replaced flag and preserved tree invariants.
4. `InsertWordSplits.lean` performs cut and prefix-drain arithmetic with
   `USize`. `InsertWordStep.lean` separately performs the source's word end
   offset and next-depth calculation, then proves the selected action and its
   execution agree with the natural-number machine. Routing key/prefix buffer
   lengths below `USize.size` are explicit input/library premises.
5. `InsertWordLoop.lean` carries native-word depth through the full loop and
   proves each projected next depth converts back exactly. The public state
   wrapper models empty-root initialization and existing-root count addition.
   An explicit counter-overflow failure is unreachable when the new resident
   entry count fits. The resulting state and flag equal the earlier `wordPut`;
   its cardinality theorem then gives the exact stored size.

The principal results are `split_leaf_at_refines`, `split_branch_at_refines`,
`step_insert_refines`, `insert_loop_refines`, `step_word_insert_refines`,
`insert_word_loop_refines` and `put_word_state_refines`.

## Source correspondence and explicit limits

The unchanged source is
[`scripts/resident-radix/src/lib.rs`](../scripts/resident-radix/src/lib.rs),
SHA-256 `00235871d6f168b3549da377793429dd4db281549425dbab38b1a66c37103fcc`.

| Rust operation | Checked value-level correspondence | Remaining implementation obligation |
| --- | --- | --- |
| `InsertAction` selection followed by execution | All six choices and return flags; relevant node kind, path, index and slice conditions | Rust/compiler and ordinary primitive contracts |
| `split_leaf` | Absolute cut, prefix bytes, terminal/child choices and unreachable both-exhausted case | Moving/cloning original key/value buffers through Arc |
| `split_branch` | Shared prefix, old edge byte, exact drained prefix and new terminal/edge | Actual heap mutation, shallow sharing and allocator behavior |
| `slot = &mut ...edges[index].node` | Original parent/child provenance, valid extended path, exact depth, safe nested value context | Mutable-place ownership, Arc uniqueness and COW relations |
| Word arithmetic and stored count | Actual word cut/drain/descent operations and checked count addition refine mathematical values | Routing buffers and resident result must be representable; heap correspondence must connect these premises to Rust |
| Preserved older snapshots | Pure earlier roots remain unchanged in the reference model | Physical sharing, immutable saved roots and concurrent reclamation remain open |

The ghost insertion frames do not describe additional runtime allocations or
a parent-stack algorithm added to Rust. Rust holds a nested mutable slot; the
frames describe how writing that slot changes the root's value. Establishing
that actual Arc/COW mutation implements this context safely is still required.

`RoutingBuffersFit` constrains routing buffers used for offsets, not the whole
heap or allocator. The resident-result count premise is separately needed for
the stored-size addition. The checked overflow outcome exposes that obligation;
this phase does not establish it merely by using `USize`. Under the premise,
checked and wrapping arithmetic agree, so both debug and release arithmetic
behave alike. Allocation failure and whole-compiler verification are outside
the successful-operation model. Standard search/Vec/slice/byte-comparison,
matching word width and Arc contracts remain explicit implementation premises.

## Local verification and evidence

The [insertion contract](../proofs/lean/radix/insert-contract.json) binds all
thirty modules, earlier contracts/verifiers and the unchanged Rust prototype
and tests. Pinned Lean 4.33.1 compiles fresh module copies with warnings denied
and implicit theorem parameters disabled. Every theorem's dependency inventory
permits only `propext`, `Classical.choice` and `Quot.sound`.

Twenty-three semantic controls change split prefixes/cut bytes/terminals,
branch drain length, action selection, depth increments, retained values,
insertion flags, edge or mutable-slot indices, ancestor contexts, word offsets
or count initialization/addition. They fail substantive proofs. A proof hole
and custom false axiom fail separate gates. Three changed Rust sources fail
reviewed hash binding only; source hashes do not prove Rust semantics.

The [result](radix-insert-proof-v1/result.json),
[audit](radix-insert-proof-v1/audit.json),
[manifest](radix-insert-proof-v1/manifest.json) and
[archive](radix-insert-proof-v1/evidence.tar.gz) retain exact source copies,
compiler commands, changed controls and authoring diagnostics. Initial contract
preparation rejected an ambiguous mutation anchor before qualification; the
anchor was made unique, and the failed setup diagnostic is retained. The
authoring helper for this phase exits nonzero after any module failure.

No candidate Rust or production crate changed, and the earlier 213-test
prototype qualification with 25 existing ignored tests is prior evidence,
not a new execution. No timing, new database QPS, Redis parity, ordinary
recovery or Chaos Mesh acceptance follows. All checks are local; hosted CI
remains manual-only.

Next finish indexed seek/depth and pending-Vec correspondence, then the Arc/COW
heap relation, snapshot/reclamation obligations and exact-source differential
execution. The declared matched write/read-range timing and allocation screens
follow the full gate. Runtime promotion still requires material database
benefit, local correctness, ordinary recovery, actual Chaos Mesh and three-copy
Redis throughput/latency with durability scopes stated.
