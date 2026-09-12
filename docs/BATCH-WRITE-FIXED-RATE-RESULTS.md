# Fixed-rate BatchPut tail diagnosis — 2026-09-12

ThinLTO's higher BatchPut(64) p99 remains visible at both common offered rates
and in both run orders. This experiment does **not** resolve the earlier
[closed-loop tail tradeoff](RELEASE-THIN-LTO-FULL72.md). Its accounting is accepted;
strict matched-work attribution remains unavailable because every cohort drops
some scheduled client slots. No server or client implementation was changed.

Main continues to select [ThinLTO `11113f6`](RELEASE-THIN-LTO-MAIN-INTEGRATION.md),
with its qualified read-throughput gains and explicit write-tail limitation.
The next implementation investigation follows the [single-GET quorum path](QUORUM-LATENCY-NEXT.md).

## Protocol and exact inputs

The [predeclared protocol](BATCH-WRITE-TAIL-NEXT.md) ran four separate two-second
correctness smokes and eight ten-second timed cohorts. Each cohort uses 64
workers, native BatchPut(64), 4,096 keys, 128-byte values, seed 71 and the fixed
v3 client's existing `FixedRate` mode. Rates are 8,000 and 12,000 batch calls/s.
The first order is 8k CRC/ThinLTO, then 12k CRC/ThinLTO; the second reverses that
entire order. The timed matrix offers 800,000 calls. No Redis trial ran.

| Role | Source revision | Retained executable SHA-256 |
| --- | --- | --- |
| CRC server | `ca0002c7f8e9ee6f595efcc9f4151085ccce87cb` | `b33b5d302f901aac5cf6a95449b9bb5dd68473747e011220dbed7aed2e591d13` |
| Integrated ThinLTO server | `11113f68f6a5df77da1ffb4fcec850953716ffa3` | `dd028cb2f61633dda133b05d814a6d173a79a83145d8ced2f81dc0cf8e0f33bc` |
| Fixed native client | `0be806d9671e2c50701a64aa7889c8859b7648ba` | `8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957` |

Build manifests and the unused Redis-client attestation remain in the original
input records. No runtime binary was rebuilt. Timed client CPUs are 0–1 and
server CPUs 2–5; helpers use 6–15 and 22–31. Fresh three-voter cohorts retain
ordinary quorum/sync calls on volatile tmpfs WAL, original admission limits,
1,500-ms deadlines and six-attempt limits. Actual measured calls needed one
attempt each. All original storage floors/caps and cleanup guards remain active.

## Pooled results

Throughput is successful **batch calls/s**. Latencies describe a whole 64-item
batch; they are never divided by 64. Each row pools two original ten-second
cohorts using counts, elapsed time and merged raw histogram buckets.

| Offered calls/s | Server | Successful calls/s | Whole mean ms | Whole p99 ms | Scheduled mean ms | Scheduled p99 ms | Dropped / offered |
| ---: | --- | ---: | ---: | --- | ---: | --- | ---: |
| 8,000 | CRC | 7,998.950 | 0.923 | 4.850–4.915 | 1.847 | 5.964–6.029 | 21 / 160,000 |
| 8,000 | ThinLTO | 7,998.758 | 0.907 | 5.439–5.505 | 1.865 | 6.685–6.750 | 22 / 160,000 |
| 12,000 | CRC | 11,947.714 | 1.559 | 6.750–6.816 | 2.360 | 8.782–8.913 | 1,030 / 240,000 |
| 12,000 | ThinLTO | 11,951.137 | 1.588 | 7.143–7.209 | 2.423 | 9.175–9.306 | 971 / 240,000 |

At 8k, ThinLTO's whole-call mean is 1.766% lower, but its scheduled mean is
1.004% higher. At 12k, its whole-call mean is 1.893% higher and scheduled mean
2.671% higher. Whole-call and scheduled p99 are higher in all four original
rate/order pairs. All means, p50/p95/p99 intervals and individual orders are in
[the immutable readout](https://github.com/c4pt0r/kv9/blob/cfa2b2879820f95324850a6a6f19b07dd63b88d2/docs/batch-write-fixed-rate-v1/READOUT.md)
and [per-order table](https://github.com/c4pt0r/kv9/blob/cfa2b2879820f95324850a6a6f19b07dd63b88d2/docs/batch-write-fixed-rate-v1/PER-REPEAT.md).

The source-bound reports account for **797,956 issued/completed calls**, all
single-attempt successes, **51,069,184 input items**, **2,044 dropped slots** and
**56 completions after the measurement cutoff**. Refused/unknown/failure counts
are zero for issued calls. Dropped slots were never issued and have no latency
sample; success for issued calls does not imply all offered work succeeded.

Mean dispatch lateness is 0.802–0.958 ms across the four pooled cells. It is
measured before issue and contributes to scheduled-to-completion latency.
The client uses worker-specific strided slots and sheds overdue slots. Its
arrival timing and dropped slot/key/nonce sets prevent a strict equal-work
comparison. This observation does not identify the server's causal cost or
prove timer granularity alone caused the delays. Two repetitions do not
establish statistical significance or sustained capacity.

Read-only analysis of the pinned [worker loop](https://github.com/c4pt0r/kv9/blob/0be806d9671e2c50701a64aa7889c8859b7648ba/crates/server/src/bin/kv9-batch-benchmark.rs#L284)
confirms that each worker awaits its previous call, then selects its latest due
owned slot. Global slot spacing is 125 us or about 83.333 us; each worker's
spacing is 8 ms or about 5.333 ms. Timer/task delays, previous-call occupancy
and cutoff races can all contribute to shedding. Several workers can still
wake together. The [metric boundary](https://github.com/c4pt0r/kv9/blob/0be806d9671e2c50701a64aa7889c8859b7648ba/crates/server/src/bin/kv9-batch-benchmark.rs#L174)
ends dispatch lateness before operation construction; whole-call latency
includes construction, SDK execution and response validation. These are not
server-only spans. A separate timer-only calibration using the same pinned
Tokio version and slot arithmetic could test arrival clustering without RPC;
it has not run and could not be subtracted from this workload's latency.

## Independent acceptance and original failure

The smoke runner finished as session **85856/0**; original timing as
**56886/0**, with no performance rerun. All 32 owned timed lifetimes exited;
24 fresh three-voter drains and 24 writer/listener bindings pass. CPU/namespace
restoration passes. Original role checks cover 2,402 tracked source files.

The first independent reader, **7177/1**, passed each of the eight cohorts,
then raised `KeyError` in an obsolete Redis pairing loop. The native-only
inventory had been frozen before execution. The correction removes exactly
that six-line loop; every existing function and all statistical arithmetic
remain byte-identical. One focused full-eight post-loop regression reproduces
the original failure and verifies the correction. The original preparation,
failure log and terminal remain preserved.

Corrected readback **66299/0** accepts the original data. Statistics finished
with direct terminal **aed7dd/0**. Acceptance includes all twelve original
smoke/timing byte-retention records, independently decoded. Timed logical
bytes total 50,806,064,378; compressed objects total 35,951,616,675. Combined
smoke/timing logical bytes are 55,933,368,810 and independently observed
allocated bytes 39,746,797,568. Originals remain cold; original-path replay
requires rehydration. Compression follows writer exit, between cohorts.

Before execution, exact old private Cargo compiler/test cache retirement
reclaimed 26,043,031,552 available bytes. Locked metadata, inode alias closure,
live-reference checks and payload hashes preceded removal. Affected profile
fingerprints were invalidated and retained externally; original server/client
hashes remained unchanged. The historical capacity model was an operational
estimate, not a proven compression bound. No storage threshold was relaxed.

[Immutable reporting evidence](https://github.com/c4pt0r/kv9/blob/cfa2b2879820f95324850a6a6f19b07dd63b88d2/docs/batch-write-fixed-rate-v1/README.md)
retains 1,280 files, 297,938,920 decoded bytes in fourteen bounded archive parts
(29,177,277 encoded bytes). Its independent integrity verifier passes. The
bundle contains reports, raw histogram populations, scripts, source/build
bindings and receipts; it is not a WAL/executable backup or standalone runtime
re-audit. Audit SHA is `35a306088a4efac626a0ee955f2ded6263362746111d0e9abe8484b89bd7a0d8`;
statistics SHA is `322ad8c270a0d32cf2d53d59a9d148b4e00d6ced5af2731f6a97cf8e27faea8d`.

## Decision and next work

Keep the write-tail tradeoff open. Common offered rates do not make the tail
difference disappear, so the closed-loop result cannot be dismissed simply
because ThinLTO completed more writes there. Lossy arrival accounting still
prevents strict server-cause attribution; do not silently lower rates or rerun
until the result looks favorable.

Prioritize the selected server's fresh Safe ReadIndex path: measure exact local
dispatch, peer queue, follower processing and confirmation/pump boundaries
before changing implementation. Retain repeated-context and route-generation
ambiguities. The fixed-rate client's scheduling fidelity is a separate bounded
calibration question; it must not redefine this completed experiment.

Raft, fsync/durable acknowledgements, admission and apply/view fences are
unchanged. This short shared-host tmpfs run adds no real-disk write result,
Redis durability equivalence, core proof or actual Chaos Mesh acceptance. Those
existing gates retain their source scope. Redis read parity remains open before
dynamic multi-Raft and automatic splits. All work here ran locally; no hosted
CI was dispatched.
