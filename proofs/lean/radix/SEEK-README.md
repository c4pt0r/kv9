# Indexed seek and physical cursor extension

This extension preserves all thirty accepted modules and five earlier contracts.
Seven additional modules prove actual seek/depth/index and physical pending Vec
correspondence, native-word arithmetic, and composition with double-ended range
histories. It adds 41 theorems (408 checked together) and 35 rejecting controls.

Run locally with the pinned Lean binary:

```sh
python3 scripts/resident-radix/prove-seek.py \
  /mnt/data/kv9-work/radix-seek-proof-FRESH \
  /mnt/data/kv9-work/lean-toolchain-restoration-20260915-first/installation/lean-4.33.1-linux/bin/lean
```

Use a fresh output directory. The verifier checks the exact source/compiler
hashes, theorem inventory, all modules and axiom dependencies, then rejects
thirty changed algorithms, a proof hole, a custom axiom and three changed Rust
sources. The last category tests source binding only.

The [report](../../../docs/RADIX-SEEK-LOOP-PROOF.md) describes the correspondence
and its remaining premises. Arc/COW heap ownership, saved roots, reclamation,
storage representability and exact-source differential execution remain open.
No timing or full Rust verification is implied by this extension.
