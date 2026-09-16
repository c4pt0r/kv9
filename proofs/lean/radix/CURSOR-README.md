# Ordered cursor and range extension

The seven modules below add 80 theorems to the 207 previously checked
point/cardinality theorems. The separate [cursor contract](cursor-contract.json)
checks all 287 together. The prior twelve modules and two contracts are
unchanged.

| Module | New theorems | Scope |
| --- | ---: | --- |
| `KeyOrder.lean` | 14 | Full-key strict byte order and bound acceptance |
| `CursorTraversal.lean` | 14 | Pending node/entry stack expansion, termination and exact sequence refinement |
| `SeekBounds.lean` | 5 | Whole-subtree mismatch pruning and terminal/sibling ordering |
| `CursorSeek.lean` | 5 | Complete forward/reverse inclusive/exclusive seek and first-result refinement |
| `RangeStep.lean` | 9 | Loaded endpoints, crossover, chosen-side advance and arbitrary mixed histories |
| `RangeInit.lean` | 19 | Exact two-bound intersection, empty crossover and independent source guard equivalence |
| `RangeProperties.lean` | 14 | Conservation, no duplicate keys, exhaustive/fused behavior and inclusive predecessor |

The cursor's top-first task list is the reverse of Rust's pending Vec.
Its well-founded traversal models the actual repeated pop/branch-expand loop;
seek uses an ordered-forest abstraction for the sorted vector partition.
Range transitions preserve both independent cursors and their loaded endpoints.
Their result is proved equal to a separate list-end specification for every
finite mixed-direction schedule.

These are reviewed source-bound models. Actual Rust depth/index/frame/vector
simulation, Arc/COW heap correspondence, word-counter addressability and
source differential execution remain open. No verified extraction or complete
runtime verification is claimed.

Run from the repository root with a fresh output path:

```sh
python3 scripts/resident-radix/prove-cursor.py \
  /mnt/data/kv9-work/radix-cursor-proof-new \
  /mnt/data/kv9-work/lean-toolchain-restoration-20260915-first/installation/lean-4.33.1-linux/bin/lean
```

The runner checks nineteen modules and all theorem axiom dependencies, then
rejects nineteen changed algorithm/guard models, a proof hole, a custom axiom
and four source substitutions. Source substitutions exercise hash binding only.
See the [report](../../../docs/RADIX-CURSOR-PROOF.md) for retained evidence and
the remaining full-index gate.
