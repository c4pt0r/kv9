# Capacity recovered for write acceptance

Four completed local capacity stages allowed the upper-bound candidate's
[release and ordinary recovery](WRITE-RECEIPT-UPPER-BOUND-RUNTIME.md) to proceed.
The cleanup preserves source snapshots, protected binaries and original test
evidence. Every retained-data cohort reaches COLD only after exact restoration
and complete corrected readback.

| Stage | Verified result |
| --- | --- |
| Initial generated-cache cleanup | 6,919 exact generated files removed; observed global available space increased by 3,846,586,368 bytes. |
| Retention cohorts 040–055 | All 16 complete/COLD; conservative net allocated recovery of 7,291,015,168 bytes. |
| Post-development cache cleanup | 6,780 exact generated files removed; observed global available space increased by 6,538,379,264 bytes. |
| Retention cohorts 056–060 | All five complete/COLD; conservative net allocated recovery of 1,602,387,968 bytes. No 061 started. |

The two retention tranches account for **8,893,403,136 bytes** of conservative
net recovery. Their original whole-cohort checks cover 1,816 objects and
41,196,202,717 logical bytes, with 8,926 successful fresh codec receipts.
Logical bytes checked are not reclaimed bytes. Cache free-space changes are
host observations and are kept separate from retention allocation accounting.

Development builds overlapped the first retention tranche. Its accounted
global available space rose by 926,867,456 bytes while retention recovered
7,291,015,168 allocated bytes. The difference includes concurrent builds,
reporting, other host activity and timing boundaries; it is not a measured
build-only cost.

The final controller observed **27,046,117,376 available bytes** after cohort
060, exceeding its release-plus-margin target. This is a historical observation,
not reserved capacity. Default release, diagnostic release and ordinary
recovery subsequently passed their own fresh capacity checks. Future Chaos
and performance phases still require fresh checks and their full resource
budgets.

[The original report and portable metadata](write-capacity-completion-v1/README.md)
preserve actual terminals, selected source inventories, exact cleanup records,
per-cohort acceptance and prior failures. The 1,709,578-byte archive has 1,213
members and passes independent complete gzip/member readback. Its SHA-256 is
`2c696f7eee38f9da62e05c4e822bd23519bbaac9b8551f75f110269f33bf7d45`.
Repository publication copied all 16 report files byte for byte (`9734e3/0`).
It did not replay any workload, payload restoration or successful control.
