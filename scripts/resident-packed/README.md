# Packed resident-index experiment

Standalone experiment, outside the production Cargo workspace. **The first
measured variant is rejected for overwrite and point-read regressions.**
See [results and limitations](../../docs/PACKED-INDEX-EXPERIMENT.md).

`src/lib.rs` implements a safe persistent B+tree with 32-slot nodes, 16-slot
non-root minima, shared immutable entries and path copying. It supports insertion,
replacement, deletion with redistribution/merge, root contraction, bytewise
lookup, predecessor and forward ranges. Root clones share structure; values in
old snapshots remain immutable. Present deletions perform a presence lookup and
a second modifying descent; absent deletions retain the original root.

`src/tests.rs` compares against `BTreeMap`, including structural invariants,
multiple snapshots, all small insertion/deletion permutations, ordered
split/merge sequences and shuffled deletion from a four-level tree. These tests
are evidence, not a formal proof of the complete implementation.

`src/main.rs` compares unchanged rpds 1.2.1 on the pinned retained group corpus.
`src/counting.rs` is compiled only with `allocation-counting`; it records requested
allocation sizes and live requested bytes, not RSS or jemalloc usable sizes.
The ordinary executable uses uninstrumented jemalloc. `key-comparisons` is a
separate count-only diagnostic for the original fixed-stride read probe bias.

Tests can be run with an explicitly selected reusable Cargo target:

```sh
CARGO_TARGET_DIR=/path/to/reusable-target cargo test \
  --manifest-path scripts/resident-packed/Cargo.toml --locked --release --lib
```

The retained qualification/build/run helpers serialize the shared target,
preserve source and executable identities, and record actual terminals. Their
exact commands, plans and originals are in the
[evidence packet](../../docs/packed-index-experiment-v1/README.md). Benchmark
execution needs the large local `groups.bin` input plus its declared plan and a
fresh output path. Do not rerun an unchanged rejected matrix.

No production selection, engine integration, formal correctness acceptance,
recovery/Chaos gate or database-QPS result follows from this experiment.
