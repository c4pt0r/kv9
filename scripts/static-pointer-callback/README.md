# Static pointer callback qualification

This isolated archery 1.2.3 patch keeps the existing `WriteBack` RAII guard,
`ManuallyDrop` and smart-pointer operations. It retains the `as_ptr` function
item as a generic `Fn + Copy` instead of coercing it to a function pointer.
The six reviewed callers are `get_mut` and `make_mut` for RcK, ArcK and ArcTK.
Production manifests and the Cargo registry are unchanged.

The [qualification report](../../docs/STATIC-POINTER-CALLBACK.md) records tests,
conditional proof boundaries and ordinary release code generation. There is
no accepted performance result or runtime promotion for this patch yet.

Prepare a fresh workspace from checksum-verified cached archives:

```sh
python3 scripts/static-pointer-callback/prepare.py \
  --cache /home/dongxu/.cargo/registry/cache/index.crates.io-1949cf8c6b5b557f \
  --output /mnt/data/kv9-work/static-pointer-callback-reproduction
taskset -c 6-15,22-31 python3 scripts/static-pointer-callback/test.py \
  --work /mnt/data/kv9-work/static-pointer-callback-reproduction
python3 scripts/check-static-pointer-callback-proof.py \
  --lean /path/to/lean-4.33.1/bin/lean \
  --baseline /mnt/data/kv9-work/static-pointer-callback-reproduction/baseline-archery \
  --candidate /mnt/data/kv9-work/static-pointer-callback-reproduction/candidate-archery \
  --registry /home/dongxu/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f \
  --output /mnt/data/kv9-work/static-pointer-callback-reproduction/proof
taskset -c 6-15,22-31 python3 scripts/static-pointer-callback/codegen.py \
  --work /mnt/data/kv9-work/static-pointer-callback-reproduction
```

Use CPUs available on the reproduction host. Build helpers serialize and
invalidate only the relevant package artifacts in the repository's shared
target directory. They refuse inherited Rust/C compiler flags. Offline
resolution requires cached development dependencies as well as runtime ones.
Codegen builds use the same existing `resident-selected-profile` harness and
unmodified engine/common sources, with only the isolated archery override
differing. The retained original helper scripts and execution logs are in the
report's evidence packet; the repository entry points parameterize work paths.

`callback_tests.rs` adds nine cases, three per pointer kind: replacement on
normal return, replacement followed by panic, and panic while cloning a shared
value. Pointer identity is checked before reconstructing the raw owner; old
views, strong counts and exactly-once destruction are checked afterward.

The inherited archery panic-safety comment overstates that every panic retains
the initial allocation. A callback may replace its owner before panicking.
The guard must publish its **current** owner. The tests and model require that
behavior; the experiment preserves the original comment for an exact patch
comparison. Correct the comment if this patch advances into a maintained fork.
