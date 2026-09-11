# Remaining write CPU after the byte-table CRC optimization

The first accepted post-CRC batch diagnostic identifies **engine frame CRC
at 18.079% of selected batch CPU samples** and the **Raft WAL's FNV loop at
9.120%**. Receipt lookup contributes **0.870%**. This makes remaining checksum
work a stronger batch target than receipt lookup. It does not predict a
database speedup or attribute end-to-end waiting time.

The exact selected source is `ca0002c7`, with its original default-feature
release and the unchanged `0be806d9` client. Three voters retain ordinary
Raft quorum, sync calls and commit/apply acknowledgement semantics, using
volatile tmpfs WAL on one shared host. Each workload uses c64, 128-byte values,
4,096 keys and a five-second measurement. `perf` records only owned voter PIDs
at 199 Hz with DWARF call stacks. Clients use CPUs 0-1, voters 2-5 and profiler
helpers 6-15,22-31. This is instrumented CPU attribution, not a QPS comparison.

| Selected leaf population | Point PUT | BatchPut(64) |
| --- | ---: | ---: |
| Total selected samples | 3,286 | 3,103 |
| Engine `frame_crc` | 83 (2.526%) | 561 (18.079%) |
| Raft `write_record_unsynced` | 51 (1.552%) | 283 (9.120%) |
| Of those, exact FNV loop instruction sites | 48 (1.461%) | 283 (9.120%) |
| Receipt `inspect_applied` | 180 (5.478%) | 27 (0.870%) |
| Allocation/copy/compare category | 406 (12.355%) | 559 (18.015%) |
| Persistent ordered map category | 85 (2.587%) | 284 (9.152%) |
| RPC framing/serialization/buffers category | 494 (15.033%) | 106 (3.416%) |
| Kernel network category | 127 (3.865%) | 49 (1.579%) |

Symbol-specific rows and category rows are different views of the same
population and must not all be summed. Inclusive recovered stacks overlap.
Generic copy/compare symbols do not establish which source allocation or
container caused them. Loopback network samples do not diagnose a physical
NIC bottleneck or justify DPDK.

The [exact disassembly attribution](../scripts/redis-reference/crc-active-prefix-cpu-v1/write-record-attribution-first.json)
maps all 283 batch `write_record_unsynced` leaf instruction pointers to the
inlined FNV-1a loops, which contain byte XOR and multiplication by `0x01000193`.
Point PUT has 48 of 51 method leaves in those ranges. This identifies the
sampled instruction sites; it does not assign every method invocation or
inclusive callee to hashing. Engine IEEE CRC and Raft FNV remain distinct
checksum algorithms and on-disk contracts.

## Recording validity and limitations

Both original runtime and unchanged decoder finish successfully. Point PUT
has **582,235 successful measured calls** and BatchPut(64) **63,324**, with
zero refused, unknown, failed or rejected calls. Both pass nominal prefix/tail
containment, the one-millisecond edge exclusion, clock-spread bounds, all
32 sample bins, zero-loss, size limits and original report/retention checks.
These instrumented call counts do not replace uninstrumented timing results.

Before each unchanged measurement client starts, 128 paced unary absent-key
reads produce an active CPU prefix over approximately one second. All **256
priming reads** succeed without retries, followed by two fresh empty voter
publications. Priming changes cache, metadata, allocation and CPU state. Its
completion alone is insufficient: actual decoded samples must still pass the
original interval checks. Seven offline contracts and two real dummy-process
failure cases qualified bounded cleanup before recording. All 268 tracked
fixture/profiler/CLI lifetimes and the four supervisor/child identities exit.

The earlier batch recording failed because it had no sample before the
measurement start. Its original failure remains published. This recording
changes the explicitly recorded setup protocol; it does not trim start times,
extend tails, weaken the decoder or replace the rejected attempt.

The [bundle](../scripts/redis-reference/crc-active-prefix-cpu-v1/BUNDLE.json)
contains **174 exact originals / 26,646,826 decoded bytes / 4,859,862 stored
bytes**, with 20 deterministic gzip files and both original/stored hashes.
The [path index](../scripts/redis-reference/crc-active-prefix-cpu-v1/original-path-index.json)
preserves original source, build, qualification, recording, analysis and
terminal inputs. Raw perf data, WALs, executables and full host process lists
remain local; the bundle cannot independently re-decode excluded perf data.
Publication-authoring failures are retained and do not change runtime evidence.

## Next implementation

A separate [slicing-by-eight IEEE CRC candidate](CRC32-SLICING-QUALIFICATION.md)
preserves the polynomial,
initial/final state, fragments and log bytes. It passes 710 workspace
tests/doctests (23 ignored), Clippy and formatting. A warm-cache standalone
function screen improves 8/16-KiB checksum time by approximately 4.48x/4.56x,
but slightly worsens 1/7-byte calls. **This is not a database performance
result.** Source `e5662bb` now passes the universal eight-byte and compiled-table
proof: 47 distinct theorem statements, all 256 + 2,048 actual entries and eight
rejected controls, with a fresh restored run. It remains unselected; recovery,
exact-source Chaos and paired throughput/latency gates are independent of this
CPU diagnostic. Master and indexed-candidate release bindings remain unchanged.
CI stays local. After bounded checksum qualification, development returns to
GET throughput and single-request latency; durable Raft write costs remain part
of the required semantics.
