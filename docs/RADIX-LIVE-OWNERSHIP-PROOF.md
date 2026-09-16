# Radix live ownership and iterative graph reclamation

Updated: 2026-09-16 UTC. Tracking: [#51](https://github.com/c4pt0r/kv9/issues/51),
parent [#9](https://github.com/c4pt0r/kv9/issues/9).

This checkpoint adds **70 Lean theorems**, with **554 checked together**, and
**27 rejecting controls**. It extends the concrete COW graph with live ownership,
exact reference changes, last-owner token transfer, physical worklist traversal
and sequential/interleaved reclamation. The previous 47 modules and seven
contracts remain unchanged. The Rust prototype and production crates are unchanged.

Every occupied node in an owned graph has a finite interpretation, a positive
incoming reference count and a path from an external owner. Root/child COW
preserves this invariant. Releasing one token either leaves the shared node or
removes the last-owned node and transfers its child tokens to pending work.
The last-pop/child-append loop terminates, preserves retained roots, and leaves
only nodes reachable from them. With no retained roots, no occupied cells remain.
The same ownership and retained-root properties hold across arbitrary finite
interleavings of multiple workers' release steps.

**These are concrete graph proofs, not verification of native Arc or complete
Rust mutations.** Standard-library final-owner linearization is an explicit
contract; physical allocation and buffer destruction, concurrent COW composition,
complete insertion/deletion, representability and differential execution remain
open. No timing or runtime promotion follows from this checkpoint.

## Proof chain

| Module | Additional theorems | Checked result |
| --- | ---: | --- |
| `HeapReferenceDelta` | 8 | Exact additive reference conservation for allocation, root replacement, indexed edge replacement and nested COW |
| `HeapLive` | 10 | Positive ownership of every occupied node; preservation through COW, leaf allocation and adding an owned reference |
| `HeapRelease` | 12 | One-token release, final-owner child-token transfer, retained-root framing, ownership preservation and a decreasing reference potential |
| `HeapRootReach` | 8 | Positive ownership plus finite acyclic representation implies reachability from a root; no occupied cells without roots |
| `HeapDropStep` | 9 | Root-inventory permutations and actual last-pop/child-append order; each valid iteration preserves ownership and retained-root values |
| `HeapDropLoop` | 4 | Terminating iterative-transition evaluator, nonfailure, retained-root preservation and complete reclamation when no roots remain |
| `HeapConcurrentDrop` | 9 | One worker's release counts all other workers' references; arbitrary finite histories preserve ownership and obey an exact release budget |
| `HeapDropExamples` | 10 | Constructed valid shared ownership, retained snapshots, branch work order, full teardown and both orders of two-worker final-owner handoff |

Principal results include `make_root_unique_owned`, `make_child_unique_owned`,
`release_external_owned`, `owned_cell_reachable`, `drop_loop_owned`,
`drop_loop_no_held_reclaims_all`, `pool_history_owned` and `pool_history_budget`.

Reference conservation uses additive equations, so removing one reference does
not hide an underflow behind natural-number subtraction. A final release removes
the consumed token and moves the outgoing references from the node to the work
inventory. It does not recursively discard descendants or assume those children
are uniquely owned. Shared children are decided when their tokens are later popped.

The reference potential is the number of stored child edges plus pending owned
tokens. Each release decreases it by exactly one, including releases that append
multiple children. Thus the mathematical loop follows a finite sequence of the
same pop/append transitions; it is not recursive tree garbage collection.

The reachability proof rules out an isolated component with positive counts:
following an incoming edge backwards strictly increases represented subtree work,
and a finite heap supplies a bound. Its classical choice is used only to obtain
that proof bound, not by an executable transition.

## Concurrency and implementation boundary

`PoolRelease` selects one worker and preserves its concrete pending-list order.
Its count inventory includes retained external roots and every other worker's
pending references. A history of `n` completed release steps satisfies
`finalPotential + n = initialPotential`; a valid nonempty selected worker can
make a step. If all workers finish, every remaining occupied node is reachable
from a retained root. Both possible release orders for two references to the
same leaf are checked as constructed histories.

This models release linearization and child-token transfer as completed abstract
steps. It does not prove native atomic instructions, memory ordering, fairness,
thread execution time, or arbitrary simultaneous insertion/clone/release behavior.
Nor does the graph remove physical allocations at a proved native deallocation
instruction: a removed cell represents ownership leaving the Arc node, while
payload lifetime and allocator ordering still need the implementation bridge.

The unchanged candidate is
[`scripts/resident-radix/src/lib.rs`](../scripts/resident-radix/src/lib.rs),
SHA-256 `00235871d6f168b3549da377793429dd4db281549425dbab38b1a66c37103fcc`.
The correspondence targets `Node::drop`'s detached edge Vec, `pending.pop()`,
`Arc::into_inner` and `pending.append(&mut branch.edges)`. Edge labels have no
ownership effect and are projected to their target identities. `NodeId` remains
a ghost allocation identity; owned Vec/Box/Entry buffers remain values. Source
hashes and these reviewed models are not verified Rust extraction.

## Qualification and retained evidence

The [contract](../proofs/lean/radix/live-ownership-contract.json) binds all 55
modules, prior contracts/verifiers, the new verifier and candidate Rust/tests.
Pinned Lean 4.33.1 compiles fresh sources with warnings denied and implicit
theorem parameters disabled. All 554 theorem dependency inventories permit only
`propext`, `Classical.choice` and `Quot.sound`.

Twenty-three semantic controls invert reference accounting, admit zero owners,
choose the wrong final owner or cell, lose/duplicate/retain consumed references,
alter pop/append order, omit retained or other-worker references, erase held roots,
or fail to advance the selected worker. Separate controls reject a proof hole and
a custom false axiom. Two substituted Rust files check exact hash binding only.

Development checks explicitly reuse the previously accepted 47 modules' compiler
outputs, with source checks and recorded artifact hashes. The complete control
preflight uses that same declared reuse. Final qualification uses fresh copies
and recompiles dependencies independently for the positive case and every
model/proof control, stopping a rejected control at its first failure.
Authoring failures are retained, including an initially ambiguous mutation anchor
and examples that needed loop-equation rewriting instead of `decide` reduction.

The [result](radix-live-ownership-proof-v1/result.json),
[audit](radix-live-ownership-proof-v1/audit.json),
[manifest](radix-live-ownership-proof-v1/manifest.json) and
[archive](radix-live-ownership-proof-v1/evidence.tar.gz) retain exact sources,
compiler commands, dependency reports, controls and development diagnostics.
Every archived member is read back and hashed. Compiler outputs are excluded.

The previous 213-test Rust prototype qualification, including 25 existing ignored
tests, remains prior evidence; it is not relabeled as a new run. There is no new
database QPS, Redis parity, recovery/Chaos Mesh acceptance or industrial checklist
closure. All verification is local; hosted CI is not triggered.

## Next implementation gate

Compose the concrete mutable root/edge places and owned-token transitions with
all six insertion actions, split helpers, deletion's detached frames and
normalization. Reuse the accepted loop semantics and the new release proofs.
Connect native Arc/no-Weak handoff and concurrent COW to these graph transitions,
including temporary payloads and owned Vec/Box storage. Derive the earlier
machine-word length/count premises from actual storage, then differentially
execute the exact Rust candidate and executable models with a pinned compiler,
library and word width. Matched timing follows that complete gate. Promotion
still requires material database benefit, ordinary recovery, actual Chaos Mesh
and three-copy Redis throughput and latency.
