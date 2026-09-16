# Persistent compressed radix prototype

This isolated, dependency-free Rust crate implements the candidate tracked by
[#51](https://github.com/c4pt0r/kv9/issues/51). It is outside the production
workspace. The selected production index remains rpds.

`RadixMap` supports arbitrary binary keys, point lookup, exact replacement,
deletion, predecessor, double-ended bounded iteration and O(1) root snapshots.
Node mutation and destruction are iterative. Key/value Vec ownership is kept
separate. The implementation forbids unsafe Rust; only the external allocation
probe's existing allocator adapter uses unsafe forwarding.

The [qualification report](../../docs/PERSISTENT-RADIX-PROTOTYPE.md) records
the source, tests, six rejecting Rust mutants and requested-memory observations.
These are **not a complete formal algorithm proof or a performance result**.
The next gate is source-bound finite-map refinement of the actual algorithms.

Run from the repository root, with a fresh output directory for each command:

```sh
python3 scripts/resident-radix/qualify.py /mnt/data/kv9-work/radix-model-new
python3 scripts/resident-radix/controls.py /mnt/data/kv9-work/radix-controls-new
python3 scripts/resident-radix/engine.py /mnt/data/kv9-work/radix-engine-new
python3 scripts/resident-radix/layout.py /mnt/data/kv9-work/radix-layout-new
```

All use local tools. Cargo builds reuse the repository target and retained-build
lock; run the three Cargo helpers sequentially. The controls compile independent
executables and disable core dumps for the deliberate stack-overflow mutant.
The layout helper measures requests through System, not RSS, usable allocation
size or speed. The engine helper changes only the map import/type plus the
isolated module declaration and added tests; production crate files are untouched.
