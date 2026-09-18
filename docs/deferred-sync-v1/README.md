# Deferred-sync validation packet

See [the measurement chain, the mechanism and the accepted
targets](../DEFERRED-SYNC.md). This increment took the lever the
previous one's published failure pointed at — and met every predeclared
target.

- 945 passing workspace tests/doctests in the serial qualifying run
  (including new WAL unit tests: the deferral window with sync_now
  idempotence, and torn-tail truncation with summary regression), 35
  ignored, zero failures; strict Clippy/formatting pass; 8 real-MinIO
  focused installer tests.
- Fourteen checked Lean theorems for the deferred-sync model with five
  semantic mutation controls and two proof-policy controls;
  twenty-eight sibling Lean models and the retention TLA/TLAPS model
  re-accepted against the exact working tree.
- The accepted median-of-3 benchmark (`defer-bench-eighth`): **T1
  throughput 2.13×** (predeclared ≥1.3×; every deferred repeat beat
  every strict repeat), **T2 data-group engine syncs per op 10.2×
  reduced** (predeclared ≥1.5×), **T3 C=1 p99 overhead 0ms**
  (predeclared ≤5ms). Seven earlier bench runs and both measurement
  runs are retained — including `defer-bench-fourth`, whose published
  T3 FAILURE (+939ms: unsynced dirty pages entangling other files'
  fsyncs in one journal commit) produced the 100ms age bound, and the
  two fixture errors (metrics attributing engine syncs to the catalog
  engine; the legacy create-keyspace path bypassing data groups
  entirely) whose publication also corrects the record on the previous
  increment's benchmark.
- Two accepted crash e2e runs (`e2e-second` covers the exact final
  source): concurrent acked writes under active deferral, SIGKILL of
  every voter mid-load TWICE with no shutdown sync, and after each
  restart every acknowledged write (373 in the final run) reads back
  through public routing via raft-log replay over the truncated engine
  tail, with the cluster taking new writes afterward.

[evidence.tar.gz](evidence.tar.gz) contains **3394 readback-verified
files**, 15292912 compressed bytes and 185283889 decoded bytes.
SHA-256:
`52e07d700b3f2d810021d3da57731a6f9cab13e95c9a4e0baa7233fea7d06b16`.
[validation.json](validation.json) inventories all members. A separate
extraction re-verified every member and every receipt — including that
defer-bench-fourth's failure is still the failure it was;
[portable-readback.json](portable-readback.json) records that result.

Tested source is based on `e5a675a` with the exact current
Rust/Cargo/proto source retained under `source/`.

The feature ships DEFAULT OFF (`KV9_DATA_SYNC_DEFER_BYTES=0`). Nothing
here claims release-profile or multi-host numbers (debug profile,
single host, one data group), backpressure budgets, the C04 dual-WAL
decision, per-tenant fairness, bounded recovery time, or scaling. No
Chaos Mesh campaign ran. The 3/6/9-host contract in
[HORIZONTAL-SCALING-PLAN.md](../HORIZONTAL-SCALING-PLAN.md) remains the
scaling acceptance path. No hosted CI was dispatched.
