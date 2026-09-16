# Native object storage and concurrent COW primitives

This checkpoint adds 66 theorems in 10 modules above the accepted graph
insertion/deletion baseline: 104 modules and 789 theorems in one inventory.
It advances the ownership/storage bridge needed before evaluating the radix
candidate as a write-path replacement. It changes no candidate Rust or
production code and reports no new performance result.

The complete implementation gate remains **false**. These are scoped native
object and COW primitives under explicit library/compiler contracts. Native
buffers, transient payload placement, all mutation-place applications,
physical word loops and exact-source differential execution are still open.

## Resident entries and machine-word cardinality

`HeapEntryLocations` locates each represented entry in a leaf payload or branch
terminal. Valid keys exclude duplicate entry-bearing node identities within a
root. `NativeEntryStorage` maps these witnesses to distinct nonzero native
Entry addresses and derives a strict cardinality bound below the address
limit. This does not infer a word bound from mathematical NodeId values or
heap length, and it does not assume the desired entry-count bound.

`NativeObjects` represents occupied Arc<Node> **payload** regions and separately
owned Box<Branch> regions. It records nonzero valid extents, disjoint placement
and exact object occupancy. Shared graph edges refer to the same object owner.
The Arc counter/header is outside the modeled Node region; byte buffers have
not yet been included. The entry placement relation follows from this object
relation and explicit field-offset/layout inputs.

The update proofs preserve these invariants for successful allocation,
same-shape payload writes, branch-to-leaf conversion and reclamation. A
read-back relation proves reclamation only removes old cells; surviving
objects keep their addresses. Composing that relation with the existing
owned drop loop and concurrent release pool supplies the liveness obligation.
The object-region invariant alone is not permission to free a live node.

## COW with concurrent snapshot release

Rust's pinned `Arc::make_mut` chooses copying when its strong-count compare
exchange fails. Another owner can be released immediately after that decision,
so copying may proceed even when the old allocation is now uniquely held.
Rechecking final-time sharing as a precondition would exclude actual behavior.

`copyOwnedRoot` keeps the source token through copying and performs ordinary
old-token destruction on assignment. It remains safe when copying an already
unique source. The allocated payload retains its outgoing child references;
old destruction releases the old edges rather than invalidating the new ones.

`PayloadCopyHistory` makes child retention explicit, one Arc token at a time.
Release workers may run between any two retains. Its invariant preserves the
source value and cell, all held tokens, saved-root values and the exact
partition of retained versus remaining child references. Allocation transfers
those retained tokens into the new payload without cloning them again.

`assignedCopyPool` moves the old owning slot into the call's destructor
worklist. `native_cow_finish` composes arbitrary interleavings of this teardown
with other release workers and proves value preservation, live ownership,
object-region preservation and a unique private new root. The return theorem
additionally requires this call's worklist to be empty; other workers may
still be running. The unique/no-copy path retains the original identity during
release-only interference.

Concrete examples cover the shared-to-unique race, release between two child
retains, child preservation when the old branch is destroyed, an unreferenced
node left by a missing old release, and a missing child retain. This checkpoint
models a release-only external environment. Concurrent external snapshot
cloning and mutable edge-slot applications are further composition work.

## Native boundary and observations

`native-library-pins.json` binds Rust 1.94.0, its compiler hash and eight standard
library sources. Source review confirms private Node/Edge/Branch/Arc fields,
`forbid(unsafe_code)`, no Weak creation or raw-Arc conversions, and public
borrowed key/value APIs. The standard library supplies the no-Weak uniqueness,
clone and last-owner handoff contracts. Neither source hashes nor these
proofs verify rustc, atomic instructions, allocator implementations or Rust's
whole memory model. The scope is successful operations; allocation failure,
reference-count abort and panic recovery are not availability claims.

A safe observer built from the unchanged candidate plus an observation entry
point recorded one actual layout: 64-bit pointers; Node payload 48 bytes,
Branch payload 96, Entry 48; leaf Entry offset 0 and branch terminal offset 48.
Empty key/value Vec buffers shared their sentinel, while occupied Entry
objects were distinct and their owner regions disjoint. This demonstrates why
buffer data pointers cannot serve as entry-identity witnesses. It is a sampled
compiler-output observation, not universal layout proof or differential
qualification. The receipt pins the candidate prefix, observer, compiler and
linked core/alloc/std libraries.

## Qualification and remaining work

Fresh local qualification passed all 789 theorems and all 18 rejecting controls.
The [contract](../proofs/lean/radix/native-primitives-contract.json) and
[local verifier](../scripts/resident-radix/prove-native-primitives.py) require
fresh compilation of all 104 modules and an axiom audit of all 789 theorems.
Each of the 16 model/proof controls starts with fresh dependencies. Fourteen
semantic faults, a proof hole and a custom false axiom must be rejected. Two
Rust source substitutions test source binding only. Allowed Lean axioms are
propext, Classical.choice and Quot.sound. Earlier development checks reused
pinned accepted artifacts; final qualification does not.

The [evidence packet](radix-native-primitives-proof-v1/manifest.json) records
the final qualification, controls and archive read-back. The unchanged prior
baseline is [graph deletion](RADIX-HEAP-DELETE-PROOF.md). Historical Rust test
counts remain historical; this proof-only change does not rerun or relabel
runtime qualification, recovery or Chaos Mesh.

Next connect these primitives to root and mutable edge slots in concrete
mutation paths, add external snapshot-clone interference, model owned byte,
edge and frame buffers plus in-flight payload moves, and compose graph loops
and size fields with machine-word arithmetic. Preserve the observed late leaf
scope release. Then qualify exact-source Rust/model differential histories
before the predeclared original/long/injective-varied-prefix timing and
allocation comparisons. Production promotion still requires matched write
benefit, local correctness, ordinary recovery, actual Chaos Mesh and
three-copy Redis throughput and latency. Hosted CI remains manual only.
