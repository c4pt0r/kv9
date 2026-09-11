# Selected-source GET and mixed CPU profile

The selected CRC runtime now has fresh CPU and thread scheduling observations
for c1 GET and c64 mixed GET/PUT. This is diagnostic evidence, not a new
throughput result. Keep `ca0002c7` selected.

The mixed profile identifies a concrete removable cost: `inspect_applied`
accounts for 157 of 3,342 selected CPU samples (4.698%). Disassembly of this
exact executable places 140 samples (4.189%) in its linear receipt-search
loop. The existing indexed-receipt candidate `74d24116` already replaces that
search and has passed local source, representation-proof and ordinary recovery
checks. Its earlier screen measured writes only. Complete its missing pure-read
and mixed screen next, retaining the original candidate and control artifacts.
The new mixed attribution supplies a reason for that qualification; it does not
resolve the candidate's earlier inconclusive batch-write result.

## CPU observations

Percentages below are mutually exclusive heuristic leaf-symbol categories.
They describe sampled server CPU, not fractions of request latency. Inclusive
stacks overlap and are retained separately in the evidence.

| Selected leaf category | c1 GET | c64 mixed |
| --- | ---: | ---: |
| User-space scheduling/synchronization | 14.504% | 12.268% |
| Kernel scheduling/futex | 8.783% | 3.381% |
| RPC/framing/serialization/buffers | 13.699% | 15.111% |
| Allocation/copy/comparison | 8.300% | 13.106% |
| Kernel network | 12.973% | 4.488% |
| Kernel generic spinlocks | 9.025% | 5.117% |
| Metadata lookup | 2.579% | 2.484% |
| Engine/storage | 2.498% | 5.236% |

The remaining samples include Raft, ordered-map operations, other symbols and
unknown frames. Categories use the retained rules, including their omissions;
they are not an exact semantic classification of every allocator or kernel
function. Instruction-pointer sampling has skid. Earlier source versions'
profiles and instruction offsets are not reused.

## Scheduling observations and limits

The recordings contain 4,561,559 switch records across 13 observed threads for
c1 and 3,773,936 across 54 observed threads for mixed traffic. Those counts cover
the entire recording. Interval populations use only the same interior window
as the accepted CPU samples. The reader pairs adjacent alternating records,
clips intervals to that window, and leaves unmatched prefixes/suffixes unknown.
Six focused parser tests and a read-only review found no interval-arithmetic or
request-latency attribution blocker.

These records identify OS threads, not async tasks. A switch-out without the
preemption flag does not identify an I/O wait. Idle workers, runnable delays,
quorum waits and request completion cannot be separated from this record alone.
Summing waits across threads is not a request-latency decomposition. Numeric
PID/TID checks do not independently rule out same-process TID reuse. The
standalone interval parser does not inspect loss records; this run depends on
the prerequisite CPU decoder's zero-total-lost-samples check for the same raw
recording. No global BPF probe was installed.

## Recording protocol and acceptance

The server is original default-feature release `ca0002c7f8e9ee6f595efcc9f4151085ccce87cb`,
SHA-256 `b33b5d302f901aac5cf6a95449b9bb5dd68473747e011220dbed7aed2e591d13`.
The fixed v3 client is `0be806d9671e2c50701a64aa7889c8859b7648ba`, SHA-256
`8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957`.
Each workload measures five seconds, 4,096 keys, 128-byte values, seed 71,
128 warmup calls and a 1,500-ms call deadline. Mixed uses 50% reads; pure GET
uses one worker. Public admission, fresh Safe ReadIndex, sealed groups and
ordinary Raft quorum/sync/commit/apply behavior remain unchanged.

`perf` records only the three owned voter processes with the monotonic clock,
199-Hz `cpu-clock`, 16-KiB DWARF stacks and switch events. A paced active prefix
and fresh drains precede the unchanged workload. Recording lasts 20 seconds;
CPU analysis excludes one millisecond at each nominal measurement edge.
The prefix and instrumentation can affect the measured system.

The first recording reached the prospective 128-MiB cap and is rejected. Its
failure, successful client termination and cleanup remain retained. A fresh
derivative increases only the capacity to 512 MiB and uses a new isolation
path; workload, sampling and acceptance predicates are unchanged. It completes
with 135,812,444 and 151,888,296 raw bytes. Both decoders report zero lost samples.

| Accepted diagnostic | c1 GET | c64 mixed |
| --- | ---: | ---: |
| Recorded CPU samples | 1,570 | 3,672 |
| Selected interior CPU samples | 1,241 | 3,342 |
| Wall/monotonic anchor spread | 461 ns | 296 ns |
| Measured completed calls | 112,548 | 800,032 |
| Successes | 112,548 | 800,032 |
| Other outcomes | 0 | 0 |

Both recordings contain the full nominal measurement and satisfy the unchanged
sample-count, edge and 32-bin aggregate coverage checks. Per-voter coverage is
retained rather than requiring dense sampling of every idle follower. Each
case reaps four fixture children and two profiler lifetimes. The three owned
background containers' CPU settings and namespace identity are restored.

Clients use CPUs 0-1, voters share CPUs 2-5, and helpers use CPUs 6-15,22-31.
This is a shared host with loopback networking and volatile tmpfs WAL, not a
disk, cross-host, NIC or power-loss experiment. No build, tests, uninstrumented
timing or fault campaign overlapped these recordings. All CI remained local.

## Evidence and next decision

The [compact evidence bundle](read-path-profile-v1/README.md) includes original
protocols, sample populations, CPU renders, interval results, commands,
terminal/cleanup records and the rejected recording's failure metadata.
Raw perf files, full switch dumps, binaries and WALs remain local with hashes;
the compact bundle is not a complete environment image or runtime archive.

The next screen uses c1/c64, pure GET/mixed traffic and two reversed repetitions
against CRC and Redis, evaluating GET mean/p95/p99 as well as total throughput.
Reuse the candidate's exact prior proof and recovery evidence after checking
its source/build identity. Its original build compiles dependencies in the
workload stage and reuses them in the server stage: the first overly strict
server-only artifact reader is retained, and the corrected reader checks both
stages in order. All 592 sources and 11 first-observed compiled units match.
This identity check does not rerun or enlarge the earlier correctness claims.

A useful result still needs full workload qualification and actual exact-source
Chaos Mesh before promotion. It must not worsen single GET or mixed GET tails.
Raft consistency and the existing full core-proof obligations remain required.
Dynamic multi-Raft and automatic splits follow the read-performance milestone.
