# Radix concrete graph deletion

Updated: 2026-09-16 UTC. Tracking: [#51](https://github.com/c4pt0r/kv9/issues/51),
parent [#9](https://github.com/c4pt0r/kv9/issues/9).

This extension adds **44 Lean theorems in 13 modules**, giving **723 theorems in
94 modules**. Fresh local qualification passed, including all **28 rejecting
controls**, with independently compiled dependencies for each case. The previous 81 modules and nine
contracts remain unchanged, as do the Rust candidate and production crates.

The new executable graph model composes the complete deletion path: borrowed
presence lookup, exact absent no-op, root token movement, detached descent,
normalization, physical parent-stack unwind, and scope-exit reclamation. A
completed erase has the specified finite-map value, removed flag and
mathematical cardinality, preserves saved roots, and maintains live ownership.

**This establishes a reviewed graph deletion model, not complete native Rust
verification.** Native library/compiler, allocation, payload, concurrent COW,
word/storage bridges and differential execution remain open. This work produces
no new database QPS and does not promote the radix candidate.

## Concrete operations and invariants

| Layer | Obligation |
| --- | --- |
| Edge detachment | Removing an edge moves its child token into the current/temporary inventory, preserving reference counts without an early release |
| Parent COW and search | Make the current branch unique before indexing the query and searching the stored labels; then remove the selected edge |
| Terminal normalization | Take the terminal, retain the second count-one make_mut, and replace the same node identity with a leaf |
| Singleton normalization | Pop the last edge; leave a leaf child's full key unchanged; copy a shared branch child before prefix moves; release the empty parent while holding the returned child |
| Prefix moves | Clear the parent prefix, clear the moved child prefix, then install the joined prefix; child COW cannot increase an ancestor's reference count |
| Parent frames | Runtime frames contain only parent identity, index and byte; the bottom-first Vec corresponds to a top-first abstract frame list |
| Unwind | Pop each saved parent, optionally insert the replacement child, then normalize; all remaining parents and saved roots remain owned |
| Lookup and loop | Stored-node lookup adds no Arc token; deletion descent consumes query bytes and unwind consumes frames, with no fuel bound |
| Erase result | Missing keys leave heap/root unchanged; present deletion returns the specified root and flag, preserves order/validity and changes cardinality by one |

`OwnedResult` requires a represented resulting root, an owned heap under its
complete external-root inventory, and unchanged values for every saved root.
Detached parents are included in that inventory while descendants are copied
or normalized. Temporary child references are moved into restored edges, not
silently duplicated. Existing release-loop proofs preserve held roots through
recursive graph reclamation.

`popOwnedChild` models the actual `Vec::pop`. Its singleton precondition proves
that it selects the same sole child as indexed removal, allowing reuse of the
ownership-transfer theorem without replacing the executable operation. Native
buffer allocation and deallocation remain distinct obligations.

## Leaf scope-exit ordering

The leaf arm of `remove_mut` breaks with `replacement = None` without moving
the local `current` Arc. That token remains live while parent frames unwind.
The terminal arm instead moves `current` into normalization.

Fresh MIR emitted by the pinned Rust 1.94.0 compiler confirms the difference:
the leaf arm retains the drop flag, the replacement root is installed, size is
updated, the emptied frame Vec is dropped, and only then is `current` released.
`finishOwnedLeafDelete` therefore holds the leaf token throughout unwind and
calls `dropOwnedTemporary` afterward. Root installation is a token move;
native size arithmetic and field-level ordering remain part of the word/source
bridge. The retained MIR is compiler-output evidence, not a proof of rustc.

## Source correspondence and limits

The unchanged candidate is
[`scripts/resident-radix/src/lib.rs`](../scripts/resident-radix/src/lib.rs),
SHA-256 `00235871d6f168b3549da377793429dd4db281549425dbab38b1a66c37103fcc`.
The reviewed correspondence covers `get`, the traversal within `get_key_value`,
`remove_mut`, `normalize`, `branch_mut` and node ownership/release operations.

`lookupHeap` observes the borrowed value; it does not model a new owning Arc
or prove a physical returned reference's lifetime. Stored edge search uses
the existing sorted-slice result contract, not a verified standard-library
binary-search implementation. Successful operations start from a valid ordered
tree. Failure markers model unsuccessful source assertions or invalid graph
reads, without claiming equivalence for invalid inputs or allocation failure.

Graph identities are natural numbers, not physical addresses. Prefix/Entry
values do not by themselves prove native Vec/Box ownership, capacities,
allocator behavior or concurrent Arc memory ordering. The physical loops still
need composition with native-word proofs and storage-derived bounds. A usize
cardinality bound must use distinct resident Entry object addresses, not key or
value buffer addresses, which may be empty or shared sentinels.

Principal results are `normalize_owned_refines`,
`unwind_owned_delete_refines`, `delete_owned_loop_refines`,
`lookup_heap_root_refines`, `erase_heap_refines`, `erase_heap_completed` and
`erase_heap_observables`. The runtime loop contains no ghost Tree or proof-only
frame payload and adds no second prefix comparison to deletion descent.

## Verification and retained evidence

The [contract](../proofs/lean/radix/heap-delete-contract.json) binds 115 source
files, including all prior modules/contracts/verifiers and the new verifier.
Lean 4.33.1 uses the pinned binary hash, warnings denied and explicit theorem
parameters. Only `propext`, `Classical.choice` and `Quot.sound` are permitted
in the theorem axiom inventory.

The 24 semantic controls alter token detachment, COW, terminal values, prefix
order, edge pop, parent release, normalization, insertion indices, unwind order,
query search/depth, frame order, late token ownership/release, lookup results,
absent no-op or the removed flag. Separate controls inject a proof hole and a
false axiom. Two changed Rust files test source-binding rejection only.

Development and control preflight reuse source-checked baseline/compiler outputs
with hashes. Full qualification uses fresh compilation independently for the
positive case and every model/proof control. An initial custom-axiom preflight
failed at the unused-parameter warning; the corrected injection explicitly
uses the hypothesis and reaches the intended axiom-inventory rejection.

The publication packet retains the [result](radix-heap-delete-proof-v1/result.json),
[audit](radix-heap-delete-proof-v1/audit.json),
[manifest](radix-heap-delete-proof-v1/manifest.json) and
[archive](radix-heap-delete-proof-v1/evidence.tar.gz). The archive includes the
source contract, command/axiom logs, negative-control diagnostics, development
attempts, MIR observation and a detailed plan for the native ownership/storage
bridge with eight pinned standard-library source files. Every member is read
back and hashed; compiler outputs are excluded. The previous insertion packet
remains unchanged. No fresh Rust tests, database QPS, recovery/Chaos Mesh acceptance or
industrial checkbox closure is claimed. The previous 213-test candidate
qualification, including 25 existing ignored tests, remains prior evidence.
All verification is local; hosted CI remains manual-only.

## Remaining implementation gate

Connect insertion, deletion and traversal graph operations to pinned native
Arc/no-Weak and compiler/library contracts, concurrent COW, Vec/Box payload
ownership, distinct Entry storage and machine-word bounds. Then run exact-source
Rust/model differential execution with snapshots and mixed mutation/range
histories before the predeclared timing/allocation comparisons. Production
promotion still requires matched database benefit, correctness, ordinary
recovery, actual Chaos Mesh and three-copy Redis throughput and latency.
