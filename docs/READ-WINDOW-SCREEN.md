# Two-context ReadIndex screen: keep the candidate experimental

Updated: 2026-09-11. Tracking: #9, #13 and #20.

Two-context candidate `5654ea59` improves c64 pure GET throughput **6.314%**
and its mean/p99 in both repetitions. It still worsens mixed GET mean/p99,
and c1 pure GET throughput falls in both repetitions. Keep CRC selected;
do not expand this candidate into a full batch or Chaos Mesh promotion campaign.
The next read optimization should reduce confirmation-path processing and
handoffs while preserving fresh quorum authorization and apply-before-read.

## Latest point GET results

These are pooled successful-call results from two new, uninstrumented,
ten-second repetitions. Quantiles merge raw histograms; intervals are the
containing buckets in microseconds, not confidence intervals.

| Concurrency | Version | Calls/s | Mean us | p99 us |
| ---: | --- | ---: | ---: | --- |
| 1 | CRC | 26,637.627 | 37.423728 | 49.664–50.175 |
| 1 | Window2 | 26,381.243 | 37.785213 | 50.176–50.687 |
| 1 | Redis | 174,163.618 | 5.664901 | 7.296–7.359 |
| 64 | CRC | 345,322.532 | 185.210035 | 352.256–356.351 |
| 64 | Window2 | 367,127.110 | 174.198977 | 311.296–315.391 |
| 64 | Redis | 511,324.927 | 125.051328 | 229.376–231.423 |

C64 GET throughput improves **5.882% / 6.749%** in the paired repetitions.
Its mean falls **5.945%** when pooled. Redis retains **1.393x** the candidate's
throughput. At c1, candidate throughput falls **0.962%** pooled and mean rises
**0.966%**; mean remains **6.670x** Redis's. The c1 throughput changes are
negative in both repetitions. All individual rows remain in the bundle.

## Mixed reads still regress

| c64, 50% reads | CRC mean us | Window2 mean us | CRC p99 us | Window2 p99 us |
| --- | ---: | ---: | --- | --- |
| GET | 383.676013 | 395.217228 | 622.592–630.783 | 638.976–647.167 |
| PUT | 359.232467 | 339.035106 | 589.824–598.015 | 557.056–565.247 |
| Combined | 371.442 | 367.095 | 606.208–614.399 | 614.400–622.591 |

Combined throughput rises **1.184%**, but GET mean increases **3.008%**.
GET p99 worsens in both repetitions, reaching **647.168–655.359 us** in the
second. Faster writes mask slower reads in the combined mean. C1 mixed
throughput improves 0.798%, which does not remove the c1 pure-read regression.
This is a CRC-versus-window2 selection screen, not a matched causal comparison
with the earlier one-context candidate. Do not infer window1-to-window2 deltas
from different runs.

## Recording, correctness and reproducibility

The **24-cohort** screen covers c1/c64 x point read50/read100 x CRC/window2/Redis
x forward/reverse repetitions. A separate **12-cohort correctness smoke** passes
first. There are no batch or write-only cohorts in this screen; the previous
[72-cohort report](READ-CREDIT-CRC-PERFORMANCE.md) remains the latest broad matrix.
The accepted auditor's inherited scope string mentions point/batch read/write;
its actual protocol, descriptors and predicates enforce this 24-cohort point
subset. Original audit bytes remain unchanged.

Candidate: `5654ea593fe561c5fd8d800cf498f716eb5bca23`; CRC:
`ca0002c7f8e9ee6f595efcc9f4151085ccce87cb`. Both native and Redis clients remain
fixed v3 `0be806d9671e2c50701a64aa7889c8859b7648ba`. This uses 4,096 keys plus a
sentinel, 128-byte values, seed 71, 128 warmup calls, closed-loop clients,
1,500-ms deadlines and the original attempt/admission limits. Clients run on
CPUs 0-1, servers 2-5, helpers/owned containers 6-15,22-31.

KV9 keeps normal three-voter quorum and sync calls on **tmpfs WAL**. Redis uses
standalone memory with persistence disabled and no pipelining. This is a
shared-host loopback memory-path diagnostic, not equal write durability,
physical-disk or cross-host NIC capacity. No builds, tests, faults, profiling
or audits overlap timing.

All **50,152,139 measured calls** succeed in one attempt, with zero dropped
slots and non-success populations. Initialization routing attempts remain
separate in the full phase accounting. The independent audit accepts all
**80 owned lifetimes exited**, **48 fresh drains**, **48 voter/writer/listener
bindings** and exact restoration of the three owned containers. It retains
636 runtime files / 4,598,354,509 bytes locally, including WAL evidence.

Root smoke session `36013` exits 0 (`698e6f`); timing session `98312` exits 0
(`716f38`); independent audit session `59029` exits 0 (`5c8efd`). Frozen inputs
are unchanged throughout. Seven driver and 17 auditor contracts passed before
recording. No failed measurement was replaced or rerun, and no hosted CI ran.

The candidate's [source/proof and ordinary recovery evidence](READ-LIFECYCLE-CRC-CREDIT.md)
passes 718 workspace tests, 14 compiled semantic-control triples, both scoped
formal gates and the stream/unary leader-loss/restart histories. Actual Chaos
Mesh and complete implementation proof composition remain separate obligations.
Those correctness results do not override the measured read regressions.

The [publication bundle](read-window-screen-v1/README.md) contains all raw client
reports, original audit, preparation/diffs, statistical arithmetic, per-repeat
rows and exact-byte inventories. It omits binaries/WAL and most host observations;
its integrity check is not a new full runtime audit. Selected mainline behavior
remains CRC. Redis-class reads, dynamic multi-Raft and automatic splits remain
open, with the development order in [RAWKV-PERFORMANCE-PATH.md](RAWKV-PERFORMANCE-PATH.md).
