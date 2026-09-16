# Radix concrete graph insertion

Updated: 2026-09-16 UTC. Tracking: [#51](https://github.com/c4pt0r/kv9/issues/51),
parent [#9](https://github.com/c4pt0r/kv9/issues/9).

This extension adds **125 Lean theorems in 26 modules**, with **679 theorems in
81 modules checked together** and **30 rejecting controls** passing fresh local qualification. The
previous 55 modules and eight contracts remain unchanged, as do the Rust
prototype and production crates.

The new executable graph model follows all six insertion actions and the actual
root/edge mutation paths. A completed put has the specified finite-map value,
correct new-key flag and mathematical cardinality, preserves every saved root's
value, and maintains the live ownership inventory. The evaluator terminates
because every descent consumes at least one remaining key byte.

**This establishes graph insertion, not complete native Rust verification.**
Deletion/normalization, native Arc and payload/storage correspondence, concurrent
COW composition, physical-loop word arithmetic and exact-source differential
execution remain open. No timing or production promotion follows from this work.

## From a borrowed place to a complete put

| Layer | Checked obligation |
| --- | --- |
| Graph updates and assignment | Changing a focused value keeps finite interpretations; temporary child tokens move into edges; the old slot token is released through the proved iterative reclamation loop |
| Physical contexts | Actual parent identities and child indices reconstruct the whole root; untouched siblings retain their represented values |
| Mutation invariant | The working root owns a token; every borrowed ancestor is private and unreachable from saved roots; a borrowed place adds no reference |
| COW and descent | Both root and nested COW preserve the invariant; copying a child does not increase any ancestor's reference count; descent borrows the selected child only after the current branch is unique |
| Completing actions | ReplaceLeaf, Terminal and AddEdge preserve ownership and snapshots and produce the exact full-root value and inserted flag |
| Split helpers | Old-leaf terminal extraction, retained child references, fresh leaf allocation, sorted edge assembly and prefix drain occur in source order |
| Selection and loop | Selection reads StoredNode payloads and stored edge labels; all successful abstract steps are simulated; remaining key length proves termination |
| Root and observables | Empty initialization and nonempty insertion refine putRoot; lookup results, structural validity/order, inserted flag and cardinality follow for the represented result |

`MutationInvariant` combines ownership, focused representation, the physical
ancestor context, private ancestor counts, saved-root separation and mutable-place
alignment. `make_place_unique_invariant` preserves it through actual root and child
COW. A count-one child alone is not considered snapshot-private: its ancestors
must already be private and isolated.

`MutationResult` requires the whole resulting working root, not just the changed
leaf or branch. For nested replacement, the proof plugs the new child through
the repointed context before dropping the old slot token. The working root is
held through reclamation, so its value survives even when teardown releases
several descendants. Saved roots keep their previous values throughout.

The old-exhausted leaf split uses `makePlaceUnique`, models both `mem::take`
operations by an empty-buffer leaf payload, then moves the captured Entry into
the new branch terminal. Other leaf split cases retain the old child with one
explicit external Arc token; they do not needlessly COW that child. Branch split
first makes the current branch unique, drains through the selected routing byte,
retains the shortened child, constructs the new parent, and assigns it.

`insertPlaceLoop` carries only the executable heap, working root, borrowed place,
external ownership inventory, depth and fresh Entry. Ghost Trees and ancestor
frames appear only in its proof. There is no added runtime frame vector, fuel
or termination guard. Principal results are `insert_place_loop_refines`,
`put_heap_refines`, `put_heap_completed` and `put_heap_observables`.

## Exact scope and native contracts

The candidate remains
[`scripts/resident-radix/src/lib.rs`](../scripts/resident-radix/src/lib.rs),
SHA-256 `00235871d6f168b3549da377793429dd4db281549425dbab38b1a66c37103fcc`.
The reviewed correspondence targets `insert_mut`, `split_leaf`, `split_branch`,
`branch_mut`, branch construction and assignment/drop behavior.

Allocation identities and external Arc tokens are ghost graph state. They do
not establish physical addresses, native reference-counter representation,
memory ordering, allocator behavior, buffer capacities, exact deallocation
instructions or scheduling fairness. Moving Entry values in the model is not
itself a proof of native Vec buffer ownership. These still require explicit
pinned Rust/compiler/library and storage bridges.

Stored edge search is an executable label-only representation of the ordinary
sorted-slice search contract. Strict ordering makes the hit/miss result unique;
this does not prove the library's binary-search code or its complexity. Similarly,
the two-edge constructor represents sorting distinct labels. The successful
insertion theorem starts from a valid ordered tree and valid source selection.
It makes no equivalence claim for deliberately invalid helper inputs or their
panic/allocation traces. The both-exhausted leaf case remains a failure marker.

The physical evaluator currently uses natural-number offsets. Earlier value-level
native-word proofs are retained, but their composition with these physical
transitions and storage-derived bounds remains open. In particular, graph IDs
do not justify a usize cardinality bound; that bridge must use distinct resident
Entry object storage, not possibly empty key/value buffer addresses.

## Local qualification and evidence

The [contract](../proofs/lean/radix/heap-insert-contract.json) binds the candidate,
tests, all proof sources, previous contracts/verifiers and the new verifier.
Pinned Lean 4.33.1 compiles with warnings denied and implicit theorem parameters
disabled. The axiom inventory permits only `propext`, `Classical.choice` and
`Quot.sound`.

The 26 semantic controls alter COW/place indices, swap or drop the wrong token,
mutate a shared leaf, invert flags, change insertion boundaries, reverse sorted
edges, lose or alias retained children, omit the moved-empty payload, change
split terminals or prefix drain, select by values or wrong label order, corrupt
fresh values, or change the loop result. Separate controls inject a proof hole
and a custom false axiom. Two changed Rust files test source-hash rejection only.

Development runs explicitly reused the accepted baseline's compiler outputs
with source and artifact hashes. Qualification recompiles fresh source copies
for the positive case and independently for every model/proof control, stopping
rejected controls at the first failure. Authoring diagnostics and corrected
successors remain in the retained development evidence.

The publication packet contains the [result](radix-heap-insert-proof-v1/result.json),
[audit](radix-heap-insert-proof-v1/audit.json),
[manifest](radix-heap-insert-proof-v1/manifest.json) and
[archive](radix-heap-insert-proof-v1/evidence.tar.gz). Every member is read back
and hashed; compiler outputs are excluded. The previous live-ownership packet
remains unchanged.

No Rust test rerun, new database QPS, Redis parity, recovery/Chaos Mesh acceptance
or industrial checkbox closure is claimed for this proof-only change. The prior
213-test prototype qualification (25 existing ignored) remains prior evidence.
Verification is local and hosted CI is not triggered.

## Remaining implementation gate

Next compose concrete deletion's detached current/parent tokens, child removal,
unwind and normalization. Then connect the physical transitions to pinned native
Arc/no-Weak and compiler contracts, concurrent COW, owned payloads, Vec/Box storage
and machine-word premises. Differentially execute the exact Rust candidate and
models before the predeclared timing/allocation comparisons. Runtime promotion
still needs matched database benefit, local correctness, ordinary recovery,
actual Chaos Mesh and three-copy Redis throughput and latency.
