# Destination-evidence validation packet

See [implementation, settlement semantics and remaining scope](../DESTINATION-EVIDENCE.md).

- 923 passing workspace tests/doctests in the serial qualifying run, 35
  ignored, zero failures; strict Clippy/formatting pass; 8 real-MinIO
  focused installer tests.
- New catalog coverage: the evidence planner requires the committed
  migration and matches it exactly (cross-bound flips refuse, a receipt
  cannot supply its own task, one operation never names a second image,
  identical resubmission confirms), the readback defect matrix, and the
  ledger's view-level probe matching only the exact owner-pair derivation.
  New retention coverage: quiesce/release keep refusing with no or
  mismatched evidence, and the matching committed row settles source
  quiesce then release with the destination pin published throughout.
- Thirteen checked Lean theorems for the install-evidence settlement model
  with nine semantic mutation controls and two proof-policy controls;
  eleven sibling Lean models and the retention TLA/TLAPS model re-accepted
  against the exact working tree.
- One accepted five-process e2e (`e2e-first`), first attempt, chaining the
  previously qualified adoption/catchup flow into settlement: quiesce and
  release refuse **before** any committed evidence → the destination
  replays its durable receipt naming the exact installed image and cut →
  a divergent-subject receipt refuses at commit against the published pin
  → recording commits once and confirms idempotently → the committed row
  unlocks source quiesce, then release, through the unchanged retention
  ledger → the destination pin refuses release and the replica keeps
  tracking the source leader's retained tail afterward.

[evidence.tar.gz](evidence.tar.gz) contains **634 readback-verified files**,
731767 compressed bytes and 4097126 decoded bytes. SHA-256:
`5e9ea1f2091c90d61085a6b836baa289fc27d80322b6813c55a1f819a5b7e1c4`.
[validation.json](validation.json) inventories all members. A separate
extraction re-verified every member and the e2e/proof/retention receipts;
[portable-readback.json](portable-readback.json) records that result.

Tested source is based on `d6bfc6ed94510d49bdde63de5c278dc905ea4e5c` with
the exact current Rust/Cargo/proto source retained under `source/`.

Nothing here promotes a voter, serves from the learner, truncates the
source log, physically deletes an object (deletion stays disabled
ledger-wide), releases the destination pin, splits, places, or claims
scaling or new QPS. This increment ran no Chaos Mesh campaign. The
independent 3/6/9-host benchmark contract in
[HORIZONTAL-SCALING-PLAN.md](../HORIZONTAL-SCALING-PLAN.md) remains the
acceptance criterion for effective horizontal scaling. No hosted CI was
dispatched.
