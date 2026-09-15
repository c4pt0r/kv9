# Isolated rpds candidate tests

The release library filter passed **53 tests**: the new
`borrowed_upsert_preserves_exact_tree_and_retained_snapshots` differential test
and 52 existing red-black-tree-map tests, each once. There were no failed or
ignored tests, zero benchmark measurements and 206 tests outside the filter.
The actual tool terminal was **56340 / 40896e / 0**; tests took 1.76 seconds
after compilation. This is local library evidence only. The candidate remains
unselected and its measured insertion regressions are not resolved by passing
these tests. No production, proof, runtime or Chaos acceptance is claimed.

Only `../rpds/Cargo.toml` changed: append an empty `[workspace]` section to
prevent Cargo walking into the archival campaign parent manifest, whose
inherited `workspace.package.edition` cannot resolve. Original and final
manifest bytes and the exact diff are retained here. All rpds Rust sources,
the fixture Cargo.lock, repository Cargo.lock and fused/source Cargo.lock
remain unchanged; before/after checks bind 28 protected files.

The parent's original manifest failure remains at `original/parent-rpds-tests.log`.
After fixing workspace discovery, `offline-first` refused before compilation
because `alloca 0.4.0` was not cached (**31ed80 / 1**, Cargo exit 101).
`dependency-fetch-first` fetched the exact existing locked graph for
`x86_64-unknown-linux-gnu` (**380bd0 / 0**). No dependency version or lockfile
was changed. Its complete download log is retained. Then `tests-second` ran:

```text
cargo test --offline --locked --release -j4 --lib map::red_black_tree_map -- --test-threads=1
```

Cargo/rustc were 1.94.0. The command used
`CARGO_TARGET_DIR=/home/dongxu/kv9/target`; the existing retained-build lock was
held exclusively and its descriptor inherited by Cargo. All three Cargo
lifetimes were reaped and are absent. The wrapper retained unique per-attempt
logs, invocation/child/terminal records and source checks. It enforced a
1,200-second command deadline, 16 MiB log bound, 8 GiB available-space floor
and 16 GiB maximum decrease on both root and data filesystems. No cleanup,
benchmark, profile, container or repository mutation was performed.
