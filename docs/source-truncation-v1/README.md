# Source-truncation validation packet

See [implementation, compaction semantics and remaining scope](../SOURCE-TRUNCATION.md).

- 925 passing workspace tests/doctests in the serial qualifying run, 35
  ignored, zero failures; strict Clippy/formatting pass; 8 real-MinIO
  focused installer tests.
- New catalog coverage: the truncation planner requires the committed
  evidence row, bounds the floor by the evidence cut, confirms identical
  decisions idempotently, refuses a second floor per operation, and the
  readback defect matrix refuses row/evidence rebinding.
- Thirteen checked Lean theorems for the source-truncation model with
  eight semantic mutation controls and two proof-policy controls; twelve
  sibling Lean models and the retention TLA/TLAPS model re-accepted
  against the exact working tree.
- One accepted five-process e2e (`e2e-fourth`) with **three failed
  launches retained as evidence**. The failures exposed one wrong seam and
  two real pre-existing restart gaps, all fixed: reusing
  `install_protocol_snapshot` for self-compaction refused correctly (it is
  a forward-install seam — a dedicated tail-preserving `REC_COMPACTION`
  seam replaced it); `validate_fixed_group` refused any source voter
  restart after learner attach (now authorizes membership the group's own
  committed log evolved from the creation voters); and the engine WAL
  replay cross-check probed batch positions below the compacted floor
  (now vouched for by the durable compaction record). The accepted run
  chains the settled transfer into: compaction refusal without the
  committed decision → floor-beyond-evidence-cut refusal → exact
  compaction to floor+1, idempotent → restart of the compacted voter under
  committed authority only → acknowledged data below the floor still
  served → the learner still tracking the tail.

[evidence.tar.gz](evidence.tar.gz) contains **1169 readback-verified
files**, 978356 compressed bytes and 5868605 decoded bytes. SHA-256:
`e53309c5c2ef1bbf1b1dc833c9dbf5547f79a7022a05479bc307204a4bf90449`.
[validation.json](validation.json) inventories all members. A separate
extraction re-verified every member and the e2e/proof/retention receipts;
[portable-readback.json](portable-readback.json) records that result.

Tested source is based on `d515f8c` with the exact current
Rust/Cargo/proto source retained under `source/`.

Nothing here reclaims physical disk space, compacts a follower, recovers a
stranded learner, promotes a voter, serves from the learner, releases the
destination pin, splits, places, or claims scaling or new QPS. This
increment ran no Chaos Mesh campaign. The independent 3/6/9-host benchmark
contract in [HORIZONTAL-SCALING-PLAN.md](../HORIZONTAL-SCALING-PLAN.md)
remains the acceptance criterion for effective horizontal scaling. No
hosted CI was dispatched.
