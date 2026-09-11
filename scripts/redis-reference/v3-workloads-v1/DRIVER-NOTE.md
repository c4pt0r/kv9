# V3 workload screen preparation

This is a new protocol, `kv9-v3-workloads-c1-c64-v1`. The driver derives from the
retained WorkSignal driver by changing role pins, the workload inventory and v3
API selection, replacing read-only value checks with the accepted mutable-value
scope, and adding the explicitly requested storage observations/guards. The
full diff and input hashes are retained. No runtime has been launched here.

The 36 forward cells iterate concurrency 1 then 64; point1 and batch64, each at
0%, 50%, 100% reads; old 5ee, new 57ff, then Redis. Repetition 1 reverses this
entire list. The shared run ID remains `p{repeat}{concurrency:04d}` so paired
targets retain the same six-byte prefix and 23-byte keys. Arm names include
workload and read percentage; grouping must also include concurrency/repeat.
The 36-case smoke uses 2 seconds; the 72 timed cohorts use 10 seconds.

Both native and Redis clients are clean 0be806d default release artifacts. Point
traffic selects actual GET/PUT and GET/SET; batch64 selects native batch APIs
and MGET/MSET. Setup and verification continue using batch APIs. The declared
client retry limit remains six. Every valid recorded outcome stays present;
`population_observations` separately exposes whether each phase is all-success
and has exactly one actual attempt per call. A valid failure-bearing recording
does not become an all-success comparison by deleting calls or rerunning it.

The existing separate per-operation/outcome histogram summaries remain. The
independent auditor combines raw buckets and integer sums for whole-call
distributions. Batch latency is never divided by its 64 items, and percentiles
are never averaged. Empty operation populations retain null latency summaries.

The complete paginated native scan and independent Redis MGET verify 4096 keys
plus the immutable sentinel. The retained independent mixer validates bytes;
additional checks validate configured nonce bounds and deterministic write-key
membership. This cannot prove exact issuance, the final concurrent write order,
or full linearizability: allocation can precede a second cutoff check. Pure
read cases still require all nonces zero, while write cases require changes.

All existing resource coverage, source/executable, PID/start/boot, listener,
fresh drain, tmpfs hash-retention and owned cleanup checks remain. The seven
finite driver tests include exact independent inventory/configuration controls,
unchanged lifecycle/scan function comparisons, failure preservation, mutable
value controls and storage-boundary rejection. No Cargo or runtime execution
is part of these checks.

Each cohort requires at least 32 GiB available tmpfs and 96 GiB available
retention-disk space before launch. Read-only `statvfs` observations accompany
each client resource observation for every target. Falling below 16 GiB tmpfs
or 64 GiB disk stops the cohort as incomplete and preserves the failure. A
filesystem observation error also fails rather than being swallowed by the
resource sampler's tolerated process-exit exceptions. The inherited 32 GiB
tmpfs startup and 8 GiB endpoint guards remain unchanged. Disk retention still
verifies both copies before removing only the owned tmpfs directory.

At preparation, available space is recorded in `driver-preparation.json`.
The configured worst case is too large to establish a sufficient space bound:
10 million batch64 writes contain 96.64 GB of key/value payload per replica,
about 290 GB across three replicas before overhead. Retained cohorts also
accumulate on disk. The complete 2-second smoke should inform whether a
10-second release is operationally reasonable; growth extrapolation is not a
hard bound. Neither caps nor workload settings may be changed after observing
results. All timing, wrapper restoration and final freeze remain parent-owned.
