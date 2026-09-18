# Storage-reclamation validation packet

See [design, authority gates and limits](../STORAGE-RECLAMATION.md).

- 933 passing workspace tests/doctests in the serial qualifying run
  (including the new unit test driving active-refusal → retire →
  reclaim → idempotent confirm → crash-resume at discovery →
  never-re-prepares), 35 ignored, zero failures; strict
  Clippy/formatting pass; 8 real-MinIO focused installer tests.
- Thirteen checked Lean theorems for the storage-reclamation model with
  five semantic mutation controls and two proof-policy controls;
  twenty-five sibling Lean models and the retention TLA/TLAPS model
  re-accepted against the exact working tree.
- Two accepted zero-verb e2e runs (`e2e-second` covers the exact final
  source; `e2e-first` first ran green pre-formatting): 860 sustained
  cascade writes all landing and reading back, then with
  `KV9_RECLAIM_RETIRED=1` EVERY retired group on all three nodes
  reclaims — 29 regions per node, 87 payload directories verified
  deleted on disk with their fence records retained — the leaves keep
  serving, and a full-cluster restart holds every fence and serves
  every key. The portable readback re-verified 29 reclaimed group
  directories INSIDE the retained node state: record present, payload
  absent.

[evidence.tar.gz](evidence.tar.gz) contains **10164 readback-verified
files**, 1728191 compressed bytes and 11989708 decoded bytes. SHA-256:
`1ef5fc01af9776e5470036c6275adc499a0b9485adec7ad4304ee90035f86adb`.
[validation.json](validation.json) inventories all members. A separate
extraction re-verified every member and the e2e/proof/retention
receipts; [portable-readback.json](portable-readback.json) records that
result.

Tested source is based on `6b9964b` with the exact current
Rust/Cargo/proto source retained under `source/`.

Nothing here reclaims object-store data (retention-ledger image pins
remain the object-store authority), enables reclamation by default,
bounds cascade depth, or makes scaling claims. This increment ran no
Chaos Mesh campaign. The independent 3/6/9-host benchmark contract in
[HORIZONTAL-SCALING-PLAN.md](../HORIZONTAL-SCALING-PLAN.md) remains the
acceptance criterion for effective horizontal scaling. No hosted CI was
dispatched.
