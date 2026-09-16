# Inline-key qualification evidence

See the [report](../INLINE-KEY-QUALIFICATION.md). Source/model/proof qualification
passes; no engine timing or production promotion is included.

[evidence.tar.gz](evidence.tar.gz) contains 231 members,
25,574,119 expanded bytes and 3,999,424 compressed bytes.
SHA-256: `a4ba06b9691df33b25ece57652898f13d79fc281da5090dabdaf3c8f568fc5ab`.
Every member size/hash passed readback; [members.json](members.json) and
[manifest.json](manifest.json) record retained bytes and excluded local binaries.

The packet includes both engine/common source workspaces, the exact private key
and patch, matching model and existing tests, complete Clippy/build/test outputs,
19-theorem / nine-control conditional proof, 33 selected allocator observations,
constructor disassembly and independent qualification audit. Upstream dependency
archives are identified by Cargo.lock SHA-256; installed source files were
checked against those archives. The allocator module is retained with probe inputs.

The selected allocation probe is `layout-formatted`. Initial preparation failure,
proof-authoring diagnostics and the unselected pre-format probe/runner remain
separate and are not pooled. Executable hashes remain in build records.

Reproduction uses source base `14f35ec` with these experiment tools and the
recorded absolute paths. Restore those inputs or adapt a fresh attempt without
relabeling existing results. The [next screen](next-performance-plan.json) is
prospective, not implemented or executed. Its original corpus is already in
the [outlined packet](../outlined-mutation-path-v1/README.md) as `outlined/groups.bin`.
No industrial checklist item is closed by this component qualification.
