# Isolated shared-clone extraction qualification

Date: 2026-09-16 UTC. Base: `0fd8fa68a452b16d05b6e7f787186d0d276fd8a4`.

The triomphe-only candidate passes source, conditional-proof, release-codegen
and engine-correctness qualification. **523 tests pass**, with three existing
ignored tests explicitly excluded. Both release preparations independently
validate **848 live prefix states / 424 old views** and exact query identities.
There is **no new performance measurement or production promotion** in this
increment. The combined candidate's earlier failed gate remains failed.

## One changed dependency

The [previous engine screen](ENGINE-INTERFACE-SCREEN.md) found real write savings
but repeatable owned-read regressions. That candidate changed both archery's
pointer callback and triomphe's shared-owner mutation path. This candidate keeps
**original registry archery 1.2.3 and rpds 1.2.1** in both release arms and changes
only triomphe 0.1.16. Registry archives match the production Cargo.lock checksums,
and every compiled archery/rpds source file matches its archive. The only changed
triomphe Rust file is `src/arc.rs`, byte-identical to the previously qualified
extraction. No engine, Raft, WAL, read-authorization or compiler-profile change
is included.

The extraction keeps one original Acquire uniqueness observation. Unique owners
fall through to the original mutable reference expression. Shared owners call
a cold, non-inlined helper containing the exact original clone, allocation and
owner-replacement statement. No reference-count operation or unsafe expression
is added. The original dynamic pointer write-back guard remains in place.

Ordinary ThinLTO/jemalloc release builds and release Clippy pass in both arms.
The shared build cache is explicitly invalidated for the harness, engine/common
and the three index dependencies; first observed artifacts are checked as fresh
compilations. Build/source/dependency records and both copied executables are
bound by hashes in the [packet](isolated-outline-v1/README.md).

## Actual mutation path

The codegen gate passes for the real `MemEngine` instantiation:

| Observation | Original dependencies | Triomphe-only candidate |
| --- | --- | --- |
| Recursive insert symbol size | 1,169 bytes | 1,192 bytes |
| Out-of-line mutation callback | 421 bytes, called before comparison | Removed |
| Unique-owner decision | Inside mutation callback | Direct load/compare in insertion |
| Original dynamic pointer callback | Retained | Retained |
| Shared clone helper | Inside mutation callback | Separate 397-byte helper |

In the candidate ELF, `0x40798` loads the reference count, `0x4079c` compares
against one and the non-unique branch reaches `0x40b0a`. Its call at `0x40b0d`
uses GOT `0x100b90`, whose relocation resolves to the clone helper at `0x43f00`.
Unique owners skip that helper but still execute the original pointer callback
at `0x407ae`. This removes the targeted mutation call without the archery patch.
The larger insertion symbol alone is neither a speedup nor a regression claim.
This is static compiled-path correspondence, not a runtime instruction trace.

## Proof and behavior checks

The unchanged 18 guard and eight extraction lemmas are rechecked with their
nine rejecting controls. Three new lemmas compose extraction with the **same
dynamic guard on both sides**, covering direct composition, one observation,
and restoration of the current owner during unwind. Five new controls reject
wrong-branch substitution, restoration of the initial owner, an admitted proof,
a custom axiom and substitution of the patched static guard. Total: **29
conditional lemmas / 14 rejecting controls**. Lean 4.33.1 checks all proofs,
and axiom reports reject custom dependencies.

The historical combined-source contract is retained only for the unchanged
dependency lemmas. The new source bindings separately identify original archery
and the exact triomphe extraction in the actual release dependency graph.
These are strict conditional control-flow/ownership equivalence proofs with
reviewed source correspondence. Original Arc contracts, lawful Rust pointer
provenance, ordinary unwinding and compiler correctness remain explicit premises.
This is not verified Rust extraction, a proof of the original Arc library, or
new Raft protocol proof coverage.

| Local check | Accepted result |
| --- | --- |
| Original archery behavior plus retained callback/unwind tests, with extracted triomphe | 58 passed |
| rpds, original registry archery and extracted triomphe | 276 passed |
| Unchanged common library | 44 passed |
| Unchanged engine library with extracted triomphe | 145 passed, 3 ignored |
| Exact release pair, both write workloads with/without old views | 848 live prefix states, 424 old views |
| Independent original corpus, final keys/values and seeded hit/miss probes | Both arms identical, four probe sets |

The three ignored tests are the real-MinIO base-validation test and two existing
WAL encoder benchmark entry points. They are not counted as passed or treated as
MinIO/recovery acceptance. The archery test fixture adds the previously retained
test-only callback cases; the actual release graph uses original registry
archery. All release preparations also verify applied index/data revision and
untouched Lock/Write column families. Both preparation processes exit normally.

The first copied engine test workspace accidentally retained the root binary
package and therefore required absent sibling crates. Cargo metadata refused
before any engine test ran. A fresh corrected workspace keeps only the intended
common/engine members and original workspace dependency settings. No Rust source
changed. The passed dependency runs, failed metadata and corrected 189-test run
are retained separately; no failed run is relabeled as passing.

## Next performance screen

The [prospective plan](isolated-outline-v1/next-performance-plan.json) fixes a
bounded engine screen for this changed candidate. It has **not been executed**.
Reuse the exact qualified timing binaries, ordinary release settings, original
corpus and 22-case engine protocol. Keep snapshot acquisition, owned/resident
lookup, first-probe/warm and per-call/whole-pass units separate. Whole-pass p99
is the latency of 512 queries, never request p99.

First implement a separate allocation-counting companion with the existing
counter module and identical operation/window boundaries. It must validate
state/probes and allocator requests, and its timing must be discarded. The plan
contains 90 timing processes / 152 timing rows and 46 counting processes / 76
counting rows, including their preparations. All four write cases must meet
the declared material mean reduction, and mean/p99 regressions are retained
against the declared read/write/snapshot bounds. No retrospective relaxation.

The prior combined candidate stays held. Do not replay its completed matrices,
alignment sweeps, named-adapter no-op or previously rejected owned-buffer removal.
Even a successful component screen does not authorize promotion: material matched
database benefit, full local correctness, ordinary recovery, actual Chaos Mesh
and matched three-copy Redis throughput/latency remain required.

Database results remain **137,873.776 Put calls/s / 1,022,750.059 BatchPut(64)
items/s**, source `86aa6fc`, c64, 128-byte values, three Raft replicas and the
shared-host volatile tmpfs fixture. No Redis rerun, new Chaos coverage or original
industrial checkbox closure follows from this qualification. CI remains local;
bulk evidence remains under `/mnt/data/kv9-work`.
