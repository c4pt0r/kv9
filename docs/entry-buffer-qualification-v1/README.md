# Single-buffer qualification evidence

See the [report](../ENTRY-BUFFER-QUALIFICATION.md). Source/model/proof/layout
qualification passes; engine timing and production promotion remain pending.

[evidence.tar.gz](evidence.tar.gz) contains 224 members,
14,626,811 expanded bytes and 2,300,495 compressed bytes.
SHA-256: `401e3194629ef3138736df1d2b08f12067a9cb2d523a3613fe4b6e92a50cb2fa`.
Every member size/hash passed readback; [members.json](members.json) and
[manifest.json](manifest.json) record retained bytes and excluded local binaries.

The packet contains the two engine/common workspaces, exact private type and
reviewed patch, inherited and payload models, full Clippy/build/test outputs,
23-theorem / 13-control conditional proof, 198 allocator rows, disassembly and
independent audit. Selected dependency sources match all 50 source/manifest
members of Cargo.lock-pinned archives. Reused build-cache/test/model/counter
helpers are included. Retained executable identities remain in build records.

The selected allocation probe is `layout`. Initial proof-authoring diagnostics
are retained separately and do not count as accepted proof. No latency is
recorded by this allocation-only probe.

Reproduction uses source base `e9ebdc4` and these experiment tools; absolute
paths and a pinned Lean binary are recorded. Restore those inputs or identify
a fresh attempt without relabeling retained results. The prospective
[next screen](next-performance-plan.json) has not been executed. Its original
corpus is in the [outlined packet](../outlined-mutation-path-v1/README.md),
member `outlined/groups.bin`.

No new database/Redis, MinIO, ordinary distributed recovery or actual Chaos
acceptance is included. No industrial checklist item closes here.
