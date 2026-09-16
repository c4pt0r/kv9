# Shared-prefix resident-index experiment

Isolated experiment outside the production Cargo workspace. See
[the evaluation](../../docs/PACKED-PREFIX-EXPERIMENT.md) for measured results,
limitations and the integration decision.

This candidate retains the original 32/16-slot persistent B+tree, shared immutable
entries, path copying, minimum-key refresh and mutation ordering. Each node adds
a conservative common-prefix length covering **every key in its subtree**.
Insertion admits a new key by shortening the prefix before comparisons. Splits,
redistribution and merges rebuild the prefix from the subtree's actual first and
last keys. Removal alone preserves a valid conservative prefix. A query first
checks that it contains the cached prefix; otherwise comparison uses whole keys.

The eight library tests compare against `BTreeMap`, exercise multiple retained
snapshots and split/merge/root transitions, and exhaustively compare guarded
suffix ordering on a small binary-key domain. The
[18 Lean lemmas and checker](../../proofs/lean/packed-prefix/README.md) establish
prefix/order/cache-set properties under explicit ordered-node assumptions.
They do not prove the complete B+tree or mechanically refine Rust execution.

`src/main.rs` compares rpds 1.2.1, the unchanged original packed library and this
candidate on three key distributions. It separates full correspondence and
query preparation from measurement. `probes.rs` supplies fixed-seed sampling
without replacement. `key-comparisons` checks full-key and sampled rpds lookup
depth, separately from timing. The `allocation-counting` executable counts
requested bytes and allocation calls, not RSS or allocator usable sizes.

```sh
CARGO_TARGET_DIR=/path/to/reusable-target cargo test \
  --manifest-path scripts/resident-packed-prefix/Cargo.toml --locked --release --lib
```

The retained helpers qualify the exact source and executables, serialize the
shared build cache, reconstruct inputs independently before timing and record
terminal results. Input is the pinned retained `groups.bin` plus the declared
`timing-plan.json`; each output path must be fresh. Do not rerun a completed
unchanged performance screen. This is not an engine, Raft, recovery, Chaos Mesh
or database-QPS result.
