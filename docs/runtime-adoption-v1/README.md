# Runtime-adoption validation packet

See [implementation, adoption semantics and remaining scope](../RUNTIME-ADOPTION.md).

- 919 passing workspace tests/doctests in the serial qualifying run, 35
  ignored, zero failures; strict Clippy/formatting pass; 8 real-MinIO
  focused installer tests including the new adoption-fencing case.
- New unit coverage: `RaftPeer::with_installed_storage` admits only the
  exact verified cut (zero/foreign/shifted bases refuse, the ordinary
  constructor still refuses snapshot-backed stores);
  `NodeDriver::with_installed_base` restores the unified position from the
  installed cut and refuses an engine watermark behind it; real-MinIO
  adoption is one-way (installer refuses `runtime-adopted` groups), exact
  across legitimate file growth, and refuses sealed-image or marker tamper.
- Fourteen checked Lean theorems for the runtime-adoption model with eight
  semantic mutation controls and two proof-policy controls; ten sibling
  Lean models and the retention TLA/TLAPS model re-accepted against the
  exact working tree.
- One accepted five-process e2e (`e2e-first`), first attempt, chaining the
  previously qualified flow (committed intent → attach → capture → offline
  install) into runtime: destination restart → reconciled adoption as the
  live replica at the exact installed cut → post-install writes at the
  source leader reach the learner through the retained log tail (MsgAppend
  only; network snapshots stay fenced) → reads at the learner refuse with a
  leader hint → a second restart re-adopts idempotently and resumes tail
  tracking.

[evidence.tar.gz](evidence.tar.gz) contains **580 readback-verified files**,
686234 compressed bytes and 3777068 decoded bytes. SHA-256:
`9047b4c821b2a0e5fc2f56755b63e4b868e0546cd7e6a7f375397c7b1f3e904a`.
[validation.json](validation.json) inventories all members. A separate
extraction re-verified every member and the e2e/proof/retention receipts;
[portable-readback.json](portable-readback.json) records that result.

Tested source is based on `e644176655b2dedc61aef99654f890c7b7b0acc9` with
the exact current Rust/Cargo/proto source retained under `source/`.

Nothing here promotes a voter, serves from the learner, records
destination-install evidence, commits a release decision, truncates the
source log, quiesces or releases a pin, splits, places, or claims scaling
or new QPS. This increment ran no Chaos Mesh campaign. The independent
3/6/9-host benchmark contract in
[HORIZONTAL-SCALING-PLAN.md](../HORIZONTAL-SCALING-PLAN.md) remains the
acceptance criterion for effective horizontal scaling. No hosted CI was
dispatched.
