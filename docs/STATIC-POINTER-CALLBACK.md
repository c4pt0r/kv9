# Static pointer callback qualification

Date: 2026-09-16 UTC. Base: `bd0f6e6f7f4101056b83de5c910b4de7b79bc6ad`.

The isolated callback patch passes dependency tests, conditional guard
equivalence checks and ordinary release semantic smokes. Release assembly
confirms removal of the targeted out-of-line pointer callback. It is ready for
the predeclared component performance screen. That [subsequent screen](STATIC-POINTER-CALLBACK-PERFORMANCE.md)
is now complete: writes improve, but material-gain and read-tail gates fail.
The patch remains isolated. No new database QPS or Redis comparison is claimed.

## Change and safety boundary

The [previous CPU diagnosis](RESIDENT-SELECTED-CPU-PROFILE.md) identified a short
`triomphe::Arc::as_ptr` function in both ordinary and diagnostic builds. The
existing `ErasedPtr::map_owned` coerces its callback to `fn(&P) -> *const T` and
stores that pointer in its write-back guard. The
[isolated patch](../scripts/static-pointer-callback/archery.patch) instead keeps
an `impl Fn(&P) -> *const T + Copy` and stores its adapter as a generic field.

All six callers pass function items: `get_mut` / `make_mut` for RcK, ArcK and
ArcTK. Reconstruction, `ManuallyDrop`, the invocation of `f`, guard lifetime,
normal/unwind write-back and reference-count operations are unchanged. No new
unsafe block, tree algorithm, Raft behavior or production dependency override
is introduced. The source review also binds pointer-kind implementations,
SharedPointer's Send/Sync boundary, triomphe and the rpds red-black tree.

The inherited panic comment incorrectly generalizes that the original pointer
always survives a panic. A callback can replace the allocation first. The
required behavior is restoration of the **current** pointer on both exits.
The tests and model cover this distinction; the original comment remains in
the isolated source so the executable patch comparison stays minimal.

## Local qualification

| Check | Result |
| --- | --- |
| Baseline archery, all features | 58/58 library tests |
| Candidate archery, all features | 58/58 library tests |
| rpds with candidate archery, all features | 276/276 library tests |
| Lean 4.33.1 guard model | 18 checked theorems; no sorry/custom axioms |
| Rejecting controls | 8/8 rejected |
| Ordinary release semantic smokes | 8/8 cases; 848 prefix states and 424 old views |

Each archery suite includes 49 upstream tests and nine added tests, covering
normal replacement, replacement followed by panic, and clone panic for all
three pointer kinds. They verify pointer identity before reconstruction,
external views, strong counts and exactly-once destruction. Upstream tests
also retain Send/Sync negative assertions and existing clone-panic coverage.
These are dependency tests, not a new full-workspace/Chaos acceptance run.

The [Lean model](../proofs/lean/static-pointer-callback/WriteBack.lean) proves
pointwise callback equivalence through the guard and through one abstract
invocation of `f`, preservation of exit/heap/views, transfer of one owned
reference to the published slot, and slot validity given a valid final owner.
It admits pointer replacement before unwind and supplies witnesses rejecting
stale write-back and an extra decrement. Controls alter the model's slot,
decrement and unwind behavior, insert a proof hole/custom axiom, and violate
three exact source bindings.

This is a **conditional abstract proof**, not verified extraction or a complete
Rust memory-safety proof. Rust/compiler correctness, pointer provenance,
matching smart-pointer contracts, ordinary unwinding, a non-panicking read-only
`as_ptr`, and a lawful callback remain explicit premises in the
[source contract](../proofs/lean/static-pointer-callback/source-contract.json).
Source hashes bind review inputs; they do not prove the mapping to Rust.

Release smokes run the unchanged `MemEngine::write_applied` harness with either
isolated dependency. They use the retained 106 groups / 100,096 mutations for
overwrite and unique insertion, with/without pinned snapshots. Both variants
agree on all prefix states, retained old views, final contents, applied index
and data revision. Eight one-pass executions apply 800,768 mutations after
their prefix validation. Their timestamps are retained only as smoke evidence,
not used as a performance comparison.

## Ordinary release code generation

Both builds use the same harness, engine/common sources, registry versions,
ThinLTO, one codegen unit and jemalloc. Neither uses forced frame pointers or
other observation flags. Clean/build/copy operations hold the shared cache
lock and verify fresh engine, common, archery and rpds artifacts.

| Evidence | Baseline | Candidate |
| --- | --- | --- |
| `Arc::as_ptr` standalone symbol | 8 bytes | absent |
| Instructions referencing that symbol | 21 | 0 |
| ELF SHA-256 | `2b3a781923a05ff263b90c93889710415dd26b40439fff58fcbf802b749446dd` | `59186473a21211e7568476ec161e0b5039cd02fbfb94a620c5f78d17a4347760` |

In `Node::insert::ins`, the baseline saves the callback address and calls
through the stack-held pointer after `make_mut`. The candidate instead loads
the current owner, adds the pointer offset and writes the result directly to
the slot. `make_mut` remains present in both binaries, preserving its shared
owner cloning path. A shorter assembly path establishes the intended compiler
effect; it does not establish a throughput/latency improvement.

## Next gate

Run a fresh baseline/candidate component comparison in both orders. Retain the
original/short/dispersed key distributions, overwrite/initial-fill/unique
insertion, pinned/unpinned snapshots and seeded representative read probes.
Separate allocation counting from timing. Preserve the declared gate: at
least 10% pooled mean improvement for original overwrite and unique insertion
in both snapshot modes, every write case faster in both orders, and no read
mean or p99 regression above 2%.

If that gate passes, integrate an explicitly bounded candidate and run full
local correctness, ordinary recovery, actual Chaos Mesh faults and a matched
database/three-copy Redis comparison before promotion. Keep CRC/rpds selected
and previously losing tree/clone-removal variants held. Current database
results remain 137,873.776 Put/s and 1,022,750.059 BatchPut(64) items/s on the
documented volatile c64 fixture.

## Evidence

The [retained packet](static-pointer-callback-v1/README.md) includes source
snapshots, preparation/readback records, dependency logs, proof controls,
ordinary release assembly and smoke outputs. The original local execution root
is `/mnt/data/kv9-work/static-pointer-callback-20260916-first`.
Use the [reproduction instructions](../scripts/static-pointer-callback/README.md)
for a fresh isolated qualification. No hosted CI was requested.
