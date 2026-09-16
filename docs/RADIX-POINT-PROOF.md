# Persistent radix point-operation proof checkpoint

Updated: 2026-09-16 UTC. Tracking: [#51](https://github.com/c4pt0r/kv9/issues/51).

Subsequent checkpoint: [cardinality and local machine-bound proofs](RADIX-CARDINALITY-PROOF.md)
add 52 theorems and 17 new controls. The original point checkpoint below is
preserved; complete Rust index verification remains open.

The isolated radix prototype now has **155 Lean-checked model theorems and
17 rejecting controls**. The model proves full-key point lookup, insertion,
replacement, deletion, the inserted/replaced flag, preservation of routing/
ordering/canonicality and arbitrary finite mutation histories. It models the
actual leaf and internal-prefix split cases, deletion collapse cases and ordered
child updates, rather than assuming that an abstract map implementation works.

**This is a partial algorithm-proof milestone, not complete Rust index
verification.** Range/predecessor cursors, entry cardinality and machine bounds,
the exact iterative-loop correspondence and Arc ownership/reclamation remain
open. The source is bound and reviewed; it is not mechanically extracted or
verified Rust. No radix performance selection or runtime promotion follows.

## What the theorems establish

The model stores full `UInt8` key/value lists in leaves and branch terminals.
A separate flattening operation produces a finite list of entries; a linear
lookup over that list is the independent point-operation specification. Routing
consumes compressed prefixes and selects unique byte-labelled edges. The proofs
connect the two interpretations for all finite trees satisfying the stated
invariants.

- `root_lookup_refines` proves routed lookup equals the independent list lookup.
- `put_root_lookup` proves insertion returns the new value at the inserted key
  and preserves every other key. `put_root_flag` distinguishes insertion from
  exact replacement according to the original root's lookup result.
- `erase_root_lookup` proves presence-guarded deletion removes exactly its key
  and preserves other point lookups, including an absent delete.
- `history_good` preserves routing validity, canonical compressed structure and
  strict edge order through every finite mutation history.
- `history_refines` proves those histories agree with ordered finite-map puts
  and erases. Saved-root retention is proved only in the pure ownership model;
  it does not establish reference-count or concurrent-heap correctness.

`common_matches_zip_count` connects common-prefix length to Rust's zip/
take-while/count computation. Separate decomposition and residual-separation
theorems cover exhausted-prefix and distinct-byte split cases. The normalizer
preserves both full entries and the path-relative routing invariant when it
joins parent prefix, edge byte and child prefix. This prevents an entries-only
proof from overlooking a malformed route after collapse.

Every theorem is checked by Lean 4.33.1 with warnings denied and implicit
parameter invention disabled. All dependency inventories contain only Lean's
ordinary `propext`, `Classical.choice` and `Quot.sound` axioms, or subsets thereof.
No `sorryAx`, custom axiom or assumed operation-correctness axiom is accepted.
This axiom audit concerns the mathematical model; Rust, compiler and library
correspondence assumptions remain explicit outside it.

## Source correspondence and remaining obligations

The [module inventory and source notes](../proofs/lean/radix/README.md) and
[source contract](../proofs/lean/radix/source-contract.json) bind the exact
candidate source `00235871d6f168b3549da377793429dd4db281549425dbab38b1a66c37103fcc`.
No Rust runtime file changed during this proof phase. The earlier
[213-test qualification](PERSISTENT-RADIX-PROTOTYPE.md), including its 25 existing
ignored tests, six compiled Rust fault controls and 54 allocation observations,
applies to that same source; it is not rerun or relabeled as new coverage here.

| Rust component | Current model evidence | Remaining correspondence |
| --- | --- | --- |
| Full-key leaves, terminal entries, compressed prefixes | `Tree`, `Valid`, independent `entries` and `linearLookup`; inductive lookup refinement | Vec/slice/UInt8 representation and safe compiler/library behavior |
| `common_prefix`, `split_leaf`, `split_branch` | Exact common-prefix count, all split cases, proper-split/unreachable-branch guards, point semantics and invariant preservation | Connect the actual mutation steps and depth offsets to the model state |
| `insert_mut` | Full recursive operational insertion; sorted child updates; root put semantics, flag and preserved invariants | Simulate the Rust mutable descent/COW path and prove stored size/arithmetic bounds |
| `remove_mut`, `normalize` | Presence guard, recursive delete/filter equivalence, exact collapse cases and ordered/canonical results | Simulate explicit detached frames and index reinsertion; prove size and loop bounds |
| Parent frames | Local put/erase lifting through arbitrarily many semantic frames | Establish the actual Rust frame stack satisfies the modeled context at every loop step |
| Snapshot clone and mutation | Pure saved roots do not change during modeled histories | Arc/COW heap relation, shallow child sharing, private buffers and concurrent teardown |
| `Cursor::seek_slice`, `next_entry`, `Range`, predecessor | Existing independent Rust model tests only | Formal byte-order, pruning, traversal, bounds and mixed-cursor crossover refinement |

The recursive `Forest` search is a functional abstraction of the sorted edge
vector. Its ordering invariants are proved; Rust binary search, vector insertion/
removal, slicing and allocation behavior remain library premises. Tree/Forest
recursion terminates structurally in Lean. This is not yet a proof of bounded
Rust call-stack use, machine-integer safety, allocator failure handling or the
whole database's Raft/lock protocol.

The next work stays on the **complete algorithm gate**: cardinality/usize
obligations, actual cursor semantics, iterative-loop simulation and the ownership
argument. Differential execution of the executable Lean model against the exact
Rust prototype will test the reviewed mapping; that test will supplement, not
replace, formal refinement.

## Rejected controls and evidence

Twelve changed algorithms/models fail proof checking: missing leaf or terminal
values, wrong edge routing, an omitted collapse byte, retained deleted leaf/
terminal, stale replacement values, a wrong inserted flag, a repeated consumed
split byte, an omitted common-prefix byte and reversed mutation history. The
semantic controls produce substantive proof failures, not only lint failures.
A `sorry` proof is rejected with warnings denied; an injected false axiom is
rejected by the dependency inventory. Three substituted Rust sources fail the
reviewed hash binding. These hash checks are not semantic Rust proofs.

The [manifest](radix-point-proof-v1/manifest.json),
[audit](radix-point-proof-v1/audit.json),
[result](radix-point-proof-v1/result.json) and
[evidence archive](radix-point-proof-v1/evidence.tar.gz) retain the accepted proof,
all controls, source identities and earlier authoring diagnostics. All checks
are local. No GitHub workflow, database benchmark, MinIO/Chaos acceptance or
Redis comparison was run in this phase.

After the remaining proof obligations close, predeclare the original/long/
injective-varied-prefix matched performance screen, including both orders,
throughput, mean/p99, read/range regressions and separate allocation measurements.
Production continues to use rpds; rejected earlier index families remain stopped.
