# Parent-retirement validation packet

See [implementation, retirement semantics and remaining scope](../PARENT-RETIREMENT.md).

- 931 passing workspace tests/doctests in the serial qualifying run, 35
  ignored, zero failures; strict Clippy/formatting pass; 8 real-MinIO
  focused installer tests.
- Eleven checked Lean theorems for the parent-retirement model with six
  semantic mutation controls and two proof-policy controls; twenty-one
  sibling Lean models and the retention TLA/TLAPS model re-accepted
  against the exact working tree.
- One accepted five-process e2e (`e2e-second`; `e2e-first` failed on a
  REAL race, retained: a publication retry after the parent had already
  auto-retired demanded local verification of a group that legitimately
  no longer runs — the idempotent confirm now reads the committed
  directory alone). The accepted run: the full manual split through the
  atomic publication, every voter retiring its sealed parent
  automatically under the published directory, the durable parent
  storage verified intact on every node, the retired state surviving a
  further restart with nothing reopened, and the children serving the
  split keyspace throughout.

[evidence.tar.gz](evidence.tar.gz) contains **1028 readback-verified
files**, 849277 compressed bytes and 5249320 decoded bytes. SHA-256:
`4430f2cfbaa00e3ef85f4883fd31c6347865020da3e9fadb18a63f1079387d0b`.
[validation.json](validation.json) inventories all members. A separate
extraction re-verified every member and the e2e/proof/retention receipts;
[portable-readback.json](portable-readback.json) records that result.

Tested source is based on `4e6b70d` with the exact current
Rust/Cargo/proto source retained under `source/`.

Nothing here physically deletes or reclaims storage, adds automatic
split triggers, serves cross-range scans, or claims scaling or new QPS.
This increment ran no Chaos Mesh campaign. The independent 3/6/9-host
benchmark contract in [HORIZONTAL-SCALING-PLAN.md](../HORIZONTAL-SCALING-PLAN.md)
remains the acceptance criterion for effective horizontal scaling. No
hosted CI was dispatched.
