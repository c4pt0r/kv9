# Split-publication validation packet

See [implementation, publication semantics and remaining scope](../SPLIT-PUBLICATION.md).

- 931 passing workspace tests/doctests in the serial qualifying run, 35
  ignored, zero failures; strict Clippy/formatting pass; 8 real-MinIO
  focused installer tests.
- Ten checked Lean theorems for the split-publication model with six
  semantic mutation controls and two proof-policy controls; twenty
  sibling Lean models and the retention TLA/TLAPS model re-accepted
  against the exact working tree.
- Two accepted five-process e2e runs (`e2e-fifth` covers the exact final
  source after a dead-code cleanup; `e2e-fourth` first ran green) and
  THREE retained failed launches, each exposing a real serving-path
  defect this increment fixed: the split intent's readback refused after
  its own publication; the raw directory resolved keyspaces assuming
  exactly one range; and directory inserts never replaced a stale
  binding. The accepted runs: the full qualified chain (intent, fence,
  digest-verified population), the atomic one-to-two catalog transaction
  with local re-verification and an idempotent confirming retry, public
  routing serving the exact pre-split data from the correct children,
  new writes landing on the correct child, and a full-cluster restart
  with the directory, both children and the sealed parent all durable.

[evidence.tar.gz](evidence.tar.gz) contains **12824 readback-verified
files**, 1304952 compressed bytes and 11393097 decoded bytes. SHA-256:
`41964ec7a420d1305f07932547aa03fd6617f82104eb2767b194872baac72e3a`.
[validation.json](validation.json) inventories all members. A separate
extraction re-verified every member and the e2e/proof/retention receipts;
[portable-readback.json](portable-readback.json) records that result.

Tested source is based on `3e029ca` with the exact current
Rust/Cargo/proto source retained under `source/`.

**Manual D04 splits are now end to end.** Nothing here retires the
sealed parent group, adds automatic triggers, serves cross-range routed
scans, reclaims storage, or claims scaling or new QPS. This increment
ran no Chaos Mesh campaign. The independent 3/6/9-host benchmark
contract in [HORIZONTAL-SCALING-PLAN.md](../HORIZONTAL-SCALING-PLAN.md)
remains the acceptance criterion for effective horizontal scaling. No
hosted CI was dispatched.
