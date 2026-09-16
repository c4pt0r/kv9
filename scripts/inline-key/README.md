# Inline-key qualification

This isolated candidate changes only MemEngine's private persistent-map key.
Keys of at most 40 bytes are copied into an inline array; longer keys use Vec.
Eq, Ord and Borrow all use the complete byte slice. No unsafe code is added to
the key or engine. Public keys, wire/storage formats, Raft, WAL and read authority
remain unchanged. Production crates are not patched in the repository.

`prepare.py NEW_ROOT` copies the source-base engine/common workspace and applies
the strict representation patch to one arm. It requires source base `14f35ec`;
restore that checkout plus these experiment tools to reproduce. Existing test
sources and the new independent BTreeMap model are identical in both arms.
`test.py ROOT` runs Clippy and complete engine/common target suites using the
retained-build cache guard. It preserves existing ignored external tests.

`layout.py ROOT [LABEL]` builds a separate ordinary-release allocation probe after tests.
It compiles this exact `mem_key.rs`, original registry rpds and the existing
jemalloc request counter. Input/value construction and result validation/drop
are outside the window. Counts describe Rust allocator requests, not physical
jemalloc size classes or RSS. No elapsed time is measured. The key/value tuple
size is reported as a tuple, not as rpds' private Entry layout.

`prove.py ROOT LEAN` checks the [conditional refinement proof](../../proofs/lean/inline-key/README.md)
and semantic, proof-hole, custom-axiom and source-substitution controls.
`audit.py ROOT [LABEL]` independently checks source/archive identities, test totals,
allocation expectations and the retained constructor's machine code.

Bulk output belongs under `/mnt/data/kv9-work`, with the existing repository
target reused. Do not overwrite a completed attempt. A preparation anchor-count
error in the first attempt is retained separately; the corrected v2 run is the
qualified result. See the [report](../../docs/INLINE-KEY-QUALIFICATION.md).

The next performance screen must include long/empty key cost, pinned snapshots,
owned/resident reads and affected range interfaces. This qualification does not
authorize production promotion or replace database/recovery/Chaos/Redis gates.
