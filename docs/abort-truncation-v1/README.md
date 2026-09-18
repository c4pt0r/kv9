# Abort-truncation validation packet

See [the settlement extension and its guards](../ABORT-TRUNCATION.md).

- 948 passing workspace tests/doctests in the serial qualifying run
  (including the new catalog test: an abort settles truncation without
  an evidence cut — plan, readback, idempotent confirm, divergent-floor
  refusal, with unsettled and double-settled paths refusing), 35
  ignored, zero failures; strict Clippy/formatting pass; 8 real-MinIO
  focused installer tests.
- Thirteen checked Lean theorems for the abort-truncation model with
  five semantic mutation controls and two proof-policy controls; thirty
  sibling Lean models and the retention TLA/TLAPS model re-accepted
  against the exact working tree.
- Two accepted five-process e2e runs (`e2e-fifth` covers the exact
  final source), extending the full stranded-recovery chain: stranding,
  the committed abort, ledger settlement, learner detach, then the
  ABORT-SETTLED truncation decision and raft-gated compaction (the
  in-packet receipt shows `first_index` advanced past the floor), every
  voter restarting on the compacted log with the group still writing,
  and a fresh incarnation re-migrating the COMPACTED region end to end.
  Three retained failures: a fixture token-rename slip, and two
  metadata-leader-migration walls that produced per-verb leader
  re-resolution in the fixture.

[evidence.tar.gz](evidence.tar.gz) contains **1465 readback-verified
files**, 1129837 compressed bytes and 7280424 decoded bytes. SHA-256:
`7be728e722aeae5b262c2d8de421fc2b3dc5f8079d3d011e6e46d58e4726cde3`.
[validation.json](validation.json) inventories all members. A separate
extraction re-verified every member and every receipt — including the
compaction receipt itself; [portable-readback.json](portable-readback.json)
records that result.

Tested source is based on `2c09a2e` with the exact current
Rust/Cargo/proto source retained under `source/`.

Healthy-group (non-migration) log compaction and bounded log growth in
general remain open; nothing here reclaims compacted segments
physically or makes performance claims. No Chaos Mesh campaign ran.
The 3/6/9-host contract in
[HORIZONTAL-SCALING-PLAN.md](../HORIZONTAL-SCALING-PLAN.md) remains the
scaling acceptance path. No hosted CI was dispatched.
