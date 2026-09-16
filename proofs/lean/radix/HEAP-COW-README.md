# Concrete heap graph and root/child COW extension

This extension preserves the previous 37 modules and six contracts. Ten new
modules add 76 theorems (484 checked together) for explicit node identities,
stored child references, derived strong counts, root/nested COW, saved-root
isolation and preservation of all old root interpretations. Seven of the new
theorems evaluate a shared-parent example and the copies needed for safe writes.

Run locally with the pinned Lean binary and a fresh output directory:

```sh
python3 scripts/resident-radix/prove-heap-cow.py \
  /mnt/data/kv9-work/radix-heap-cow-proof-FRESH \
  /mnt/data/kv9-work/lean-toolchain-restoration-20260915-first/installation/lean-4.33.1-linux/bin/lean
```

The verifier checks exact source/compiler hashes, theorem inventory, all modules
and axiom dependencies. Eighteen changed models, one proof hole and one custom
axiom must be rejected. Two changed Rust sources test hash binding only.

The [report](../../../docs/RADIX-HEAP-COW-PROOF.md) states the scope and premises.
The graph uses ghost allocation identities and owned payload values. Full
mutation/refcount composition, native Arc contracts, live allocation accounting,
buffer storage, iterative/concurrent reclamation, representability bridges and
exact-source differential execution remain open. No full Rust verification,
candidate timing, runtime promotion or industrial checklist closure is implied.
