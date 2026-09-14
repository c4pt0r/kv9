# FNV writer: complete matched write evidence

Measured 2026-09-14 UTC. All 8 smoke and 16 timed cohorts pass independent
acceptance: 7,283,648 successful one-attempt calls, 65,259,587 items, zero
errors, unknown writes or drops. Loaded batch throughput improves 4.131%, but
pooled p99 worsens to 9.830–9.961 ms from 7.602–7.668 ms. Loaded point changes
+0.351% with an order reversal. Keep CRC main selected; FNV `12f44d3` remains
experimental.

See [pooled results](REPORT.md), [every repetition](PER-REPEAT.md),
[comparisons](COMPARISONS.md) and [machine-readable accounting](summary.json).
The matched CRC control is `bd42e60`, the candidate is `12f44d3`, and the fixed
native v3 client is `0be806d`. Three voters use 128-byte values, concurrency
1/64, point Put/BatchPut64 and two opposite ten-second orders on shared-host
loopback with volatile tmpfs WAL. Quorum, sync, apply and response fences remain
enabled. Redis was not rerun. This is not a physical-disk, power-loss,
cross-host or equal-durability comparison.

The 19-part archive retains 2,472 original metadata files / 319,208,049 decoded
bytes in 38,598,684 compressed bytes. It includes controls, exact preparation,
source/build bindings, complete cohort reports, resource samples, retention
catalogs, independent audits, original failure records, storage-policy review
and exact dev-cache cleanup evidence. All 74,608,085,855 original logical WAL
bytes were separately decoded by the runtime audit. Payloads and executables
remain local with [explicit references](omitted-payload-references.json).
This portable verifier checks metadata bytes without re-executing the runtime
audit or decoding external WAL.

The first publisher refused before compression after counting synthetic control
catalogs together with runtime catalogs. The corrected selector preserves the
synthetic evidence while applying the unchanged 24-runtime-catalog requirement
only to exact runtime roots. Both versions and the original failure are retained.
No database cohort or independent acceptance audit was repeated.

Package `13753/8c9728/0` and independent verification `d4fbb5/0` pass.
Verify without extraction:

```sh
python3 -B verify.py --inventory-sha256 8ab64f544ffa7d2e6d72f2ef5dfbc0cef656ee7965d910d4721b1e7ddb7ea6b0
```

All validation is local. No hosted CI was dispatched and no original industrial
work-package checkbox closes.
