# Parallel stream point-read CPU profiles

Two instrumented five-second runs against accepted server
`f2c4e856d03f75ac6e4718f0aac7f4e7a1cf3d02` completed with the unchanged
`03c1c77` native client. Root runtime session 95599 and decoding session 74283
exited 0; the unchanged final readback also exited 0. These CPU samples guide
optimization and are not new throughput or latency acceptance results.

The workload remains 64 closed-loop workers, 4,096 keys plus a sentinel,
128-byte values, batch size one, 128 warmup calls, a 1,500 ms request deadline
and ten-million-call cap. Point GET and BatchGet(1) use ordinary streaming gRPC
and normal three-voter Raft read barriers/WAL sync calls on volatile tmpfs.
Clients use CPUs 0–1; all voters share 2–5; profiler/helpers use 6–15,22–31.
No build, test or Chaos work overlapped sampling. This is a shared host without
exclusive isolation or background-container CPU changes.

Each perf recording attaches only to the three owned voter PIDs, uses cpu-clock
at 199 Hz with dwarf,16384 stacks, and is bounded to 20 seconds and 128 MiB.
Both recorded zero lost samples. Clock anchors select only the five-second
measurement interval, excluding one millisecond at each edge. All 32 interval
bins contain aggregate samples. Setup, warmup and drain samples are excluded.

## Exclusive sampled leaf categories

Each sample is classified once by the unchanged ordered symbol rules. Generic
allocation/copy/comparison rules precede metadata names. These percentages are
on-CPU sample fractions, not end-to-end latency fractions or predicted gains.

| Category | Single GET | BatchGet(1) |
|---|---:|---:|
| allocation_copy_compare | 22.83% | 22.88% |
| scheduler_synchronization | 14.95% | 14.91% |
| rpc_framing_serialization_buffers | 14.36% | 13.93% |
| metadata_lookup | 2.83% | 3.43% |
| raft_consensus_driver | 2.01% | 1.63% |
| engine_storage | 2.77% | 2.22% |
| kernel_network | 6.55% | 7.68% |
| kernel_scheduler_futex | 1.98% | 2.26% |
| kernel_generic_spinlock | 3.75% | 3.56% |
| kernel_other | 5.70% | 5.39% |
| unknown_or_unsymbolized | 3.89% | 3.82% |
| other_symbols | 18.38% | 18.27% |

Allocation/copy/comparison remains the largest named leaf group, approximately
23% in both APIs. Recovered leaves include malloc/free, chunk management, memory
copying and comparisons. This motivates testing the Linux binary's Rust global
allocator independently, followed by eliminating unnecessary allocations and
copies. The grouping includes work a different allocator cannot remove.

Inclusive patterns overlap and cannot be added: allocation/copy/comparison
appears in 28.43% / 28.24% of recovered stacks, scheduling/synchronization in
21.71% / 22.85%, and RPC/framing/buffers in 20.75% / 20.43%. Optimized frames and
async boundaries limit attribution. Low on-CPU Raft shares do not bound time
waiting for a quorum.

## Resource observations and identity

| Observation | Single GET | BatchGet(1) |
|---|---:|---:|
| Selected samples | 3,036 | 3,059 |
| All recorded samples | 3,350 | 3,368 |
| Successful measured calls | 1,394,341 | 1,372,049 |
| Raw perf bytes | 56,113,264 | 56,421,760 |
| client core equivalents | 0.905 | 0.951 |
| voter-1 core equivalents | 0.375 | 0.379 |
| voter-2 core equivalents | 2.442 | 2.456 |
| voter-3 core equivalents | 0.358 | 0.362 |
| All voter core equivalents | 3.175 | 3.197 |

CPU core equivalents use retained first/last in-window process counters over
approximately 4.94 seconds, with PID/start and per-thread placement checks.
Leader CPU consumption is distributed across its runtime workers; this does
not establish a complete hardware utilization limit or exclude serial waits.

Both complete reports passed the original strict validator. All measured
outcomes were successful, with attempts equal to logical calls. Fresh drains,
ordinary listener identities, complete 4,097-value nonce-zero readbacks, source
and default-feature executable bindings, retained tmpfs files and process
cleanup passed. All eight client/voter lifetimes and four sudo/perf lifetimes
exited. The current executable's own symbols were decoded; no offsets from
older binaries were reused.

The first setup at `/tmp/kv9-parallel-stream-profile-first` failed before client
or perf launch: the newer six-arm driver generated a Redis v2 reference config
for the older profile's paired validator. All three started voter lifetimes
were cleaned. That original failure is retained. Attempt two copies the
original profile driver with only three candidate revision/build pins changed;
the profile changes only its label and paths/pins. Original workload schemas,
decoder and readback predicates are unchanged. Offline paired-config and
source/build preflight passed before the corrected launch.

Raw artifacts: `/tmp/kv9-parallel-stream-profile-attempt2`.

| Artifact | SHA-256 |
|---|---|
| preparation.json | `970895088c8970c45d3eada7a2865968901a6b80763c77b08955c88f7bd70439` |
| protocol.json | `af22c1780ee9e6e772fbcc7857ce35600edffe2c62103e0abe7c24b72e754e02` |
| summary.json | `18994d856c7aa8efc05714a300a05df909acfacfb9dc91a3775640417ca1b8bf` |
| analysis-summary.json | `7a0999e6905350ed589f3c024c4cdae46e25adc0dd346a8e6368022fdb6f4f5d` |
| readback-summary.json | `5e2458ab709cd9a6b192373fae4035f5ca09a51a07b91000c6e0b56788c35e84` |
| point_get/perf.data | `3229715b870880c741658969e7d7b0d3b7cc5b37601c83279d35af074909c8b7` |
| batch_get/perf.data | `66cd5c269d440494d219a4a540712efb83e37126eecbd0281f5ff91e5087065a` |

No hosted CI ran. The [accepted performance comparison](PARALLEL-STREAM-GET-PERFORMANCE.md) remains the throughput/latency reference.
