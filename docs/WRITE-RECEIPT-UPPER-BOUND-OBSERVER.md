# Upper-bound receipt lookup: actual observer capture

The independent candidate `e2e23cca5e70a9ea0cc241877b3b35b5b6433d27`
completes all 16 two-second default/diagnostic cohorts and the schema-2 checks.
In these captures, every recorded lookup miss takes the conservative upper-bound
branch. At c64 Put, **87.973–88.013% of lookups skip the scan**, and actual linear
comparisons average **121.234–121.650 per lookup**. The filter is exercised by
the real workload; these counts do not measure CPU savings or predict speedup.

**CRC remains selected. The subsequent [8-smoke/16-ten-second matched comparison](WRITE-RECEIPT-UPPER-BOUND-PERFORMANCE.md)
has completed with a separately requalified client. It holds promotion because
of the loaded-batch tradeoff.** The observer figures below retain their original
client, scope and capture duration.
The [previous schema-1 observer](WRITE-OBSERVER-CAPTURE.md) and
[accepted ten-second performance baseline](WRITE-RECEIPT-TAIL-PERFORMANCE.md)
retain their separate source, workload and interpretation.

## Actual lookup and queue populations

Each range below preserves both repetitions, using node 2, which is leader at
both status endpoints. This does not prove uninterrupted leadership. Complete
results retain all 24 instrumented per-node deltas, including empty populations.

| API / concurrency | Lookups skipping the scan | Actual comparisons per lookup | Commands per apply group | Requests per nonempty service pass |
| --- | ---: | ---: | ---: | ---: |
| Put / 1 | 56.032–56.062% | 444.584–444.897 | 1 | 1 |
| Put / 64 | 87.973–88.013% | 121.234–121.650 | 10.688–10.717 | 29.272–29.435 |
| BatchPut(64) / 1 | 50.031–50.051% | 490.204–490.439 | 1 | 1 |
| BatchPut(64) / 64 | 85.993–86.262% | 136.939–139.623 | 14.562–14.752 | 36.319–36.558 |

Recorded misses equal recorded skips in every node delta. No skipped-empty-ring
sample appears in this capture; the schema and existing controls retain that
case. This observed equality is not a general guarantee: an arbitrary query
below the maximum can still miss after a fallback scan.

Successful lookups retain the original first-match scan. Because recorded
misses use zero comparisons here, the c64 leader totals imply approximately
1,011 comparisons per Put hit and 997 per batch hit. This identifies remaining
logical work, without assigning it a CPU-time share. The held tail-hint
candidate's actual hint/fallback counts and batch tradeoffs remain separate.

All distributions span the full before/after status interval, including setup
inside those endpoints, warmup, measurement, drains and final readback. Repeated
pending inspections count repeatedly. They are not measurement-only or
per-client causal traces. Summed ring lengths satisfy
`ring lengths = actual probes + hit tail slots + skipped ring lengths`;
the checker also verifies zero-probe/empty-ring overlap, histogram bounds,
monotonicity, exporter/process continuity, and service/Ready conservation.

## Same-candidate observer overhead

These measurements compare the candidate with its own diagnostics enabled.
They do not compare the candidate against selected CRC. Throughput uses summed
successful work over summed elapsed time; latency means use summed latency over
calls, and p99 comes from merged integer histograms. Batch latency is for the
complete 64-item call.

| API / concurrency | Default throughput | Diagnostic throughput | Change | Default mean / p99 | Diagnostic mean / p99 |
| --- | ---: | ---: | ---: | --- | --- |
| Put / 1 | 19,573.944 calls/s | 19,333.963 calls/s | -1.226% | 50.987 / 65.536–66.559 us | 51.622 / 65.536–66.559 us |
| Put / 64 | 140,529.368 calls/s | 139,144.969 calls/s | -0.985% | 455.251 / 737.280–745.471 us | 459.794 / 753.664–761.855 us |
| BatchPut(64) / 1 | 390,487.064 items/s | 385,370.414 items/s | -1.310% | 163.784 / 210.944–212.991 us | 165.954 / 219.136–221.183 us |
| BatchPut(64) / 64 | 1,061,675.640 items/s | 1,066,393.099 items/s | +0.444% | 3.855 / 8.651–8.782 ms | 3.838 / 7.406–7.471 ms |

Diagnostics reduce Put and c1 batch throughput in both execution orders. The
c64 batch throughput and p99 improve in both orders in this short capture.
These mixed effects do not establish negligible overhead, statistical
confidence or a candidate-versus-CRC speedup. No Redis reference was rerun.

## Execution, identity and capacity

Both server releases bind the same 1,116 clean candidate source files, compiler
and ThinLTO settings; only the diagnostic feature differs. Server SHA256 values:

- Default: `6f074e867eae45274c17b54aa763888e92db2c45d5757974fa52ee0f15936d49`.
- Diagnostic: `dbfb271302cc079c5fdd249a4c011a28aa759f4ad036b5e3707d446e078bc2e5`.
- Fixed native v3 client: `8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957`, revision `0be806d` / 581 source files.

The original protocol retains three loopback voters, tmpfs WAL, 4,096 keys plus
sentinel, 128-byte values, seed 71, 128 warmups, TonicStream, client CPUs 0–1 and
voter CPUs 2–5. All Raft quorum, synchronization, apply and reply fences remain.
It does not establish physical-disk durability or cross-host performance.

All **1,456,098 measured calls / 12,897,528 input items** succeed in one attempt,
with no dropped slots or other measured outcomes. Each row has one successful
typed initialization-read `not_leader` retry outside timing; initialization
writes, warmup, measurement and verification remain single attempt. All 48
fresh drains and 64 client/voter lifetime exits pass. Exact original container
CPU settings and historical cluster identities are restored.

The new campaign executes all rows 0–15 once. It uses the original full runner
with the already qualified initialization-read health check, not the previous
campaign's row-zero continuation. Original failed attempts remain preserved.
Actual role readback is `a2dd86/0`, runtime is `45414/21ff48/0`, completed metadata
freeze is `c389bf/0`, and independent schema-2 checking is `211df0/0`. The unchanged
timing extraction passes `dfca11/0`; new lookup extraction passes `8d4bda/0`.
Four new arithmetic/refusal controls precede the latter; old controls were not
replayed. These counters do not replace the candidate's separately accepted
[actual Chaos histories](WRITE-RECEIPT-UPPER-BOUND-CHAOS.md).

A finite lossless migration of original old-data cohorts 061–063 first restores
the observer launch allowance. [Its completed records](observer-capacity-completion-v1/README.md)
account for 954,626,048 bytes of conservative net recovery. Its actual terminal
is `46334/062ac3/0`; all
three cohorts complete exact reconstruction, original-path restoration, full
readback and final COLD. No 064 runs. The root's preceding missing-invocation
metadata failure remains recorded; no child ran during that failed preparation.

The observer's fresh baseline is **26,358,562,816 bytes** available. The unchanged
policy requires 24 GiB + 8 MiB initially, an 8 GiB floor, at most 16 GiB campaign
decrease and at most 3 GiB payload per cohort. Retained payload contents total
**13,176,171,178 bytes**, copied and hashed/read back after writers exit. Available
space after the completed metadata freeze is **13,043,941,376 bytes**, before
later reporting. These checks occur at workload and copy boundaries; they are
not a continuous global allocator. Every future phase needs fresh checks.

## Next work

The [portable metadata and original records](receipt-upper-bound-observer-v1/README.md)
retain complete per-node snapshots, native reports, source/feature bindings,
actual terminals and the independent analyses. Database payloads remain local.

The later full matched comparison passed its original capacity and acceptance
gates with output on the data volume. The [retained batch review](write-batch-tail-review-v1/README.md)
now motivates [bounded group/inspection timestamps](WRITE-STAGE-TRACE.md);
their actual capture and observer-overhead qualification remain next. The
successful observer capture itself establishes actual skip counts, not the
ten-second performance-selection decision. Preserve the held tail-hint experiment and
its unresolved batch tradeoffs. Continue the original storage, bounded
multi-Raft and split dependencies in issue #9. No original industrial roadmap
item closes from this diagnostic result, and no hosted CI was dispatched.
