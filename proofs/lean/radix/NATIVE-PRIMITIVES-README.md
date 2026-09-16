# Native object storage and concurrent COW primitives

This scoped extension adds 10 modules and 66 theorems to the unchanged
94-module, 723-theorem graph insertion/deletion baseline. The inventory is
104 modules and 789 theorems. It is a primitive-level ownership/storage bridge;
the complete implementation gate remains false.

```sh
python3 scripts/resident-radix/prove-native-primitives.py \
  /mnt/data/kv9-work/radix-native-primitives-proof-FRESH \
  /mnt/data/kv9-work/lean-toolchain-restoration-20260915-first/installation/lean-4.33.1-linux/bin/lean
```

The verifier recompiles all dependencies separately for the positive case and
every proof/model control. Fourteen semantic faults, one proof hole, one custom
false axiom and two Rust source-hash substitutions must be rejected. Source
substitutions check binding, not semantic verification of changed Rust.
The allowed Lean axioms are propext, Classical.choice and Quot.sound.

`native-library-pins.json` records Rust 1.94.0 and reviewed standard library
source hashes. `KV9_NATIVE_RUSTC` and `KV9_NATIVE_RUST_LIBRARY` can relocate those
exact inputs. The native library/compiler contract is explicit; this does not
prove rustc, machine atomics, allocator correctness or all native layouts.

The [report](../../../docs/RADIX-NATIVE-PRIMITIVES-PROOF.md) describes the
resident Entry cardinality witness, occupied Node/Box regions, reclamation,
individual child-token cloning interleaved with snapshot releases, and old
slot destruction after COW. Byte buffers, transient payload placement,
mutable edge-slot specialization, physical word loops and exact-source
Rust/model differential execution still need composition. No fresh Rust
runtime qualification, database timing or promotion is implied.
