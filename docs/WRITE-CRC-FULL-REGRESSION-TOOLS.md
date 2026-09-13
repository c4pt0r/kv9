# CRC full regression tools: local controls qualified

The [full regression plan](write-crc-full-regression-plan-v1/README.md) now has
executable driver, auditor, smoke-reader and reporting derivatives. **48 local
controls pass**: 35 runtime-tool controls and 13 reporting controls. No cohort
has run, capacity remains unqualified, and the CRC candidate is not promoted.

The matrix retains selected `11113f6` versus CRC `e748620`, the exact retained
server binaries and native v3 client `0be806d9`. It covers point and batch(64)
APIs, 0/50/100% reads, concurrency 1/64, 24 two-second smokes and 48 ten-second
timed cohorts. Timing uses the full forward order and its complete reverse.
No write-only subset substitutes for this regression gate.

## Driver and acceptance

The accepted write comparison supplies the original driver, auditor, corrected
smoke schema, process isolation and retained-byte machinery. The full-matrix
derivative changes workload constants, counts, paths and bound helper hashes.
Retention and isolation functions are unchanged. Original Raft/sync behavior,
source/binary identities, CPU placement, uncertainty accounting, resource limits,
complete retention and failed-attempt evidence remain required.

The 35 controls comprise the adapted 8 driver, 15 auditor and 5 smoke-schema
tests, plus 7 checks for the changed scope. They exercise exact API/order/count
mapping, inactive-operation rejection, mixed read/write populations without an
exact-half assumption, deterministic final nonce membership, missing/duplicate/
reordered smoke rejection and both retention root allowlists. These are finite
tool controls, not successful workload observations or full operation histories.

## Reporting

The arithmetic core is byte-identical to the qualified full native comparison.
The report retains the accepted per-process CPU calculation and latency-bucket
direction rules. It requires the new complete audit, its exact input inventory,
actual timing session, audit terminal and source/helper hashes. Historical result
data are not pooled into this comparison.

Each result includes separate read/write populations and merged whole-call
histograms. Rates use complete intervals; means use total nanoseconds/counts;
percentiles use merged buckets. Empty populations remain unavailable. Batch
latency is not divided by 64. The 13 controls cover these rules, non-success
accounting, unequal-count pooling, CPU weighting, process identity changes and
incorrect source/role/order/count/report bindings. Reporting records 48 cohort
rows, 24 pooled role/cell rows, 24 per-order comparisons and 12 pooled comparisons
when the future complete input passes acceptance.

## Remaining execution gate

Fresh resource/source/environment checks and sufficient retained-campaign
capacity must precede all 24 smokes and all 48 timed cohorts. The original
capacity scenario required 202.38 GB available, with an 80.88 GB gap at its
historical observation; it is not a current free-space reading or fit guarantee.
Every original preflight/floor, restore reservation, member/cohort/campaign cap
and deadline remains unchanged. No workload or reporting execution is released
by passing these local controls.

[Portable originals](write-crc-full-regression-tools-v1/README.md) retain both
frozen preparations and all five command outputs/terminals. The earlier pending
test fields remain intact; the later successful terminals supersede only those
fields. Real benchmark acceptance, performance results and default promotion
remain open. CI ran locally; no original industrial checklist item closes.
