# Published-directory default release and ordinary recovery

Measured and checked locally on 2026-09-14 UTC. Experimental runtime
`483b8c3629b033734f5d7a2b8653a1352304a4b5` now passes a clean default release,
independent binary/source verification and three-voter process recovery.
CRC main remains selected. This release/recovery report contains no QPS result;
the subsequent [actual Chaos baseline](WRITE-PUBLISHED-DIRECTORY-CHAOS.md) is
qualified separately.

All 871 source files are bound to the clean release worktree; every non-document
source file matches the completed source gate. The runtime files also match the
24-obligation conditional proof. The build preserves opt-level 3, ThinLTO and
one codegen unit, with empty default production feature sets and no CPU/ISA/panic
override. Shared-cache locking, first-party release invalidation and first-use
recompilation checks pass. The retained artifacts are copies outside Cargo target.

| Artifact | SHA256 |
| --- | --- |
| Default server | `43d8bcb2d852d6fcbebb7f528808e53b3eb93588e2637195e9053cd91e8d5450` |
| Same-source correctness client | `205bcf4451d8f1d59023bcd4893d3388ed73846cf0980cbc9efa6cb5df578a2e` |

The correctness client exercises overlapping point Put/Get/Delete and atomic
BatchPut/BatchGet. It is not the fixed native performance client. The original
fixture and byte-identical independent history auditor require successful work
with a voter lost, after its original-directory restart, complete histories,
unchanged durability predicates and fresh drained exporters.

| Transport | Complete operations | OK | Unknown | Fresh drained voters |
| --- | ---: | ---: | ---: | ---: |
| Streaming | 188 | 169 | 19 | 3 |
| Explicit unary | 176 | 164 | 12 | 3 |
| Total | 364 | 333 | 31 | 6 |

Unknown writes stay unknown. The audit verifies overlapping point/batch calls,
progress in both fault windows, original inventories and all five voter/two
client lifetimes exited with no cleanup errors. These are process-recovery
checks, not a physical power-loss test or actual Chaos Mesh campaign.

Release `80235/57e676/0`, independent readback `f68dc7/0`, recovery
`39585/962723/0` and independent audit `027b71/0` pass. The separate explicit
`kv9-published-directory-release-recovery-v2` resource policy requires 64 GiB
plus 8 MiB available at launch, a continuous 48 GiB floor, a 16 GiB maximum
sampled decrease and the original 1,200-second deadline. It budgets the full
16 GiB above the floor at launch. Original FNV policy/helpers are unchanged;
all source/build/feature, quorum, sync, history and cleanup requirements remain.
Accounting uses `f_bavail`, includes unrelated host writers, and is sampled
rather than a hard allocation reservation. This is not the benchmark or Chaos
policy. Minimum observed free space is 67,995,017,216 bytes during release and
73,163,796,480 bytes during recovery.

After release, inactive first-party dev-only cache retirement reclaimed an
observed 5,172,379,648 bytes. All eight retained executable hashes match before
and after. Dependency/release caches, source and original test payloads remain.

[Portable original build, histories, WAL bytes, policy derivation and audit](https://github.com/c4pt0r/kv9/blob/4e8cd198587b556abe6534260c09bf921615f077/docs/published-directory-runtime-v1/README.md)
contains 138 files / 4,365,387 decoded bytes in 416,043 compressed bytes. All
archive members pass independent hash verification. The subsequent actual
21-window Chaos baseline passes. Positive segment-rotation/crash coverage and
matched write throughput and latency remain pending.
All work ran locally; no hosted CI was dispatched and no original industrial
roadmap checkbox closes.
