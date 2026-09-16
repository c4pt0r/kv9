# Concrete graph insertion extension

The mutable-place insertion chain adds 26 modules and 125 theorems, extending
the unchanged 55-module, 554-theorem ownership/reclamation baseline. The full
inventory has 81 modules and 679 theorems. Fresh local qualification passed,
including all 30 rejecting controls.

```sh
python3 scripts/resident-radix/prove-heap-insert.py \
  /mnt/data/kv9-work/radix-heap-insert-proof-FRESH \
  /mnt/data/kv9-work/lean-toolchain-restoration-20260915-first/installation/lean-4.33.1-linux/bin/lean
```

The verifier checks exact source/compiler hashes, recompiles fresh dependencies,
audits every theorem's axioms, and checks 26 semantic mutations, a proof hole,
a custom axiom and two Rust source-hash substitutions. The last two controls
test source binding, not a semantic proof of mutated Rust.

The [report](../../../docs/RADIX-HEAP-INSERT-PROOF.md) covers borrowed root/edge
places, COW isolation, ownership transfer, six insertion actions, split move/drain
paths, a terminating physical-place loop, root initialization, lookup behavior,
returned flags and mathematical cardinality. The executable loop carries no
ghost Tree, parent-frame vector or fuel.

This is a reviewed graph model under explicit library/compiler contracts.
Concrete deletion/normalization, native payload/concurrent-COW/storage and word
bridges, and exact-source differential execution remain open. The full index
implementation gate stays false; there is no new timing or runtime promotion.
