# Partition-directory validation packet

See [implementation, partition semantics and remaining scope](../PARTITION-DIRECTORY.md).

- 930 passing workspace tests/doctests in the serial qualifying run, 35
  ignored, zero failures; strict Clippy/formatting pass; 8 real-MinIO
  focused installer tests.
- New catalog coverage hand-crafts the post-split directory before any
  writer exists: a sealed parent with no children refuses (uncovered), a
  gapped child set refuses, the exact partition is accepted with routing
  skipping the sealed parent and the boundary key belonging to the high
  child, and an overlapping extra child refuses the whole readback.
- Eleven checked Lean theorems for the partition-directory model with
  five semantic mutation controls and two proof-policy controls;
  seventeen sibling Lean models and the retention TLA/TLAPS model
  re-accepted against the exact working tree.
- One accepted five-process regression e2e (`e2e-first`, the committed
  split-intent flow rerun against the generalized read model): the legacy
  trivial partition keeps serving unchanged under live writes.

[evidence.tar.gz](evidence.tar.gz) contains **743 readback-verified
files**, 761213 compressed bytes and 4489612 decoded bytes. SHA-256:
`46e72852848276c9f7992fac3bc3f53e713077b5be243c4b99ad0af1a2fc533d`.
[validation.json](validation.json) inventories all members. A separate
extraction re-verified every member and the e2e/proof/retention receipts;
[portable-readback.json](portable-readback.json) records that result.

Tested source is based on `17714a5` with the exact current
Rust/Cargo/proto source retained under `source/`.

Nothing here writes a child binding, seals a parent, populates a child,
reroutes a key differently for existing directories, reclaims storage, or
claims scaling or new QPS. This increment ran no Chaos Mesh campaign. The
independent 3/6/9-host benchmark contract in
[HORIZONTAL-SCALING-PLAN.md](../HORIZONTAL-SCALING-PLAN.md) remains the
acceptance criterion for effective horizontal scaling. No hosted CI was
dispatched.
