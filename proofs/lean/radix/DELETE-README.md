# Indexed edges and iterative deletion

Five modules extend the prior 287-theorem proof chain with 52 theorems.
The separate [deletion contract](delete-contract.json) checks all 339 together
without changing the earlier nineteen modules or their three contracts.

| Module | Theorems | Scope |
| --- | ---: | --- |
| `EdgeVector.lean` | 17 | Ordered vector contents, indexed access/update, removal, restoration and bounds |
| `EdgeSearch.lean` | 9 | Independent sorted-search contract, unique result and safe hit/miss indices |
| `EdgeMutation.lean` | 7 | Indexed lookup, insertion, deletion and detached-edge restoration refine recursive forest operations |
| `DeleteLoop.lean` | 9 | Actual depth-based descent, detached parent frames, checked unwind, termination and root deletion refinement |
| `DeleteWordLoop.lean` | 10 | Word-offset simulation, absent/present outcomes, lookup semantics and stored word-count/flag refinement |

The deletion loop deliberately does not repeat prefix checks: the initial
presence check establishes a condition preserved by every descent. The proof
derives safe byte access, a successful sorted search and presence in the chosen
child. Saved frames retain the original index and the parent with that edge
removed. Unwinding reinserts at the saved index before normalizing; every
reachable restoration index is within the detached vector's insertion bound.

The checked model distinguishes assertion failure (`none`) from successful
removal of the final entry (`some none`). A decreasing tree-work measure proves
termination without a fuel limit. The machine-word loop uses wrapping `USize`
operations and proves agreement with the natural-number loop for representable
query lengths. The public state wrapper preserves the absent-delete flag/root
and decrements the count exactly once after a successful deletion.

These are reviewed value-level source models, not verified Rust extraction.
Rust Vec/slice/search, matching word widths, Arc and compiler behavior remain
explicit implementation premises. Arc/COW heap ownership, immutable snapshots,
concurrent reclamation, insertion/seek loop correspondence and exact-source
differential execution remain open. The resident-entry representability premise
from the insertion/count proof is not discharged by this deletion result.

Run from the repository root with a fresh output path:

```sh
python3 scripts/resident-radix/prove-delete.py \
  /mnt/data/kv9-work/radix-delete-proof-new \
  /mnt/data/kv9-work/lean-toolchain-restoration-20260915-first/installation/lean-4.33.1-linux/bin/lean
```

The runner checks twenty-four modules and all theorem axiom dependencies, then
rejects nineteen changed algorithms, a proof hole, a custom false axiom and
three Rust source substitutions. The source substitutions test hash binding
only. See the [report](../../../docs/RADIX-DELETE-LOOP-PROOF.md).
