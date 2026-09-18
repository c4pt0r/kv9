# Auto-split validation packet

See [implementation, trigger semantics and known limits](../AUTO-SPLIT.md).

- 931 passing workspace tests/doctests in the serial qualifying run, 35
  ignored, zero failures; strict Clippy/formatting pass; 8 real-MinIO
  focused installer tests.
- Eleven checked Lean theorems for the auto-split model with five
  semantic mutation controls and two proof-policy controls; twenty-three
  sibling Lean models and the retention TLA/TLAPS model re-accepted
  against the exact working tree.
- Two accepted five-process e2e runs (`e2e-fifth` covers the exact final
  source; `e2e-fourth` first ran green) and three retained failures,
  each a real finding: the disk metric missed WAL segmentation (the
  active file is 117 bytes; data lives in segments); the fill helper
  pinned the parent group's leader, which a mid-fill split retires; and
  the CASCADE experiment — a threshold small enough for children to
  re-trigger produced two split levels and a second-level parent stuck
  sealed-but-unpublished, permanently fencing its slice. Publish errors
  are no longer silently swallowed; cascade correctness is explicitly
  out of the qualified scope and recorded as the open edge.
- The accepted runs: 160 sustained writes with ZERO manual verbs → the
  whole committed pipeline self-drives (trigger, children, intent, seal,
  population, publication) → the parent retires and two children serve →
  every sampled written key survives through public routing → the split
  world survives a full-cluster restart.

[evidence.tar.gz](evidence.tar.gz) contains **2669 readback-verified
files**, 1220123 compressed bytes and 7921112 decoded bytes. SHA-256:
`ca6ee2499c1ed3e653d703955a5692d60bf57e33b4c325efe6acd69ab376538a`.
[validation.json](validation.json) inventories all members. A separate
extraction re-verified every member and the e2e/proof/retention receipts;
[portable-readback.json](portable-readback.json) records that result.

Tested source is based on `aaeba7d` with the exact current
Rust/Cargo/proto source retained under `source/`.

Nothing here qualifies cascade splits, load-based triggers, richer
hysteresis, physical reclamation, or scaling claims. This increment ran
no Chaos Mesh campaign. The independent 3/6/9-host benchmark contract in
[HORIZONTAL-SCALING-PLAN.md](../HORIZONTAL-SCALING-PLAN.md) remains the
acceptance criterion for effective horizontal scaling. No hosted CI was
dispatched.
