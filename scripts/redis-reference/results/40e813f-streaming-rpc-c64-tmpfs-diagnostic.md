# Streaming gRPC selection: same-artifact transport comparison

Select **streaming gRPC with tonic** for the next RawKV integration and
performance work. In this repeated local comparison, it has the highest GET,
PUT and mixed throughput and the lowest mean latency of the three tested point
transports. Both streaming repetitions exceed all four tarpc and all four unary
repetitions for each workload. This selects the development direction; the
experimental listener is not yet the default production transport.

## Same-artifact protocol

All five KV9 arms use clean release revision
`40e813f37672eb66b80b307c0e20f9bde0adf31c` with explicit `rpc-experiment`:

- Server SHA-256: `cfa9bd9abe6b2729cf2ceb9b47e751ec3c70949ed1ddec2e27f1e409d7bad204`.
- Client SHA-256: `e68deb079cd4cf9ff15e7d7bf8b10a1c4bde391ca93818c0dc5a4db7f04133d8`.
- Fixed Redis client SHA-256: `57947d5a94c739dd58ca98e2c9a85c40814040826f13f99e85cd0e2c7fb2f636`.
- Redis 7.0.15: standalone memory, save/AOF disabled, no replicas.

The order is unary before, tarpc before, streaming, tarpc after, unary after.
Each arm uses a fresh three-voter fixture, two repetitions of GET100, PUT100 and
GET50/PUT40/DELETE10, and a Redis reference paired with every KV9 cohort. The
KV9/Redis order reverses in the second repetition. All 60 measured cohorts and
ten separate complete-history guards are retained.

Each closed-loop worker waits for its point response before sending another
operation. All arms use 64 workers, 64 hot keys, 23-byte keys, 128-byte values,
32 warmup operations, 1,500-ms measurement windows and request deadlines, six
maximum routing attempts, 5-ms routing backoff and a 64-call / 16-MiB public
admission limit. No operation cap is reached. The 1.5-second window keeps the
unchanged one-million total KV9 operation limit above the Redis-class target;
Redis retains its five-million measured-operation cap.

Clients use logical CPUs 0–1; voters and Redis use 2–5. No build, test, profile
or Chaos run overlaps timing. This remains a shared host with background
services, not dedicated physical-core isolation. Voter storage is volatile tmpfs
with ordinary Raft quorum and sync calls; retained disk copies do not establish
runtime power-loss durability. Compare it as a short memory-path diagnostic,
not sustained or cross-host production capacity.

Handlers, authentication, public admission, quorum reads, exact committed/applied
write receipts and Raft transport are shared across all three point transports.
The [streaming design and correctness evidence](https://github.com/c4pt0r/kv9/blob/40e813f/docs/RPC-STREAM-CONTROL.md)
document correlated responses, bounded work and cancellation without write replay.
This is a new executable build; do not attribute its measurements to b49 or the
older a00e three-second/default-server protocol.

## Throughput

Rates pool successful operations over summed complete measurement-plus-drain
time. The comparison columns pool both surrounding arms of the named transport.

| Workload | Pooled unary | Pooled tarpc | Streaming gRPC | vs unary | vs tarpc | Streaming's paired Redis |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| GET | 126,029.0/s | 188,909.3/s | **208,848.8/s** | +65.71% | +10.56% | 497,639.3/s |
| PUT | 74,332.1/s | 94,137.1/s | **96,078.4/s** | +29.26% | +2.06% | 478,989.2/s |
| Mixed | 87,512.9/s | 116,803.2/s | **123,136.6/s** | +40.71% | +5.42% | 486,961.7/s |

For completeness, the five separate arm rates are:

| Workload | Unary before | Tarpc before | Streaming | Tarpc after | Unary after |
| --- | ---: | ---: | ---: | ---: | ---: |
| GET | 126,426.0/s | 189,063.3/s | 208,848.8/s | 188,755.3/s | 125,632.0/s |
| PUT | 74,615.7/s | 94,344.8/s | 96,078.4/s | 93,929.3/s | 74,048.5/s |
| Mixed | 87,873.4/s | 116,544.3/s | 123,136.6/s | 117,062.1/s | 87,152.4/s |

Streaming reaches **41.97% of Redis GET, 20.06% of PUT and 25.29% of mixed**;
Redis remains approximately **2.38x, 4.99x and 3.95x faster**. KV9 replicates
through three Raft voters whereas the Redis reference is standalone. This
distinction does not attribute the entire remaining gap to inevitable consensus
cost. The observed PUT advantage over tarpc is modest; these two repetitions
are not a long-duration capacity or statistical-confidence claim.

All **5,411,907 measured KV9** and **22,085,601 measured Redis** operations
succeeded. Measured KV9 attempts equal logical operations: no failed, unknown,
refused or retried measured request. The ten separate complete-history guards
contain 1,620 successful operations. Setup/routing and guards remain separate
populations; performance cohorts retain complete outcome/latency aggregates,
not per-operation histories.
Setup separately has 30 typed NotLeader attempts, six per arm; all initialization
logical operations succeed.

## Latency and CPU

Streaming mean acknowledged latency is 301–311 us for GET, 664–667 us for PUT,
and 512–526 us for mixed. The surrounding tarpc ranges are 337–339 us,
676–682 us and 540–557 us; unary is 505–510 us, 849–864 us and 727–739 us.

In both streaming repetitions:

- Pure GET p99 falls into the **0.262144–0.524287-ms bucket**, versus
  0.524288–1.048575 ms for both surrounding transports.
- Pure PUT p99 stays in **1.048576–2.097151 ms** for all transports.
- Mixed GET/PUT/DELETE p99 falls into **0.524288–1.048575 ms**. Tarpc mixed
  GET shares that bucket, but its PUT/DELETE and all unary mixed operations
  remain in 1.048576–2.097151 ms.
- Paired Redis p99 remains 0.131072–0.262143 ms.

These are logarithmic histogram buckets, not precise p99 point estimates.

Streaming GET samples use 0.54–0.57 client CPU cores and 2.66–2.68 aggregate
server cores. Tarpc uses 0.75–0.77 and 2.35–2.38 respectively: streaming improves
throughput and client cost, but does not reduce aggregate GET server CPU here.
PUT uses 0.36–0.37 client cores and 3.53–3.55 server cores; its small throughput
gain accompanies similar server saturation to tarpc. Samples cover interior
measurement windows and are tick-quantized. They do not attribute the remaining
write cost to a particular lock, Raft event or WAL operation.

## Evidence and next action

Independent acceptance revalidated all 30 KV9 performance reports and ten full
histories and recomputed every one of the 60 summaries. All 425 clean source
inputs and both Cargo graphs were checked, with engine/Raft features empty.
All 115 owned process lifetimes exited. The 1,835 resource sample rounds preserve
the requested CPU masks, and all 35 fresh drains / 105 voter captures have two
serial post-client-exit export advances, unchanged PID/start/boot identity,
zero public/read/apply occupancy and stopped=false.

The 30 experimental listeners have before/after owned socket-inode and selected
non-secret environment captures bracketing the clients and final drains. All
219 retained tmpfs data files / 2,298,284,740 bytes match their original manifests.
The independent audit is `/tmp/kv9-stream-comparison-independent/audit.json`,
SHA-256 `42795b67082a17333559feefb0b097665c4e32e7d87905ca0faccaf69fef749b`.
Its inventory has 1,311 hash references / 141,563,965 bytes, SHA-256
`863c1d0fc13758bf25d43d729d93ccc945449d0f988ae8c8e5ec42323d8be83d`;
every reference was read back. Tmpfs data is separately counted above. All audit
stages passed on their first attempt. No cohort or fixture was rerun/discarded.
KV9 executable identity is captured through `/proc/PID/exe`; the Redis helper
hashes selected executable paths and binds PID/start and resource samples.

Raw matrix: `/tmp/kv9-stream-comparison-first`. Retained clean build:
`/tmp/kv9-stream-release-first`. The [exact executed driver](40e813f-streaming-rpc-driver.py)
has SHA-256
`f54a418b03c27cb8060579435d496e9dcb602d6ca4ac5b5d921cd0bfb7be807a`.
Its absolute source/build/output paths identify this run; replay requires fresh
output and verified source/helper/executable identities. Its frozen copy and
helper hashes accompany the matrix. Root aggregates and latency/CPU details are
`/tmp/kv9-stream-comparison-aggregate.json` and
`/tmp/kv9-stream-comparison-latency-cpu.json`.

Proceed with streaming gRPC as the selected point transport. Integrate it into
the ordinary service/client path with explicit transport reporting, preserve
unary administration and the existing Raft service, and retain conservative
unknown-write behavior across stream failure. Require exact integrated-source
functional/proof checks and actual Chaos Mesh histories before default runtime
promotion. The current experiment's loopback-only listener is not a distributed
deployment interface; test or integration networking must be explicit.

Then profile the selected write path and optimize measured Raft/WAL scheduling
and allocation costs. Keep tarpc and unary as reproducible reference controls.
DPDK remains conditional on a demonstrated network-I/O bottleneck. Redis-class
RawKV performance remains open before dynamic multi-Raft and automatic range
splits. No hosted CI was dispatched.
