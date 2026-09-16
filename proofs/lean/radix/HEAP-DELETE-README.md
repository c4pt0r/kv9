# Concrete graph deletion extension

This extension adds 13 modules and 44 theorems to the unchanged 81-module,
679-theorem insertion baseline. The complete inventory has 94 modules and
723 theorems. Fresh local qualification passed, including all 28 rejecting
controls, with independent fresh dependencies for every model/proof case.

```sh
python3 scripts/resident-radix/prove-heap-delete.py \
  /mnt/data/kv9-work/radix-heap-delete-proof-FRESH \
  /mnt/data/kv9-work/lean-toolchain-restoration-20260915-first/installation/lean-4.33.1-linux/bin/lean
```

The verifier binds exact source/compiler hashes, recompiles fresh dependencies,
and inventories every theorem's axioms. It tests 24 semantic faults, a proof
hole, a custom false axiom and two Rust source-hash substitutions. Source-hash
substitutions do not constitute semantic verification of changed Rust.

The [report](../../../docs/RADIX-HEAP-DELETE-PROOF.md) explains concrete detached
ownership, normalization, physical parent frames, borrowed presence lookup,
the terminating deletion loop, absent no-op, removed flag, and mathematical
cardinality. The leaf arm retains its current Arc through frame unwind and
releases it at function exit, matching the observed pinned-compiler MIR.

These are graph proofs under explicit native library/compiler contracts.
Native payload/concurrent-COW/storage and word bridges, and exact-source
differential execution remain open. The complete implementation gate stays
false. No new timing or runtime promotion is claimed.
