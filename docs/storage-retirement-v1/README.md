# Storage-retirement validation packet

See [implementation, retirement semantics and remaining scope](../STORAGE-RETIREMENT.md).

- 927 passing workspace tests/doctests in the serial qualifying run, 35
  ignored, zero failures; strict Clippy/formatting pass; 8 real-MinIO
  focused installer tests.
- Eleven checked Lean theorems for the storage-retirement model with six
  semantic mutation controls and two proof-policy controls; fifteen
  sibling Lean models and the retention TLA/TLAPS model re-accepted
  against the exact working tree.
- Two accepted five-process e2e runs (`e2e-second` covers the exact final
  source; `e2e-first`, also accepted, preceded a mechanical dead-field
  cleanup): the full qualified chain through committed removal, then the
  removed node retiring its replica AUTOMATICALLY under the committed
  decision while running, the durable group storage verified intact, the
  retired state surviving restart with nothing reopened, and the
  surviving three-voter quorum committing throughout.

[evidence.tar.gz](evidence.tar.gz) contains **933 readback-verified
files**, 845489 compressed bytes and 5010346 decoded bytes. SHA-256:
`1102161c45fdb4687d72fc55b57aa14f21dbce3c5377df215d94f07cce5628b4`.
[validation.json](validation.json) inventories all members. A separate
extraction re-verified every member and the e2e/proof/retention receipts;
[portable-readback.json](portable-readback.json) records that result.

Tested source is based on `e21a7bb` with the exact current
Rust/Cargo/proto source retained under `source/`.

Nothing here physically deletes or reclaims storage, recovers a stranded
learner, serves from a non-leader, splits, places, or claims scaling or
new QPS. This increment ran no Chaos Mesh campaign. The independent
3/6/9-host benchmark contract in
[HORIZONTAL-SCALING-PLAN.md](../HORIZONTAL-SCALING-PLAN.md) remains the
acceptance criterion for effective horizontal scaling. No hosted CI was
dispatched.
