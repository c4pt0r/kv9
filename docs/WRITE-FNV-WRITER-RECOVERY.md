# Bounded FNV writer: default release and ordinary recovery

The isolated FNV writer candidate now passes a clean default release,
independent source/binary verification and ordinary three-voter recovery.
Both streaming and unary transports retain checked histories across leader
loss and restart from the original directories: **365 complete operations,
337 OK and 28 unknown**. Subsequent [actual Chaos Mesh acceptance](WRITE-FNV-WRITER-CHAOS.md)
also passes all 21 windows. Matched database measurements remain required;
CRC main is still selected and no new QPS gain is claimed.

## Exact runtime and retained build

The build uses the already tested source commit
`12f44d35590ede5f89337fe731dd950162865154`, containing the bounded four-body,
64 KiB writer and its proof/tests. All **887 source files** match the original
clean source-gate snapshot. Source-tree SHA-256:
`ef5201783441fc0bff44fa0256f0ad473ab669dc5338ba5af7cfa4c7e71c562f`.

The first preflight on subsequent documentation commit `37a491e` failed before
compilation: evidence archives totaled 67,251,881 bytes, exceeding the existing
67,108,864-byte inventory ceiling by **143,017 bytes**. The failure is retained.
A new detached worktree at the exact tested commit resolves the packaging
constraint. The published commit differs only in twelve documentation paths;
all runtime and tested source bytes match. No source limit was raised, evidence
deleted, production feature changed or test bypassed.

The retained build uses rustc **1.94.0**, LLVM **21.1.8**, opt-level 3, ThinLTO
and one codegen unit. Default server and same-source correctness-workload
features are empty; there is no explicit CPU/ISA/panic override. The existing
shared-cache lock, first-party release invalidation and first-observation
recompilation checks pass. Dependency artifacts remain reusable. Older retained
CRC server/workload executables are unchanged.

| Artifact | SHA-256 |
| --- | --- |
| Default server | `d84b0ec8e466a9dc3f4953751605d91c90a90a66df1b03b57a3412e43e42603e` |
| Same-source correctness workload | `6b1867110dd7eaab5d3a8571b2227e77658ecb6b89f59dd4460836b691935ff3` |

This workload is the recovery correctness client, not the fixed native v3
benchmark client. Its results are not throughput measurements.

## Recovery and independent history acceptance

The original fixture and unchanged independent auditor exercise overlapping
point Put/Get/Delete and atomic BatchPut/BatchGet. They require successful
operations with one voter lost and after its original-directory restart,
complete histories, original durability predicates, fresh exporter drains,
unchanged bound source/artifacts and all owned process exits.

| Transport | Complete operations | OK | Unknown | Fresh drained voters |
| --- | ---: | ---: | ---: | ---: |
| Streaming | 182 | 170 | 12 | 3 |
| Explicit unary | 183 | 167 | 16 | 3 |
| Total | 365 | 337 | 28 | 6 |

All histories and original 26-exporter predicates pass. Unknown writes remain
unknown; they are not converted to failures or successful acknowledgments.
Five voter lifetimes and two client lifetimes exit, with no cleanup errors.
The audit independently checks operation overlap and progress in both fault
windows, source and binary lineage, complete original inventories and cleanup.
This process fixture is separate from the required actual Chaos Mesh campaign.

Release session **53741**, terminal **b9574b/0**, and independent readback
**b71ed8/0** pass. Recovery session **10589**, terminal **bc3b35/0**, and the
unchanged independent audit **103df6/0** pass. Original limits remain: 96 GiB
launch minimum for both, a continuous 96 GiB release floor / 80 GiB recovery
floor, and a 16 GiB maximum observed decrease. Minimum observed available
space is 113,321,172,992 bytes during release and 113,317,961,728 during recovery.
These observations do not reserve the later Chaos or performance campaign.

[Portable original build, histories, payloads and audit](https://github.com/c4pt0r/kv9/blob/c9d84fa236b45fccb626ef8afd13c91c95984183/docs/fnv-writer-runtime-v1/README.md)
passes independent archive readback. The earlier [source qualification](WRITE-FNV-WRITER.md)
remains separate: 797 tests/doctests, 23 existing ignored, 15 writer plus six
kernel proof statements and deterministic storage-failure matrices.

Subsequent actual 21-window Chaos Mesh acceptance on this bound runtime passes,
including pressure/test-image feature isolation, independent full-history
audits, original unknown outcomes, final drains and scoped cleanup. The
subsequent [matched point/batch c1/c64 write comparison](WRITE-FNV-WRITER-PERFORMANCE.md)
now passes eight smokes, sixteen timed cohorts and independent acceptance.
Batch throughput improves 4.131%, but pooled p99 worsens; CRC main stays selected.
Hosted CI remains manual and was not dispatched. This checkpoint closes no
original industrial roadmap work package.
