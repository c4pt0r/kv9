# FNV write comparison: capacity work

The exact FNV writer has passed source/proof, ordinary recovery and actual
21-window Chaos Mesh acceptance. The capacity work below records the original
policy's unresolved gap. A subsequent, separately versioned
[storage policy and cleanup](WRITE-FNV-STORAGE-POLICY.md) now pass 71 local
controls. The [complete write comparison](WRITE-FNV-WRITER-PERFORMANCE.md) now
passes: eight smokes, sixteen timed cohorts and independent audit. Batch
throughput improves 4.131%, but pooled p99 worsens; CRC main stays selected.

The previous screen retained 52,317,179,904 allocated bytes. Keeping the existing
96 GiB floor, 16 GiB restoration reserve and 1 GiB margin gives an empirical
planning requirement of 173,650,006,016 free bytes. This is a previous-run sizing
reference, not a proven upper bound on the new candidate's allocation.

## Nondestructive compression pilot

Two exact historical frame/CRC smoke objects were decoded, compressed with
Zstandard level 19 and the same 64 MiB window limit, decoded again, then
recompressed with the original level-3 invocation. Both the original raw SHA-256
and the original compressed SHA-256 match. All original objects and catalogs
remain unchanged. The codec executable and original catalog entries are pinned.

| Object | Original raw bytes | Original encoded bytes | Level-19 bytes | Encoded reduction |
| --- | ---: | ---: | ---: | ---: |
| Point Raft log | 71,102,954 | 42,586,678 | 41,826,311 | 1.785% |
| BatchPut engine segment | 16,772,164 | 11,865,357 | 11,676,863 | 1.589% |
| Total | 87,875,118 | 54,452,035 | 53,503,174 | 948,861 bytes |

All eight codec processes exit successfully and are reaped. Actual session
`32057` ends at `fb6f01/0`; a separate post-terminal size/SHA readback verifies
all eight generated files. The experiment takes 19.816 seconds. Its final
generated allocation is 283,824,128 bytes, below the 512 MiB allowance; the
minimum sampled free space is 111,495,495,680 bytes, above the 96 GiB floor.
This is an environment experiment, not a database or general codec benchmark.

These two objects do not establish a campaign-wide savings bound. Their small
reduction does not justify migrating the full history pool. No source object
was evicted and no new retention representation was adopted.

The catalog review also establishes that the three reviewed CRC/frame campaigns
already use compressed retention: 184,225,595,392 allocated bytes represent
262,909,859,933 logical original bytes. The earlier cold transaction has no
remaining staging duplicates. Declared identical compressed objects offer only
3,977,687,040 bytes before overhead and fresh verification, which cannot cover
the current gap. Historical savings cannot be counted again.

## Cache cleanup and next work

The additional cache review found ten historical KV9 control build directories
outside the previous target inventory. Combining their prospective intermediates
with the earlier selection gave 16,428,072,960 bytes. Exact Cargo-unit mapping
excluded 4,925,300,736 bytes with missing, ambiguous or unsupported bindings.

The remaining **5,985 inactive single-link, non-executable files** were removed
under existing BuildCache/Cargo locks after fresh identity, process-reference
and inode-alias checks. First, all **2,912 affected fingerprint directories and
12 native build completion markers** were moved to distinct retained locations,
verified and directory-synchronized. Future builds cannot reuse those success
markers after their compiled outputs disappear. Preparation failures remain
recorded; the actual retirement ran once.

Actual session `12594` ends at `2e3d45/0` after 101.726 seconds. All selected paths
are absent, retired metadata verifies, and all lock handles close. Selected
allocation was 11,502,772,224 bytes; filesystem free space increases by
**11,491,069,952 bytes**, from 111,455,981,568 to **122,947,051,520 bytes**.
After charging retained metadata and the result allowance, the conservative
accounting estimate is 11,434,840,064 bytes. Filesystem observations and accounting
estimates are distinct. All six protected server/client executables retain their
original sizes and SHA-256 values. Named locks, source, offline dependency inputs
and every original test payload are preserved.

At this cleanup checkpoint, the original empirical full-screen reservation
still exceeded available space by **50,702,954,496 bytes**, so that preparation
remained unreleased. The subsequent policy review is linked above; it does not
retroactively change this campaign's storage arithmetic or measured evidence.

Next qualify enough storage for the complete eight-smoke/sixteen-timed-cohort
comparison. Keep its workloads, duration, quorum/sync/response fences, retention
coverage and capacity checks unchanged. While that prerequisite is unresolved,
the separate [client-link/reset/quorum-loss campaign](WRITE-FNV-WRITER-LINK-CHAOS.md)
has passed all eleven windows on the same already qualified binary, with 2,126
complete operations and independent acceptance. It uses much smaller retention.
Neither activity substitutes for the missing database performance measurements.
CI remains local.

The [portable original metadata and audit evidence](https://github.com/c4pt0r/kv9/blob/30f1fd2506f76fb59f2bfa0877a5a968a2e6fd2d/docs/fnv-writer-capacity-v1/README.md)
preserves 11,763 members / 68,084,499 decoded bytes in four parts totaling
6,961,235 compressed bytes. Packaging completes at `57906/0490fc/0`;
independent byte verification completes at `309aaa/0`. Original selector and
pre-execution terminal-schema failures remain included. Raw/compressed WAL and
executable payloads remain local with hash references; this is a reporting
bundle, not a replacement payload backup.
