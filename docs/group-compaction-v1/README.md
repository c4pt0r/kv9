# Group-compaction validation packet

See [the mechanism, the configuration-recovery fix and the
limits](../GROUP-COMPACTION.md).

- 950 passing workspace tests/doctests in the serial qualifying run
  (including new compaction catalog tests, the rewritten
  configuration-recovery test — a compacted base resolves its
  configuration; a cut below refuses — and four updated snapshot-install
  tests reflecting the widened contract), 35 ignored, zero failures;
  strict Clippy/formatting pass; 8 real-MinIO focused installer tests.
- Thirteen checked Lean theorems for the group-compaction model with
  five semantic mutation controls and two proof-policy controls;
  thirty-one sibling Lean models and the retention TLA/TLAPS model
  re-accepted against the exact working tree.
- Two accepted five-process e2e runs (`e2e-tenth` covers the exact final
  source): two write bursts, each followed by a committed kind-110 floor
  whose execution advances the group leader's `log_first_index` past the
  floor; the SECOND round exercises the configuration-recovery fix; a
  stale floor refuses; a full-cluster restart recovers on the compacted
  logs with every sampled key serving and the group still writing.
- The retained failures are load-bearing findings: `e2e-first` proved
  the raft log file is append-only (the byte-size metric never shrinks —
  the compaction signal is the `log_first_index` advance); the
  `e2e-second..eighth` chain was chasing a STALE BINARY (a leftover
  brace broke kv9-raft compilation, so the runner reused the old
  binary), which finally surfaced the real defect: the raft
  configuration lookup refused ALL compacted logs, so a second
  compaction floor failed with "no committed configuration" — fixed in
  `configuration_at_committed` to inherit the durable base configuration.

[evidence.tar.gz](evidence.tar.gz) contains **1473 readback-verified
files**, 1445173 compressed bytes and 9925724 decoded bytes. SHA-256:
`eef90b5602597acad5b5e0dc16fb91c1f7b446e41f3061fcf5ee00555e600526`.
[validation.json](validation.json) inventories all members. A separate
extraction re-verified every member and every receipt;
[portable-readback.json](portable-readback.json) records that result.

Tested source is based on `f47cfde` with the exact current
Rust/Cargo/proto source retained under `source/`.

v1 compacts at the group LEADER only (follower log bounding is the
documented open edge). Nothing here selects floors automatically,
bounds absolute log size, reclaims compacted records from the
append-only log file physically, or makes performance claims. No Chaos
Mesh campaign ran. The 3/6/9-host contract in
[HORIZONTAL-SCALING-PLAN.md](../HORIZONTAL-SCALING-PLAN.md) remains the
scaling acceptance path. No hosted CI was dispatched.
