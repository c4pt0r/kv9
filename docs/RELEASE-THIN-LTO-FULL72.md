# ThinLTO: complete point and batch comparison

The complete 72-cohort comparison supports advancing candidate `02d0c01` to
main integration. All 12 workload/concurrency cells improve throughput and
mean latency against CRC, in both run orders. There is one tail tradeoff:
loaded batch-only writes have a higher pooled p99. Keep that result visible;
this is not an across-the-board latency win. [Fresh main integration](RELEASE-THIN-LTO-MAIN-INTEGRATION.md)
now passes; selected `11113f6` reproduces the qualified server/client bytes.
The measurements below retain their original source and execution identity.

## Same-run results

Each row pools two ten-second repetitions in opposite orders. Values are
successful API calls/s; a batch call contains 64 items. Mixed rows count read
and write calls together. All measured calls succeeded on their first attempt.

| Concurrency | Workload | CRC calls/s | ThinLTO calls/s | Change |
| ---: | --- | ---: | ---: | ---: |
| 1 | PUT | 16,621.296 | 19,270.909 | +15.941% |
| 1 | GET/PUT 50/50 | 18,940.578 | 21,706.144 | +14.601% |
| 1 | GET | 26,510.350 | 28,314.673 | +6.806% |
| 1 | BatchPut(64) | 5,455.813 | 5,815.392 | +6.591% |
| 1 | BatchGet/BatchPut 50/50 | 6,497.795 | 7,000.505 | +7.737% |
| 1 | BatchGet(64) | 9,630.225 | 10,069.090 | +4.557% |
| 64 | PUT | 125,164.435 | 131,260.228 | +4.870% |
| 64 | GET/PUT 50/50 | 171,741.670 | 189,585.599 | +10.390% |
| 64 | GET | 346,550.757 | 375,885.286 | +8.465% |
| 64 | BatchPut(64) | 13,728.332 | 14,150.607 | +3.076% |
| 64 | BatchGet/BatchPut 50/50 | 20,666.638 | 21,449.219 | +3.787% |
| 64 | BatchGet(64) | 35,587.969 | 36,594.572 | +2.828% |

| Metric | CRC | ThinLTO | Redis |
| --- | ---: | ---: | ---: |
| c1 GET calls/s | 26,510.350 | 28,314.673 | 174,264.654 |
| c1 GET mean us | 37.605 | 35.206 | 5.660 |
| c1 GET p99 us | 49.664–50.175 | 44.544–45.055 | 7.104–7.167 |
| c64 GET calls/s | 346,550.757 | 375,885.286 | 512,498.244 |
| c64 GET mean us | 184.555 | 170.140 | 124.766 |
| c64 GET p99 us | 352.256–356.351 | 323.584–327.679 | 231.424–233.471 |
| c64 BatchGet(64) items/s | 2,277,629.998 | 2,342,052.591 | 5,964,711.568 |
| c64 BatchGet(64) whole-call mean us | 1,793.177 | 1,743.640 | 684.704 |
| c64 BatchGet(64) whole-call p99 us | 3,014.656–3,047.423 | 2,883.584–2,916.351 | 1,032.192–1,040.383 |
| c64 BatchPut(64) items/s | 878,613.230 | 905,638.841 | 6,091,968.177 |
| c64 BatchPut(64) whole-call mean us | 4,660.433 | 4,521.561 | 667.664 |
| c64 BatchPut(64) whole-call p99 us | 8,257.536–8,323.071 | 8,388.608–8,519.679 | 1,007.616–1,015.807 |

ThinLTO reaches **73.344% of Redis c64 GET throughput**. Its isolated GET
mean remains **6.220 times Redis**. Native batch reads reach **39.265%** of
Redis MGET item throughput at c64. These are fresh same-run comparisons, not
historical rates relabeled as current results. The earlier favorable
[24-cohort screen](RELEASE-THIN-LTO-PERFORMANCE.md) remains separately scoped.

## Tail and resource tradeoffs

For c64 BatchPut(64), the forward repetition's p99 moves from
**8.061–8.126 ms to 8.389–8.520 ms**. In reverse order both versions occupy
the **8.389–8.520 ms** bucket. The pooled p99 is therefore worse, despite
better throughput, mean and p95. The other 11 pooled cells improve p99;
among the 24 individual comparisons, 22 improve, one worsens and one shares
the same bucket. Two short repetitions do not establish statistical significance
or justify calling the regression noise.

Separate mixed GET and PUT means and tails remain in the complete API tables.
For example, c64 mixed GET mean falls from **384.632 to 348.680 us**, and PUT
mean from **360.417 to 326.225 us**. The combined rate does not hide those APIs.

Sampled read-only voter RSS is lower for ThinLTO in every recorded pair;
fixed-duration write workloads generally retain more memory while completing
more writes. This experiment does not separate allocation efficiency from
the higher completed-write count. CPU estimates include their recorded sample
windows, and HWM includes setup; neither is a measurement-only per-request cost.
All per-repeat memory and CPU rows remain published without a pooled HWM.

## Method, identity and acceptance

The frozen matrix covers c1/c64, point1/batch64 and read0/50/100, with CRC,
ThinLTO and Redis in two opposite orders. It uses 4,096 keys, 128-byte values
and separate 36-cohort two-second smoke. Timed clients remain the original
`0be806d9` v3 artifacts. Redis uses GET/SET or MGET/MSET as appropriate,
without persistence or pipelining. KV9 uses three voters, fresh Safe ReadIndex
and ordinary quorum/sync calls on **volatile tmpfs WAL**. This is shared-host
loopback evidence, not equal write durability, physical-disk latency,
independent-host failure, NIC saturation or sustained capacity.

The exact server remains `dd028cb2f61633dda133b05d814a6d173a79a83145d8ced2f81dc0cf8e0f33bc`
from source `02d0c01024b65a84b220c6948ff2224bfa7900bc`; CRC remains
`ca0002c7f8e9ee6f595efcc9f4151085ccce87cb`. The profile change is codegen-only
relative to parent `40f014f`, which contains the default-off read-stage observer.
CRC predates that parent. Consequently the complete candidate comparison does
not isolate every compiler contribution; the retained generated README's
"codegen only" wording describes the parent delta, not an identical-source A/B.
Neither measured production build enables the observer.

All **81,648,272 measured calls / 822,289,187 input items** succeed with one
attempt and zero dropped slots. The independent audit accepts all 72 cases,
**240 exited lifetimes, 144 fresh drains and 144 voter/listener bindings**,
13,990 resource samples, and exact CPU/container-namespace restoration.
Initialization routing remains separately counted.

All **24 smoke and 48 timed native retention records** were independently
decoded. Timed originals comprise **4,396 files / 96,930,392,255 logical bytes**;
their compressed objects are **67,887,203,210 bytes**. Combined smoke/timing
logical bytes are **108,740,443,661**, and independently counted allocated
bytes are **77,212,495,872**. Originals are cold, not resident; future full
original-path audits require rehydration. Compression occurs only after writers
exit and before the next cohort; longer gaps can affect cache/thermal state.
No codec overlaps measurement, and no storage cap or floor was lowered.

Actual terminals: smoke `18397/0`, timing `47339/0` (`ee81e8`), independent
audit `61202/0` (`256391`), statistics direct tool `96ae5e/0` with no numeric
root session. The initial smoke Git-ownership preflight failure occurred before
output creation or any database launch; only exact source-directory trust was
corrected before the successful smoke. No timed cohort was retried.

The exact candidate's [709 local tests and ordinary recovery](RELEASE-THIN-LTO-VALIDATION.md)
and [21-window actual Chaos Mesh acceptance](RELEASE-THIN-LTO-CHAOS.md) retain
their original scope. Fresh main-source/default-build/recovery confirmation
is now complete in `11113f6`; no new timing or Chaos campaign is inferred.
No hosted CI was dispatched.

## Next development steps

1. **Completed:** integrate the exact qualified profile/default-off observer
   source as `11113f6`. Fresh workspace/observer release checks, both Clippy
   configurations, a separate default build and ordinary recovery pass. The
   server/client executable hashes reproduce the original qualified artifacts.
   Preserve the batch-write tail tradeoff and original experiment identity.
2. Investigate the batch-write tail with a separately frozen [common offered
   load](BATCH-WRITE-TAIL-NEXT.md) to distinguish increased saturation/work volume from implementation
   cost. This is a new hypothesis, not a rerun of the same closed-loop test to
   replace its result. Keep acknowledgement boundaries unchanged and measure
   operation-specific tails and outcome accounting.
3. Follow the [quorum-path investigation](QUORUM-LATENCY-NEXT.md) for isolated
   GET latency. Exact context/generation correlation must account for repeated
   heartbeat contexts; do not equate a received response with confirmed/applied
   read completion. Keep notification candidate `42e0117` separate.

Redis read parity, bounded storage, full implementation proofs and broader
industrial fault/host acceptance remain open before dynamic multi-Raft and
automatic splits. Only object storage may be a service-critical singleton.

## Immutable evidence

[Complete tables, receipts and verification bundle](https://github.com/c4pt0r/kv9/blob/02c5ebcf9371a3095a829ca6882adecef974dad4/docs/release-thin-lto-full72-v1/README.md)
retain the original statistical output, all repetitions and API rows, frozen
helpers, accepted audit inventories and original failures. This reporting bundle
does not back up the compressed WAL objects or runtime binaries; those remain
local under their accepted inventories.
