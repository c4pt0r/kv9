# Admission-floor validation packet

See [the floor, the typed refusal and the client whitelist
fix](../ADMISSION-FLOOR.md).

- 947 passing workspace tests/doctests in the serial qualifying run
  (including new admission unit tests: flood-typed refusal at the
  floor with metadata admitting to the full limit, release reopening
  the shared pool, configuration refusal at/above the limit, env
  parsing), 35 ignored, zero failures; strict Clippy/formatting pass;
  8 real-MinIO focused installer tests.
- Fourteen checked Lean theorems for the admission-floor model with
  five semantic mutation controls and two proof-policy controls;
  twenty-nine sibling Lean models and the retention TLA/TLAPS model
  re-accepted against the exact working tree.
- Two accepted five-process e2e runs (`e2e-fifth` covers the exact
  final source): a 40-writer raw flood against a deliberately small
  pool (12 total requests, 8 reserved) for a 20-second window —
  **20,867 typed `metadata_floor` refusals**, **19/19 idempotent
  metadata probes succeeded throughout**, metadata classes counted
  zero floor refusals, and the acked flood writes read back. Three
  retained failures, each a real finding: the probe gate first
  conflated admission liveness with raft-apply latency under the
  flood's disk load (idempotent bounded retries now separate them,
  with admission refusals still required to be zero); the first pool
  size never saturated under subprocess-diluted concurrency; and the
  typed refusal was INVISIBLE to clients until `metadata_floor`
  joined the RPC clients' refusal-label whitelist — without that fix
  the flood saw generic unconfirmed errors instead of the typed
  backoff signal.

[evidence.tar.gz](evidence.tar.gz) contains **1208 readback-verified
files**, 5090361 compressed bytes and 56847166 decoded bytes. SHA-256:
`bd87505576504e181ebf28436abcc8053369965a53faf7d395a4b44be7fad9e2`.
[validation.json](validation.json) inventories all members. A separate
extraction re-verified every member and every receipt;
[portable-readback.json](portable-readback.json) records that result.

Tested source is based on `f027ae6` with the exact current
Rust/Cargo/proto source retained under `source/`.

The floor is DEFAULT ON (`KV9_PUBLIC_METADATA_RESERVED=8`; 0
disables). Nothing here claims byte-level floors, cross-layer
backpressure budgets, per-tenant fairness, performance improvements,
or scaling. No Chaos Mesh campaign ran. The 3/6/9-host contract in
[HORIZONTAL-SCALING-PLAN.md](../HORIZONTAL-SCALING-PLAN.md) remains
the scaling acceptance path. No hosted CI was dispatched.
