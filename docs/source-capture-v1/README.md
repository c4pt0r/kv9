# Unified-cut source capture validation packet

See [implementation, cut semantics, proofs and remaining scope](../SOURCE-CAPTURE.md).

- 917 passing workspace tests/doctests in the serial qualifying run, 34
  ignored, zero failures; strict Clippy/formatting pass.
- Five live-path unit tests over a real single-voter driver group: the cut
  binds the engine's durable applied position with its scope and range taken
  from the same frozen view; a configuration committed **past** the cut never
  attaches to the image, appears only after a later data command moves the
  cut beyond it, and then names its own commit position; capture without
  applied ownership refuses.
- One real-MinIO component loop: live capture with a committed learner in
  the configuration → upload → installation of the exact record by the
  **unchanged** joint installer at the learner's prepared store → recovery →
  full value readback. The pre-learner image is refused by the installer's
  existing membership gate — the documented reason learner attach precedes
  runtime installation.
- 11 checked Lean theorems for the capture model with nine semantic mutation
  controls and two proof-policy controls; the eight sibling Lean models and
  the retention TLA/TLAPS model are re-accepted against the exact working
  tree.
- One accepted four-process e2e (`e2e-third`): plan at the group leader,
  bind owners at the metadata leader, capture at the group leader with
  committed pins verified from local applied state before any upload;
  leader-only capture with typed follower refusal, unpinned and uncommitted
  refusals, canonical `KV9RSN01` record with matching digest and exact cut,
  idempotent re-capture of an unchanged cut, and the second-image refusal
  after the cut advances — one image per operation, by construction. Two
  failed e2e launches are retained: both exposed the same real architecture
  defect (capture wrongly required metadata leadership on the group-leader
  node), fixed by the plan/bind/capture split recorded here.

[evidence.tar.gz](evidence.tar.gz) contains **688 readback-verified files**,
729532 compressed bytes and 4337131 decoded bytes. SHA-256:
`fe8ca70c5c920f946132d94a567151bfa4fa1d9fa5290d4717f0aeb4a32e493e`.
[validation.json](validation.json) inventories all members. A separate
extraction re-verified every member, the embedded e2e/proof receipts and the
captured record's magic and digest;
[portable-readback.json](portable-readback.json) records that result.

Tested source is based on `2aefab048fc8ca6e6330b9a806922e3db45b387e` with the
exact current Rust/Cargo/proto source retained under `source/`. The root
bootstrap credential is excluded; MinIO credentials never enter the repo.

Nothing here attaches a learner, changes membership, installs at runtime,
serves, records destination-install evidence, quiesces or releases a pin,
admits a network snapshot, or claims catchup, split, placement, scaling or
new QPS. This increment ran no Chaos Mesh campaign. The independent
3/6/9-host benchmark contract in
[HORIZONTAL-SCALING-PLAN.md](../HORIZONTAL-SCALING-PLAN.md) remains the
acceptance criterion for effective horizontal scaling. No hosted CI was
dispatched.
