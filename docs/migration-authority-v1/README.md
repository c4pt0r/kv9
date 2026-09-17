# Migration authority validation packet

See [implementation, owner semantics, proofs and remaining scope](../MIGRATION-AUTHORITY.md).

- 913 passing workspace tests/doctests in the final serial qualifying run, 33
  ignored, zero failures; strict Clippy/formatting pass. The first serial run
  correctly failed once: the new ledger fence refused the generic transfer
  fixture's Snapshot-kind owners, and the kind-agnostic fixture now uses
  Pending, whose settlement seam actually exists. An explicit new test pins the
  fence for Snapshot/Migration kinds.
- Five meta migration tests: committed creation/activation prerequisites,
  active-destination and initial-replica refusals, exact idempotent retries,
  one live migration per group, cross-bound row defects and the shared 255-row
  budget. Ten meta retention tests including the quiesce/release fence.
- One full-cluster runtime test: a genuinely admitted fourth store through the
  production join flow, the committed intent RPC, image binding against the
  committed range, published owner readback, idempotent rebinding, second-image
  and foreign-scope refusals, and refused quiesce/release through the generic
  retention surface.
- 14 checked Lean theorems for the authority model with ten semantic mutation
  controls and two proof-policy controls. The retention-ledger TLA/TLAPS model
  re-passes on the fenced source (7 theorems / 55 obligations, three models,
  six counterexamples, three proof rejections), and the data-range,
  group-activation, group-control, group-preparation and routed-client Lean
  models are rechecked against the exact working tree.
- One accepted four-process e2e campaign (`e2e-third`): credential/root/
  follower refusals, production fourth-store admission and join, the
  destination bound to its current exact incarnation, idempotent confirmation,
  image owners bound once with second-image and foreign-scope refusals, exact
  confirmations and byte-identical owner observations after metadata-leader
  death and after all-process restart. Two failed launches are retained; both
  repairs were e2e-runner-only.

[evidence.tar.gz](evidence.tar.gz) contains **743 readback-verified files**,
763410 compressed bytes and 4518491 decoded bytes. SHA-256:
`4004e9420de2b97b0d57803495ae24797f500fa89ac737cf3126ea414a570f53`.
[validation.json](validation.json) inventories all members. A separate
extraction re-verified every member and the embedded local-validation, e2e and
proof receipts; [portable-readback.json](portable-readback.json) records that
result.

Tested source is based on `de3e6a2e4c384839cbdaec009150d02c700091cf` with the
exact current Rust/Cargo/proto source retained under `source/`. The root
bootstrap credential file, TLC state scratch and GitHub issue snapshots are
excluded.

Nothing here transfers data, captures a source cut, records or consumes
destination-install evidence, quiesces or releases a pin, starts a peer,
attaches a learner, changes membership or reclaims logs/objects. This
increment ran no Chaos Mesh campaign; the e2e uses real process kills only.
No new QPS or measured scaling exists; the independent 3/6/9-host benchmark
contract in [HORIZONTAL-SCALING-PLAN.md](../HORIZONTAL-SCALING-PLAN.md)
remains the acceptance criterion. No hosted CI was dispatched.
