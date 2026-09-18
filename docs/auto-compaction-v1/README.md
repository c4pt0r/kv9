# Auto-compaction validation packet

See [the self-driving trigger and its limits](../AUTO-COMPACTION.md).

- 950 passing workspace tests/doctests in the serial qualifying run, 35
  ignored, zero failures; strict Clippy/formatting pass; 8 real-MinIO
  focused installer tests.
- Twelve checked Lean theorems for the auto-compaction trigger model
  (execution safety is entirely the group-compaction model's premise)
  with five semantic mutation controls and two proof-policy controls;
  thirty-two sibling Lean models and the retention TLA/TLAPS model
  re-accepted against the exact working tree.
- Two accepted five-process e2e runs (`e2e-second` covers the exact
  final source): 800 sustained writes with ZERO compaction verbs and
  `KV9_AUTO_COMPACT_ENTRIES=64` — the metadata leader auto-proposes a
  committed floor, the gated reconcile advances the group leader's
  `log_first_index` past 1, a second burst auto-advances the floor
  further (strictly increasing), and a full-cluster restart recovers on
  the auto-compacted logs with every sampled key serving.

[evidence.tar.gz](evidence.tar.gz) contains **1078 readback-verified
files**, 1300328 compressed bytes and 7908324 decoded bytes. SHA-256:
`549029c1ddf2edd1dbb9727a98a35ecdbfc4aeb6cfa5d7eb54b8dab3d939679a`.
[validation.json](validation.json) inventories all members. A separate
extraction re-verified every member and every receipt;
[portable-readback.json](portable-readback.json) records that result.

Tested source is based on `55e547d` with the exact current
Rust/Cargo/proto source retained under `source/`.

The trigger is a fixed entry-count threshold and ships DEFAULT OFF
(`KV9_AUTO_COMPACT_ENTRIES=0`). Execution stays leader-only
(follower-side log bounding remains open). Nothing here claims an
age/byte policy, bounded absolute log size, physical reclamation of the
append-only log, or performance. No Chaos Mesh campaign ran. The
3/6/9-host contract in
[HORIZONTAL-SCALING-PLAN.md](../HORIZONTAL-SCALING-PLAN.md) remains the
scaling acceptance path. No hosted CI was dispatched.
