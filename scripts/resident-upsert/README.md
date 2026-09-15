# Resident upsert experiment

This is an isolated CPU experiment, not a selected engine dependency. It compares
the existing rpds insertion with a borrowed upsert in the same executable. The
candidate follows the original red-black insertion traversal and balancing,
reusing a value only when `SharedPointer::get_mut` grants exclusive access to
its entry. Shared entries are replaced with cloned inputs. Existing snapshots
remain immutable. No unsafe code or reference-count guess is added.

The modified API retains the existing key when it compares equal. kv9's byte
vector keys then have identical bytes; this does not promise the old key-object
replacement behavior for arbitrary user-defined `Ord` implementations.

The tests compare exact tree structure, colors, size and retained snapshots
against the original insertion, in addition to checking red-black invariants.
The microbenchmark uses the server's jemalloc version and reports both mean and
p99 index time. Its cold-start replay is mostly overwrites after initialization;
the separate unique-insert case rewrites the last eight key bytes with a unique
per-mutation identifier to exercise actual insertion throughout each pass.

Prepare with Python 3.12+, `git`, `patch`, and the exact cached registry archive:

```sh
python3 scripts/resident-upsert/prepare.py \
  --output /mnt/data/kv9-work/resident-upsert-reproduction \
  --rpds-crate /path/to/cargo/registry/cache/index.crates.io-.../rpds-1.2.1.crate
cd /mnt/data/kv9-work/resident-upsert-reproduction/source
CARGO_TARGET_DIR=/path/to/reusable/target cargo test --release --offline \
  -p kv9-engine --lib mem::value_reuse
KV9_INDEX_CORPUS=../batches.bin CARGO_TARGET_DIR=/path/to/reusable/target \
  taskset -c 4 cargo test --release --offline -p kv9-engine --lib \
  mem::value_reuse::tests::retained_corpus_microbenchmark -- --ignored --nocapture
```

Run the dependency's structural tests separately from timing:

```sh
cd /mnt/data/kv9-work/resident-upsert-reproduction/rpds-1.2.1
CARGO_TARGET_DIR=/path/to/reusable/target cargo test --release --lib \
  map::red_black_tree_map -- --test-threads=1
```

The patch is against MIT-licensed rpds 1.2.1; its license is retained in
[RPDS-LICENSE.md](RPDS-LICENSE.md). Preparation verifies its registry checksum,
applies patches without fuzz, and refuses an existing output directory. It does
not change the working repository, its lockfile, or the system Cargo cache.
Builds, tests and benchmarks are explicit separate commands; preparation alone
does not qualify a candidate.

See [the results](../../docs/RESIDENT-INDEX-EXPERIMENTS.md). In particular, the
pure-insert regression currently prevents runtime integration. A structural
refinement argument, mechanized/source-bound acceptance, representative engine
and Raft tests, recovery, actual Chaos Mesh and end-to-end throughput/latency
checks remain required before a production change.
