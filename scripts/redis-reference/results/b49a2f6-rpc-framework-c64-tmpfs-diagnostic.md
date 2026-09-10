# Public RPC comparison: unary gRPC versus tarpc/TCP

Exact `b49a2f6` tarpc/TCP improves client-visible GET throughput by **48.07%**,
PUT by **26.31%** and 50/40/10 mixed throughput by **32.98%** against the pooled
surrounding unary-gRPC runs in this local diagnostic. Both tarpc repetitions
exceed all four unary repetitions for each workload. Raft transport, quorum
reads, exact committed/applied write receipts and public admission are shared.

This is a **new 1.5-second, single-host, volatile-tmpfs diagnostic** with an
experimental feature-enabled client/server. It does not establish disk/power-loss
durability, sustained capacity, cross-host results or production RPC acceptance.
Do not splice these numbers into the older a00e three-second/default-server and
retained-Ready-client comparison as an unchanged-protocol improvement.

## Same-artifact comparison

All three arms use clean release revision
`b49a2f6abe6e91b986a633e8a36b19ffccea9a0b`, with explicit `rpc-experiment`:

- Server SHA-256: `db84ff25f8a5414cfa6b7f685c0c633cf120a7d6bd8e90956f2fff38aa42fb1c`.
- Workload SHA-256: `d0c225ab3f1548503b5ee3929e63cc6a9238c0665c9dba48f70fdc657141afd7`.
- Fixed Redis client SHA-256: `57947d5a94c739dd58ca98e2c9a85c40814040826f13f99e85cd0e2c7fb2f636`.
- Redis reference: 7.0.15, standalone memory, persistence disabled, no replicas.

The selected database runtime is `23bc58b` plus the opt-in public RPC adapter.
Both listeners share the same authenticated point handlers, backend and admission
ledger. Tarpc uses bincode framing with the same inner protobuf payloads;
inter-voter Raft stays on its existing transport. The experiment and its local
correctness evidence are [documented at the exact revision](https://github.com/c4pt0r/kv9/blob/b49a2f6/docs/RPC-FRAMEWORK-EXPERIMENT.md).

Each arm uses one fresh three-voter fixture: unary before, tarpc, unary after.
For every arm, two repetitions cover GET100, PUT100 and GET50/PUT40/DELETE10.
Each KV9 cohort has a paired Redis reference, with KV9/Redis order reversed in
the second repetition. There are 36 measured cohorts and six separate complete-
history correctness guards. Names are fixed at six bytes, yielding 23-byte
keys; each workload uses 64 hot keys, 128-byte values, 64 persistent workers,
32 warmup operations, 1,500-ms request deadlines and a 64-call admission limit.
Each worker waits for its point response before issuing another operation.

Every measured window is 1,500 ms. The unchanged one-million total-operation KV9
cap would truncate a three-second run above roughly 333,000/s, below the intended
Redis-class target. Using the shorter window in every arm avoids that constraint
without raising the client cap. Redis retains its five-million measured-operation
cap. A cap stop, missing response, unknown/refused operation or extra measured
KV9 attempt invalidates this diagnostic; none occurred.

Clients use logical CPUs 0–1, voters and Redis 2–5, on a shared host. No build,
proof run, profiler or Chaos matrix overlaps the measurements. Existing background
services remain, so this is not exclusive physical-core isolation. Normal Raft
quorum and sync calls remain enabled, but voter data lives on volatile tmpfs.
The later retained disk copies do not make the running database durable.

## Throughput

Rates pool acknowledged operations over complete measurement-plus-drain time.
The improvement column compares tarpc with both surrounding unary arms pooled.

| Workload | Unary before | Tarpc/TCP | Unary after | Tarpc change | Tarpc's paired Redis |
| --- | ---: | ---: | ---: | ---: | ---: |
| GET | 125,605.9/s | 185,664.6/s | 125,177.0/s | +48.07% | 495,688.1/s |
| PUT | 74,176.4/s | 93,692.3/s | 74,170.8/s | +26.31% | 483,858.3/s |
| Mixed | 86,893.8/s | 115,714.2/s | 87,143.2/s | +32.98% | 489,807.2/s |

Tarpc reaches **37.46% of Redis GET, 19.36% of PUT and 23.62% of mixed**;
Redis remains approximately 2.67x, 5.16x and 4.23x faster. KV9 replicates through
three Raft voters while this Redis reference is standalone; that distinction
does not assign the entire remaining gap to an unavoidable consensus cost.

All **2,905,473 measured KV9** and **13,249,408 measured Redis** operations
were acknowledged. Every cohort completed its duration and full outcome/latency
accounting. Measured KV9 attempts equal logical operations; there were no hidden
NotLeader retries, admission refusals, unknown outcomes or failures in these
timed populations. Correctness guards and setup/verification are counted separately.

## Latency and sampled CPU

Across the two tarpc repetitions, mean acknowledged latency is 341–347 us for
GET, 680–684 us for PUT and 549–556 us for mixed operations. The surrounding
unary ranges are 506–513 us, 856–868 us and 731–738 us, respectively.

The coarse logarithmic p99 bucket is unchanged for pure GET
(0.524288–1.048575 ms) and pure PUT (1.048576–2.097151 ms). Mixed GET moves from
1.048576–2.097151 ms to 0.524288–1.048575 ms in both tarpc repetitions;
keep per-repetition buckets in the raw report rather than infer a precise p99
from bucket endpoints. The paired Redis p99 buckets are 0.131072–0.262143 ms.

GET's sampled aggregate server CPU falls from 3.03–3.11 cores in unary arms to
2.35–2.39 in tarpc, while client CPU falls from 1.37–1.39 to 0.71–0.73 cores.
PUT server CPU remains approximately 3.48–3.52 cores while completing more work;
client CPU falls from 0.88–0.89 to 0.41–0.42 cores. These are measured-window
process samples with tick quantization, not an attribution of every request's
latency. Idle CPU alone does not identify the next bottleneck.

## Evidence and decision

Independent validation passed all 36 cohorts, all 18 KV9 performance reports and
all six full-history guards (972 successful operations). It recomputed every
summary with the unchanged outcome/history validators, reopened 421 source
files against clean `b49a2f6`, and verified both Cargo feature graphs. All 69
owned process lifetimes exited. The 1,100 resource sample rounds preserve the
requested CPU masks. All 21 fresh-drain records contain three voter captures:
each has two serially observed post-client-exit export advances under the same
PID/start/boot identity, zero final public/read/apply occupancy and no stopped
backend. All 129 retained tmpfs data files (1,274,621,083 bytes) match their
original manifests.

Measured attempts had no retries. Setup separately contains 18 typed NotLeader
attempts, six per arm; every setup logical operation completed successfully.
Performance cohorts retain outcome/latency aggregates, not per-operation
histories; the six history guards are separate checks. Tarpc endpoint selection
is supported by the frozen launch environment mapping, selected client configs
and successful calls; no socket-inode or `/proc/environ` census was retained.
KV9 hashes the running `/proc/PID/exe`; the Redis helper hashes the selected
executable path and binds process identity through PID/start and resource samples.

The accepted audit is
`/tmp/kv9-rpc-framework-comparison-independent/audit.json`, SHA-256
`19b0d1ad2d93e8fead30afab18c5e4bc847c14c3bc2032faf9c6d59a31bc7f83`.
Its inventory has 795 references and 94,132,616 referenced bytes, SHA-256
`462cc26380b30649233361bc4c047489657dc9a920672705813b3611d98e9b08`;
every reference was read back. The separately checked tmpfs data is counted
above. An initial audit adapter incorrectly expected profile/rustc metadata in
`sources.json`; its failure is retained, and only the adapter was corrected.
No measured fixture was rerun or discarded.

Raw matrix, every attempted cohort, all six guard histories, resource samples,
fresh drain observations and retained tmpfs stores are at
`/tmp/kv9-rpc-framework-comparison-b49a2f6-first`. The driver is
`/tmp/kv9-rpc-framework-comparison.py`; the [exact executed driver](b49a2f6-rpc-framework-driver.py)
is also retained in this repository. Its absolute paths identify this run's
owned source/build/output directories; a replay must select fresh output and
verify the recorded build/helper hashes. Its frozen copy and helper hashes
accompany the matrix. The executed driver SHA-256 is
`664579a6f87df15167c2e7027dec9f33e31cf0ff9dc4213886aaad92c298a6bd`.
Retained clean builds are at
`/tmp/kv9-rpc-framework-release-b49a2f6`. Root aggregation and per-repetition
latency/CPU records are `/tmp/kv9-rpc-framework-comparison-aggregate.json` and
`/tmp/kv9-rpc-framework-comparison-latency-cpu.json`.

The result supports continuing the transport/allocation work. Keep tarpc an
experimental candidate while adding the streaming-gRPC control and exact-feature
Chaos acceptance before promotion. Earlier default-runtime Chaos and core
source-mapped proofs retain their separate boundaries. DPDK remains conditional
on a measured network bottleneck; this loopback result does not establish a NIC
limit. Redis-class performance remains open, followed by dynamic multi-Raft and
automatic range splits. No hosted CI was dispatched.
