# Insertion action and mutable-slot correspondence

Six modules add 28 theorems to the prior 339. The separate
[insertion contract](insert-contract.json) checks all 367 together while
preserving the previous twenty-four modules and four contracts.

| Module | Theorems | Scope |
| --- | ---: | --- |
| `InsertSplits.lean` | 4 | Absolute cut/slice/byte accesses and actual split helper cases |
| `InsertStep.lean` | 3 | Separate six-way action selection/execution, source child provenance, semantic refinement and decreasing work |
| `InsertLoop.lean` | 7 | Nested mutable-slot context, safe replacement, termination and public root/lookup refinement |
| `InsertWordSplits.lean` | 4 | Word cut/drain arithmetic, split equivalence and routing-buffer bounds |
| `InsertWordStep.lean` | 2 | Word action selection/execution agrees with the natural-number machine |
| `InsertWordLoop.lean` | 8 | Word descent, root result, exact stored count and return flag under explicit representability premises |

The insertion frame is a ghost description of Rust's nested `&mut` slot.
It is not a claim that insertion allocates a parent Vec or reconstructs the
tree by copying every ancestor. Each descent identifies the original parent,
selected indexed child, valid extended key path and exact next depth. The
proof plugs the changed child into that value context and preserves siblings.
Arc sharing, shallow COW and physical ownership are separate remaining work.

The split models retain the source's failure cases. Leaf splitting cannot
exhaust both keys after its distinct-key guard; branch splitting accesses and
removes exactly the first non-shared prefix byte. Word execution computes
the actual cut, drain endpoint and next depth before proving their exactness.
The public count wrapper exposes an overflow failure and proves it unreachable
under the resident-result premise. That premise still needs the heap proof.

Run with a fresh output path:

```sh
python3 scripts/resident-radix/prove-insert.py \
  /mnt/data/kv9-work/radix-insert-proof-new \
  /mnt/data/kv9-work/lean-toolchain-restoration-20260915-first/installation/lean-4.33.1-linux/bin/lean
```

The runner checks thirty modules and the complete theorem axiom inventory.
Twenty-three changed algorithms, a proof hole, a custom axiom and three Rust
source substitutions must be rejected. Source substitutions check binding only;
they are not verified Rust extraction. See the
[report](../../../docs/RADIX-INSERT-LOOP-PROOF.md) for evidence and remaining work.
