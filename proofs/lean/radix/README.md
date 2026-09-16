# Radix point-operation model proofs

The subsequent [cardinality and local machine-bound extension](SIZE-README.md)
adds two modules and 52 theorems under a separate contract. The ten-module
point-operation checkpoint and its original contract remain preserved below.

These ten Lean modules contain 155 kernel-checked theorems for the isolated
[Rust prototype](../../../scripts/resident-radix/src/lib.rs). The reviewed
[source contract](source-contract.json) binds the exact Rust source, tests,
proofs and verifier. The pinned Lean 4.33.1 run rejects 17 controls. Only
`propext`, `Classical.choice` and `Quot.sound` occur in the accepted dependency
inventory. `autoImplicit` is disabled and compiler warnings are errors.

**The complete algorithm gate remains open.** This is a reviewed source mapping
to an explicit operational model, not verified Rust extraction. Cursor/range
refinement, cardinality and machine bounds, exact iterative-loop simulation,
Arc/heap reasoning and differential execution against Rust remain to be closed.
Pure snapshot retention in `History.lean` assumes the ownership abstraction;
it does not prove Rust concurrency or reference counting.

| Module | Established model properties |
| --- | --- |
| `Branches.lean` | Prefix stripping, local split/collapse semantics and parent-frame lifting of put/erase |
| `Tree.lean` | Full-key leaf/terminal representation; route validity and unique child labels; routed lookup equals independent finite-list lookup |
| `Normalize.lean` | Exact normalization cases preserve entries and routing; canonical children produce a canonical result |
| `Delete.lean` | Presence-guarded deletion refines key filtering; validity, canonicality and point lookup preserved |
| `Common.lean` | Common prefix equals zip/take-while/count; decomposition, bounds and distinct residual byte heads |
| `SplitLeaf.lean` | All leaf-split cases preserve validity/canonicality and implement the expected put; impossible double exhaustion is excluded under the source guard |
| `SplitBranch.lean` | Internal-prefix split preserves validity/canonicality and adds the absent new key; proper-split guard excludes the unreachable empty old suffix |
| `Ordered.lean` | Strict unsigned-byte edge order; split, normalization and deletion preserve it |
| `Insert.lean` | Full recursive insertion preserves validity, order and canonicality, implements put, and returns the correct inserted/replaced flag |
| `History.lean` | Good roots remain good; arbitrary finite put/erase histories refine their finite-map specification; pure saved roots remain unchanged |

The tree stores **full keys** in leaves and terminals, matching the Rust fields.
`entries` flattens those keys independently of routing. `linearLookup` searches
that list without using compressed prefixes or edge labels. Root lookup,
put and erase are proved against this independent interpretation. Inductive
definitions and proofs cover arbitrary finite trees and histories, not a bounded
list of examples. There is no axiom assuming that insertion or deletion is correct.

The model's `Forest` represents the ordered edge vector. It locates an edge by
structural ordered search; Rust uses `Vec::binary_search_by_key`. Agreement
requires the proved ordering invariant and the standard-library search/insert/
remove contracts. Rust `common_prefix` is modeled by `common_matches_zip_count`;
`prefix_take`/`prefix_drop` connect the shared prefix and retained suffix to slicing.
Allocation failure and panic recovery are outside these model theorems.

The top-level theorems are `root_lookup_refines`, `put_root_lookup`,
`erase_root_lookup`, `put_root_flag`, `history_good` and `history_refines`.
The runtime map's `size` field, exact Rust path loops and physical snapshot
ownership still need their own correspondence arguments; these are not inferred
from the point-operation theorems.

Run from the repository root with a fresh output path:

```sh
python3 scripts/resident-radix/prove.py \
  /mnt/data/kv9-work/radix-point-proof-new \
  /mnt/data/kv9-work/lean-toolchain-restoration-20260915-first/installation/lean-4.33.1-linux/bin/lean
```

The runner compiles every module in a fresh directory, checks every theorem's
axiom dependencies and rejects twelve algorithm/model mutations, a proof hole,
a custom axiom and three Rust source substitutions. Hash rejection binds the
reviewed source; it does not turn the manual Rust/model mapping into a theorem.
See the [report](../../../docs/RADIX-POINT-PROOF.md) for evidence and remaining gates.
