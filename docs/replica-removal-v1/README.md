# Replica-removal validation packet

See [implementation, removal semantics and remaining scope](../REPLICA-REMOVAL.md).

- 927 passing workspace tests/doctests in the serial qualifying run, 35
  ignored, zero failures; strict Clippy/formatting pass; 8 real-MinIO
  focused installer tests. (One post-run mechanical Clippy rewrite in the
  removal planner was re-proven and the meta suite re-run green; the
  packet's proof receipt covers the exact final source.)
- New catalog coverage: only an initial creation replica may be named
  (never the migration destination, never a replaced disk), one
  retirement per operation with idempotent confirmation, and the readback
  defect matrix refuses row/authority rebinding.
- Twelve checked Lean theorems for the replica-removal model with seven
  semantic mutation controls and two proof-policy controls; fourteen
  sibling Lean models and the retention TLA/TLAPS model re-accepted
  against the exact working tree.
- One accepted five-process e2e (`e2e-second`; one failed launch retained
  — the fixture reused a stale metadata leader after earlier restarts, a
  harness lesson, not a server defect): the full qualified chain through
  settlement, compaction and promotion, then removal refusal without the
  committed decision, the destination refused as removal target, the
  exact committed replica leaving the voter set to exactly three voters
  including the migrated destination (idempotent retry), writes committed
  by the surviving quorum, and the removed node restarting as a healthy
  isolated non-member while survivors keep committing.

[evidence.tar.gz](evidence.tar.gz) contains **909 readback-verified
files**, 850153 compressed bytes and 5013848 decoded bytes. SHA-256:
`f520372547f4a371b54004c8388899a0130659f442898c1e87ab6e38d03950a6`.
[validation.json](validation.json) inventories all members. A separate
extraction re-verified every member and the e2e/proof/retention receipts;
[portable-readback.json](portable-readback.json) records that result.

Tested source is based on `ec84035` with the exact current
Rust/Cargo/proto source retained under `source/`.

Nothing here retires or reclaims the removed replica's local storage,
recovers a stranded learner, serves from a non-leader, splits, places, or
claims scaling or new QPS. This increment ran no Chaos Mesh campaign. The
independent 3/6/9-host benchmark contract in
[HORIZONTAL-SCALING-PLAN.md](../HORIZONTAL-SCALING-PLAN.md) remains the
acceptance criterion for effective horizontal scaling. No hosted CI was
dispatched.
