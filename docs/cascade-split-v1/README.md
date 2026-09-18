# Cascade-split validation packet

See [diagnosis, fixes and qualified scope](../CASCADE-SPLIT.md).

- 932 passing workspace tests/doctests in the serial qualifying run
  (including the new regression test
  `a_cascade_publication_never_invalidates_its_grandparent_intent`,
  which fails on the old code with the exact historical error), 35
  ignored, zero failures; strict Clippy/formatting pass; 8 real-MinIO
  focused installer tests.
- Twelve checked Lean theorems for the cascade-split model with five
  semantic mutation controls and two proof-policy controls; twenty-four
  sibling Lean models and the retention TLA/TLAPS model re-accepted
  against the exact working tree.
- The full diagnosis trail, retained: `repro-second` reproduces the
  sealed-unpublished hang from scratch on the pre-fix code (write 114 of
  800 refused everywhere; control status EMPTY — the error blackout);
  six revivals of the preserved auto-split `e2e-third` hang state under
  successive binaries — `revive-fourth`'s logs carry the diagnosed
  refusal (`split parent range is already sealed`, 6742×), `revive-fifth`
  shows both frozen publications committing but retirement starving, and
  `revive-sixth` shows the historical hang state recovering completely
  (parents 100/101/102 retired, all four leaves serving). `repro-first`
  is retained as a real finding about weak acceptance criteria.
- The qualifying run `e2e-first` (verdict `completed`): 860 sustained
  writes with ZERO manual verbs at a cascade-provoking threshold — the
  cascades ran MANY levels (57 regions: 28 published-and-retired
  parents, 29 serving leaves), every write landed, every written key
  read back through public routing, and the cascade world survived a
  full-cluster restart.

[evidence.tar.gz](evidence.tar.gz) contains **12438 readback-verified
files**, 2898193 compressed bytes and 21990590 decoded bytes. SHA-256:
`56103efb5138799f32824b4b4f6f28abb228af82d420b0ba747983248276c0da`.
[validation.json](validation.json) inventories all members. A separate
extraction re-verified every member and the e2e/proof/retention
receipts; [portable-readback.json](portable-readback.json) records that
result.

Tested source is based on `b9847c8` with the exact current
Rust/Cargo/proto source retained under `source/`.

Nothing here qualifies load-based triggers, hysteresis, bounded cascade
depth, physical reclamation, or scaling claims. This increment ran no
Chaos Mesh campaign. The independent 3/6/9-host benchmark contract in
[HORIZONTAL-SCALING-PLAN.md](../HORIZONTAL-SCALING-PLAN.md) remains the
acceptance criterion for effective horizontal scaling. No hosted CI was
dispatched.
