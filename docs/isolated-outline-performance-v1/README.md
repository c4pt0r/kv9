# Isolated shared-clone performance evidence

See the [report](../ISOLATED-OUTLINE-PERFORMANCE.md). Correctness and allocator
accounting pass; the material-write and read gates fail. Production is unchanged.

[evidence.tar.gz](evidence.tar.gz) contains 776 members,
73,576,821 expanded bytes and 4,480,521 compressed bytes.
SHA-256: `0879754d8d85c2a2e4efe82a0994eadadb62eddeb143d741794c82f6dcd9934d`.
Every member size/hash passed readback; [members.json](members.json) and
[manifest.json](manifest.json) record the retained bytes and exclusions.

The packet retains all 136 process launch/terminal records and outputs, all
152 timing / 76 allocation rows and raw samples, independent input/statistics/
allocation analyses, source correspondence, counting builds, cache checks and
Clippy logs. Timing reuses the exact qualified uninstrumented release pair.
Counted processes emit no latency measurement. Snapshot, owned/resident,
first-probe/warm and per-call/512-query-pass scopes remain separate.

Dependency sources, timing-harness build inputs and prior conditional proof/test
qualification are in the [qualification packet](../isolated-outline-v1/README.md).
The original corpus is already published as `outlined/groups.bin` in the
[outlined packet](../outlined-mutation-path-v1/README.md). Exact hashes and local
executable identities are in the manifest. Binaries remain local. Reproduction
uses recorded absolute paths; restore those inputs or adapt a fresh experiment
without relabeling an existing result.

The [declared plan](declared-plan.json) and [executed plan](plan.json) retain the
same cases, binary identities, counts, windows and gates; the pre-execution
clarification is included in the archive. The [next inline-key plan](next-inline-key-plan.json)
is prospective, not implemented or measured. No database/Redis QPS, ordinary
recovery, actual Chaos acceptance or industrial checkbox closure is claimed.
