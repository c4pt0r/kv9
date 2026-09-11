# CRC write CPU diagnostic with active prefix

Both exact CRC profiles passed the unchanged analyzer on their first authorized runtime and decoder attempts. Runtime session 76156 exited 0; decoder session 21261 exited 0. Original rejected batch profile remains unchanged.

| API | Selected samples | Measured successful calls | WAL append leaf samples | frame_crc leaf samples | inspect_applied leaf samples | memcpy leaf samples |
|---|---:|---:|---:|---:|---:|---:|
| point_put | 3286 | 582235 | 1 (0.03%) | 83 (2.53%) | 180 (5.48%) | 207 (6.30%) |
| batch_put | 3103 | 63324 | 0 (0.00%) | 561 (18.08%) | 27 (0.87%) | 210 (6.77%) |

Each profile retained 128 successful absent-key RawGet priming calls, with zero retries/errors and all CLI children exited. The unchanged analyzer accepted nominal prefix/tail containment, all 32 aggregate bins, edge coverage, bounded clock anchors, zero lost samples, report identity and retained data. All 8 fixture, 4 recorded profiler and 256 prefix CLI lifetimes are absent; runtime/analyzer supervisors are also absent.

The priming changes cache and CPU state. These single instrumented windows support CPU attribution only; their call counts are not throughput acceptance. Leaf buckets are exclusive according to the unchanged classification; inclusive recovered stacks overlap and cannot be summed. The retained per-profile summaries contain complete category and stack counts.

Source: CRC ca0002c7f8e9ee6f595efcc9f4151085ccce87cb; fixed measurement client 0be806d9671e2c50701a64aa7889c8859b7648ba. Shared host, volatile tmpfs WAL, unchanged quorum/sync behavior; no durability or full-history proof claim.
