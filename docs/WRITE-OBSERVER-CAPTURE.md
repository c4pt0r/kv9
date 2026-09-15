# Write queue and receipt observations

The local observer sweep completed on 2026-09-15 UTC: 16 two-second cohorts,
matching default and diagnostic builds, Put and BatchPut(64), c1 and c64, and
two opposite execution orders. The selected CRC algorithm remains unchanged.
The complete ten-second [receipt performance screen](WRITE-RECEIPT-TAIL-PERFORMANCE.md)
remains the published performance baseline.

The clearest new observation is repeated unsuccessful receipt inspection.
In the instrumented c64 Put captures, **86.00–86.11% of lookups miss**, while
the actual linear scan averages **1,021.85–1,021.90 comparisons per lookup**.
Successful hits average only **11.26–11.47 slots behind the tail**. The
asynchronous queue is much smaller than the 1,024-entry receipt history.
These are measured logical events, not CPU instructions or predicted speedups.

## Observer throughput and latency cost

These short measurements compare the observer with its matching default build.
They are not a replacement candidate screen or a new Redis comparison. Rates
use summed successful work divided by summed actual elapsed time; means use
summed latency divided by summed calls. P99 bounds come from merged original
integer histograms, not averaged percentiles. Batch latency is per 64-item call.

| API / concurrency | Default throughput | Diagnostic throughput | Change | Default mean / p99 | Diagnostic mean / p99 |
| --- | ---: | ---: | ---: | --- | --- |
| Put / 1 | 19,563.101 calls/s | 19,392.597 calls/s | -0.872% | 51.012 / 65.536–66.559 us | 51.465 / 65.536–66.559 us |
| Put / 64 | 135,969.405 calls/s | 134,515.316 calls/s | -1.069% | 470.517 / 770.048–778.239 us | 475.608 / 778.240–786.431 us |
| BatchPut(64) / 1 | 387,026.904 items/s | 388,559.191 items/s | +0.396% | 165.253 / 210.944–212.991 us | 164.594 / 221.184–223.231 us |
| BatchPut(64) / 64 | 1,052,000.446 items/s | 1,045,534.240 items/s | -0.615% | 3.890 / 8.192–8.258 ms | 3.914 / 8.520–8.651 ms |

Put throughput is lower with diagnostics in both orders: -0.434%/-1.309%
at c1 and -0.776%/-1.364% at c64. Batch throughput changes direction:
-1.160%/+1.999% at c1 and -1.670%/+0.468% at c64. Batch p99 is worse with
diagnostics in both orders. The short shared-host observations do not establish
statistical confidence, negligible observer overhead or a causal explanation
for the held receipt candidate's batch regressions.

## Actual queue, lookup and group populations

The following ranges retain the two diagnostic repetitions separately. They
describe node 2, the leader at both endpoints of each capture. Endpoint status
does not prove uninterrupted leadership. Full results retain all three nodes.

| API / concurrency | Mean commands per successful apply group | Mean requests per nonempty service pass | Lookup miss fraction | Mean linear comparisons per lookup | Mean hit slots behind tail |
| --- | ---: | ---: | ---: | ---: | ---: |
| Put / 1 | 1 | 1 | 56.00–56.10% | 1,013.18–1,013.29 | 0 |
| Put / 64 | 10.968–11.116 | 27.25–27.61 | 86.00–86.11% | 1,021.85–1,021.90 | 11.26–11.47 |
| BatchPut(64) / 1 | 1 | 1 | 50.04–50.04% | 981.70–981.73 | 0 |
| BatchPut(64) / 64 | 15.133–15.162 | 36.58–36.83 | 84.08–85.41% | 1,008.69–1,010.08 | 11.84–12.23 |

Every c1 group and nonempty service pass is a singleton. At c64, singleton
groups account for 27.53–27.80% of Put groups and 13.37–14.19% of batch groups.
Group p99 lies in 32–63 commands; the observed leader maximum is 64. This does
not suggest that the existing 128-command group limit is the immediate bound.

Nonempty Ready persistence groups average 4.85–4.94 entries for c64 Put and
6.71–7.36 for c64 batch. Nonempty committed Ready groups average 10.97–11.12
and 15.13–15.16 respectively. Leader LightReady committed samples are zero in
these captures. Persistence grouping and application grouping are distinct.

Mean age at successful terminal inspection is 268.54–270.62 us for c64 Put and
2.727–2.791 ms for c64 batch. This clock starts at registration after proposal;
it excludes proposal submission and response delivery. It is not full RPC
latency, exclusive service time or time attributable to receipt search.

In these leader deltas, lookup misses equal requeued inspections and hits equal
resolved inspections. That observed equality is not a general API invariant:
synchronous receipt callers also exist. Only 89/95 loaded Put owner turns and
1/2 loaded batch turns resolve requests without applying a command in the same
turn. Late registration therefore does not appear to dominate this capture;
the table does not establish per-request causality.

All distributions span the before/after status endpoints, including
initialization, warmup, measurement, drains and verification within those
boundaries. They are **not measurement-only distributions**. Repeated pending
inspections count repeatedly. The [schema and safety mapping](WRITE-PATH-DIAGNOSTICS.md)
define cancellation, timeout, replacement, rejection and snapshot boundaries.

## Execution and independent validation

Both retained ThinLTO releases use clean source `9317e63` and the same compiler;
only `write-path-diagnostics` differs. Default binary SHA-256 is
`58385b184e3ccac4edd98664620a29cae33b5726bdebb11800ac3b12b7efaef6`, diagnostic
binary SHA-256 is `e8ec4396b9b419c1f3401884bc50fd15a331aa5830fc16c9130ca2513aa6602b`.
The unchanged native v3 client is revision `0be806d`, binary
`8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957`.
Build qualification checks all 1,102 server and 581 client source inputs and
actual Cargo feature graphs. The first build wrapper refused mixed verbose
Cargo output after compilation succeeded; its original failure is retained.
The corrected matched build exits `83432/e70fbd/0`.

Each cohort uses three fresh loopback voters, volatile tmpfs WAL, 4,096 keys plus
sentinel, 128-byte values, seed 71, 128 warmups, TonicStream, client CPUs 0–1
and voter CPUs 2–5. Existing Raft quorum, synchronization, application and reply
fences remain. This does not establish physical-disk durability, cross-host
availability, overload behavior or long-duration storage bounds.

All **1,417,714 measured calls / 12,742,027 input items** succeed in one attempt,
with no other measured outcomes or dropped slots. Each cohort has exactly one
successful initialization-read `not_leader` retry, outside warmup and timing.
Initialization writes, warmup, measurement and verification remain single
attempt. The original wrapper incorrectly rejected this first setup retry.
Its failed root remains failed; its completed ordinal 0 is reused unchanged.
A narrowly corrected continuation runs only ordinals 1–15 in the original order
and exits `7565/275fe1/0`, with exact container CPU restoration and no cleanup
errors. Eleven focused continuation controls pass `ba08cb/0`; two earlier
metadata-control failures remain preserved.

Native validation, final data readbacks, 48 fresh drains, source/feature and
process bindings pass. All 64 client/voter lifetimes exit. The independent
diagnostic checker passes once, `9465de/0`, validating all 24 instrumented
per-node deltas: schema, integer bounds, process continuity, monotonicity,
histograms, lookup identities, Ready populations, service conservation and
applied/resolved joint marginals. No failed-pump, canceled, expired or closed
branch appears in those deltas. These counters do not replace an independent
per-call linearizability history or actual Chaos acceptance for a promoted
optimization. The reviewed pooled extraction passes `08a7db/0`.

The original campaign baseline remains 28,108,746,752 available bytes across
the failed root and continuation. The fixed policy retains an 8 GiB host floor,
16 GiB maximum campaign decrease and 3 GiB payload cap per cohort. Checks run
at workload samples and phase/copy boundaries, not as a continuous global
allocation watchdog. Retained database file contents total 13,015,897,757 bytes;
each exact copy is hashed and read back after its writers exit. Post-capture
availability is 14,948,511,744 bytes before subsequent publication. Fresh checks
are required for future work; these observations reserve no capacity.

## Next implementation

The data supports investigating redundant pending inspections and the cost of
misses, alongside the held candidate's actual checked-hint and fallback paths.
Bind those path counts to a separate candidate observer; tail distance in this
baseline cannot be relabeled as a measured candidate hint hit. Separately, the
[conservative upper-bound experiment](WRITE-RECEIPT-UPPER-BOUND.md) now passes
source-bound refinement and local development checks. It rejects impossible
future-index lookups without caching pending outcomes. Its retained release,
ordinary recovery, actual Chaos and complete matched performance gates remain
open; actual upper-bound skip counts are not yet measured.

For a subsequent pending-inspection optimization, establish an explicit proof
of when a previous pending result remains valid. Any skipped lookup must account
for receipt publication and eviction, command and unified watermarks, fatal
state, replacement and all timeout/cancellation/stop branches. Preserve the
original logical deadline and exact `(term, index, outcome)` receipt. Inspect
again whenever relevant state changes; do not infer readiness from a timer or
acknowledge before durable quorum commit and local application.

The group observations do not justify adding batching delays or merely raising
the group limit. A changed algorithm still needs source-mapped proofs, ordinary
recovery, actual Chaos Mesh, and the original eight-smoke/sixteen-ten-second
throughput/latency screen. CRC remains selected. No original industrial roadmap
checkbox closes at this observation checkpoint. CI remains local.

[Original records and portable evidence](write-observer-capture-v1/README.md).
