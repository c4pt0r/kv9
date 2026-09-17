# Voter-promotion validation packet

See [implementation, promotion semantics and remaining scope](../VOTER-PROMOTION.md).

- 925 passing workspace tests/doctests in the serial qualifying run, 35
  ignored, zero failures; strict Clippy/formatting pass; 8 real-MinIO
  focused installer tests.
- Eleven checked Lean theorems for the voter-promotion model with seven
  semantic mutation controls and two proof-policy controls; thirteen
  sibling Lean models and the retention TLA/TLAPS model re-accepted
  against the exact working tree.
- One accepted five-process e2e (`e2e-first`, first attempt) chaining the
  previously qualified settlement and committed compaction into promotion:
  a wrong-operation refusal → promotion to voters `1,2,3,4` through the
  group's own log with an idempotent confirming retry → a write committed
  with one ORIGINAL voter stopped (the promoted voter participates in the
  quorum) → non-leader read refusal → restart of the promoted voter
  through its adopted base → rejoin of the stopped original voter under
  the evolved committed configuration → every member tracking the final
  write.

[evidence.tar.gz](evidence.tar.gz) contains **701 readback-verified
files**, 762841 compressed bytes and 4348662 decoded bytes. SHA-256:
`5a6a8a1d6e845aca750cb99809341a4ca898807df5b29fa532ec5f1ed60538aa`.
[validation.json](validation.json) inventories all members. A separate
extraction re-verified every member and the e2e/proof/retention receipts;
[portable-readback.json](portable-readback.json) records that result.

Tested source is based on `2b8f5c0` with the exact current
Rust/Cargo/proto source retained under `source/`.

Nothing here removes a replica, recovers a stranded learner, reclaims
disk space, serves from a non-leader, splits, places, or claims scaling
or new QPS. This increment ran no Chaos Mesh campaign. The independent
3/6/9-host benchmark contract in
[HORIZONTAL-SCALING-PLAN.md](../HORIZONTAL-SCALING-PLAN.md) remains the
acceptance criterion for effective horizontal scaling. No hosted CI was
dispatched.
