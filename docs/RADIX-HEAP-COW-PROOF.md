# Radix concrete heap graph and COW primitive proof

Updated: 2026-09-16 UTC. Tracking: [#51](https://github.com/c4pt0r/kv9/issues/51),
parent [#9](https://github.com/c4pt0r/kv9/issues/9).

This checkpoint adds **76 Lean theorems**, with **484 checked together**, for
concrete node identities, strong-reference locations, root and nested child
copy-on-write primitives, and preservation of saved roots. **22 rejecting
controls** cover changed models, invalid proofs and substituted Rust sources.
The earlier 37 modules and six proof contracts remain unchanged.

The graph stores nonrecursive node payloads and child identities. Its inductive
representation relation unfolds actual stored references into the previously
proved trees; it does not assume an address-to-already-proved-tree map. Reference
counts are derived from external slots and all stored child edges. For valid
represented graphs, making a root or an eligible child unique preserves its
tree value, isolates it from saved roots, and gives the returned node exactly
one counted strong reference under the stated closure/root premises.

**This is a heap-graph and COW primitive checkpoint, not full Rust verification.**
Complete mutations, temporary ownership, native Arc behavior, live allocation
accounting, reclamation, buffer storage and exact-source differential execution
remain open. No candidate timing or runtime promotion follows from this proof.

## What is now checked

| Modules | Additional theorems | Result |
| --- | ---: | --- |
| `HeapGraph` | 15 | Exact heap reads, allocation and writes; finite node/edge representation; unique interpretation; reachable subtrees; write framing and fresh-node separation |
| `HeapOwnership` | 6 | Explicit external/child reference locations; count-one implies one owning slot; root and child snapshot separation; writes preserve separated saved roots |
| `HeapClone` | 7 | Shallow graph clone preserves child identities and tree value; root COW returns a separated node and preserves every previously represented root |
| `HeapAcyclic` | 7 | A represented child's tree work is smaller; a descendant cannot return to its ancestor; allocation/write framing for child edges |
| `HeapCounts` | 9 | Counts equal external references plus stored child edges; represented heaps are closed; a fresh root clone has exactly one incoming reference |
| `HeapFrameReach` | 6 | Reachability framing, clone/ancestor separation, saved-root preservation, and indexed child-reference replacement |
| `HeapChild` | 3 | Nested COW with a private parent preserves parent/child values and saved roots while replacing the selected edge |
| `HeapChildCounts` | 7 | Fresh node references are absent before cloning; indexed replacement introduces exactly one incoming reference; child COW returns count one |
| `HeapSubstitute` | 9 | Equal-value replacement preserves all old root interpretations, including ancestors; root/child COW preserves representation of every occupied node |
| `HeapExamples` | 7 | Kernel-checked shared-parent counterexample and the successive parent/child copies needed for an isolated write |

Principal results are `make_root_unique_refines`, `make_root_unique_count`,
`make_child_unique_refines`, `make_child_unique_count`,
`clone_child_preserves_all_roots`, and `make_child_unique_modelled`.

## Why child reference counts alone are insufficient

Two snapshots can point to the same parent allocation while its child has only
one stored incoming edge. That child's strong count is one, but both snapshots
can reach it. Mutating it directly would therefore affect both snapshots.

`HeapExamples.lean` constructs this graph and proves its counts and reachability.
Copying the shared parent creates a second edge to the child. The subsequent
child COW copies that child and repoints only the working parent's edge. A final
write changes the copied leaf while preserving the old leaf and old parent edge.
These are executable Lean-model examples, not new Rust differential tests or a
claim that the unchanged Rust implementation contained this bug.

The general child proof requires both parent separation from saved roots and
the selected child's unique slot (or a fresh clone). `makeChildUnique` also
checks parent count one as a ghost precondition for a mutable nested slot. The
Rust implementation obtains this permission through prior `Arc::make_mut` and
borrowing; no extra runtime parent-count check is proposed.

## Source correspondence and remaining assumptions

The candidate remains
[`scripts/resident-radix/src/lib.rs`](../scripts/resident-radix/src/lib.rs),
SHA-256 `00235871d6f168b3549da377793429dd4db281549425dbab38b1a66c37103fcc`.

| Rust operation or property | Checked graph correspondence | Still to establish |
| --- | --- | --- |
| Root `Arc::make_mut` | Keep a count-one node or shallow-clone its payload and replace the working root; retain all saved-root values | Native no-Weak Arc contract, compiler/borrow semantics and live-state invariants |
| `branch_mut(slot).edges[index].node` followed by COW | Select the actual indexed slot; preserve parent value; copy a shared child; replace exactly that parent edge | Compose all insertion, split, deletion and normalization transitions with the graph |
| Cloned branch child Arcs | Every copied edge contributes an additional reference at a distinct parent/index location | Native atomic counter correspondence, overflow behavior and concurrent operations |
| Immutable map snapshots | Separated-node writes cannot change a saved root's interpretation; COW itself preserves all prior root interpretations | Complete mutation histories and cursor lifetimes through actual ownership |
| Owned keys, values, prefixes, Vec edges and boxed branches | Payload values and ordered child-reference lists | Distinct physical buffers, native addresses/lengths, moves, allocation failure and destruction |
| Iterative `Node::drop` and `Arc::into_inner` | External slots can represent temporary owned references | Actual worklist transitions, eventual reclamation, concurrent final-owner handoff and resource bounds |

`NodeId` is an unbounded ghost allocation identity, not a machine address.
Allocation appends a fresh identity; an empty cell can represent removed storage.
`NodeRep` is a finite unfolding of stored cells. `HeapModelled` requires every
occupied cell to have such an unfolding, while `HeapClosed` excludes dangling
child references. These predicates do not establish that every occupied cell has
a live owner or that all unreachable allocations are reclaimed.

`strongCount` counts supplied external slots and every edge in occupied cells.
Its mathematical inventory is exact for that graph; supplying all real roots,
detached deletion frames, temporary Arcs and destructor worklist references is a
remaining implementation obligation. No native atomic implementation is proved.
Owned `Vec`/`Box`/`Entry` payloads remain value abstractions. The private candidate
node API creates no Weak references, but the relevant standard-library and
compiler contracts must remain explicit in the eventual implementation bridge.

## Local qualification and evidence

The [contract](../proofs/lean/radix/heap-cow-contract.json) pins all 47 modules,
earlier contracts/verifiers, the new verifier and unchanged Rust candidate/tests.
Pinned Lean 4.33.1 compiles fresh copies with warnings denied and implicit theorem
parameters disabled. The dependency audit covers all 484 theorem declarations
and permits only `propext`, `Classical.choice` and `Quot.sound`.

Eighteen semantic controls change heap addresses, allocation order, reference
slots/counts, root uniqueness, copying, nested parent eligibility, selected edges
or cloned payloads. They must fail substantive proof obligations. Separate
controls reject a proof hole and a custom false axiom. Two Rust substitutions
test exact source-hash binding only; they are not semantic Rust proofs.

The [result](radix-heap-cow-proof-v1/result.json),
[audit](radix-heap-cow-proof-v1/audit.json),
[manifest](radix-heap-cow-proof-v1/manifest.json) and
[archive](radix-heap-cow-proof-v1/evidence.tar.gz) retain sources, compiler commands,
dependency reports, controls and development diagnostics. Compiler outputs are
excluded from the archive and every retained member is read back and hashed.

No candidate Rust or production crate changed. The previous 213-test prototype
qualification, including 25 existing ignored tests, remains prior evidence and
was not rerun for this proof-only extension. There is no new database QPS, Redis
parity, recovery/Chaos Mesh acceptance or full implementation gate closure.
All checks are local; hosted CI is not triggered.

## Next implementation gate

Compose the heap primitives with the actual insertion actions and split helpers,
then deletion's detached frames and normalization. Track reference changes and
live allocation invariants across those complete transitions. Prove iterative
and concurrent reclamation, and connect routing/query lengths and distinct
resident Entry storage to the earlier machine-word representability premises.

Execute the exact Rust source against executable models with a pinned Rust
compiler/library and matching machine-word width. After that complete gate,
run predeclared matched throughput/mean/p99 and separate allocation comparisons.
Runtime promotion still needs material database benefit, ordinary recovery,
actual Chaos Mesh and a three-copy Redis comparison.
