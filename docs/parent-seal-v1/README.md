# Parent-seal validation packet

See [implementation, fence semantics and remaining scope](../PARENT-SEAL.md).

- 930 passing workspace tests/doctests in the serial qualifying run, 35
  ignored, zero failures; strict Clippy/formatting pass; 8 real-MinIO
  focused installer tests.
- Eleven checked Lean theorems for the parent-seal model with five
  semantic mutation controls and two proof-policy controls; eighteen
  sibling Lean models and the retention TLA/TLAPS model re-accepted
  against the exact working tree.
- One accepted five-process e2e (`e2e-first`, first attempt): the
  committed split-intent flow, a wrong-operation seal refusal, the seal
  committing through the parent group's own log at version 2 (one-way CAS
  from the current row digest via the pre-existing `may_follow` shape),
  an idempotent confirming retry, immediate TYPED write and read refusals
  at the sealed parent, and the durable fence surviving a leader restart
  with the group recovered active while the catalog directory stays
  untouched and unchanged.

[evidence.tar.gz](evidence.tar.gz) contains **773 readback-verified
files**, 787539 compressed bytes and 4643744 decoded bytes. SHA-256:
`5cbe1abf58dc50d7c5866fc3b36bbbe4c303305864ea612f341847bc30e8e377`.
[validation.json](validation.json) inventories all members. A separate
extraction re-verified every member and the e2e/proof/retention receipts;
[portable-readback.json](portable-readback.json) records that result.

Tested source is based on `dc45d22` with the exact current
Rust/Cargo/proto source retained under `source/`.

Nothing here populates a child, republishes the range directory, reroutes
a key, unseals anything, reclaims storage, or claims scaling or new QPS.
During the fence window the keyspace deliberately refuses with typed
errors — the bounded pause of a manual split. This increment ran no Chaos
Mesh campaign. The independent 3/6/9-host benchmark contract in
[HORIZONTAL-SCALING-PLAN.md](../HORIZONTAL-SCALING-PLAN.md) remains the
acceptance criterion for effective horizontal scaling. No hosted CI was
dispatched.
