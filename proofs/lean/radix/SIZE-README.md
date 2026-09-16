# Cardinality and local machine-bound extension

`Cardinality.lean` adds 23 theorems to the existing point-operation model.
`MachineBounds.lean` adds 29 more. All 207 theorems, including the 155 prior
theorems, are checked together by the separate
[size contract](size-contract.json). The original point contract and verifier
remain unchanged.

The first module derives full-key uniqueness from routing validity, establishes
put/erase cardinalities and proves that a separate stored Nat counter remains
exact through arbitrary finite histories. The second checks local prefix,
slice, descent, edge-index and 256-label bounds, then refines stored USize
counts under an explicit resident-result representability premise.

`word_history_size` does not establish `ResidentHistory` by itself. The Rust
heap simulation must establish distinct addressable storage for every live
entry. Similarly, local arithmetic lemmas do not prove that the actual Rust
loops preserve their path and vector premises. Range/cursor semantics, explicit
loop simulation, Arc/COW ownership and differential execution remain open.

Run with a fresh output path:

```sh
python3 scripts/resident-radix/prove-size.py \
  /mnt/data/kv9-work/radix-size-proof-new \
  /mnt/data/kv9-work/lean-toolchain-restoration-20260915-first/installation/lean-4.33.1-linux/bin/lean
```

The runner rechecks all twelve modules and every theorem dependency in a fresh
directory. Its seventeen controls consist of eleven changed counter models or
false local bounds, a proof hole, a custom axiom and four substituted Rust
sources. Source substitutions exercise hash binding only.

The [report](../../../docs/RADIX-CARDINALITY-PROOF.md) records the correspondence
limits, retained diagnostics and remaining complete-index obligations.
