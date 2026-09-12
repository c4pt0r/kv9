# Retained build cache safety

The retained-build helpers serialize a complete build sequence and invalidate
first-party package artifacts before compiling. This prevents a different
worktree with older source timestamps from silently receiving the previous
worktree's executable through a shared Cargo target.

The observed failure involved different source content under two clean
worktrees, identical Cargo artifact filenames in one target, relative dep-info
paths, and timestamp-based freshness checks. Cargo reported the second
worktree's package IDs while returning the first worktree's binary bytes as
`fresh=true`. A manifest containing the current source hashes and the returned
binary hash does not establish that one produced the other.

## Build sequence

`build-workload.py`, `build-native-batch.py`, `build-benchmark.py` and
`build-rpc-experiment.py` all use `build_cache.py`:

1. Read locked Cargo metadata to resolve the actual target directory and local
   workspace members. Redis's standalone workspace uses its own manifest.
   Reject retained output inside the resolved target before invalidation.
2. Acquire an exclusive, nonblocking `.kv9-retained-build.lock` in that target.
   A competing retained build fails before cleaning or compiling.
3. Run one `cargo clean --profile release` (or `dev`) with explicit `-p`
   selectors for those workspace members. For the main workspace these are
   `kv9`, `kv9-common`, `kv9-engine`, `kv9-raft`, `kv9-region`, `kv9-meta`,
   `kv9-txn`, and `kv9-server`. Third-party dependency packages are not selected.
4. Hold the same lock across all component builds, original executable copies,
   manifest writes and final source readback. Combined helpers call the workload
   component directly, without a nested lock or another clean.
5. Require the first observed artifact for each first-party unit to have
   `fresh=false`. Later commands may reuse units compiled earlier in this same
   transaction. This includes project build scripts, libraries and executables.

Cargo's explicit package and profile selection is described in the
[Cargo clean documentation](https://doc.rust-lang.org/cargo/commands/cargo-clean.html).
The original build command vectors, default/experimental feature selections,
and existing `build.json`/`sources.json` schemas remain unchanged. Additional
`cache-safety.json`, metadata, and clean logs retain the invalidation commands,
target/lock identity, observed project freshness, failures and completion.
An output directory with a failed/incomplete cache-safety receipt is not an
accepted build, even if intermediate executable or manifest files exist.

The lock coordinates these helpers. Raw Cargo commands, other cache cleaners
and older helper versions still require external scheduling by the target's
owner. The descriptor is passed to Cargo and closed without an explicit unlock;
if the Python owner dies while Cargo remains alive, the inherited descriptor
keeps the lock held. This is not permission to launch a second build or delete
the lock file. Keep the failed build and investigate any remaining child before
continuing. Third-party cache preservation does not imply concurrent builds are
safe.

Existing releases and acceptance artifacts are never relabeled or changed by
this helper. The invalidation affects rebuildable selected-package caches,
including previously cached first-party executables in the chosen profile;
retained original executable copies, manifests, logs, histories and WALs must
stay outside the target.

## Focused regression

The following command is for the build owner to run in a new directory, outside
any performance window:

```sh
env PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 \
  taskset -c 6-15,22-31 python3 scripts/check-build-cache-safety.py \
  --output /tmp/kv9-build-cache-safety-regression-first
```

This is a small real Rust regression, with no KV9 server, network, registry
download, benchmark or production-target access. It creates two same-layout
workspaces and a tiny target beneath the supplied output directory. Both
workspaces exist before compilation; their first-party library differs and has
an explicitly old mtime. A separate external path dependency acts as an offline
dependency-cache control.

The fixture retains: A's correct build, the intentionally unsafe B build that
reproduces A's stale behavior, B's correctly invalidated build, a competing lock
attempt rejected before cleaning, an output-inside-target rejection before
cleaning, and unchanged external dependency bytes and mtimes. It fails visibly
if the unsafe control does not reproduce on the
selected Cargo/toolchain, rather than silently calling that a qualified
negative control. Every output and first failure is kept.

The normal original-release command remains:

```sh
env CARGO_TARGET_DIR=/home/dongxu/kv9/target PYTHONOPTIMIZE=0 \
  PYTHONDONTWRITEBYTECODE=1 taskset -c 6-15,22-31 \
  python3 scripts/build-native-batch.py --release --output /path/to/new-release
```

The target owner must still serialize all raw Cargo activity with this command.

## Local validation

The real offline regression passes: the unsafe B build returns A's value `1`,
while the invalidated B build returns `2`. Lock contention and retained output
inside the target both fail before cleaning, and the external dependency's
bytes and mtimes remain unchanged. The final regression is retained at
`/tmp/kv9-benchmark-cache-safety-regression-second`.

Actual native combined and standalone Redis release builds also pass. Native
performs one clean, compiles 11 initially observed project units, and reuses
9 of those units during its second component build. Redis independently
cleans and compiles its standalone package. Original manifest schemas and
source snapshots match. These integration builds precede the final
output-path guard; its allowed and rejected paths are covered by the final
regression. Integration records remain at
`/tmp/kv9-benchmark-cache-safety-integration-first`.

The inherited lock descriptor's behavior after owner death was reviewed in
source, not exercised by this regression. Raw Cargo coordination remains an
explicit limitation. No hosted CI was dispatched.
## Retained evidence in source inventories

The quorum capture publication exposed a pre-Cargo inventory failure: the two
original evidence archives are 18,936,215 and 19,410,367 bytes, while ordinary
source files are limited to 2 MiB each and 64 MiB combined. The full tree totals
about 89 MiB. The original failed owner-poll source invocation is retained at
[the original source receipt](source-inventory-v1/original-source-result.json); no compile or runtime test
had started in that invocation.

`build-workload.py` now hashes `docs/**/original-evidence.tar.gz` in 64-KiB
chunks under a separate 32-MiB-per-archive / 64-MiB-combined archive budget.
Every archive byte still contributes to the full source identity and the
before/after comparison. Ordinary source limits and the 10,000-file limit
remain unchanged. Other archive names and locations receive no allowance;
live and dangling symlinks are rejected. These are source inventory limits,
independent of runtime retention, timing and disk-reservation limits.

Run `python3 -B scripts/check-source-inventory.py` for the seven local boundary
controls. They require no Cargo, service, network access or hosted CI. Aggregate
and archive overflow controls use smaller test-only limits; production limits
remain fixed in the helper.

The [seven control outcomes](source-inventory-v1/controls-output.log) passed on
the first invocation. The [publication inventory](source-inventory-v1/inventory.json)
retains the original failure and the exact revised helper/control source hashes.
