# FNV comparison: separately versioned storage policy

The original FNV performance preparation remains unchanged and unexecuted.
Its 96 GiB retained-filesystem floor makes the empirical full-screen reservation
173,650,006,016 bytes. The storage review traces that floor to operational
guards added before compressed retention existed; it is not derived from Raft,
the decoder's memory requirement or a worst-case completion proof.

A separate policy now passes 71 local environment controls and has been
released for the same complete comparison with an explicit lower host-space
reservation. All eight smoke and sixteen timed cohorts, independent acceptance
and reporting now pass. The [matched results](WRITE-FNV-WRITER-PERFORMANCE.md)
keep CRC main selected: batch throughput improves 4.131%, but the batch tail
worsens. The storage-policy change itself is not a database optimization.

## Policy and remaining protections

| Retained-filesystem check | Existing preparation | New policy |
| --- | ---: | ---: |
| Before each cohort | 96 GiB available | 64 GiB available |
| During measured client execution | 64 GiB available | 64 GiB available |
| Compression, independent decode and restore host floor | 96 GiB available | 48 GiB available |
| Serial cohort restoration reserve | 16 GiB | 16 GiB |
| Planning margin | 1 GiB | 1 GiB |

All tmpfs checks, finite file/cohort/campaign/codec limits, source and binary
identities, CPU placement, call caps and workloads remain. The comparison still
requires eight two-second smokes and sixteen ten-second timed cohorts, both
orders, point Put and BatchPut(64) at c1/c64, complete outcomes and latency
histograms, every original payload, independent decode/readback, fresh drains
and reaped processes. Existing Raft quorum, synchronization, apply and response
fences are unchanged. No codec overlaps measurement.

This is a new operational policy and protocol identity, not a passing result
under the old constants. Producer catalogs, independent reader, driver, auditor
and smoke bindings must agree exactly on it; old and new catalog policies must
not be accepted interchangeably. Historical recordings retain their original
policies, and their performance samples must not be pooled with the new run.

The 48 GiB floor remains additional available host space during retention.
The producer separately reserves its per-member compression requirement;
fresh restoration requires that floor plus actual decoded cohort bytes and
metadata. The largest declared member's reserve is 8,666,480,640 bytes, below
the retained 16 GiB serial restoration allowance. The cohort cap is checked
after writer shutdown and is not a live allocation bound.

## Capacity evidence

The accepted frame/CRC screen's whole resident allocation is 52,317,179,904
bytes. The new empirical planning equation is:

```
resident precedent + max(measured floor, retention floor + restore) + margin
= 52,317,179,904 + max(64 GiB, 48 GiB + 16 GiB) + 1 GiB
= 122,110,398,464 bytes
```

After receipt-source qualification, the existing BuildCache cleaned only the
eight first-party packages' inactive dev artifacts. A fresh privileged process
review found no debug-target references or Cargo/compiler owners. Source files,
third-party dependencies, release artifacts and all retained runtime payloads
remain. All six protected server/client binaries retain their original hashes.
Actual cleanup completes at `4bc58d/0`; observed free space rises by
**6,261,227,520 bytes**, from 120,760,619,008 to **127,021,846,528 bytes**.
This leaves **4,911,448,064 bytes** above the new historical planning equation.

The ext4 snapshot separately reports 99,980,251,136 free bytes excluded from
`f_bavail`. That space is not counted, altered or relied on by the new budget.
All checks continue to use available-space observations. These observations
are not a filesystem quota or a reservation against other writers.

Neither policy proves completion for every permitted output volume. Faster
writes, different compression, metadata allocation or external host activity
can exceed the historical precedent. Fresh checks are mandatory before launch,
after smoke and throughout runtime/retention; already resident smoke must not
be counted twice. A boundary breach preserves the incomplete attempt and its
payloads, and does not authorize shortening, skipping or rerunning cohorts.

## Qualification before execution

The frozen preparation passes all **37 metadata/lifecycle controls and 34
tiny retention controls** at session `4441`, terminal `9adeee/0`. This includes
the original 28 driver/auditor/smoke controls and 31 retention cases, seven new
policy checks, two cleanup checks, and three tiny producer/reader/restore
checks. Threshold equality and one-byte shortfalls, missing/device evidence,
foreign policies and dependency hashes are checked. The tiny real
producer-to-independent-reader-to-fresh-restore path preserves exact original
bytes; no large synthetic codec qualification was repeated for changed limits.
The unchanged reporting arithmetic also passes five controls (`8d0931/0`).

The review found one inherited cleanup bug: a failure writing
`pre-cleanup.json` could skip `fixture.close()`. The new driver catches this
metadata failure separately, still invokes owned shutdown/retention, preserves
workload and cleanup exceptions, and marks the cohort incomplete. The actual
measured body is unchanged. Independent source review and injected ENOSPC/EIO
controls pass. This does not promise shutdown after arbitrary kernel failures
or SIGKILL. Before verified cleanup intent, original scratch and partial
objects remain on failure; after every member is verified and catalogued,
scratch removal can be partial while verified objects preserve the bytes.

Root independently verifies all 90 frozen preparation files. Inventory SHA256
is `2cfdbd6945e46ffff2e85638bbb6e023e18ed3e64b8bf0aba0f2932f55922ca2`;
driver SHA256 is
`48d93eb24d73ba33721efa5c6053d32c703d126c5842471fa50e081794e86708`.
The fresh launch observation (`558906/0`) records **127,012,003,840 available
host bytes**, **66,274,996,224 tmpfs bytes** and no competing compiler, proof,
codec, profiler or database process. Smoke `36365/df736d/0` and independent
readback `d3d06f/0` pass. The fresh post-smoke check records 120,941,125,632
available bytes against the conservative remaining-timing scenario of
116,232,887,908 bytes, without counting resident smoke objects again. Timing
`61012/61e0b7/0`, full audit `89236/321c49/0` and reporting `2fe68b/0` pass.
Final combined allocation is 52,612,304,896 bytes; all 74,608,085,855 logical
WAL bytes are independently decoded. Post-audit availability is 74,389,651,456
bytes, above the maximum fresh restore threshold at that observation. The
48 GiB floor alone is not a restore-readiness claim; an actual restore still
requires its fresh capacity check.

Original local roots are `/tmp/kv9-fnv-writer-performance-budget-v2-preparation-20260914-first`
and `/tmp/kv9-fnv-writer-performance-budget-v2-root-20260914-first`.
[Portable evidence](https://github.com/c4pt0r/kv9/blob/fd2e18bd9471ad4f4e8aab6e3d0cd6f8c145e0ce/docs/fnv-writer-performance-v2/README.md)
includes original preparations, controls, reviews, cleanup and runtime records.
Selected runtime remains CRC main. All work is local; no hosted CI was dispatched.
