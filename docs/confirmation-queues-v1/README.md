# Confirmation queue diagnostic evidence

This compact bundle supports [the diagnostic report](../CONFIRMATION-QUEUE-RESULTS.md)
for source `65302397880f6e580505cff5241b10941479c161`. It contains 299 original
files / 25,342,630 decoded bytes in one 1,232,426-byte archive part. Top-level
readout aliases preserve the original derivation bytes.

[READOUT.md](READOUT.md) reports 974,650 measured successes in one attempt and
999,488 successful client calls / 999,490 attempts across all phases. All 90
stage/kind rows, 630 outcome rows, integer sum/count deltas, histogram buckets,
interval quantiles and endpoint identities remain available. This is an
instrumented two-cell diagnostic, not a new Redis/performance comparison.
Different queues and message kinds are not an additive request partition.

The archive retains original source/build/cache gates, their failed and corrected
attempts, frozen fixture/validator/readout code, contracts, two-cell protocols,
raw client reports and endpoint metrics/status, accepted independent inventories
and all root terminal receipts. Both complete ordinary leader-loss/restart
histories are included: 369 operations, 330 OK and 39 unknown, with five server
and two client lifetimes. Root recording 34360 exits 0 (`16e5d8`), readback exits
0 (`fb8779`), and readout derivation exits 0 (`ca961b`).

The runtime audit's 78 files / 492,189,442 bytes have a different scope from this
publication. Raw WALs, executables and bulky host/container observations remain
local under original hashes. This selection does not support a standalone full
runtime re-audit and does not provide a new Chaos Mesh or implementation proof.

Run `python3 verify.py` here to check archive parts, exact decoded member bytes,
safe paths and top-level aliases. The verifier checks integrity only; it does
not rerun the diagnostic, statistics, correctness tests or independent audit.
No hosted CI was dispatched. The uninstrumented Append-payload candidate is a
separate subsequent experiment, not this bundle's implementation.
