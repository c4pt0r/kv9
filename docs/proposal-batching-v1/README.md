# Proposal-batching validation packet

See [the mechanism, the predeclared targets and the honest
result](../PROPOSAL-BATCHING.md). This increment ships a
correctness-qualified, DEFAULT-OFF mechanism and publishes its
benchmark against predeclared targets — **two of three targets FAILED,
and the numbers are published as failed**, exactly as the predeclared
discipline requires. No performance improvement is claimed.

- 943 passing workspace tests/doctests in the serial qualifying run
  (including four new aggregator unit tests: merge order, fence
  segregation, ops/byte caps, taken-batch exclusion, typed shared
  errors), 35 ignored, zero failures; one earlier serial run hit a
  lease-test timing flake (the test passes 3/3 in isolation at ~1s) —
  that log is retained (`workspace-serial-flake-retained.log`); strict
  Clippy/formatting pass; 8 real-MinIO focused installer tests.
- Fifteen checked Lean theorems for the proposal-batching model with
  five semantic mutation controls and two proof-policy controls;
  twenty-seven sibling Lean models and the retention TLA/TLAPS model
  re-accepted against the exact working tree.
- Two accepted zero-shortcut e2e runs (`e2e-second` covers the exact
  final source): 32 concurrent writers all landing with nonzero applied
  receipts under batching; an automatic split fires MID-FILL so the
  epoch fence closes open batches under live load; every key reads
  back; a full-cluster restart recovers.
- The predeclared benchmark (`bench-fourth`, per-trial fresh clusters,
  identical durability both sides): **T1 FAILED** — C=32 batched
  throughput 0.82× unbatched (target ≥1.4×; an earlier informal run
  measured 1.12× — within run-to-run variance); **T2 FAILED** —
  raft-log syncs per op 0.92× (target ≥2.0× reduction; unbatched
  already ran at ~0.44 syncs/op because Ready-persistence group commit
  already amortizes); **T3 PASSED** — 0ms measured C=1 p99 overhead.
  Three earlier bench attempts are retained (a crashed compute step,
  and a drain-verification failure that reproduced until trials got
  per-trial fresh clusters).

[evidence.tar.gz](evidence.tar.gz) contains **1739 readback-verified
files**, 5877168 compressed bytes and 55879816 decoded bytes. SHA-256:
`7576dc58f23f2cfbd3590726068aac2744c5d00c9de7a93fb03318d14d0aa473`.
[validation.json](validation.json) inventories all members. A separate
extraction re-verified every member and every receipt — including that
the bench verdicts really are the failed ones;
[portable-readback.json](portable-readback.json) records that result.

Tested source is based on `e00c1f1` with the exact current
Rust/Cargo/proto source retained under `source/`.

Nothing here claims a throughput or latency improvement (the feature
stays DEFAULT OFF), backpressure budgets, dual-WAL unification,
per-tenant fairness, or scaling. Debug-profile, single-host
measurements only. No Chaos Mesh campaign ran. The 3/6/9-host contract
in [HORIZONTAL-SCALING-PLAN.md](../HORIZONTAL-SCALING-PLAN.md) remains
the scaling acceptance path. No hosted CI was dispatched.
