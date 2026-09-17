# Split-intent validation packet

See [implementation, intent semantics and remaining scope](../SPLIT-INTENT.md).

- 929 passing workspace tests/doctests in the serial qualifying run, 35
  ignored, zero failures; strict Clippy/formatting pass; 8 real-MinIO
  focused installer tests.
- New catalog coverage: planner refusals (unbound parent, identical or
  missing children), idempotent confirmation, one split key per operation,
  one live intent per parent region, and the readback defect matrix over
  the cross-bound fields (parent binding row, both child tasks).
- Eleven checked Lean theorems for the split-intent model with seven
  semantic mutation controls and two proof-policy controls; sixteen
  sibling Lean models and the retention TLA/TLAPS model re-accepted
  against the exact working tree.
- Two accepted five-process e2e runs (`e2e-second` covers the exact final
  source; `e2e-first`, also accepted, preceded a mechanical Clippy
  rewrite): a bound parent keyspace under writes, two activated unbound
  child groups, the refusal matrix, the exact intent committing once and
  confirming idempotently, and the parent range still serving the same
  data afterward — the intent alone seals, populates, republishes and
  reroutes nothing.

[evidence.tar.gz](evidence.tar.gz) contains **844 readback-verified
files**, 808567 compressed bytes and 4879664 decoded bytes. SHA-256:
`26f6a9bb6ee729ebae6f9467d0ec872fcd65a9c3be8065b9971dcb83224966da`.
[validation.json](validation.json) inventories all members. A separate
extraction re-verified every member and the e2e/proof/retention receipts;
[portable-readback.json](portable-readback.json) records that result.

Tested source is based on `6f3ee42` with the exact current
Rust/Cargo/proto source retained under `source/`.

Nothing here seals a parent, populates a child, republishes the range
directory, reroutes a key, reclaims storage, or claims scaling or new
QPS. This increment ran no Chaos Mesh campaign. The independent
3/6/9-host benchmark contract in
[HORIZONTAL-SCALING-PLAN.md](../HORIZONTAL-SCALING-PLAN.md) remains the
acceptance criterion for effective horizontal scaling. No hosted CI was
dispatched.
