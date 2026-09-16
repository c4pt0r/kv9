# Selected resident-index CPU attribution

Updated 2026-09-16. The observation method is now qualified: four offline cases
retain **13,601 / 13,605 named apply boundaries (99.971%)**, with no reported
sample loss. Traversal/shared-pointer leaves dominate these samples; named
byte-copy leaves account for only 0.993%–1.838%. The next changed hypothesis is
monomorphic pointer write-back in the existing dependency, preserving its
panic-safe guard. **No runtime optimization or database-QPS gain is claimed.**

## Scope and observation qualification

The [harness](../scripts/resident-selected-profile/README.md) calls the unchanged
current `MemEngine::write_applied`, using source revision `36a9a58`. This is a
new offline component executable, not the selected server ELF. It uses the
pinned 106-group / 100,096-mutation corpus and jemalloc. There is no network,
Raft or WAL. Sources and lock resolution bind to the actual build; archery 1.2.3,
rpds 1.2.1 and triomphe 0.1.16 extracted files additionally match every member of
their checksum-verified registry archives.

The first default-release DWARF recording retained **2,145 samples**, including
**1,755 inside apply spans**, but only **54 (3.077%)** with a named apply boundary;
1,190 selected samples had only one frame. It fails the predeclared 95% caller
coverage gate. The exact ELF already has unwind-table entries for insertion,
`write_applied`, the harness boundary and malloc. We do not claim a missing-table
root cause or repair the missing callers by assigning them to a favored theory.

That first supervisor also required exit zero after deliberately stopping perf
with SIGINT; perf wrote its artifact and exited by signal 2. The original failed
terminal is retained. Its artifact was decoded once and independently checked,
then rejected for coverage. It was not rerun or relabeled as a qualified capture.
Later supervision accepts either normal exit or the specifically recorded,
owned-PID SIGINT after successful harness completion, followed by artifact,
loss, sample-count and independent state/span checks.

The changed observation method uses **Rust `-Cforce-frame-pointers=yes` and C
`-fno-omit-frame-pointer`**, with perf frame-pointer call chains. Core engine and
workload source are identical. Both builds pass local Clippy; first-party source
and executable identities are retained. The four relevant function prologues
are inspected. This changes code generation and potentially timing: these CPU
sample fractions are not production CPU shares or an observer-overhead result.
The default and frame-pointer executables have separate provenance.

Each case has 300 passes, CPU 4 on a shared host, `cpu-clock:u` at 499 Hz,
monotonic timestamps, a 128 MiB raw-file cap and a 90-second harness timeout.
Helpers use CPUs 6–15 and 22–31. Only the owned harness PID is sampled. The first
frame-pointer case must qualify before the remaining three launch. Samples are
selected by exact half-open apply spans; input preparation, snapshot handling,
engine initialization and final scans stay outside. There are **127,200 spans /
120,115,200 mutations** across the four cases. Before capture, **424 batch states
and 212 old views** are checked. Every pass verifies final contents, applied
position and revision. Independent parsing reconstructs the original corpus,
final-state digests, span ordering, PID scope, decoded sample totals and coverage.

## Results

Percentages are exclusive **leaf sample counts**, not measured latency fractions.
Allocation includes freeing and allocator maintenance. Inclusive stack views
would overlap and must not be added to this table.

| Case | Selected samples | Named apply boundary | Tree / ownership | Comparison | Allocation / free | Copy |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Overwrite | 1,795 | 99.944% | 59.944% | 20.000% | 16.267% | 1.560% |
| Overwrite + snapshot | 2,067 | 99.952% | 57.716% | 17.997% | 18.239% | 1.838% |
| Unique insert | 4,102 | 100.000% | 58.289% | 24.915% | 12.652% | 1.536% |
| Unique insert + snapshot | 5,641 | 99.965% | 56.462% | 22.372% | 17.515% | 0.993% |

There are 21,295 total recorded samples, including setup/finalization outside
apply. Three selected leaves remain unknown and four samples lack the named
boundary; those limitations remain explicit. The category remainder is engine
or other code. All original leaf symbols, offsets, call chains and selected
sample ordinals are retained; no unknown frame is assigned a specific caller.

The `ArcTK::make_mut` closure is not itself a clone counter. Exact disassembly
separates its common/unique path from its clone-body instruction range. Neither
unpinned case has a selected leaf in that clone-body range; pinned overwrite and
insert have 149 and 1,098 respectively. These are instruction samples, subject
to skid and common-path attribution, not numbers of cloned nodes or copied bytes.
This resolves the earlier ambiguity enough to avoid calling every `make_mut`
sample a copy.

## Concrete next candidate

In archery's `ErasedPtr::map_owned`, the pointer accessor is accepted as a
function pointer and stored in a `WriteBack` guard. The guard restores the
possibly replaced pointer on normal return **and unwind**, while `ManuallyDrop`
preserves the owned strong reference. That restoration must remain intact.

Both default and frame-pointer component binaries retain a separate
`triomphe::Arc::as_ptr` helper. Its default body is eight bytes; the diagnostic
body adds a frame-pointer prologue/epilogue. In unpinned overwrite, **103 of
1,795 selected leaves (5.738%)** land there. The source's function-pointer
conversion and retained call justify testing static callback specialization;
they do not establish how much time a changed implementation would save.

An **isolated, unqualified patch** keeps the accessor generic through the guard,
using the same function-item callback and an erased-pointer adapter. It retains
`from_raw`, the owned smart pointer, `ManuallyDrop`, write-back ordering and the
original reference-count operations; it introduces no new unsafe block. The
patch and prospective plan are in the packet. It has **not yet passed tests,
a proof, code-generation inspection or timing**, and is not a runtime selection.

Before timing, review every instantiated callback and establish normal/unwind
slot and ownership equivalence under the existing smart-pointer contract. Run
the upstream dependency tests and targeted panic-after-replacement/clone-panic
controls, retain Send/Sync and snapshot behavior, and inspect a normal release
build for actual removal of the callback call. Only then run a fresh matched
comparison covering insert, overwrite, initial fill, snapshots, varied prefixes
and read mean/p99. Preserve the previous borrowed-upsert and packed-index
regressions; do not repeat or pool those matrices.

A successful component change still needs source-bound correctness, ordinary
recovery, actual Chaos Mesh and matched database throughput/latency before
promotion. No industrial work package closes here. The latest database result
remains **137,873.776 Put/s / 1,022,750.059 BatchPut(64) items/s** at c64 on the
previous volatile fixture; Redis parity remains unachieved.

## Evidence and execution

[Portable metadata, decoded profiles and original failures](resident-selected-profile-v1/README.md)
include the four small frame-pointer raw recordings. The larger failed DWARF
recording, corpus and ELF files remain local. All new bulk outputs use
`/mnt/data/kv9-work/resident-selected-profile-20260916-first`.

Two initial builds encountered root-owned Cargo fingerprint directories. A
scoped repair changes ownership of only ten metadata paths in two first-party
cache directories while holding the build lock; no live Cargo/rustc was found.
Original failures are retained. No production source, global perf setting,
fixture storage placement or hosted CI workflow was changed.
